use std::collections::HashMap;

use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use rand::Rng;
use uuid::Uuid;

use crate::crypto::aes::EncryptionKeys;
use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::{
    COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
};
use crate::models::provider_config::{COLLECTION_NAME as PROVIDER_CONFIGS, ProviderConfig};
use crate::models::ssh_auth_mode::SshAuthMode;
use crate::models::user_api_key::{COLLECTION_NAME as USER_API_KEYS, UserApiKey};
use crate::models::user_endpoint::UserEndpoint;
use crate::models::user_provider_token::{
    COLLECTION_NAME as USER_PROVIDER_TOKENS, UserProviderToken,
};
use crate::models::user_service::UserService;
use crate::models::ws_frame_injection::WsFrameInjection;
use crate::services::{
    node_service, ssh_service, user_api_key_service, user_endpoint_service, user_service_service,
    ws_frame_injector,
};

const AUTO_PROVISION_SOURCE: &str = "auto_provision";
const MAX_SERVICE_SLUG_LEN: usize = 80;
const HUMAN_SLUG_SUFFIX_MAX: u8 = 9;
const RANDOM_SLUG_SUFFIX_ATTEMPTS: usize = 5;
const RANDOM_SLUG_SUFFIX_LEN: usize = 4;
const USER_SERVICE_SLUG_INSERT_RETRIES: usize = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SlugCollisionStrategy {
    PreserveExact,
    AutoDisambiguate,
}

/// Generate a clean slug base from a label so the first saved service keeps
/// the label-derived slug without random noise.
fn generate_slug_from_label(label: &str) -> String {
    let base: String = label
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");

    let base = if base.is_empty() {
        "service".to_string()
    } else {
        base
    };

    let truncated: String = base.chars().take(MAX_SERVICE_SLUG_LEN).collect();
    let truncated = truncated.trim_end_matches('-');
    if truncated.is_empty() {
        "service".to_string()
    } else {
        truncated.to_string()
    }
}

fn random_slug_suffix() -> String {
    let mut rng = rand::thread_rng();
    (0..RANDOM_SLUG_SUFFIX_LEN)
        .map(|_| {
            let idx: u8 = rng.gen_range(0..36);
            if idx < 10 {
                (b'0' + idx) as char
            } else {
                (b'a' + idx - 10) as char
            }
        })
        .collect()
}

fn slug_candidate_with_suffix(base_slug: &str, suffix: &str) -> String {
    let max_base_len = MAX_SERVICE_SLUG_LEN.saturating_sub(suffix.len() + 1);
    let trimmed_base: String = base_slug.chars().take(max_base_len).collect();
    let trimmed_base = trimmed_base.trim_end_matches('-');
    let prefix = if trimmed_base.is_empty() {
        "service"
    } else {
        trimmed_base
    };
    format!("{prefix}-{suffix}")
}

/// Keep normal service slugs readable; only add numbering and random entropy
/// after the preferred slug is already taken.
async fn resolve_unique_slug(
    db: &mongodb::Database,
    user_id: &str,
    base_slug: &str,
    strategy: SlugCollisionStrategy,
) -> AppResult<String> {
    let base_slug = match strategy {
        SlugCollisionStrategy::PreserveExact => {
            user_service_service::validate_slug(base_slug)?;
            base_slug.to_string()
        }
        SlugCollisionStrategy::AutoDisambiguate => {
            // Legacy catalog slugs may violate the stricter current validator
            // (historic `--` allowed, 64->80 cap). Sanitize quietly so auto-
            // derived paths never fail on data that predates this validator.
            match user_service_service::validate_slug(base_slug) {
                Ok(()) => base_slug.to_string(),
                Err(_) => generate_slug_from_label(base_slug),
            }
        }
    };

    if user_service_service::find_by_slug(db, user_id, &base_slug)
        .await?
        .is_none()
    {
        return Ok(base_slug);
    }

    if strategy == SlugCollisionStrategy::PreserveExact {
        return Err(exact_slug_conflict(&base_slug));
    }

    for n in 2..=HUMAN_SLUG_SUFFIX_MAX {
        let candidate = slug_candidate_with_suffix(&base_slug, &n.to_string());
        if user_service_service::find_by_slug(db, user_id, &candidate)
            .await?
            .is_none()
        {
            return Ok(candidate);
        }
    }

    for _ in 0..RANDOM_SLUG_SUFFIX_ATTEMPTS {
        let candidate = slug_candidate_with_suffix(&base_slug, &random_slug_suffix());
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

fn exact_slug_conflict(slug: &str) -> AppError {
    AppError::Conflict(format!("Service slug '{slug}' is already in use"))
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

fn is_duplicate_slug_app_error(error: &AppError) -> bool {
    matches!(error, AppError::DatabaseError(db_error) if is_duplicate_key_error(db_error))
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
    pub ssh_auth_mode: crate::models::ssh_auth_mode::SshAuthMode,
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

/// User-provided OAuth Custom App credentials for `credential_mode: "user"`
/// providers (Lark / Feishu / Twitter). Three-state, mutually exclusive:
///
/// - `None`: caller did not supply BYO creds. For BYO providers this is
///   rejected upstream (see `create_key`); for other providers it's the
///   normal path.
/// - `Raw`: caller submitted a fresh `client_id` + `client_secret` pair.
///   `create_api_key` encrypts both onto the new `UserApiKey` row.
/// - `CopyFrom`: caller pointed at an existing `UserApiKey` to copy the
///   Custom App credentials from. Server-side decrypt-then-re-encrypt;
///   the client never sees or re-transmits the source secret. The source
///   must be owned by the same principal and must itself carry BYO creds
///   (legacy / provider-owned / credential-less placeholders are rejected).
pub enum OauthClientCredentialsInput<'a> {
    None,
    Raw {
        client_id: &'a str,
        client_secret: &'a str,
    },
    CopyFrom {
        source_key_id: &'a str,
    },
}

/// Resolve a `OauthClientCredentialsInput` into a concrete plaintext
/// `(client_id, client_secret)` pair suitable for passing into
/// `CreateApiKeyParams.oauth_client_id` / `oauth_client_secret`.
///
/// - `None` → `Ok(None)` (caller decides whether None is acceptable for
///   this provider).
/// - `Raw { client_id, client_secret }` → both trimmed and validated for
///   non-emptiness; returned as-is.
/// - `CopyFrom { source_key_id }` → loads the source `UserApiKey`,
///   verifies it is owned by `user_id` and carries
///   `user_oauth_client_id_encrypted` (so legacy / provider-owned /
///   credential-less placeholders are rejected), decrypts the pair, and
///   returns it. The plaintext stays in process memory only long enough
///   for the caller to encrypt it onto the new row.
async fn resolve_oauth_client_credentials_input(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    user_id: &str,
    input: &OauthClientCredentialsInput<'_>,
) -> AppResult<Option<(String, String)>> {
    use zeroize::Zeroizing;

    match input {
        OauthClientCredentialsInput::None => Ok(None),
        OauthClientCredentialsInput::Raw {
            client_id,
            client_secret,
        } => {
            let id = client_id.trim();
            let secret = client_secret.trim();
            if id.is_empty() || secret.is_empty() {
                return Err(AppError::ValidationError(
                    "oauth_client_id and oauth_client_secret must be non-empty when supplied"
                        .to_string(),
                ));
            }
            Ok(Some((id.to_string(), secret.to_string())))
        }
        OauthClientCredentialsInput::CopyFrom { source_key_id } => {
            let trimmed = source_key_id.trim();
            if trimmed.is_empty() {
                return Err(AppError::ValidationError(
                    "copy_oauth_client_from must not be empty".to_string(),
                ));
            }
            // Ownership check is the same `(_id, user_id)` predicate used
            // by `get_api_key`. We surface NotFound (rather than Forbidden)
            // so existence of a foreign key is not leaked through the
            // error type — R9 / §12 of the design doc.
            let source = db
                .collection::<UserApiKey>(USER_API_KEYS)
                .find_one(doc! { "_id": trimmed, "user_id": user_id })
                .await?
                .ok_or_else(|| {
                    AppError::NotFound("copy_oauth_client_from source key not found".to_string())
                })?;

            let enc_cid = source.user_oauth_client_id_encrypted.as_ref().ok_or_else(|| {
                AppError::BadRequest(
                    "Source key does not carry user-provided OAuth client credentials (it is legacy, provider-owned, or a credential-less placeholder)".to_string(),
                )
            })?;
            let enc_sec = source
                .user_oauth_client_secret_encrypted
                .as_ref()
                .ok_or_else(|| {
                    AppError::BadRequest(
                        "Source key is missing the OAuth client_secret half of its credentials"
                            .to_string(),
                    )
                })?;

            let dec_id = Zeroizing::new(encryption_keys.decrypt(enc_cid).await?);
            let id = String::from_utf8((*dec_id).clone()).map_err(|_| {
                AppError::Internal("Source key client_id is not valid UTF-8".to_string())
            })?;
            let dec_sec = Zeroizing::new(encryption_keys.decrypt(enc_sec).await?);
            let secret = String::from_utf8((*dec_sec).clone()).map_err(|_| {
                AppError::Internal("Source key client_secret is not valid UTF-8".to_string())
            })?;
            Ok(Some((id, secret)))
        }
    }
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
    pub ssh_auth_mode: SshAuthMode,
    pub ssh_node_keys_stale: bool,
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
    /// User-configured default request headers (NyxID#356). Returns only
    /// the user-owned entries; catalog-level admin defaults are surfaced
    /// separately on the catalog payload.
    pub default_request_headers:
        Option<Vec<crate::models::default_request_header::DefaultRequestHeader>>,
    /// User-configured WebSocket frame-auth injection rules. Empty means
    /// no user override; proxy resolution may still fall back to catalog
    /// rules for catalog-backed services.
    pub ws_frame_injections: Vec<WsFrameInjection>,
    pub auto_connected: bool,
    /// Developer app (OAuth client) ID that triggered this auto-provision.
    pub source_app_id: Option<String>,
    /// Human-readable name of the developer app (resolved from OauthClient).
    pub source_app_name: Option<String>,
    /// Per-add OAuth connection identifier. Present for multi-connection
    /// adds (oauth2 / device_code), `None` for legacy and non-OAuth keys.
    /// Surfaced so the UI can distinguish multiple connections to the
    /// same provider (e.g. two Lark Custom Apps) and so audit consumers
    /// can correlate `connection_id` from logs back to a visible key.
    pub connection_id: Option<String>,
    /// User-provided OAuth Custom App `client_id`, decrypted from
    /// `user_oauth_client_id_encrypted` when present. Returned for BYO
    /// providers (Lark / Feishu / Twitter) so the UI can show
    /// "App: cli_aaa…" and let users disambiguate connections without
    /// re-typing the credential. `None` otherwise. The client_secret is
    /// never surfaced (write-only across the API).
    pub oauth_client_id: Option<String>,
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

/// Compute the effective `(auth_method, auth_key_name)` for a catalog service.
///
/// For provider-delegated services, the `DownstreamService` itself stores
/// `auth_method = "none"` and `auth_key_name = ""` -- the real injection
/// method/key live on the `ServiceProviderRequirement`. Callers that need
/// to snapshot these onto a `UserService` or show them to the client must
/// derive the effective values here, matching
/// `catalog_service::build_catalog_entry`.
pub(crate) fn derive_effective_auth(
    svc: &DownstreamService,
    spr: Option<&crate::models::service_provider_requirement::ServiceProviderRequirement>,
) -> (String, String) {
    let auth_method = if svc.auth_method == "none" {
        spr.map(|r| r.injection_method.clone())
            .unwrap_or_else(|| svc.auth_method.clone())
    } else {
        svc.auth_method.clone()
    };
    let auth_key_name = if svc.auth_key_name.is_empty() {
        spr.and_then(|r| r.injection_key.clone())
            .unwrap_or_else(|| "Authorization".to_string())
    } else {
        svc.auth_key_name.clone()
    };
    (auth_method, auth_key_name)
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
    ws_frame_injections: Option<&[WsFrameInjection]>,
    oauth_client_credentials: OauthClientCredentialsInput<'_>,
    hosted_mode: bool,
) -> AppResult<CreateKeyResult> {
    let node_id = node_id.filter(|nid| !nid.is_empty());
    if let Some(rules) = ws_frame_injections {
        ws_frame_injector::validate_rules(rules)?;
    }

    if let Some(node_id) = node_id {
        node_service::ensure_node_writable_by_actor(db, actor_user_id, node_id).await?;
    }

    // BYO OAuth Custom App credentials. Resolved once up front so the
    // server-side copy lookup happens before any side effects. The
    // resulting plaintext pair (or `None`) is reused at every mint site
    // below. Plaintext stays in process memory only long enough to be
    // re-encrypted onto the new `UserApiKey` row.
    let byo_oauth_client_creds = resolve_oauth_client_credentials_input(
        db,
        encryption_keys,
        user_id,
        &oauth_client_credentials,
    )
    .await?;
    let byo_supplied = byo_oauth_client_creds.is_some();
    let byo_oauth_client_id = byo_oauth_client_creds.as_ref().map(|(id, _)| id.as_str());
    let byo_oauth_client_secret = byo_oauth_client_creds
        .as_ref()
        .map(|(_, secret)| secret.as_str());

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
        let provider_supports_byo = provider
            .as_ref()
            .is_some_and(crate::services::user_credentials_service::supports_user_credentials);
        let provider_requires_byo = provider
            .as_ref()
            .is_some_and(|p| p.credential_mode == "user");

        // BYO OAuth Custom App credentials are only meaningful for
        // providers with `credential_mode` in {"user", "both"}. If the
        // caller supplied them for an admin-only provider (or a
        // catalog service with no provider config), reject with the
        // same wording as `PUT /providers/{id}/credentials` (§16.2).
        if byo_supplied && !provider_supports_byo {
            return Err(AppError::BadRequest(
                "This provider does not accept user-provided OAuth client credentials".to_string(),
            ));
        }
        let provider_requirement = db
            .collection::<ServiceProviderRequirement>(SERVICE_PROVIDER_REQUIREMENTS)
            .find_one(doc! { "service_id": &svc.id })
            .await?;
        // Multi-connection: OAuth2 / device-code adds are ALWAYS
        // independent. We never reuse an existing provider token for
        // them — `create_key` mints a fresh `connection_id` below and
        // the wizard runs the full auth flow, so adding a second codex
        // / Lark service authorizes a separate account instead of
        // silently aliasing onto the first. Token reuse via
        // `find_existing_provider_token` stays ONLY for `api_key`-type
        // providers, which are out of scope for the multi-connection
        // work and keep their existing single-credential behavior.
        let existing_provider_token = if matches!(provider_type, Some("oauth2" | "device_code")) {
            None
        } else if let Some(provider_config_id) = svc.provider_config_id.as_deref() {
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

        if endpoint_url.is_some() && node_id.is_none() {
            crate::services::url_validation::validate_user_endpoint_url(
                &ep_url,
                hosted_mode,
                "endpoint_url",
            )
            .await?;
        }

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

        let requested_slug = match slug_override {
            Some(slug) if !slug.is_empty() => (slug, SlugCollisionStrategy::PreserveExact),
            _ => (svc.slug.as_str(), SlugCollisionStrategy::AutoDisambiguate),
        };

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
            label,
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
                // Multi-connection: a fresh pending OAuth/device-code add
                // gets its own `connection_id`. The wizard's OAuth-initiate
                // call threads this id into the `OAuthState`, and the
                // callback writes the resulting token straight onto THIS
                // `UserApiKey` (via `write_oauth_tokens_to_key`) — never
                // onto `user_provider_tokens`. That is what lets two codex
                // / Lark services coexist with independent tokens. Adds
                // that aren't pending OAuth (api_key providers, or an
                // OAuth add with an inline credential) stay connection-less
                // and follow the legacy path.
                let connection_id = pending_oauth.then(|| uuid::Uuid::new_v4().to_string());

                // BYO Custom App credentials are REQUIRED for multi-connection
                // `credential_mode: "user"` providers (Lark / Feishu / Twitter):
                // without them the authorize-URL + token-exchange + refresh
                // paths have no client_id to use, and the connection would
                // be unusable. We enforce the gate only when we are actually
                // minting a multi-connection placeholder (`pending_oauth`),
                // so non-OAuth adds aren't affected.
                if pending_oauth && provider_requires_byo && !byo_supplied {
                    return Err(AppError::BadRequest(
                        "This provider requires user-provided OAuth client credentials (oauth_client_id + oauth_client_secret, or copy_oauth_client_from an existing connection)".to_string(),
                    ));
                }
                // Only attach BYO creds to a multi-connection mint. A
                // legacy / inline-credential add on a "both" provider
                // shouldn't end up with BYO creds glued onto a row that
                // still resolves via `user_provider_credentials`.
                let (byo_id_for_key, byo_secret_for_key) = if pending_oauth {
                    (byo_oauth_client_id, byo_oauth_client_secret)
                } else {
                    (None, None)
                };
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
                            connection_id: connection_id.as_deref(),
                            oauth_client_id: byo_id_for_key,
                            oauth_client_secret: byo_secret_for_key,
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
                        connection_id: None,
                        oauth_client_id: None,
                        oauth_client_secret: None,
                        status: "active",
                        source: Some("user_created"),
                        source_id: None,
                    },
                )
                .await?,
            )
        };

        let catalog_identity =
            identity.unwrap_or_else(|| identity_config_from_downstream_service(&svc));

        // Snapshot the *effective* auth_method / auth_key_name onto the
        // UserService. The `DownstreamService` itself stores `auth_method
        // = "none"` for provider-delegated catalog entries (Anthropic,
        // OpenAI, Gemini, ...) and instead carries the real injection
        // config on the `ServiceProviderRequirement`. The proxy reads
        // `auth_method` directly off the UserService snapshot -- if we
        // snapshot the raw "none" we'd never inject the credential at
        // proxy time even though the user stored a valid `UserApiKey`.
        // Mirrors `catalog_service::build_catalog_entry` exactly so the
        // auth shape the frontend sees in the catalog equals what the
        // proxy actually applies.
        let (snap_auth_method, snap_auth_key_name) =
            derive_effective_auth(&svc, provider_requirement.as_ref());
        let endpoint_id = endpoint.id.clone();
        let api_key_id = api_key.as_ref().map(|k| k.id.clone());
        let catalog_service_id = svc.id.clone();
        let service_type = svc.service_type.clone();
        let ssh_auth_mode = if service_type == "ssh" {
            svc.ssh_config
                .as_ref()
                .map(|ssh| ssh.ssh_auth_mode)
                .unwrap_or_default()
        } else {
            SshAuthMode::ProxyOnly
        };
        let base_slug = requested_slug.0.to_string();
        let strategy = requested_slug.1;
        let retry_node_id = node_id.map(str::to_string);
        let mut attempts_left = USER_SERVICE_SLUG_INSERT_RETRIES;
        let service = loop {
            let resolved_slug = resolve_unique_slug(db, user_id, &base_slug, strategy).await?;
            match user_service_service::create_user_service(
                db,
                user_id,
                actor_user_id,
                &resolved_slug,
                &endpoint_id,
                api_key_id.as_deref(),
                &snap_auth_method,
                &snap_auth_key_name,
                Some(&catalog_service_id),
                retry_node_id.as_deref(),
                0,
                &service_type,
                ssh_auth_mode,
                None,
                None,
                None,
                &catalog_identity,
                ws_frame_injections,
            )
            .await
            {
                Ok(service) => break service,
                Err(error) if is_duplicate_slug_app_error(&error) => {
                    if attempts_left == 0 || strategy == SlugCollisionStrategy::PreserveExact {
                        return Err(exact_slug_conflict(&resolved_slug));
                    }
                    attempts_left -= 1;
                }
                Err(error) => return Err(error),
            }
        };

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

        let requested_slug = match slug_override {
            Some(slug) if !slug.is_empty() => {
                (slug.to_string(), SlugCollisionStrategy::PreserveExact)
            }
            _ => (
                generate_slug_from_label(label),
                SlugCollisionStrategy::AutoDisambiguate,
            ),
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
                ssh_auth_mode: Some(ssh.ssh_auth_mode),
                certificate_ttl_minutes: ssh.certificate_ttl_minutes,
                allowed_principals: &ssh.principals,
            },
        )
        .await?;

        let now = Utc::now();
        let base_url = ssh_service::target_base_url(&built_ssh_config.host, built_ssh_config.port);
        let empty_credential = encryption_keys.encrypt(b"").await?;
        let internal_ds_slug = format!("_ssh_{ds_id}");
        let ds = DownstreamService {
            id: ds_id.clone(),
            name: label.to_string(),
            // New SSH rows keep an internal UUID-derived backing slug so the
            // global `downstream_services.slug` index never blocks two users
            // from sharing the same visible `UserService.slug`. Legacy SSH
            // rows may still carry human-readable slugs until a later cleanup.
            slug: internal_ds_slug,
            description: None,
            base_url: base_url.clone(),
            service_type: "ssh".to_string(),
            visibility: "private".to_string(),
            auth_method: "none".to_string(),
            auth_type: Some("ssh".to_string()),
            auth_key_name: String::new(),
            credential_encrypted: empty_credential.clone(),
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
            default_request_headers: None,
            ws_frame_injections: Vec::new(),
            developer_app_ids: None,
            token_exchange_config: None,
            created_at: now,
            updated_at: now,
        };
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
                connection_id: None,
                oauth_client_id: None,
                oauth_client_secret: None,
                status: "active",
                source: Some("user_created"),
                source_id: None,
            },
        )
        .await?;
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(&ds)
            .await?;
        let endpoint_id = endpoint.id.clone();
        let api_key_id = api_key.id.clone();
        let base_slug = requested_slug.0.clone();
        let strategy = requested_slug.1;
        let retry_node_id = node_id.map(str::to_string);
        let mut attempts_left = USER_SERVICE_SLUG_INSERT_RETRIES;
        let service = loop {
            let resolved_slug = resolve_unique_slug(db, user_id, &base_slug, strategy).await?;
            match user_service_service::create_user_service(
                db,
                user_id,
                actor_user_id,
                &resolved_slug,
                &endpoint_id,
                Some(&api_key_id),
                "none",
                "",
                Some(&ds_id),
                retry_node_id.as_deref(),
                0,
                "ssh",
                built_ssh_config.ssh_auth_mode,
                None,
                None,
                None,
                &user_service_service::IdentityConfig::none(),
                ws_frame_injections,
            )
            .await
            {
                Ok(service) => break service,
                Err(error) if is_duplicate_slug_app_error(&error) => {
                    if attempts_left == 0 || strategy == SlugCollisionStrategy::PreserveExact {
                        return Err(exact_slug_conflict(&resolved_slug));
                    }
                    attempts_left -= 1;
                }
                Err(error) => return Err(error),
            }
        };

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
        // Skip URL validation for node-routed services: the URL is delivered
        // to the node agent and never used by NyxID's outbound HTTP client.
        if node_id.is_none() && !ep_url.is_empty() {
            crate::services::url_validation::validate_user_endpoint_url(
                ep_url,
                hosted_mode,
                "endpoint_url",
            )
            .await?;
        }

        let requested_slug = match slug_override {
            Some(slug) if !slug.is_empty() => {
                (slug.to_string(), SlugCollisionStrategy::PreserveExact)
            }
            _ => (
                generate_slug_from_label(label),
                SlugCollisionStrategy::AutoDisambiguate,
            ),
        };
        let am = auth_method.unwrap_or("bearer").to_string();
        let akn = auth_key_name.unwrap_or("Authorization").to_string();
        let is_no_auth = am == "none";

        if user_service_service::auth_method_requires_key_name(&am) && akn.trim().is_empty() {
            return Err(AppError::ValidationError(
                user_service_service::auth_key_name_required_message(&am),
            ));
        }

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
                        connection_id: None,
                        oauth_client_id: None,
                        oauth_client_secret: None,
                        status: "active",
                        source: Some("user_created"),
                        source_id: None,
                    },
                )
                .await?,
            )
        };

        let custom_identity = identity.unwrap_or_else(user_service_service::IdentityConfig::none);
        let endpoint_id = endpoint.id.clone();
        let api_key_id = api_key.as_ref().map(|k| k.id.clone());
        let base_slug = requested_slug.0.clone();
        let strategy = requested_slug.1;
        let retry_node_id = node_id.map(str::to_string);
        let mut attempts_left = USER_SERVICE_SLUG_INSERT_RETRIES;
        let service = loop {
            let resolved_slug = resolve_unique_slug(db, user_id, &base_slug, strategy).await?;
            match user_service_service::create_user_service(
                db,
                user_id,
                actor_user_id,
                &resolved_slug,
                &endpoint_id,
                api_key_id.as_deref(),
                &am,
                &akn,
                None,
                retry_node_id.as_deref(),
                0,
                "http",
                SshAuthMode::ProxyOnly,
                None,
                None,
                None,
                &custom_identity,
                ws_frame_injections,
            )
            .await
            {
                Ok(service) => break service,
                Err(error) if is_duplicate_slug_app_error(&error) => {
                    if attempts_left == 0 || strategy == SlugCollisionStrategy::PreserveExact {
                        return Err(exact_slug_conflict(&resolved_slug));
                    }
                    attempts_left -= 1;
                }
                Err(error) => return Err(error),
            }
        };

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

        let unique_slug = match resolve_unique_slug(
            db,
            user_id,
            &svc.slug,
            SlugCollisionStrategy::AutoDisambiguate,
        )
        .await
        {
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
            SshAuthMode::ProxyOnly,
            Some(AUTO_PROVISION_SOURCE),
            Some(&source_id),
            *source_app_id,
            &catalog_identity,
            None,
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
pub async fn list_keys(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    user_id: &str,
) -> AppResult<Vec<KeyView>> {
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

    let mut views: Vec<KeyView> = tagged
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

    // Enrich views with the (non-secret) BYO Custom App client_id where
    // present. Sequential await is fine — N is bounded by the user's
    // key count and decrypt is fast.
    for view in views.iter_mut() {
        let enc = view
            .api_key_id
            .as_deref()
            .and_then(|id| ak_map.get(id).copied())
            .and_then(|k| k.user_oauth_client_id_encrypted.as_ref());
        enrich_view_with_oauth_client_id(encryption_keys, view, enc).await;
    }

    Ok(views)
}

/// GET /api/v1/keys/:id -- get single combined view.
pub async fn get_key(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
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
    let mut view = build_key_view(
        &svc,
        &ep,
        ak.as_ref(),
        &cat_map,
        &app_name_map,
        user_service_service::CredentialSource::Personal,
    );

    enrich_view_with_oauth_client_id(
        encryption_keys,
        &mut view,
        ak.as_ref()
            .and_then(|k| k.user_oauth_client_id_encrypted.as_ref()),
    )
    .await;

    Ok(view)
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
        // Preserve a user-supplied server credential (NyxID#418 server-
        // held model). Routing is governed by `UserService.node_id` —
        // MCP's `classify_credential` treats node-routed services as
        // "node or nothing" regardless of the underlying
        // `credential_type`, so keeping the encrypted blob here is safe
        // and serves two purposes: (1) if a fire-and-forget WS push
        // failed to reach the node, the server still has the credential
        // for a retry on the next `PUT /keys` call; (2) rotation via
        // `update_api_key` works because the record stays on a direct
        // credential_type. Records that had no server credential to
        // begin with (e.g., created via `{node_id, auth_method: bearer}`
        // without a `credential`) still flip to `node_managed` so the
        // node agent remains the sole source of truth for those.
        //
        // Provider-backed keys (`provider_config_id.is_some()`) are an
        // important exception: `sync_provider_token_to_api_keys` and
        // `push_oauth_credential_to_nodes` walk provider-linked keys by
        // `credential_type != "node_managed"` on every OAuth refresh
        // and push the refreshed token to any node-routed services
        // using them. Leaving them as `oauth2` / `api_key` after a node
        // bind would let those refreshes copy provider secrets onto
        // the node, bypassing the "node-routed provider services must
        // be authorized on the node agent" contract. Flip them to
        // `node_managed` regardless of the server credential state so
        // the provider-refresh path filters them out. Twenty-seventh-
        // round Codex P1.
        let provider_backed = api_key.provider_config_id.is_some();
        if provider_backed || !user_api_key_service::has_server_credential(&api_key) {
            user_api_key_service::activate_node_managed_api_key(db, user_id, &api_key.id).await?;
        }
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

/// What PUT /keys should do with the credential / auth_method fields on a
/// given service. Derived purely from the (current service state, new field
/// values) pair so the decision logic is unit-testable without a database.
///
/// Closes NyxID#419: a service POSTed with `auth_method: "none"` could not
/// be upgraded to bearer/basic via PUT, because `reconcile_provider_key_
/// for_service_routing` short-circuits when `api_key_id` is missing and
/// `update_user_service` refuses to flip `auth_method` away from `"none"`
/// under the same condition. Both short-circuits are correct for the
/// "no api_key yet" state — the fix is to provision one first.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum UpdateCredentialAction<'a> {
    /// Nothing to do — service already in the target state, or caller
    /// didn't touch credential/auth_method.
    Nothing,
    /// Provision a new `UserApiKey` with this credential_type and
    /// credential value, then link it to the service.
    Provision {
        credential_type: &'static str,
        credential: &'a str,
    },
    /// Rotate the credential on the existing `UserApiKey` via
    /// `update_api_key` (direct credential types: bearer, basic, api_key,
    /// oauth2).
    Rotate { credential: &'a str },
    /// Existing `UserApiKey` is `node_managed`; caller is supplying a new
    /// credential to be stored server-side. Transition the record to
    /// `credential_type` and store the encrypted credential. Bypasses
    /// `update_api_key`'s node_managed rejection because this transition
    /// is an explicit opt-in to the NyxID#418 server-held model.
    Promote {
        credential_type: &'static str,
        credential: &'a str,
    },
    /// Caller's inputs are inconsistent — reject up front instead of
    /// letting the service-layer guards return a misleading error.
    Reject(&'static str),
}

/// Pure decision: classify what a PUT /keys request intends with respect
/// to credential state. The caller passes the current service's
/// `(auth_method, api_key_id, credential_type)` plus the optional new
/// values coming in from the request body; the enum tells the handler
/// whether to provision a new `UserApiKey`, rotate an existing one,
/// promote a node_managed record to hold a server credential, or pass
/// through. `current_credential_type` is `None` when the service has no
/// linked `UserApiKey` yet.
pub(crate) fn classify_update_credential_action<'a>(
    current_auth_method: &str,
    current_has_api_key: bool,
    current_credential_type: Option<&str>,
    new_auth_method: Option<&str>,
    new_credential: Option<&'a str>,
    effective_node_id_is_set: bool,
) -> UpdateCredentialAction<'a> {
    // Reject an explicit empty-string credential. The previous behavior
    // collapsed `Some("")` to `None`, which made blank rotations look
    // successful (no-op) and could even provision a `node_managed`
    // placeholder on upgrades from `auth_method: "none"` + `node_id`
    // without any real secret. `update_api_key` already rejects empty
    // values, but the classifier silenced them before they got that
    // far. Surface a clear rejection so UIs / automation that submit a
    // blank field don't get a 200 while the old credential stays in
    // effect (twenty-second-round Codex P2).
    if new_credential.is_some_and(|c| c.is_empty()) {
        return UpdateCredentialAction::Reject(
            "Credential must not be empty. Omit the field to leave the stored value \
             unchanged, or send a non-empty value to rotate it.",
        );
    }
    let credential = new_credential.filter(|c| !c.is_empty());
    let effective_auth_method = new_auth_method.unwrap_or(current_auth_method);
    let wants_credential_auth = effective_auth_method != "none";

    // Fast path: service already has a credential record.
    if current_has_api_key {
        return match credential {
            Some(value) => {
                // Services whose effective auth_method is "none" skip
                // credential injection entirely at proxy time, so a write
                // here would persist an unusable secret. Reject up front
                // (second Codex review P2) — a service that has a leftover
                // api_key_id after being downgraded to `auth_method: none`
                // must be re-upgraded before it can accept new credentials.
                if !wants_credential_auth {
                    return UpdateCredentialAction::Reject(
                        "Cannot store a credential while auth_method is 'none'. \
                         Set auth_method to bearer/basic/header/query first.",
                    );
                }
                // Node-managed records can't be rotated via `update_api_key`
                // (it refuses by design). Promote them to a direct type so
                // the server owns the credential going forward (NyxID#418).
                if current_credential_type == Some("node_managed") {
                    let target_type = match effective_auth_method {
                        "bearer" => "bearer",
                        "basic" => "basic",
                        _ => "api_key",
                    };
                    UpdateCredentialAction::Promote {
                        credential_type: target_type,
                        credential: value,
                    }
                } else {
                    UpdateCredentialAction::Rotate { credential: value }
                }
            }
            None => UpdateCredentialAction::Nothing,
        };
    }

    // Service has no api_key. Four sub-cases from here:
    // (a) caller isn't adding auth or credential — nothing to do
    // (b) caller set a credential while keeping auth_method=none — reject
    // (c) caller set auth_method != none without credential + no node — reject
    // (d) caller is upgrading: provision
    if !wants_credential_auth {
        return match credential {
            None => UpdateCredentialAction::Nothing,
            Some(_) => UpdateCredentialAction::Reject(
                "Cannot store a credential while auth_method is 'none'. \
                 Set auth_method to bearer/basic/header/query to enable credential storage.",
            ),
        };
    }

    // Direct (non-node) routing: credential is mandatory.
    if credential.is_none() && !effective_node_id_is_set {
        return UpdateCredentialAction::Reject(
            "Credential is required when upgrading auth_method for direct routing. \
             Either supply `credential` or bind a `node_id` first so the node agent \
             can inject the credential locally.",
        );
    }

    // Provision path. Credential_type derived from auth_method so the
    // credential plane matches what the direct-routing proxy path expects.
    let credential_type = match effective_auth_method {
        "bearer" => "bearer",
        "basic" => "basic",
        _ => "api_key",
    };

    match credential {
        Some(value) => UpdateCredentialAction::Provision {
            credential_type,
            credential: value,
        },
        None => {
            // node_id is set and no credential supplied — create a node_managed
            // record. Credential flows through `nyxid node credentials add`
            // locally, same as if the service had been created with
            // auth_method=bearer + node_id + no credential in the POST body.
            UpdateCredentialAction::Provision {
                credential_type: "node_managed",
                credential: "",
            }
        }
    }
}

/// Apply the upgrade/rotation decided by `classify_update_credential_action`.
/// Creates and links a new `UserApiKey` when transitioning from
/// `auth_method: "none"` to a credential-bearing method, or rotates the
/// existing credential. Returns the api_key_id now attached to the service
/// (if any), which the caller can use to trigger a node-side credential push.
#[allow(clippy::too_many_arguments)]
pub async fn ensure_user_api_key_for_update(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    user_id: &str,
    service_id: &str,
    new_auth_method: Option<&str>,
    new_credential: Option<&str>,
    new_node_id: Option<&str>,
    preferred_label: &str,
    oauth_client_credentials: OauthClientCredentialsInput<'_>,
) -> AppResult<Option<String>> {
    // BYO OAuth Custom App credentials. Resolved once up front (same
    // semantics as `create_key`) so the eventual placeholder mint
    // below can attach them, and so the source-key lookup happens
    // before any side effect.
    let byo_oauth_client_creds = resolve_oauth_client_credentials_input(
        db,
        encryption_keys,
        user_id,
        &oauth_client_credentials,
    )
    .await?;
    let byo_supplied = byo_oauth_client_creds.is_some();
    let byo_oauth_client_id = byo_oauth_client_creds.as_ref().map(|(id, _)| id.as_str());
    let byo_oauth_client_secret = byo_oauth_client_creds
        .as_ref()
        .map(|(_, secret)| secret.as_str());
    let service = user_service_service::get_user_service(db, user_id, service_id).await?;

    // Load credential_type for the classifier when an api_key is already
    // linked — the classifier needs it to distinguish node_managed from
    // direct types (promote vs rotate).
    let current_credential_type: Option<String> = if let Some(ref ak_id) = service.api_key_id {
        Some(
            user_api_key_service::get_api_key(db, user_id, ak_id)
                .await?
                .credential_type,
        )
    } else {
        None
    };

    // Effective node_id after the pending update: empty-string clears,
    // Some(nid) sets, None keeps the current value. Mirrors the mapping
    // used by `update_user_service`.
    //
    // Legacy `service.node_id == Some("")` is normalized to "no node"
    // here: some rows in the wild carry the empty string instead of
    // `None`, and the rest of the PUT flow already filters those out.
    // Without this, a `PUT /keys/:id {"auth_method":"bearer"}` on such
    // a row would classify as node-routed and provision a `node_managed`
    // key, while `update_user_service` + the strict push normalize back
    // to "no node" — leaving the service direct-routed with a
    // `node_managed` credential it can't actually use (sixteenth-round
    // Codex review P2).
    let effective_node_id_is_set = match new_node_id {
        Some("") => false,
        Some(_) => true,
        None => service.node_id.as_deref().is_some_and(|n| !n.is_empty()),
    };

    // Look up catalog metadata so the upgrade path honors provider-backed
    // services: (1) preserve `provider_config_id` so the new `UserApiKey`
    // stays in sync with `sync_provider_token_to_api_keys` / OAuth refresh
    // callbacks; (2) allow "pending_auth" upgrades where the credential is
    // deferred to the provider's OAuth / device-code flow, mirroring what
    // `create_key` does on POST. Seventh-round Codex review P2.
    let (catalog_service, provider_config, existing_provider_token): (
        Option<DownstreamService>,
        Option<ProviderConfig>,
        Option<UserProviderToken>,
    ) = if let Some(ref cat_id) = service.catalog_service_id {
        let ds = db
            .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .find_one(doc! { "_id": cat_id })
            .await?;
        let provider = match ds.as_ref().and_then(|d| d.provider_config_id.as_deref()) {
            Some(pid) => {
                db.collection::<ProviderConfig>(PROVIDER_CONFIGS)
                    .find_one(doc! { "_id": pid })
                    .await?
            }
            None => None,
        };
        // Multi-connection: never reuse an existing provider token when
        // upgrading a service to an OAuth2 / device-code provider — same
        // rule as `create_key`'s catalog POST path. Each upgrade-to-OAuth
        // mints a fresh `connection_id` (below) and runs the full auth
        // flow, so it authorizes its own account rather than aliasing
        // onto a sibling service's token. `api_key`-type providers keep
        // the existing reuse behavior (out of scope for multi-connection).
        let is_oauth_like = provider
            .as_ref()
            .is_some_and(|p| matches!(p.provider_type.as_str(), "oauth2" | "device_code"));
        let token = if is_oauth_like {
            None
        } else {
            match ds.as_ref().and_then(|d| d.provider_config_id.as_deref()) {
                Some(pid) => find_existing_provider_token(db, user_id, pid).await?,
                None => None,
            }
        };
        (ds, provider, token)
    } else {
        (None, None, None)
    };
    let provider_type = provider_config.as_ref().map(|p| p.provider_type.as_str());
    let deferred_auth_supported = matches!(provider_type, Some("oauth2" | "device_code"));
    let provider_supports_byo = provider_config
        .as_ref()
        .is_some_and(crate::services::user_credentials_service::supports_user_credentials);
    let provider_requires_byo = provider_config
        .as_ref()
        .is_some_and(|p| p.credential_mode == "user");

    // BYO compatibility gate, mirroring the POST path. We do not need to
    // reach the placeholder mint to surface a clear error.
    if byo_supplied && !provider_supports_byo {
        return Err(AppError::BadRequest(
            "This provider does not accept user-provided OAuth client credentials".to_string(),
        ));
    }

    let mut action = classify_update_credential_action(
        &service.auth_method,
        service.api_key_id.is_some(),
        current_credential_type.as_deref(),
        new_auth_method,
        new_credential,
        effective_node_id_is_set,
    );

    // Upgrade-by-rejection → deferred-auth Provision when the catalog
    // entry is backed by an OAuth / device-code provider. Matches the
    // `pending_oauth` branch inside `create_key`'s catalog path: server
    // stores a placeholder `oauth2` UserApiKey with status=pending_auth,
    // and the caller is expected to complete the provider OAuth flow
    // afterwards. The credential requirement the classifier enforces for
    // direct routing is correct for raw-secret auth methods but wrong
    // for provider-deferred flows.
    //
    // Guard: only trigger when the caller's requested `auth_method`
    // matches the catalog's declared auth_method. Without this check an
    // OAuth-backed catalog service could be PUT with `auth_method:
    // "basic"` (or any other method) and get silently upgraded via the
    // deferred branch; once authorization completes, the proxy would
    // inject the OAuth access token using the caller-supplied auth
    // method, leaving the service misconfigured. `create_key` never
    // accepts arbitrary auth-method overrides for catalog services, and
    // the PUT path shouldn't either (eighth-round Codex review P2).
    if let UpdateCredentialAction::Reject(msg) = action
        && msg.starts_with("Credential is required")
        && deferred_auth_supported
        && let Some(ref cat) = catalog_service
        && new_auth_method.is_some_and(|am| am == cat.auth_method)
    {
        action = UpdateCredentialAction::Provision {
            credential_type: "oauth2",
            credential: "",
        };
    }

    match action {
        UpdateCredentialAction::Nothing => Ok(service.api_key_id.clone()),
        UpdateCredentialAction::Reject(msg) => Err(AppError::BadRequest(msg.to_string())),
        UpdateCredentialAction::Rotate { credential } => {
            let ak_id = service
                .api_key_id
                .as_deref()
                .expect("Rotate requires an existing api_key");
            user_api_key_service::update_api_key(
                db,
                encryption_keys,
                user_id,
                ak_id,
                None,
                Some(credential),
            )
            .await?;
            Ok(service.api_key_id.clone())
        }
        UpdateCredentialAction::Promote {
            credential_type,
            credential,
        } => {
            let ak_id = service
                .api_key_id
                .as_deref()
                .expect("Promote requires an existing api_key");
            user_api_key_service::promote_node_managed_api_key(
                db,
                encryption_keys,
                user_id,
                ak_id,
                credential_type,
                credential,
            )
            .await?;
            Ok(service.api_key_id.clone())
        }
        UpdateCredentialAction::Provision {
            credential_type,
            credential,
        } => {
            // Use the caller-supplied label (current display label of the
            // service — either `UserEndpoint.label` on a previously no-auth
            // service, or the explicit `label` from this same PUT). Seeding
            // with `service.slug` here would silently rename the service in
            // GET responses because `build_key_view` prefers
            // `api_key.label` over `endpoint.label`. Falls back to the slug
            // when the caller passed an empty string.
            let trimmed_label = preferred_label.trim();
            let label = if trimmed_label.is_empty() {
                service.slug.as_str()
            } else {
                trimmed_label
            };

            // Preserve the catalog-declared provider linkage so OAuth /
            // device-code refreshes via `sync_provider_token_to_api_keys`
            // and `push_oauth_credential_to_nodes` continue to update
            // this service after the upgrade (seventh-round Codex review
            // P2). Dropping `provider_config_id` here silently turned the
            // service into an untracked manual credential.
            let catalog_provider_config_id = catalog_service
                .as_ref()
                .and_then(|ds| ds.provider_config_id.as_deref());

            // If the user already has an active provider token for this
            // provider, reuse it — same semantics as `create_key`. That
            // path attaches the existing encrypted material so the
            // upgrade is immediately active instead of forcing a fresh
            // OAuth handshake.
            //
            // Strict gating so the reuse is tied to the deferred-auth
            // pathway:
            //  - `credential_type == "oauth2"`: only the deferred-auth
            //    branch upstream selects this type (see the "Upgrade-
            //    by-rejection → deferred-auth Provision" block). A
            //    caller-requested direct type (`bearer`/`basic`/`api_key`)
            //    must NOT silently be replaced with an OAuth access token
            //    — the proxy would then inject that token using the
            //    wrong injection scheme (thirteenth-round Codex P2).
            //  - `credential.is_empty()`: a freshly supplied secret
            //    from the caller always wins over any existing provider
            //    token.
            //  - `credential_type != "node_managed"` stays implicit via
            //    the first condition — node_managed never appears as a
            //    direct type in Provision actions from this path.
            let api_key = if credential_type == "oauth2"
                && credential.is_empty()
                && let Some(ref provider_token) = existing_provider_token
                && let Some(pid) = catalog_provider_config_id
                && deferred_auth_supported
            {
                user_api_key_service::create_api_key_from_provider_token(
                    db,
                    user_id,
                    label,
                    pid,
                    provider_token,
                )
                .await?
            } else {
                // Pending OAuth / device-code state: store a placeholder
                // `oauth2` record with status=pending_auth so the
                // provider flow can populate it later. Matches the
                // `pending_oauth` branch inside `create_key`.
                let is_deferred_pending =
                    credential_type == "oauth2" && credential.is_empty() && deferred_auth_supported;
                let status = if is_deferred_pending {
                    "pending_auth"
                } else {
                    "active"
                };
                // Multi-connection: an upgrade-to-OAuth placeholder gets
                // its own `connection_id`, exactly like a fresh
                // `create_key` POST. The wizard's OAuth-initiate call
                // threads it into the `OAuthState`, and the callback
                // writes the token straight onto this `UserApiKey`.
                let connection_id = is_deferred_pending.then(|| uuid::Uuid::new_v4().to_string());

                // BYO requirement gate, mirroring `create_key`. A PUT
                // upgrade to OAuth on a `credential_mode: "user"`
                // provider must supply Custom App credentials.
                if is_deferred_pending && provider_requires_byo && !byo_supplied {
                    return Err(AppError::BadRequest(
                        "This provider requires user-provided OAuth client credentials (oauth_client_id + oauth_client_secret, or copy_oauth_client_from an existing connection)".to_string(),
                    ));
                }
                let (byo_id_for_key, byo_secret_for_key) = if is_deferred_pending {
                    (byo_oauth_client_id, byo_oauth_client_secret)
                } else {
                    (None, None)
                };
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
                        provider_config_id: catalog_provider_config_id,
                        connection_id: connection_id.as_deref(),
                        oauth_client_id: byo_id_for_key,
                        oauth_client_secret: byo_secret_for_key,
                        status,
                        source: Some("user_created"),
                        source_id: None,
                    },
                )
                .await?
            };

            // Compare-and-set link. If a concurrent upgrade PUT already
            // attached a different `UserApiKey` to this service,
            // `link_api_key` returns `Conflict` — in that case we MUST
            // reclaim the credential record we just provisioned so it
            // does not linger as an orphan under external key
            // management (twenty-ninth-round Codex P2). We delete the
            // record directly (it's the caller's just-created row, not
            // bound to any service yet, so `delete_api_key`'s
            // "no active service references" check will succeed).
            if let Err(e) =
                user_service_service::link_api_key(db, user_id, service_id, &api_key.id).await
            {
                if matches!(e, AppError::Conflict(_))
                    && let Err(cleanup_err) =
                        user_api_key_service::delete_api_key(db, user_id, &api_key.id).await
                {
                    tracing::error!(
                        user_id = %user_id,
                        api_key_id = %api_key.id,
                        error = %cleanup_err,
                        "failed to reclaim orphaned UserApiKey after concurrent upgrade race"
                    );
                }
                return Err(e);
            }
            Ok(Some(api_key.id))
        }
    }
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
    let api_key_provider_config_id = if let Some(ref ak_id) = svc.api_key_id {
        user_api_key_service::get_api_key(db, user_id, ak_id)
            .await?
            .provider_config_id
    } else {
        None
    };

    user_service_service::deactivate_user_service(db, user_id, actor_user_id, service_id).await?;
    if let Some(ref ak_id) = svc.api_key_id {
        user_api_key_service::delete_api_key(db, user_id, ak_id).await?;
        revoke_provider_token_if_unused(db, user_id, api_key_provider_config_id.as_deref()).await?;
    }
    user_endpoint_service::delete_endpoint(db, user_id, &svc.endpoint_id).await?;

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

/// Atomic variant of `revoke_key` used by the browser `only_if_pending`
/// cleanup path. The destructive step is gated by a single
/// `update_one` that includes `status == pending_auth` in its filter,
/// so a concurrent OAuth/device-code callback flipping the key to
/// `active` cannot be followed by a revoke. On a lost race we
/// short-circuit before touching the `UserService` or node binding.
///
/// Returns `Ok(true)` when the revoke actually happened, `Ok(false)`
/// when the credential was no longer `pending_auth` and cleanup was
/// skipped.
pub async fn revoke_key_if_pending(
    db: &mongodb::Database,
    user_id: &str,
    actor_user_id: &str,
    service_id: &str,
) -> AppResult<bool> {
    let svc = user_service_service::get_user_service(db, user_id, service_id).await?;

    // A UserService with no api_key_id can't be in `pending_auth`
    // (only credential-backed flows have that status) — report it
    // as "not pending" so the handler leaves the key alone. The
    // unconditional DELETE path still works for those, users just
    // don't get the race-free cleanup semantics they didn't need.
    let Some(ak_id) = svc.api_key_id.as_deref() else {
        return Ok(false);
    };
    let api_key_provider_config_id = user_api_key_service::get_api_key(db, user_id, ak_id)
        .await?
        .provider_config_id;

    // Atomic gate: flips pending_auth -> revoked in one write. If
    // the provider callback already flipped to `active`, the filter
    // misses and we report `false` without touching anything else.
    let flipped = user_api_key_service::revoke_api_key_if_pending(db, user_id, ak_id).await?;
    if !flipped {
        return Ok(false);
    }
    revoke_provider_token_if_unused(db, user_id, api_key_provider_config_id.as_deref()).await?;

    // API key was in pending_auth and is now revoked (by the atomic
    // status-filter update above). Tear down the owning UserService
    // and any node binding to keep the records consistent.
    user_service_service::deactivate_user_service(db, user_id, actor_user_id, service_id).await?;
    node_service::sync_node_binding_for_user_service(
        db,
        user_id,
        actor_user_id,
        svc.catalog_service_id.as_deref(),
        None,
        svc.node_id.as_deref(),
    )
    .await?;

    Ok(true)
}

async fn revoke_provider_token_if_unused(
    db: &mongodb::Database,
    user_id: &str,
    provider_config_id: Option<&str>,
) -> AppResult<()> {
    let Some(provider_config_id) = provider_config_id else {
        return Ok(());
    };

    // Node-managed keys store credentials on the node agent and do not reference
    // the central provider token; match `sync_provider_token_to_api_keys` skip logic.
    let remaining_key_count = db
        .collection::<UserApiKey>(USER_API_KEYS)
        .count_documents(doc! {
            "user_id": user_id,
            "provider_config_id": provider_config_id,
            "status": { "$nin": ["revoked", "failed"] },
            "credential_type": { "$ne": "node_managed" },
        })
        .await?;

    if remaining_key_count > 0 {
        return Ok(());
    }

    db.collection::<UserProviderToken>(USER_PROVIDER_TOKENS)
        .update_many(
            doc! {
                "user_id": user_id,
                "provider_config_id": provider_config_id,
                "status": { "$ne": "revoked" },
            },
            doc! {
                "$set": {
                    "status": "revoked",
                    "updated_at": bson::DateTime::from_chrono(Utc::now()),
                }
            },
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
        ssh_auth_mode: svc.ssh_auth_mode,
        ssh_node_keys_stale: svc.ssh_node_keys_stale,
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
        default_request_headers: svc.default_request_headers.clone(),
        ws_frame_injections: svc.ws_frame_injections.clone(),
        auto_connected,
        source_app_id: svc.source_app_id.clone(),
        source_app_name,
        // Multi-connection identifier — None for legacy / non-OAuth keys,
        // a UUID for fresh oauth2 / device_code adds. Set straight from
        // the api_key (plaintext, no decrypt needed).
        connection_id: ak.and_then(|k| k.connection_id.clone()),
        // `oauth_client_id` is decrypted in a follow-up async pass
        // (see `enrich_view_with_oauth_client_id`) because the
        // `EncryptionKeys` operations are async and `build_key_view`
        // is intentionally sync.
        oauth_client_id: None,
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

/// Async post-pass for `build_key_view`: decrypt
/// `UserApiKey.user_oauth_client_id_encrypted` (BYO Custom App client_id)
/// and place the plaintext on `view.oauth_client_id`.
///
/// Best-effort: on decrypt or UTF-8 failure we log and leave the field
/// `None` rather than failing the whole list response. The client_id is
/// non-secret (it appears in OAuth redirect URLs); the secret half is
/// never surfaced.
async fn enrich_view_with_oauth_client_id(
    encryption_keys: &EncryptionKeys,
    view: &mut KeyView,
    encrypted_client_id: Option<&Vec<u8>>,
) {
    let Some(enc) = encrypted_client_id else {
        return;
    };
    match encryption_keys.decrypt(enc).await {
        Ok(plain) => match String::from_utf8(plain) {
            Ok(s) => view.oauth_client_id = Some(s),
            Err(e) => tracing::warn!(
                error = %e,
                view_id = %view.id,
                "Failed to decode oauth_client_id as UTF-8; surfacing as None"
            ),
        },
        Err(e) => tracing::warn!(
            error = %e,
            view_id = %view.id,
            "Failed to decrypt oauth_client_id for key view; surfacing as None"
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use chrono::Utc;
    use mongodb::bson::doc;

    use super::{
        AUTO_PROVISION_SOURCE, OauthClientCredentialsInput, OpenApiSpecUrlInput,
        SlugCollisionStrategy, SshCreateParams, UpdateCredentialAction, auto_provision_source_id,
        build_key_view, classify_update_credential_action, create_key, derive_effective_auth,
        direct_credential_type_for_service, direct_credential_type_from_auth_method,
        generate_slug_from_label, identity_config_from_downstream_service,
        resolve_openapi_spec_url, resolve_unique_slug, revoke_key,
        validate_token_exchange_catalog_credential,
    };
    use crate::errors::{AppError, AppResult};
    use crate::models::downstream_service::{
        COLLECTION_NAME as DOWNSTREAM_SERVICES, CredentialFieldSpec, DownstreamService,
        TokenExchangeConfig,
    };
    use crate::models::node::{COLLECTION_NAME as NODES, Node, NodeMetrics, NodeStatus};
    use crate::models::provider_config::{COLLECTION_NAME as PROVIDER_CONFIGS, ProviderConfig};
    use crate::models::service_provider_requirement::ServiceProviderRequirement;
    use crate::models::ssh_auth_mode::SshAuthMode;
    use crate::models::user_api_key::COLLECTION_NAME as USER_API_KEYS;
    use crate::models::user_api_key::UserApiKey;
    use crate::models::user_endpoint::COLLECTION_NAME as USER_ENDPOINTS;
    use crate::models::user_endpoint::UserEndpoint;
    use crate::models::user_provider_token::{
        COLLECTION_NAME as USER_PROVIDER_TOKENS, UserProviderToken,
    };
    use crate::models::user_service::COLLECTION_NAME as USER_SERVICES;
    use crate::models::user_service::UserService;
    use crate::services::user_service_service::validate_slug;
    use crate::test_utils::{connect_test_database, test_encryption_keys};

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
            connection_id: None,
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
            ssh_auth_mode: SshAuthMode::ProxyOnly,
            ssh_node_keys_stale: false,
            identity_propagation_mode: "none".to_string(),
            identity_include_user_id: false,
            identity_include_email: false,
            identity_include_name: false,
            identity_jwt_audience: None,
            forward_access_token: false,
            inject_delegation_token: false,
            delegation_token_scope: "llm:proxy".to_string(),
            custom_user_agent: None,
            default_request_headers: None,
            ws_frame_injections: Vec::new(),
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

    async fn insert_provider_token(
        db: &mongodb::Database,
        user_id: &str,
        provider_id: &str,
    ) -> String {
        let now = Utc::now();
        let token_id = uuid::Uuid::new_v4().to_string();
        db.collection::<UserProviderToken>(USER_PROVIDER_TOKENS)
            .insert_one(UserProviderToken {
                id: token_id.clone(),
                user_id: user_id.to_string(),
                provider_config_id: provider_id.to_string(),
                connection_id: None,
                credential_user_id: None,
                token_type: "oauth2".to_string(),
                access_token_encrypted: Some(vec![1, 2, 3]),
                refresh_token_encrypted: Some(vec![4, 5, 6]),
                token_scopes: None,
                expires_at: None,
                api_key_encrypted: None,
                status: "active".to_string(),
                last_refreshed_at: None,
                last_used_at: None,
                error_message: None,
                label: None,
                metadata: None,
                gateway_url: None,
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();
        token_id
    }

    async fn insert_provider_backed_service(
        db: &mongodb::Database,
        user_id: &str,
        provider_id: &str,
        service_id: &str,
        credential_type: &str,
    ) {
        let endpoint_id = format!("ep-{service_id}");
        let api_key_id = format!("key-{service_id}");

        let mut endpoint = sample_endpoint();
        endpoint.id = endpoint_id.clone();
        endpoint.user_id = user_id.to_string();
        db.collection::<UserEndpoint>(USER_ENDPOINTS)
            .insert_one(endpoint)
            .await
            .unwrap();

        let mut api_key = sample_api_key(credential_type);
        api_key.id = api_key_id.clone();
        api_key.user_id = user_id.to_string();
        api_key.provider_config_id = Some(provider_id.to_string());
        db.collection::<UserApiKey>(USER_API_KEYS)
            .insert_one(api_key)
            .await
            .unwrap();

        let mut service = sample_service("bearer");
        service.id = service_id.to_string();
        service.user_id = user_id.to_string();
        service.slug = service_id.to_string();
        service.endpoint_id = endpoint_id;
        service.api_key_id = Some(api_key_id);
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(service)
            .await
            .unwrap();
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
            default_request_headers: None,
            ws_frame_injections: Vec::new(),
            developer_app_ids: None,
            token_exchange_config: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn sample_spr(
        injection_method: &str,
        injection_key: Option<&str>,
    ) -> ServiceProviderRequirement {
        ServiceProviderRequirement {
            id: "spr-1".to_string(),
            service_id: "cat-1".to_string(),
            provider_config_id: "prov-1".to_string(),
            required: true,
            scopes: None,
            injection_method: injection_method.to_string(),
            injection_key: injection_key.map(String::from),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    async fn insert_user_service_slug(db: &mongodb::Database, user_id: &str, slug: &str) {
        let mut service = sample_service("bearer");
        service.id = uuid::Uuid::new_v4().to_string();
        service.user_id = user_id.to_string();
        service.slug = slug.to_string();

        db.collection::<UserService>(USER_SERVICES)
            .insert_one(&service)
            .await
            .unwrap();
    }

    async fn insert_active_node(db: &mongodb::Database, user_id: &str, node_id: &str) {
        let now = Utc::now();
        let node = Node {
            id: node_id.to_string(),
            user_id: user_id.to_string(),
            name: format!("node-{node_id}"),
            status: NodeStatus::Online,
            auth_token_hash: "deadbeef".repeat(8),
            signing_secret_encrypted: None,
            signing_secret_hash: "feedface".repeat(8),
            last_heartbeat_at: Some(now),
            connected_at: Some(now),
            metadata: None,
            metrics: NodeMetrics::default(),
            is_active: true,
            created_at: now,
            updated_at: now,
        };

        db.collection::<Node>(NODES)
            .insert_one(&node)
            .await
            .unwrap();
    }

    #[test]
    fn generate_slug_from_label_is_deterministic() {
        let label = "My Cool Service 2024!!!";

        assert_eq!(generate_slug_from_label(label), "my-cool-service-2024");
        assert_eq!(
            generate_slug_from_label(label),
            generate_slug_from_label(label),
        );
    }

    #[test]
    fn generate_slug_from_label_falls_back_to_service_for_empty_inputs() {
        for label in ["", "   \t\n", "你好世界", "🔥✨"] {
            assert_eq!(generate_slug_from_label(label), "service");
        }
    }

    #[test]
    fn generate_slug_from_label_truncates_to_eighty_characters() {
        let label = "a".repeat(120);
        let slug = generate_slug_from_label(&label);

        assert_eq!(slug.len(), 80);
        assert_eq!(slug, "a".repeat(80));
    }

    #[test]
    fn derive_effective_auth_uses_spr_when_svc_is_none() {
        // Anthropic-style catalog shape: the DownstreamService stores `none`
        // and the real injection config lives on the SPR. The effective
        // tuple must come from the SPR or the proxy won't inject the
        // caller's credential.
        let mut svc = sample_catalog_service();
        svc.auth_method = "none".to_string();
        svc.auth_key_name = "".to_string();

        let spr = sample_spr("header", Some("x-api-key"));
        let (method, key) = derive_effective_auth(&svc, Some(&spr));

        assert_eq!(method, "header");
        assert_eq!(key, "x-api-key");
    }

    #[test]
    fn derive_effective_auth_preserves_non_none_svc_fields() {
        // If the catalog already carries explicit auth config, the SPR
        // does not override. Avoids double-derivation for services that
        // don't use the provider-delegated pattern.
        let mut svc = sample_catalog_service();
        svc.auth_method = "bearer".to_string();
        svc.auth_key_name = "Authorization".to_string();

        let spr = sample_spr("header", Some("x-api-key"));
        let (method, key) = derive_effective_auth(&svc, Some(&spr));

        assert_eq!(method, "bearer");
        assert_eq!(key, "Authorization");
    }

    #[test]
    fn derive_effective_auth_falls_back_to_none_when_no_spr() {
        let mut svc = sample_catalog_service();
        svc.auth_method = "none".to_string();
        svc.auth_key_name = "".to_string();

        let (method, key) = derive_effective_auth(&svc, None);

        assert_eq!(method, "none");
        // No SPR, empty catalog -> Authorization is the safe default the
        // build_catalog_entry logic also picks.
        assert_eq!(key, "Authorization");
    }

    #[test]
    fn derive_effective_auth_defaults_key_when_spr_has_no_injection_key() {
        let mut svc = sample_catalog_service();
        svc.auth_method = "none".to_string();
        svc.auth_key_name = "".to_string();

        let spr = sample_spr("bearer", None);
        let (method, key) = derive_effective_auth(&svc, Some(&spr));

        assert_eq!(method, "bearer");
        assert_eq!(key, "Authorization");
    }

    #[tokio::test]
    async fn resolve_unique_slug_keeps_base_when_available() {
        let Some(db) = connect_test_database("unified_key_slug").await else {
            eprintln!("skipping unified_key_service integration test: no local MongoDB available");
            return;
        };

        let user_id = uuid::Uuid::new_v4().to_string();

        let resolved = resolve_unique_slug(
            &db,
            &user_id,
            "clean-service",
            SlugCollisionStrategy::AutoDisambiguate,
        )
        .await
        .unwrap();

        assert_eq!(resolved, "clean-service");
    }

    #[tokio::test]
    async fn resolve_unique_slug_uses_small_numeric_suffixes_before_randomness() {
        let Some(db) = connect_test_database("unified_key_slug").await else {
            eprintln!("skipping unified_key_service integration test: no local MongoDB available");
            return;
        };

        let user_id = uuid::Uuid::new_v4().to_string();
        insert_user_service_slug(&db, &user_id, "clean-service").await;
        insert_user_service_slug(&db, &user_id, "clean-service-2").await;

        let resolved = resolve_unique_slug(
            &db,
            &user_id,
            "clean-service",
            SlugCollisionStrategy::AutoDisambiguate,
        )
        .await
        .unwrap();

        assert_eq!(resolved, "clean-service-3");
    }

    #[tokio::test]
    async fn resolve_unique_slug_falls_back_to_random_suffix_after_nine() {
        let Some(db) = connect_test_database("unified_key_slug").await else {
            eprintln!("skipping unified_key_service integration test: no local MongoDB available");
            return;
        };

        let user_id = uuid::Uuid::new_v4().to_string();
        insert_user_service_slug(&db, &user_id, "clean-service").await;
        for suffix in 2..=9 {
            insert_user_service_slug(&db, &user_id, &format!("clean-service-{suffix}")).await;
        }

        let resolved = resolve_unique_slug(
            &db,
            &user_id,
            "clean-service",
            SlugCollisionStrategy::AutoDisambiguate,
        )
        .await
        .unwrap();

        let random_suffix = resolved
            .strip_prefix("clean-service-")
            .expect("random suffix should preserve the base slug");
        assert_eq!(random_suffix.len(), 4);
        assert!(
            random_suffix
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        );
        assert!(resolved.len() <= 80);
    }

    #[tokio::test]
    async fn resolve_unique_slug_rejects_taken_user_supplied_slug() {
        let Some(db) = connect_test_database("unified_key_slug").await else {
            eprintln!("skipping unified_key_service integration test: no local MongoDB available");
            return;
        };

        let user_id = uuid::Uuid::new_v4().to_string();
        insert_user_service_slug(&db, &user_id, "clean-service").await;

        let err = resolve_unique_slug(
            &db,
            &user_id,
            "clean-service",
            SlugCollisionStrategy::PreserveExact,
        )
        .await
        .expect_err("duplicate user-provided slug should conflict");

        assert!(matches!(
            err,
            AppError::Conflict(message) if message == "Service slug 'clean-service' is already in use"
        ));
    }

    #[tokio::test]
    async fn resolve_unique_slug_sanitizes_legacy_base_for_auto_disambiguate() {
        let Some(db) = connect_test_database("unified_key_slug").await else {
            eprintln!("skipping unified_key_service integration test: no local MongoDB available");
            return;
        };

        let user_id = uuid::Uuid::new_v4().to_string();

        let resolved = resolve_unique_slug(
            &db,
            &user_id,
            "legacy--slug",
            SlugCollisionStrategy::AutoDisambiguate,
        )
        .await
        .unwrap();

        validate_slug(&resolved).expect("sanitized slug should validate");
        assert!(!resolved.contains("--"));
    }

    #[tokio::test]
    async fn resolve_unique_slug_rejects_invalid_base_for_preserve_exact() {
        let Some(db) = connect_test_database("unified_key_slug").await else {
            eprintln!("skipping unified_key_service integration test: no local MongoDB available");
            return;
        };

        let user_id = uuid::Uuid::new_v4().to_string();

        let err = resolve_unique_slug(
            &db,
            &user_id,
            "bad--slug",
            SlugCollisionStrategy::PreserveExact,
        )
        .await
        .expect_err("invalid exact slug should fail validation");

        assert!(matches!(err, AppError::ValidationError(_)));
    }

    #[tokio::test]
    async fn create_key_recovers_from_concurrent_service_slug_race() {
        let Some(db) = connect_test_database("unified_key_slug_race").await else {
            eprintln!("skipping unified_key_service integration test: no local MongoDB available");
            return;
        };

        let encryption_keys = test_encryption_keys();
        let user_id = uuid::Uuid::new_v4().to_string();
        let (left, right) = tokio::join!(
            create_key(
                &db,
                &encryption_keys,
                &user_id,
                &user_id,
                None,
                Some("https://api.example.com"),
                "secret-token",
                "Race Service",
                None,
                Some("bearer"),
                Some("Authorization"),
                None,
                None,
                None,
                OpenApiSpecUrlInput::Inherit,
                None,
                OauthClientCredentialsInput::None,
                false,
            ),
            create_key(
                &db,
                &encryption_keys,
                &user_id,
                &user_id,
                None,
                Some("https://api.example.com"),
                "secret-token",
                "Race Service",
                None,
                Some("bearer"),
                Some("Authorization"),
                None,
                None,
                None,
                OpenApiSpecUrlInput::Inherit,
                None,
                OauthClientCredentialsInput::None,
                false,
            )
        );

        let left = left.expect("left create should succeed");
        let right = right.expect("right create should succeed");

        assert_ne!(left.service.slug, right.service.slug);
        validate_slug(&left.service.slug).expect("left slug should validate");
        validate_slug(&right.service.slug).expect("right slug should validate");
    }

    #[tokio::test]
    async fn create_key_allows_same_ssh_label_for_different_users() {
        let Some(db) = connect_test_database("unified_key_ssh_slug_scope").await else {
            eprintln!("skipping unified_key_service integration test: no local MongoDB available");
            return;
        };

        let encryption_keys = test_encryption_keys();
        let user_a = uuid::Uuid::new_v4().to_string();
        let user_b = uuid::Uuid::new_v4().to_string();
        let node_a = uuid::Uuid::new_v4().to_string();
        let node_b = uuid::Uuid::new_v4().to_string();
        insert_active_node(&db, &user_a, &node_a).await;
        insert_active_node(&db, &user_b, &node_b).await;

        let created_a = create_key(
            &db,
            &encryption_keys,
            &user_a,
            &user_a,
            None,
            None,
            "",
            "Shared Label",
            None,
            None,
            None,
            Some(&node_a),
            Some(SshCreateParams {
                host: "server-a.example.com",
                port: 22,
                certificate_auth: true,
                ssh_auth_mode: crate::models::ssh_auth_mode::SshAuthMode::Cert,
                principals: vec!["ubuntu".to_string()],
                certificate_ttl_minutes: 60,
            }),
            None,
            OpenApiSpecUrlInput::Inherit,
            None,
            OauthClientCredentialsInput::None,
            false,
        )
        .await
        .expect("user A SSH create should succeed");

        let created_b = create_key(
            &db,
            &encryption_keys,
            &user_b,
            &user_b,
            None,
            None,
            "",
            "Shared Label",
            None,
            None,
            None,
            Some(&node_b),
            Some(SshCreateParams {
                host: "server-b.example.com",
                port: 22,
                certificate_auth: true,
                ssh_auth_mode: crate::models::ssh_auth_mode::SshAuthMode::Cert,
                principals: vec!["ubuntu".to_string()],
                certificate_ttl_minutes: 60,
            }),
            None,
            OpenApiSpecUrlInput::Inherit,
            None,
            OauthClientCredentialsInput::None,
            false,
        )
        .await
        .expect("user B SSH create should succeed");

        assert_eq!(created_a.service.slug, "shared-label");
        assert_eq!(created_b.service.slug, "shared-label");
        validate_slug(&created_a.service.slug).expect("user A slug should validate");
        validate_slug(&created_b.service.slug).expect("user B slug should validate");

        let ds_a_id = created_a
            .service
            .catalog_service_id
            .as_deref()
            .expect("SSH service should keep a backing downstream_service id");
        let ds_b_id = created_b
            .service
            .catalog_service_id
            .as_deref()
            .expect("SSH service should keep a backing downstream_service id");
        let ds_a = db
            .collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
            .find_one(doc! { "_id": ds_a_id })
            .await
            .unwrap()
            .expect("user A downstream_service should exist");
        let ds_b = db
            .collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
            .find_one(doc! { "_id": ds_b_id })
            .await
            .unwrap()
            .expect("user B downstream_service should exist");

        assert_eq!(ds_a.slug, format!("_ssh_{}", ds_a.id));
        assert_eq!(ds_b.slug, format!("_ssh_{}", ds_b.id));
        assert_ne!(ds_a.slug, ds_b.slug);

        let owner_service_a = crate::services::user_service_service::find_by_catalog_service_id(
            &db, &user_a, ds_a_id,
        )
        .await
        .unwrap()
        .expect("user A lookup by backing downstream_service should exist");
        let owner_service_b = crate::services::user_service_service::find_by_catalog_service_id(
            &db, &user_b, ds_b_id,
        )
        .await
        .unwrap()
        .expect("user B lookup by backing downstream_service should exist");
        assert_eq!(owner_service_a.slug, "shared-label");
        assert_eq!(owner_service_b.slug, "shared-label");
    }

    #[tokio::test]
    async fn create_key_uses_explicit_slug_override_for_catalog_services() {
        let Some(db) = connect_test_database("unified_key_service").await else {
            eprintln!("skipping unified_key_service integration test: no local MongoDB available");
            return;
        };

        let encryption_keys = test_encryption_keys();
        let user_id = uuid::Uuid::new_v4().to_string();
        let mut catalog = sample_catalog_service();
        catalog.id = uuid::Uuid::new_v4().to_string();
        catalog.slug = format!("catalog-{}", uuid::Uuid::new_v4());

        db.collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
            .insert_one(&catalog)
            .await
            .unwrap();

        let created = create_key(
            &db,
            &encryption_keys,
            &user_id,
            &user_id,
            Some(&catalog.slug),
            None,
            "secret-token",
            "Catalog Service",
            Some("explicit-slug"),
            None,
            None,
            None,
            None,
            None,
            OpenApiSpecUrlInput::Inherit,
            None,
            OauthClientCredentialsInput::None,
            false,
        )
        .await
        .unwrap();

        assert_eq!(created.service.slug, "explicit-slug");
    }

    #[tokio::test]
    async fn create_key_persists_user_label_on_no_auth_catalog_service() {
        // Regression for issue #429. The catalog branch used to seed
        // `UserEndpoint.label` with `svc.name` (the catalog default)
        // instead of the user-supplied `label`. For no-auth services
        // there is no `UserApiKey` to mask this, so `build_key_view`'s
        // endpoint-label fallback surfaced the wrong value in the
        // wizard's success page and in `nyxid service list`.
        let Some(db) = connect_test_database("ukey_label_429").await else {
            eprintln!("skipping unified_key_service integration test: no local MongoDB available");
            return;
        };

        let encryption_keys = test_encryption_keys();
        let user_id = uuid::Uuid::new_v4().to_string();
        let mut catalog = sample_catalog_service();
        catalog.id = uuid::Uuid::new_v4().to_string();
        catalog.slug = format!("noauth-{}", uuid::Uuid::new_v4());
        catalog.name = "Catalog Default Name".to_string();
        catalog.auth_method = "none".to_string();
        catalog.requires_user_credential = false;

        db.collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
            .insert_one(&catalog)
            .await
            .unwrap();

        let custom_label = "User Custom Label";
        let created = create_key(
            &db,
            &encryption_keys,
            &user_id,
            &user_id,
            Some(&catalog.slug),
            None,
            "",
            custom_label,
            None,
            None,
            None,
            None,
            None,
            None,
            OpenApiSpecUrlInput::Inherit,
            None,
            OauthClientCredentialsInput::None,
            false,
        )
        .await
        .unwrap();

        assert!(
            created.api_key.is_none(),
            "no-auth catalog flow should skip UserApiKey creation"
        );
        assert_eq!(
            created.endpoint.label, custom_label,
            "user-supplied label must persist on UserEndpoint (issue #429)"
        );
    }

    #[tokio::test]
    async fn revoke_key_hard_deletes_backing_endpoint_and_api_key() {
        let Some(db) = connect_test_database("unified_key_service").await else {
            eprintln!("skipping unified_key_service integration test: no local MongoDB available");
            return;
        };

        let encryption_keys = test_encryption_keys();
        let user_id = uuid::Uuid::new_v4().to_string();
        let created = create_key(
            &db,
            &encryption_keys,
            &user_id,
            &user_id,
            None,
            Some("https://api.example.com"),
            "secret-token",
            "Custom Service",
            Some("custom-service"),
            Some("bearer"),
            Some("Authorization"),
            None,
            None,
            None,
            OpenApiSpecUrlInput::Inherit,
            None,
            OauthClientCredentialsInput::None,
            false,
        )
        .await
        .unwrap();

        revoke_key(&db, &user_id, &user_id, &created.service.id)
            .await
            .unwrap();

        let api_key_count = db
            .collection::<mongodb::bson::Document>(USER_API_KEYS)
            .count_documents(doc! { "_id": &created.api_key.as_ref().unwrap().id })
            .await
            .unwrap();
        let endpoint_count = db
            .collection::<mongodb::bson::Document>(USER_ENDPOINTS)
            .count_documents(doc! { "_id": &created.endpoint.id })
            .await
            .unwrap();
        let service = db
            .collection::<mongodb::bson::Document>(USER_SERVICES)
            .find_one(doc! { "_id": &created.service.id })
            .await
            .unwrap()
            .unwrap();

        assert_eq!(api_key_count, 0);
        assert_eq!(endpoint_count, 0);
        assert!(!service.get_bool("is_active").unwrap());
    }

    #[tokio::test]
    async fn revoke_key_soft_revokes_provider_token_after_last_provider_key() {
        let Some(db) = connect_test_database("unified_key_service_provider_revoke").await else {
            eprintln!("skipping unified_key_service integration test: no local MongoDB available");
            return;
        };

        let user_id = uuid::Uuid::new_v4().to_string();
        let provider_id = uuid::Uuid::new_v4().to_string();
        let token_id = insert_provider_token(&db, &user_id, &provider_id).await;
        insert_provider_backed_service(&db, &user_id, &provider_id, "svc-1", "oauth2").await;
        insert_provider_backed_service(&db, &user_id, &provider_id, "svc-2", "oauth2").await;

        revoke_key(&db, &user_id, &user_id, "svc-1").await.unwrap();
        let token_after_first_delete = db
            .collection::<UserProviderToken>(USER_PROVIDER_TOKENS)
            .find_one(doc! { "_id": &token_id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(token_after_first_delete.status, "active");

        revoke_key(&db, &user_id, &user_id, "svc-2").await.unwrap();
        let token_after_second_delete = db
            .collection::<UserProviderToken>(USER_PROVIDER_TOKENS)
            .find_one(doc! { "_id": &token_id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(token_after_second_delete.status, "revoked");
    }

    #[tokio::test]
    async fn revoke_key_ignores_node_managed_keys_when_cascading_provider_token() {
        let Some(db) = connect_test_database("unified_key_service_node_managed_revoke").await
        else {
            eprintln!("skipping unified_key_service integration test: no local MongoDB available");
            return;
        };

        let user_id = uuid::Uuid::new_v4().to_string();
        let provider_id = uuid::Uuid::new_v4().to_string();
        let token_id = insert_provider_token(&db, &user_id, &provider_id).await;
        insert_provider_backed_service(&db, &user_id, &provider_id, "central-svc", "oauth2").await;
        insert_provider_backed_service(
            &db,
            &user_id,
            &provider_id,
            "node-managed-svc",
            "node_managed",
        )
        .await;

        revoke_key(&db, &user_id, &user_id, "central-svc")
            .await
            .unwrap();
        let token_after_delete = db
            .collection::<UserProviderToken>(USER_PROVIDER_TOKENS)
            .find_one(doc! { "_id": &token_id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(token_after_delete.status, "revoked");
    }

    #[tokio::test]
    async fn create_key_with_missing_node_fails_before_inserting_resources() {
        let Some(db) = connect_test_database("unified_key_service").await else {
            eprintln!("skipping unified_key_service integration test: no local MongoDB available");
            return;
        };

        let encryption_keys = test_encryption_keys();
        let user_id = uuid::Uuid::new_v4().to_string();

        let err = create_key(
            &db,
            &encryption_keys,
            &user_id,
            &user_id,
            None,
            Some("https://api.example.com"),
            "",
            "Node Routed Service",
            Some("node-routed-service"),
            Some("bearer"),
            Some("Authorization"),
            Some("missing-node"),
            None,
            None,
            OpenApiSpecUrlInput::Inherit,
            None,
            OauthClientCredentialsInput::None,
            false,
        )
        .await
        .err()
        .expect("missing node should fail");

        assert!(
            matches!(err, AppError::NodeNotFound(ref message) if message == "Node not found"),
            "expected NodeNotFound, got {err}"
        );

        let endpoint_count = db
            .collection::<mongodb::bson::Document>(USER_ENDPOINTS)
            .count_documents(doc! { "user_id": &user_id })
            .await
            .unwrap();
        let api_key_count = db
            .collection::<mongodb::bson::Document>(USER_API_KEYS)
            .count_documents(doc! { "user_id": &user_id })
            .await
            .unwrap();
        let service_count = db
            .collection::<mongodb::bson::Document>(USER_SERVICES)
            .count_documents(doc! { "user_id": &user_id })
            .await
            .unwrap();

        assert_eq!(endpoint_count, 0);
        assert_eq!(api_key_count, 0);
        assert_eq!(service_count, 0);
    }

    #[tokio::test]
    async fn create_key_rejects_header_auth_with_empty_auth_key_name_before_writes() {
        let Some(db) = connect_test_database("unified_key_empty_header_auth_key").await else {
            eprintln!("skipping unified_key_service integration test: no local MongoDB available");
            return;
        };

        let encryption_keys = test_encryption_keys();
        let user_id = uuid::Uuid::new_v4().to_string();

        let err = create_key(
            &db,
            &encryption_keys,
            &user_id,
            &user_id,
            None,
            Some("https://api.example.com"),
            "secret-token",
            "Header Service",
            Some("header-service"),
            Some("header"),
            Some(""),
            None,
            None,
            None,
            OpenApiSpecUrlInput::Inherit,
            None,
            OauthClientCredentialsInput::None,
            false,
        )
        .await
        .err()
        .expect("empty header auth_key_name should fail");

        assert!(matches!(
            err,
            AppError::ValidationError(message)
                if message.contains("auth_method is 'header'")
        ));

        let endpoint_count = db
            .collection::<mongodb::bson::Document>(USER_ENDPOINTS)
            .count_documents(doc! { "user_id": &user_id })
            .await
            .unwrap();
        let api_key_count = db
            .collection::<mongodb::bson::Document>(USER_API_KEYS)
            .count_documents(doc! { "user_id": &user_id })
            .await
            .unwrap();
        let service_count = db
            .collection::<mongodb::bson::Document>(USER_SERVICES)
            .count_documents(doc! { "user_id": &user_id })
            .await
            .unwrap();

        assert_eq!(endpoint_count, 0);
        assert_eq!(api_key_count, 0);
        assert_eq!(service_count, 0);
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

    // --- classify_update_credential_action (NyxID#418, #419) ---

    #[test]
    fn classify_does_nothing_when_caller_omits_credential_and_auth_method() {
        // Bare `PUT /keys/:id` with only node_id or label fields should leave
        // the credential plane untouched.
        let action = classify_update_credential_action("none", false, None, None, None, false);
        assert_eq!(action, UpdateCredentialAction::Nothing);

        let action =
            classify_update_credential_action("bearer", true, Some("bearer"), None, None, false);
        assert_eq!(action, UpdateCredentialAction::Nothing);
    }

    #[test]
    fn classify_provisions_on_upgrade_from_none_with_credential() {
        // NyxID#419 repro: service created with `auth_method: none`, PUT
        // upgrades to bearer with credential supplied in the same body.
        let action = classify_update_credential_action(
            "none",
            false,
            None,
            Some("bearer"),
            Some("secret-token"),
            false,
        );
        assert_eq!(
            action,
            UpdateCredentialAction::Provision {
                credential_type: "bearer",
                credential: "secret-token",
            }
        );
    }

    #[test]
    fn classify_provisions_node_managed_when_upgrade_targets_node_without_credential() {
        // HA add-on flow: the add-on POSTs `auth_method: none` to reserve
        // the slug, then PUTs to bind a node + bearer. Credential lives on
        // the node (pushed via `nyxid node credentials add` locally), so
        // the server creates a `node_managed` record.
        let action =
            classify_update_credential_action("none", false, None, Some("bearer"), None, true);
        assert_eq!(
            action,
            UpdateCredentialAction::Provision {
                credential_type: "node_managed",
                credential: "",
            }
        );
    }

    #[test]
    fn classify_rejects_direct_upgrade_without_credential_or_node() {
        // Pure direct routing upgrade with no credential and no node is
        // ambiguous — reject instead of silently creating an unusable record.
        let action =
            classify_update_credential_action("none", false, None, Some("bearer"), None, false);
        match action {
            UpdateCredentialAction::Reject(msg) => {
                assert!(msg.contains("Credential is required"));
            }
            other => panic!("expected Reject, got {other:?}"),
        }
    }

    #[test]
    fn classify_rejects_credential_while_auth_method_stays_none() {
        // Caller supplied a credential but didn't set auth_method — the
        // resulting record would never be injected into proxy calls.
        // Fail loudly so callers can't end up with credentials on a
        // `auth_method: none` service.
        let action =
            classify_update_credential_action("none", false, None, None, Some("secret"), false);
        match action {
            UpdateCredentialAction::Reject(msg) => {
                assert!(msg.contains("auth_method"));
            }
            other => panic!("expected Reject, got {other:?}"),
        }
    }

    #[test]
    fn classify_rejects_credential_write_when_service_still_auth_method_none() {
        // Second Codex review P2: a service can hold an `api_key_id` after
        // being downgraded back to `auth_method: none` (the handler doesn't
        // unlink). A later PUT that only supplies `credential` would
        // otherwise rotate/promote silently, storing an unusable secret
        // because the proxy skips injection for no-auth services.
        let action = classify_update_credential_action(
            "none",
            true,
            Some("bearer"),
            None,
            Some("new-secret"),
            false,
        );
        match action {
            UpdateCredentialAction::Reject(msg) => {
                assert!(msg.contains("auth_method"));
            }
            other => panic!("expected Reject, got {other:?}"),
        }
    }

    #[test]
    fn classify_rotates_credential_on_existing_direct_api_key() {
        // Standard "rotate the stored bearer" flow on a service that already
        // had a direct credential — mirrors `PUT /api-keys/external/:id`.
        let action = classify_update_credential_action(
            "bearer",
            true,
            Some("bearer"),
            None,
            Some("new-secret"),
            false,
        );
        assert_eq!(
            action,
            UpdateCredentialAction::Rotate {
                credential: "new-secret",
            }
        );
    }

    #[test]
    fn classify_promotes_node_managed_api_key_when_caller_supplies_credential() {
        // Codex review P1.2: existing node-routed service backed by a
        // `node_managed` record. Supplying a fresh credential via PUT must
        // promote the record to a direct type so the new value is actually
        // stored — `update_api_key` refuses node_managed rotations outright.
        let action = classify_update_credential_action(
            "bearer",
            true,
            Some("node_managed"),
            None,
            Some("rotated-secret"),
            true,
        );
        assert_eq!(
            action,
            UpdateCredentialAction::Promote {
                credential_type: "bearer",
                credential: "rotated-secret",
            }
        );
    }

    #[test]
    fn classify_promote_uses_auth_method_for_target_type_when_upgrade_accompanies_rotation() {
        // If the caller also changes auth_method in the same PUT, promote
        // to the target auth_method's direct type rather than the current.
        let action = classify_update_credential_action(
            "bearer",
            true,
            Some("node_managed"),
            Some("basic"),
            Some("basic-auth-string"),
            true,
        );
        assert_eq!(
            action,
            UpdateCredentialAction::Promote {
                credential_type: "basic",
                credential: "basic-auth-string",
            }
        );
    }

    #[test]
    fn classify_rejects_empty_string_credential() {
        // Blank rotations are an explicit error now (twenty-second-round
        // Codex P2). Previously they were collapsed to `None` and could
        // even provision a `node_managed` placeholder on upgrades — that
        // made it look like a rotation succeeded while the old secret
        // stayed in effect. Omit the field to leave the stored value
        // unchanged; send a non-empty value to rotate.
        let action = classify_update_credential_action(
            "bearer",
            true,
            Some("bearer"),
            None,
            Some(""),
            false,
        );
        match action {
            UpdateCredentialAction::Reject(msg) => assert!(msg.contains("must not be empty")),
            other => panic!("expected Reject, got {other:?}"),
        }

        let action =
            classify_update_credential_action("none", false, None, Some("bearer"), Some(""), true);
        match action {
            UpdateCredentialAction::Reject(msg) => assert!(msg.contains("must not be empty")),
            other => panic!("expected Reject, got {other:?}"),
        }
    }

    #[test]
    fn classify_rejects_upgrade_when_node_id_explicitly_cleared_without_credential() {
        // Caller sends `{"auth_method": "bearer", "node_id": ""}` — the
        // effective node is None after the update, so the classifier must
        // see `effective_node_id_is_set = false` and reject when credential
        // is also missing. This test pins the caller's responsibility to
        // compute `effective_node_id_is_set` from the empty-clears-set-keeps
        // three-state mapping before calling the classifier.
        let action =
            classify_update_credential_action("none", false, None, Some("bearer"), None, false);
        match action {
            UpdateCredentialAction::Reject(_) => {}
            other => panic!("expected Reject, got {other:?}"),
        }
    }

    #[test]
    fn classify_header_auth_uses_api_key_credential_type() {
        // `auth_method: "header"` and `"query"` fall through the default
        // branch and should produce a generic `"api_key"` credential_type —
        // matches the existing custom-HTTP create path.
        let action = classify_update_credential_action(
            "none",
            false,
            None,
            Some("header"),
            Some("xoxb-secret"),
            false,
        );
        assert_eq!(
            action,
            UpdateCredentialAction::Provision {
                credential_type: "api_key",
                credential: "xoxb-secret",
            }
        );
    }

    /// Regression guard for NyxID#356: `service update --endpoint-url`
    /// reflects immediately in both `service show` (GET /keys/:id) and
    /// `service list` (GET /keys). Both handler paths funnel through
    /// `build_key_view` and derive `endpoint_url` from `UserEndpoint.url`.
    /// This test pins that invariant: given one `UserEndpoint`, both call
    /// sites -- the list batch path (which builds a shared `cat_map` over
    /// many services) and the single-key path (which builds a one-entry
    /// map) -- must produce the same `endpoint_url`. If someone changes
    /// one path to source from a different field (e.g., a cached value on
    /// `UserService`), this test fails.
    #[test]
    fn build_key_view_endpoint_url_is_consistent_between_list_and_show() {
        let service = sample_service("bearer");
        let api_key = sample_api_key("bearer");
        let endpoint = UserEndpoint {
            id: "ep-1".to_string(),
            user_id: "user-1".to_string(),
            label: "Updated label".to_string(),
            url: "https://new.example.com/v2".to_string(),
            catalog_service_id: None,
            openapi_spec_url: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        // Show path: per-service one-entry cat_map.
        let show_view = build_key_view(
            &service,
            &endpoint,
            Some(&api_key),
            &HashMap::new(),
            &HashMap::new(),
            crate::services::user_service_service::CredentialSource::Personal,
        );

        // List path: batch-built shared cat_map (same function, same inputs).
        let shared_cat_map: HashMap<&str, &crate::models::downstream_service::DownstreamService> =
            HashMap::new();
        let shared_app_map: HashMap<String, String> = HashMap::new();
        let list_view = build_key_view(
            &service,
            &endpoint,
            Some(&api_key),
            &shared_cat_map,
            &shared_app_map,
            crate::services::user_service_service::CredentialSource::Personal,
        );

        assert_eq!(list_view.endpoint_url, show_view.endpoint_url);
        assert_eq!(list_view.endpoint_url, "https://new.example.com/v2");
        assert_eq!(list_view.endpoint_id, show_view.endpoint_id);
        // Label also flows from the same source on both paths (the api_key's
        // label when present, else the endpoint label). Pin both.
        assert_eq!(list_view.label, show_view.label);
    }

    /// Companion guard: when the service has no api key (auto-provisioned
    /// no-auth), the label falls back to the endpoint label. If a caller
    /// later updates the endpoint label, both paths must reflect the new
    /// value since they both read from `UserEndpoint.label`.
    #[test]
    fn build_key_view_label_fallback_is_consistent_between_list_and_show() {
        let mut service = sample_service("none");
        service.api_key_id = None;
        let endpoint = UserEndpoint {
            id: "ep-1".to_string(),
            user_id: "user-1".to_string(),
            label: "Renamed endpoint".to_string(),
            url: "https://svc.example.com".to_string(),
            catalog_service_id: None,
            openapi_spec_url: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let show_view = build_key_view(
            &service,
            &endpoint,
            None,
            &HashMap::new(),
            &HashMap::new(),
            crate::services::user_service_service::CredentialSource::Personal,
        );
        let list_view = build_key_view(
            &service,
            &endpoint,
            None,
            &HashMap::new(),
            &HashMap::new(),
            crate::services::user_service_service::CredentialSource::Personal,
        );

        assert_eq!(list_view.label, "Renamed endpoint");
        assert_eq!(list_view.label, show_view.label);
        assert_eq!(list_view.endpoint_url, show_view.endpoint_url);
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

    // ───────────────────────────────────────────────────────────────────
    // Multi-connection OAuth: end-to-end `create_key` integration tests.
    // These prove step 19 — the silent-alias removal — actually works:
    // a second codex / Lark add produces an independent connection
    // instead of aliasing onto the first.
    // ───────────────────────────────────────────────────────────────────

    /// Minimal valid `ProviderConfig` for the multi-connection tests.
    /// `provider_type` drives the create_key branch under test
    /// (`oauth2` / `device_code` → mint a fresh connection_id;
    /// `api_key` → legacy reuse path).
    fn multi_conn_provider(provider_type: &str) -> ProviderConfig {
        ProviderConfig {
            id: uuid::Uuid::new_v4().to_string(),
            slug: format!("prov-{}", uuid::Uuid::new_v4()),
            name: "Multi-Conn Test Provider".to_string(),
            description: None,
            provider_type: provider_type.to_string(),
            authorization_url: Some("https://example.com/authorize".to_string()),
            token_url: Some("https://example.com/token".to_string()),
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
            credential_mode: "admin".to_string(),
            token_endpoint_auth_method: "client_secret_post".to_string(),
            extra_auth_params: None,
            device_code_format: "rfc8628".to_string(),
            client_id_param_name: None,
            requires_gateway_url: false,
            created_by: "system".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    /// Catalog `DownstreamService` backed by `provider`, shaped so
    /// `create_key` with an empty credential mints a `pending_auth`
    /// `UserApiKey` (auth_method != "none" so it isn't `is_truly_no_auth`).
    fn multi_conn_catalog(provider: &ProviderConfig) -> DownstreamService {
        let mut svc = sample_catalog_service();
        svc.id = uuid::Uuid::new_v4().to_string();
        svc.slug = format!("cat-{}", uuid::Uuid::new_v4());
        svc.auth_method = "bearer".to_string();
        svc.auth_key_name = "Authorization".to_string();
        svc.provider_config_id = Some(provider.id.clone());
        svc
    }

    #[allow(clippy::too_many_arguments)]
    async fn create_catalog_key(
        db: &mongodb::Database,
        encryption_keys: &crate::crypto::aes::EncryptionKeys,
        user_id: &str,
        slug: &str,
        label: &str,
    ) -> AppResult<super::CreateKeyResult> {
        create_key(
            db,
            encryption_keys,
            user_id,
            user_id,
            Some(slug),
            None,
            "",
            label,
            None,
            None,
            None,
            None,
            None,
            None,
            OpenApiSpecUrlInput::Inherit,
            None,
            OauthClientCredentialsInput::None,
            false,
        )
        .await
    }

    #[tokio::test]
    async fn create_key_oauth_multi_add_mints_distinct_connection_ids() {
        // THE step-19 proof: adding a device-code service (codex) twice
        // produces two INDEPENDENT pending_auth keys, each with its own
        // connection_id — not a silent alias onto the first.
        let Some(db) = connect_test_database("ukey_multi_add_distinct").await else {
            eprintln!("skipping unified_key_service integration test: no local MongoDB available");
            return;
        };
        let encryption_keys = test_encryption_keys();
        let user_id = uuid::Uuid::new_v4().to_string();

        let provider = multi_conn_provider("device_code");
        let catalog = multi_conn_catalog(&provider);
        db.collection::<ProviderConfig>(PROVIDER_CONFIGS)
            .insert_one(&provider)
            .await
            .unwrap();
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(&catalog)
            .await
            .unwrap();

        let first = create_catalog_key(&db, &encryption_keys, &user_id, &catalog.slug, "codex one")
            .await
            .expect("first codex add should succeed");
        let second =
            create_catalog_key(&db, &encryption_keys, &user_id, &catalog.slug, "codex two")
                .await
                .expect("second codex add must NOT be blocked or silently aliased");

        let key_a = first.api_key.expect("first add mints a UserApiKey");
        let key_b = second.api_key.expect("second add mints a UserApiKey");

        // Both are fresh pending_auth placeholders awaiting their own
        // device-code flow — neither short-circuited to `active`.
        assert_eq!(key_a.status, "pending_auth");
        assert_eq!(key_b.status, "pending_auth");

        // Distinct rows, distinct connection_ids — the heart of the fix.
        assert_ne!(key_a.id, key_b.id, "each add must mint its own UserApiKey");
        let conn_a = key_a.connection_id.expect("first add has a connection_id");
        let conn_b = key_b.connection_id.expect("second add has a connection_id");
        assert_ne!(
            conn_a, conn_b,
            "each add must mint a DISTINCT connection_id (no silent alias)"
        );

        // Distinct UserService rows too (different slugs, auto-disambiguated).
        assert_ne!(first.service.id, second.service.id);
        assert_ne!(first.service.slug, second.service.slug);
    }

    #[tokio::test]
    async fn create_key_oauth_ignores_existing_provider_token() {
        // Even when the user already has a `user_provider_tokens` row for
        // this provider (e.g. a legacy single-connection codex), a NEW
        // device-code add must still mint a fresh pending_auth key with
        // its own connection_id — never reuse / alias the legacy token.
        let Some(db) = connect_test_database("ukey_multi_add_ignores_legacy").await else {
            eprintln!("skipping unified_key_service integration test: no local MongoDB available");
            return;
        };
        let encryption_keys = test_encryption_keys();
        let user_id = uuid::Uuid::new_v4().to_string();

        let provider = multi_conn_provider("device_code");
        let catalog = multi_conn_catalog(&provider);
        db.collection::<ProviderConfig>(PROVIDER_CONFIGS)
            .insert_one(&provider)
            .await
            .unwrap();
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(&catalog)
            .await
            .unwrap();

        // Pre-existing legacy provider token for (user, provider).
        let now = Utc::now();
        db.collection::<UserProviderToken>(USER_PROVIDER_TOKENS)
            .insert_one(UserProviderToken {
                id: uuid::Uuid::new_v4().to_string(),
                user_id: user_id.clone(),
                provider_config_id: provider.id.clone(),
                connection_id: None,
                credential_user_id: None,
                token_type: "oauth2".to_string(),
                access_token_encrypted: Some(vec![1, 2, 3]),
                refresh_token_encrypted: Some(vec![4, 5, 6]),
                token_scopes: None,
                expires_at: None,
                api_key_encrypted: None,
                status: "active".to_string(),
                last_refreshed_at: None,
                last_used_at: None,
                error_message: None,
                label: None,
                metadata: None,
                gateway_url: None,
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();

        let created =
            create_catalog_key(&db, &encryption_keys, &user_id, &catalog.slug, "codex new")
                .await
                .expect("add should succeed");
        let key = created.api_key.expect("add mints a UserApiKey");

        // Fresh pending_auth placeholder — NOT activated from the legacy
        // token, NOT aliased (no source_id pointing at the legacy token).
        assert_eq!(
            key.status, "pending_auth",
            "must NOT inherit `active` from the pre-existing provider token"
        );
        assert!(
            key.connection_id.is_some(),
            "must mint its own connection_id"
        );
        assert!(
            key.access_token_encrypted.is_none(),
            "must not copy the legacy token's access token"
        );
        assert!(
            key.source_id.is_none(),
            "must not be aliased to the legacy provider token via source_id"
        );
    }

    #[tokio::test]
    async fn create_key_api_key_provider_still_reuses_existing_token() {
        // Regression guard: the silent-alias removal is scoped to
        // oauth2 / device_code providers ONLY. An `api_key`-type
        // provider with an existing token must STILL reuse it — that
        // behavior is out of scope for multi-connection and unchanged.
        let Some(db) = connect_test_database("ukey_api_key_still_reuses").await else {
            eprintln!("skipping unified_key_service integration test: no local MongoDB available");
            return;
        };
        let encryption_keys = test_encryption_keys();
        let user_id = uuid::Uuid::new_v4().to_string();

        let provider = multi_conn_provider("api_key");
        let catalog = multi_conn_catalog(&provider);
        db.collection::<ProviderConfig>(PROVIDER_CONFIGS)
            .insert_one(&provider)
            .await
            .unwrap();
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(&catalog)
            .await
            .unwrap();

        let now = Utc::now();
        let token_id = uuid::Uuid::new_v4().to_string();
        db.collection::<UserProviderToken>(USER_PROVIDER_TOKENS)
            .insert_one(UserProviderToken {
                id: token_id.clone(),
                user_id: user_id.clone(),
                provider_config_id: provider.id.clone(),
                connection_id: None,
                credential_user_id: None,
                token_type: "api_key".to_string(),
                access_token_encrypted: None,
                refresh_token_encrypted: None,
                token_scopes: None,
                expires_at: None,
                api_key_encrypted: Some(vec![9, 9, 9]),
                status: "active".to_string(),
                last_refreshed_at: None,
                last_used_at: None,
                error_message: None,
                label: None,
                metadata: None,
                gateway_url: None,
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();

        let created = create_catalog_key(
            &db,
            &encryption_keys,
            &user_id,
            &catalog.slug,
            "api-key svc",
        )
        .await
        .expect("add should succeed");
        let key = created.api_key.expect("add mints a UserApiKey");

        // api_key provider: legacy reuse path is preserved — the new
        // UserApiKey is sourced from the existing provider token and
        // carries no connection_id.
        assert_eq!(
            key.source_id.as_deref(),
            Some(token_id.as_str()),
            "api_key provider must still reuse the existing provider token"
        );
        assert!(
            key.connection_id.is_none(),
            "api_key provider reuse path stays connection-less"
        );
    }

    // ──────────────────────────────────────────────────────────────────
    // BYO Custom App credentials — end-to-end POST /keys integration.
    // These prove the user-facing flow: a Lark-style
    // `credential_mode: "user"` provider with user-supplied raw creds
    // produces a multi-connection placeholder that carries its OWN
    // encrypted Custom App credentials. Without this, refresh after
    // the first 2h token expiry fails because the connection has no
    // client_id to refresh against.
    // ──────────────────────────────────────────────────────────────────

    /// Variant of `multi_conn_catalog` with `credential_mode: "user"` —
    /// the Lark / Feishu shape.
    fn byo_user_mode_provider() -> ProviderConfig {
        let mut p = multi_conn_provider("oauth2");
        p.credential_mode = "user".to_string();
        p
    }

    #[allow(clippy::too_many_arguments)]
    async fn create_byo_catalog_key(
        db: &mongodb::Database,
        encryption_keys: &crate::crypto::aes::EncryptionKeys,
        user_id: &str,
        slug: &str,
        label: &str,
        byo: OauthClientCredentialsInput<'_>,
    ) -> AppResult<super::CreateKeyResult> {
        create_key(
            db,
            encryption_keys,
            user_id,
            user_id,
            Some(slug),
            None,
            "",
            label,
            None,
            None,
            None,
            None,
            None,
            None,
            OpenApiSpecUrlInput::Inherit,
            None,
            byo,
            false,
        )
        .await
    }

    #[tokio::test]
    async fn create_key_byo_lark_persists_credentials_on_new_key() {
        let Some(db) = connect_test_database("ukey_byo_lark_persists").await else {
            eprintln!("skipping integration test: no local MongoDB available");
            return;
        };
        let encryption_keys = test_encryption_keys();
        let user_id = uuid::Uuid::new_v4().to_string();

        let provider = byo_user_mode_provider();
        let catalog = multi_conn_catalog(&provider);
        db.collection::<ProviderConfig>(PROVIDER_CONFIGS)
            .insert_one(&provider)
            .await
            .unwrap();
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(&catalog)
            .await
            .unwrap();

        let result = create_byo_catalog_key(
            &db,
            &encryption_keys,
            &user_id,
            &catalog.slug,
            "marketing-lark",
            OauthClientCredentialsInput::Raw {
                client_id: "cli_marketing",
                client_secret: "super-secret",
            },
        )
        .await
        .expect("BYO add should succeed");

        let key = result.api_key.expect("placeholder mints a UserApiKey");
        assert_eq!(key.status, "pending_auth");
        assert!(
            key.connection_id.is_some(),
            "new add must mint connection_id"
        );
        let stored_id = key
            .user_oauth_client_id_encrypted
            .as_ref()
            .expect("BYO client_id must be encrypted on the key");
        let stored_sec = key
            .user_oauth_client_secret_encrypted
            .as_ref()
            .expect("BYO client_secret must be encrypted on the key");
        let plain_id = encryption_keys.decrypt(stored_id).await.unwrap();
        let plain_sec = encryption_keys.decrypt(stored_sec).await.unwrap();
        assert_eq!(String::from_utf8(plain_id).unwrap(), "cli_marketing");
        assert_eq!(String::from_utf8(plain_sec).unwrap(), "super-secret");
    }

    #[tokio::test]
    async fn create_key_byo_lark_rejects_when_creds_missing() {
        // `credential_mode: "user"` provider with no BYO supplied must
        // surface a clear error rather than mint an unusable placeholder.
        let Some(db) = connect_test_database("ukey_byo_lark_required").await else {
            eprintln!("skipping integration test: no local MongoDB available");
            return;
        };
        let encryption_keys = test_encryption_keys();
        let user_id = uuid::Uuid::new_v4().to_string();
        let provider = byo_user_mode_provider();
        let catalog = multi_conn_catalog(&provider);
        db.collection::<ProviderConfig>(PROVIDER_CONFIGS)
            .insert_one(&provider)
            .await
            .unwrap();
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(&catalog)
            .await
            .unwrap();

        let err = create_byo_catalog_key(
            &db,
            &encryption_keys,
            &user_id,
            &catalog.slug,
            "no-byo-lark",
            OauthClientCredentialsInput::None,
        )
        .await
        .err()
        .expect("BYO-required provider should reject add without creds");
        assert!(matches!(err, AppError::BadRequest(ref m) if m.contains("requires user-provided")));
    }

    #[tokio::test]
    async fn create_key_byo_rejected_for_admin_mode_provider() {
        // `credential_mode: "admin"` provider should reject BYO input
        // with a clear message — same wording as the existing
        // `PUT /providers/{id}/credentials` gate.
        let Some(db) = connect_test_database("ukey_byo_admin_rejected").await else {
            eprintln!("skipping integration test: no local MongoDB available");
            return;
        };
        let encryption_keys = test_encryption_keys();
        let user_id = uuid::Uuid::new_v4().to_string();
        let provider = multi_conn_provider("oauth2");
        assert_eq!(provider.credential_mode, "admin");
        let catalog = multi_conn_catalog(&provider);
        db.collection::<ProviderConfig>(PROVIDER_CONFIGS)
            .insert_one(&provider)
            .await
            .unwrap();
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(&catalog)
            .await
            .unwrap();

        let err = create_byo_catalog_key(
            &db,
            &encryption_keys,
            &user_id,
            &catalog.slug,
            "admin-mode-with-byo",
            OauthClientCredentialsInput::Raw {
                client_id: "cli_x",
                client_secret: "sec_x",
            },
        )
        .await
        .err()
        .expect("admin-mode provider should reject BYO");
        assert!(
            matches!(err, AppError::BadRequest(ref m) if m.contains("does not accept user-provided"))
        );
    }

    #[tokio::test]
    async fn create_key_copy_oauth_client_from_copies_source_credentials() {
        // The link-to-existing path: create connection A with BYO creds,
        // then create connection B with `copy_oauth_client_from = A.id`.
        // B must end up with its own encrypted copy of A's plaintext
        // creds — proving the server-side decrypt-then-re-encrypt
        // (so the client never sees the source secret).
        let Some(db) = connect_test_database("ukey_byo_copy_from").await else {
            eprintln!("skipping integration test: no local MongoDB available");
            return;
        };
        let encryption_keys = test_encryption_keys();
        let user_id = uuid::Uuid::new_v4().to_string();
        let provider = byo_user_mode_provider();
        let catalog = multi_conn_catalog(&provider);
        db.collection::<ProviderConfig>(PROVIDER_CONFIGS)
            .insert_one(&provider)
            .await
            .unwrap();
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(&catalog)
            .await
            .unwrap();

        // Connection A — raw creds.
        let result_a = create_byo_catalog_key(
            &db,
            &encryption_keys,
            &user_id,
            &catalog.slug,
            "marketing-lark",
            OauthClientCredentialsInput::Raw {
                client_id: "cli_marketing",
                client_secret: "super-secret",
            },
        )
        .await
        .expect("first BYO add should succeed");
        let key_a = result_a.api_key.expect("placeholder");

        // Connection B — copy_oauth_client_from = key_a.id.
        let result_b = create_byo_catalog_key(
            &db,
            &encryption_keys,
            &user_id,
            &catalog.slug,
            "support-lark",
            OauthClientCredentialsInput::CopyFrom {
                source_key_id: &key_a.id,
            },
        )
        .await
        .expect("copy-from add should succeed");
        let key_b = result_b.api_key.expect("placeholder");

        // Distinct connection_ids — these are independent connections,
        // not aliases.
        assert_ne!(
            key_a.connection_id, key_b.connection_id,
            "copy-from must mint a fresh connection_id"
        );

        let stored_id = key_b
            .user_oauth_client_id_encrypted
            .as_ref()
            .expect("copy must encrypt client_id onto B");
        let plain_id = encryption_keys.decrypt(stored_id).await.unwrap();
        assert_eq!(String::from_utf8(plain_id).unwrap(), "cli_marketing");

        // Independence proof: the two ciphertexts must NOT be byte-equal
        // (AES-GCM has a fresh nonce per encrypt). Even though the
        // plaintext is the same Custom App, the rows are independent
        // — deleting / rotating one cannot mechanically clobber the
        // other (§5.1 of the design doc).
        let enc_a = key_a
            .user_oauth_client_id_encrypted
            .expect("A has encrypted client_id");
        let enc_b = key_b
            .user_oauth_client_id_encrypted
            .expect("B has encrypted client_id");
        assert_ne!(
            enc_a, enc_b,
            "encrypted ciphertexts must differ (fresh nonce per encrypt)"
        );
    }

    #[tokio::test]
    async fn create_key_copy_oauth_client_from_rejects_foreign_owner() {
        let Some(db) = connect_test_database("ukey_byo_copy_foreign").await else {
            eprintln!("skipping integration test: no local MongoDB available");
            return;
        };
        let encryption_keys = test_encryption_keys();
        let alice = uuid::Uuid::new_v4().to_string();
        let bob = uuid::Uuid::new_v4().to_string();
        let provider = byo_user_mode_provider();
        let catalog = multi_conn_catalog(&provider);
        db.collection::<ProviderConfig>(PROVIDER_CONFIGS)
            .insert_one(&provider)
            .await
            .unwrap();
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(&catalog)
            .await
            .unwrap();

        // Alice creates a BYO connection.
        let alice_result = create_byo_catalog_key(
            &db,
            &encryption_keys,
            &alice,
            &catalog.slug,
            "alice-lark",
            OauthClientCredentialsInput::Raw {
                client_id: "cli_alice",
                client_secret: "alice-secret",
            },
        )
        .await
        .unwrap();
        let alice_key_id = alice_result.api_key.unwrap().id;

        // Bob attempts to copy from Alice's key. Must be rejected with
        // NotFound so existence isn't leaked through the error type.
        let err = create_byo_catalog_key(
            &db,
            &encryption_keys,
            &bob,
            &catalog.slug,
            "bob-tries-alice",
            OauthClientCredentialsInput::CopyFrom {
                source_key_id: &alice_key_id,
            },
        )
        .await
        .err()
        .expect("foreign-owner copy should be rejected");
        assert!(matches!(err, AppError::NotFound(_)));
    }

    #[tokio::test]
    async fn create_key_copy_oauth_client_from_rejects_credentialless_source() {
        // The source key exists and is owned by the same user, but has
        // no BYO creds (e.g. legacy connection-less key). Copy must
        // be rejected with a clear message.
        let Some(db) = connect_test_database("ukey_byo_copy_no_source_creds").await else {
            eprintln!("skipping integration test: no local MongoDB available");
            return;
        };
        let encryption_keys = test_encryption_keys();
        let user_id = uuid::Uuid::new_v4().to_string();
        let provider = byo_user_mode_provider();
        let catalog = multi_conn_catalog(&provider);
        db.collection::<ProviderConfig>(PROVIDER_CONFIGS)
            .insert_one(&provider)
            .await
            .unwrap();
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(&catalog)
            .await
            .unwrap();

        // Insert a legacy-style key (no BYO encrypted blob).
        let stripped_key_id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();
        db.collection::<UserApiKey>(USER_API_KEYS)
            .insert_one(UserApiKey {
                id: stripped_key_id.clone(),
                user_id: user_id.clone(),
                label: "Legacy".to_string(),
                credential_type: "oauth2".to_string(),
                credential_encrypted: None,
                access_token_encrypted: None,
                refresh_token_encrypted: None,
                token_scopes: None,
                expires_at: None,
                provider_config_id: Some(provider.id.clone()),
                connection_id: None,
                user_oauth_client_id_encrypted: None,
                user_oauth_client_secret_encrypted: None,
                status: "active".to_string(),
                last_used_at: None,
                error_message: None,
                source: Some("user_created".to_string()),
                source_id: None,
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();

        let err = create_byo_catalog_key(
            &db,
            &encryption_keys,
            &user_id,
            &catalog.slug,
            "copy-from-empty",
            OauthClientCredentialsInput::CopyFrom {
                source_key_id: &stripped_key_id,
            },
        )
        .await
        .err()
        .expect("credential-less source should be rejected");
        assert!(matches!(
            err,
            AppError::BadRequest(ref m)
                if m.contains("does not carry user-provided OAuth client credentials")
        ));
    }
}
