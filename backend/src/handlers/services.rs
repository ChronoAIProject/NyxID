use axum::{
    Json,
    extract::{Path, Query, State},
};
use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::AppState;
use crate::crypto::token::{generate_random_token, hash_token};
use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::{
    COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService, ServiceCapabilities,
    TokenExchangeConfig,
};
use crate::models::oauth_client::{COLLECTION_NAME as OAUTH_CLIENTS, OauthClient};
use crate::models::user::{COLLECTION_NAME as USERS, User};
use crate::mw::auth::AuthUser;
use crate::services::url_validation::{validate_base_url, validate_optional_spec_url};
use crate::services::{api_docs_service, audit_service, oauth_client_service, ssh_service};
use crate::telemetry::{TelemetryContext, TelemetryEvent, emit_event};

use super::services_helpers::{
    DeleteServiceResponse, fetch_service, require_admin_or_creator, service_to_response,
    validate_developer_app_ids,
};

// --- Request / Response types ---

#[derive(Deserialize, ToSchema)]
pub struct CreateServiceRequest {
    pub name: String,
    pub slug: Option<String>,
    pub description: Option<String>,
    pub service_type: Option<String>,
    pub base_url: Option<String>,
    /// Accepts "auth_method" or "auth_type" from frontend
    #[serde(alias = "auth_type")]
    pub auth_method: Option<String>,
    pub auth_key_name: Option<String>,
    pub credential: Option<String>,
    /// "provider", "connection", or "internal". Defaults to "connection".
    pub service_category: Option<String>,
    /// "public" or "private". Defaults to "public" for HTTP, "private" for SSH.
    pub visibility: Option<String>,
    pub ssh_config: Option<SshServiceConfigRequest>,
    // Rich metadata for AI agent discovery
    pub homepage_url: Option<String>,
    pub repository_url: Option<String>,
    pub issues_url: Option<String>,
    pub capabilities: Option<ServiceCapabilities>,
    pub auth_notes: Option<String>,
    pub known_limitations: Option<String>,
    pub required_permissions: Option<Vec<String>>,
    pub examples_url: Option<String>,
    pub recommended_skills: Option<Vec<String>>,
    /// Developer app (OAuth client) IDs that grant access to this service.
    /// Only relevant for private services -- users who consent to any of these
    /// apps will have the service auto-provisioned in their AI Services.
    pub developer_app_ids: Option<Vec<String>>,
    /// Forward the caller's NyxID access token as Authorization: Bearer to downstream
    #[serde(default)]
    pub forward_access_token: bool,
    /// Declarative token exchange config. Required when `auth_method` is
    /// `token_exchange`; ignored for every other method. Describes how to
    /// POST the stored credential JSON, parse the token out of the
    /// response, cache it, and inject it on outbound requests.
    #[serde(default)]
    pub token_exchange_config: Option<TokenExchangeConfig>,
    /// Initial admin-configured default HTTP headers (NyxID#356). Each
    /// entry is validated against the shared denylist + length caps.
    #[serde(default)]
    pub default_request_headers:
        Option<Vec<crate::models::default_request_header::DefaultRequestHeader>>,
}

impl std::fmt::Debug for CreateServiceRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CreateServiceRequest")
            .field("name", &self.name)
            .field("slug", &self.slug)
            .field("description", &self.description)
            .field("base_url", &self.base_url)
            .field("service_type", &self.service_type)
            .field("auth_method", &self.auth_method)
            .field("auth_key_name", &self.auth_key_name)
            .field(
                "credential",
                &self.credential.as_ref().map(|_| "[REDACTED]"),
            )
            .field("service_category", &self.service_category)
            .field("visibility", &self.visibility)
            .field("ssh_config", &self.ssh_config)
            .finish()
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct SshServiceConfigRequest {
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub certificate_auth_enabled: bool,
    #[serde(default = "default_certificate_ttl_minutes")]
    pub certificate_ttl_minutes: u32,
    #[serde(default)]
    pub allowed_principals: Vec<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SshServiceConfigResponse {
    pub host: String,
    pub port: u16,
    pub certificate_auth_enabled: bool,
    pub certificate_ttl_minutes: u32,
    pub allowed_principals: Vec<String>,
    pub ca_public_key: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ServiceResponse {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub base_url: String,
    pub service_type: String,
    pub visibility: String,
    pub auth_method: String,
    pub auth_type: Option<String>,
    pub auth_key_name: String,
    pub is_active: bool,
    pub oauth_client_id: Option<String>,
    pub openapi_spec_url: Option<String>,
    pub api_spec_url: Option<String>,
    pub asyncapi_spec_url: Option<String>,
    pub streaming_supported: bool,
    pub ssh_config: Option<SshServiceConfigResponse>,
    pub service_category: String,
    pub requires_user_credential: bool,
    pub identity_propagation_mode: String,
    pub identity_include_user_id: bool,
    pub identity_include_email: bool,
    pub identity_include_name: bool,
    pub identity_jwt_audience: Option<String>,
    pub forward_access_token: bool,
    pub inject_delegation_token: bool,
    pub delegation_token_scope: String,
    // Rich metadata
    #[serde(skip_serializing_if = "Option::is_none")]
    pub homepage_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issues_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<ServiceCapabilities>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_notes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub known_limitations: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required_permissions: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub examples_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recommended_skills: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_user_agent: Option<String>,
    /// Admin-configured default HTTP headers injected on every proxied
    /// request (NyxID#356).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_request_headers:
        Option<Vec<crate::models::default_request_header::DefaultRequestHeader>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub developer_app_ids: Option<Vec<String>>,
    pub created_by: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ServiceListResponse {
    pub services: Vec<ServiceResponse>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateServiceRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub base_url: Option<String>,
    pub is_active: Option<bool>,
    /// "public" or "private"
    pub visibility: Option<String>,
    #[serde(alias = "api_spec_url")]
    pub openapi_spec_url: Option<String>,
    pub asyncapi_spec_url: Option<String>,
    pub identity_propagation_mode: Option<String>,
    pub identity_include_user_id: Option<bool>,
    pub identity_include_email: Option<bool>,
    pub identity_include_name: Option<bool>,
    pub identity_jwt_audience: Option<String>,
    pub forward_access_token: Option<bool>,
    pub inject_delegation_token: Option<bool>,
    pub delegation_token_scope: Option<String>,
    pub ssh_config: Option<SshServiceConfigRequest>,
    // Rich metadata for AI agent discovery
    pub homepage_url: Option<String>,
    pub repository_url: Option<String>,
    pub issues_url: Option<String>,
    pub capabilities: Option<ServiceCapabilities>,
    pub auth_notes: Option<String>,
    pub known_limitations: Option<String>,
    pub required_permissions: Option<Vec<String>>,
    pub examples_url: Option<String>,
    pub recommended_skills: Option<Vec<String>>,
    /// Developer app (OAuth client) IDs that grant access to this service.
    /// Pass `[]` to clear. Only meaningful for private services.
    pub developer_app_ids: Option<Vec<String>>,
    /// Custom User-Agent override for this service. Set to "" to clear.
    pub custom_user_agent: Option<String>,
    /// Replace the admin-configured default request headers for this
    /// service (NyxID#356). Field omitted leaves the existing value
    /// unchanged; explicit JSON `null` or `[]` clears; a non-empty array
    /// replaces with a validated list. See `nullable_field::deserialize`
    /// for why we can't just use `Option<Option<_>>` here.
    #[serde(
        default,
        deserialize_with = "crate::models::nullable_field::deserialize"
    )]
    pub default_request_headers:
        Option<Option<Vec<crate::models::default_request_header::DefaultRequestHeader>>>,
    /// Replace the declarative token exchange config on a `token_exchange`
    /// service. Validated through the generic helpers before being
    /// persisted so typos in the template / injection format surface at
    /// update time rather than at first proxy request.
    #[serde(default)]
    pub token_exchange_config: Option<TokenExchangeConfig>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct OidcCredentialsResponse {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uris: Vec<String>,
    pub allowed_scopes: String,
    pub delegation_scopes: String,
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub userinfo_endpoint: String,
    pub jwks_uri: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateRedirectUrisRequest {
    pub redirect_uris: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RedirectUrisResponse {
    pub redirect_uris: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RegenerateSecretResponse {
    pub client_secret: String,
    pub message: String,
}

#[derive(Debug, Deserialize)]
pub struct ListServicesQuery {
    pub category: Option<String>,
}

fn default_certificate_ttl_minutes() -> u32 {
    30
}

fn normalize_service_type(service_type: Option<&str>) -> AppResult<String> {
    match service_type.unwrap_or("http") {
        "http" => Ok("http".to_string()),
        "ssh" => Ok("ssh".to_string()),
        other => Err(AppError::ValidationError(format!(
            "Invalid service_type: {other}. Must be http or ssh"
        ))),
    }
}

/// Validate a `TokenExchangeConfig` at admin create/update time.
///
/// Catches misconfiguration early (empty endpoints, unknown encodings,
/// broken templates, unknown injection formats) so admins see a clear
/// error in the API response instead of a 500 the first time anyone
/// proxies through the service.
fn validate_token_exchange_config(config: &TokenExchangeConfig) -> AppResult<()> {
    if config.endpoint.trim().is_empty() {
        return Err(AppError::ValidationError(
            "token_exchange_config.endpoint must not be empty".to_string(),
        ));
    }
    match config.request_encoding.as_str() {
        "json" | "form" => {}
        other => {
            return Err(AppError::ValidationError(format!(
                "token_exchange_config.request_encoding must be 'json' or 'form' (got '{other}')"
            )));
        }
    }
    if !matches!(&config.request_template, serde_json::Value::Object(_)) {
        return Err(AppError::ValidationError(
            "token_exchange_config.request_template must be a JSON object".to_string(),
        ));
    }
    if config.token_response_path.trim().is_empty() {
        return Err(AppError::ValidationError(
            "token_exchange_config.token_response_path must not be empty".to_string(),
        ));
    }
    if config.default_ttl_secs < 60 {
        return Err(AppError::ValidationError(
            "token_exchange_config.default_ttl_secs must be >= 60".to_string(),
        ));
    }
    // Recognised injection formats mirror apply_injection() in
    // provider_token_exchange_service. Keep these in sync when adding
    // new variants.
    let injection = config.injection.as_str();
    let injection_ok = matches!(injection, "bearer" | "bot_bearer" | "token")
        || injection
            .strip_prefix("header:")
            .is_some_and(|h| !h.trim().is_empty());
    if !injection_ok {
        return Err(AppError::ValidationError(format!(
            "token_exchange_config.injection must be 'bearer' | 'bot_bearer' | 'token' | 'header:<name>' \
             (got '{injection}')"
        )));
    }
    if config.credential_fields.is_empty() {
        return Err(AppError::ValidationError(
            "token_exchange_config.credential_fields must declare at least one field".to_string(),
        ));
    }
    // Every $field placeholder in the request template must be backed by
    // a declared credential field so clients know what to collect.
    let mut declared_names: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for spec in &config.credential_fields {
        if spec.name.trim().is_empty() {
            return Err(AppError::ValidationError(
                "token_exchange_config.credential_fields[].name must not be empty".to_string(),
            ));
        }
        if !declared_names.insert(spec.name.as_str()) {
            return Err(AppError::ValidationError(format!(
                "token_exchange_config.credential_fields contains duplicate name '{}'",
                spec.name
            )));
        }
    }
    // Walk the template and confirm every placeholder names a declared
    // field. This catches the classic "$app_secret vs $appsecret" typo.
    fn check_placeholders(
        value: &serde_json::Value,
        declared: &std::collections::HashSet<&str>,
    ) -> AppResult<()> {
        match value {
            serde_json::Value::String(s) => {
                if let Some(field) = s.strip_prefix('$')
                    && !declared.contains(field)
                {
                    return Err(AppError::ValidationError(format!(
                        "token_exchange_config.request_template references unknown \
                         credential field '${field}'"
                    )));
                }
                Ok(())
            }
            serde_json::Value::Object(map) => {
                for v in map.values() {
                    check_placeholders(v, declared)?;
                }
                Ok(())
            }
            serde_json::Value::Array(items) => {
                for v in items {
                    check_placeholders(v, declared)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
    check_placeholders(&config.request_template, &declared_names)?;
    Ok(())
}

fn derive_http_service_category(
    auth_method: &str,
    service_category: Option<&str>,
) -> AppResult<String> {
    if auth_method == "oidc" {
        return Ok("provider".to_string());
    }

    if auth_method == "none" {
        return Ok("internal".to_string());
    }

    match service_category {
        Some("provider") => Err(AppError::ValidationError(
            "Only OIDC services can be categorized as provider".to_string(),
        )),
        Some("internal") => Ok("internal".to_string()),
        Some("connection") | None => Ok("connection".to_string()),
        Some(other) => Err(AppError::ValidationError(format!(
            "Invalid service_category: {other}. Must be provider, connection, or internal"
        ))),
    }
}

fn derive_ssh_service_category(service_category: Option<&str>) -> AppResult<String> {
    match service_category {
        Some("provider") => Err(AppError::ValidationError(
            "SSH services cannot be categorized as provider".to_string(),
        )),
        Some("connection") => Ok("connection".to_string()),
        Some("internal") | None => Ok("internal".to_string()),
        Some(other) => Err(AppError::ValidationError(format!(
            "Invalid service_category: {other}. Must be connection or internal"
        ))),
    }
}

fn derive_visibility(service_type: &str, explicit: Option<&str>) -> AppResult<String> {
    match explicit {
        Some("private") => Ok("private".to_string()),
        Some("public") => Ok("public".to_string()),
        Some(other) => Err(AppError::ValidationError(format!(
            "Invalid visibility: {other}. Must be public or private"
        ))),
        // Default: SSH services are private, HTTP services are public
        None => {
            if service_type == "ssh" {
                Ok("private".to_string())
            } else {
                Ok("public".to_string())
            }
        }
    }
}

fn should_refresh_openapi_url(service: &DownstreamService, body: &UpdateServiceRequest) -> bool {
    body.openapi_spec_url.is_some()
        || service.openapi_spec_url.is_none()
        || service.openapi_spec_url.as_deref().is_some_and(|url| {
            api_docs_service::is_auto_discovered_openapi_spec_url(&service.base_url, url)
        })
}

fn should_refresh_asyncapi_url(service: &DownstreamService, body: &UpdateServiceRequest) -> bool {
    body.asyncapi_spec_url.is_some()
        || service.asyncapi_spec_url.is_none()
        || service.asyncapi_spec_url.as_deref().is_some_and(|url| {
            api_docs_service::is_auto_discovered_asyncapi_spec_url(&service.base_url, url)
        })
}

fn resolve_spec_url_update(
    explicit_update: Option<Option<String>>,
    current_url: Option<&String>,
    discovered_url: Option<&String>,
    should_refresh: bool,
) -> Option<String> {
    if let Some(explicit_url) = explicit_update {
        explicit_url
    } else if should_refresh {
        discovered_url.cloned()
    } else {
        current_url.cloned()
    }
}

// --- Handlers ---
// TODO(SEC-7): Credential endpoints (get_oidc_credentials, update_redirect_uris,
// regenerate_oidc_secret) should have stricter per-endpoint rate limiting (e.g.,
// 5 requests/minute) instead of sharing the global rate limiter. This requires
// a separate PerIpRateLimiter applied as middleware on these specific routes.

/// GET /api/v1/services
///
/// List all downstream services. Supports optional `?category=` filter.
#[utoipa::path(
    get,
    path = "/api/v1/services",
    params(
        ("category" = Option<String>, Query, description = "Optional service category filter")
    ),
    responses(
        (status = 200, description = "List of downstream services", body = ServiceListResponse),
        (status = 400, description = "Validation error", body = crate::errors::ErrorResponse)
    ),
    tag = "Services"
)]
pub async fn list_services(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(query): Query<ListServicesQuery>,
) -> AppResult<Json<ServiceListResponse>> {
    let user_id_str = auth_user.user_id.to_string();
    let is_admin = state
        .db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": &user_id_str })
        .await?
        .is_some_and(|u| u.is_admin);

    // Private services are only visible to their creator (or admins).
    // Public services (and legacy services without a visibility field) remain visible to all.
    let mut filter = if is_admin {
        doc! { "is_active": true }
    } else {
        doc! {
            "is_active": true,
            "$or": [
                { "visibility": { "$ne": "private" } },
                { "visibility": { "$exists": false } },
                { "visibility": "private", "created_by": &user_id_str },
            ],
        }
    };
    if let Some(ref category) = query.category {
        let valid = ["provider", "connection", "internal"];
        if !valid.contains(&category.as_str()) {
            return Err(AppError::ValidationError(format!(
                "Invalid category filter: {category}. Must be one of: {}",
                valid.join(", ")
            )));
        }
        filter.insert("service_category", category.as_str());
    }

    let services: Vec<DownstreamService> = state
        .db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find(filter)
        .sort(doc! { "name": 1 })
        .await?
        .try_collect()
        .await?;

    let items: Vec<ServiceResponse> = services.into_iter().map(service_to_response).collect();

    Ok(Json(ServiceListResponse { services: items }))
}

/// POST /api/v1/services
///
/// Register a new downstream service. Requires admin privileges.
#[utoipa::path(
    post,
    path = "/api/v1/services",
    request_body = CreateServiceRequest,
    responses(
        (status = 200, description = "Created downstream service", body = ServiceResponse),
        (status = 400, description = "Validation error", body = crate::errors::ErrorResponse),
        (status = 403, description = "Forbidden", body = crate::errors::ErrorResponse),
        (status = 409, description = "Conflict", body = crate::errors::ErrorResponse)
    ),
    tag = "Services"
)]
pub async fn create_service(
    State(state): State<AppState>,
    auth_user: AuthUser,
    tele: TelemetryContext,
    Json(body): Json<CreateServiceRequest>,
) -> AppResult<Json<ServiceResponse>> {
    if body.name.is_empty() {
        return Err(AppError::ValidationError("name is required".to_string()));
    }

    let service_type = normalize_service_type(body.service_type.as_deref())?;

    // CR-18: Validate input lengths before doing work based on input
    if body.name.len() > 200 {
        return Err(AppError::ValidationError(
            "Input exceeds maximum length".to_string(),
        ));
    }

    // Derive slug from name if not provided (CR-9: collapse consecutive hyphens)
    let slug = body.slug.clone().unwrap_or_else(|| {
        body.name
            .to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '-' })
            .collect::<String>()
            .split('-')
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("-")
    });

    // CR-3: Validate slug is non-empty after derivation
    if slug.is_empty() {
        return Err(AppError::ValidationError(
            "Service name must contain at least one alphanumeric character".to_string(),
        ));
    }

    if slug.len() > 100 {
        return Err(AppError::ValidationError(
            "Slug exceeds maximum length of 100 characters".to_string(),
        ));
    }

    // Check slug uniqueness among active services
    let existing = state
        .db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find_one(doc! { "slug": &slug, "is_active": true })
        .await?;

    if existing.is_some() {
        return Err(AppError::Conflict(
            "A service with this slug already exists".to_string(),
        ));
    }

    let id = Uuid::new_v4().to_string();
    let now = Utc::now();
    let user_id_str = auth_user.user_id.to_string();

    let (
        base_url,
        auth_method,
        auth_type,
        auth_key_name,
        encrypted_cred,
        oauth_client_id,
        openapi_spec_url,
        asyncapi_spec_url,
        streaming_supported,
        ssh_config,
        service_category,
        requires_user_credential,
        token_exchange_config,
    ) = if service_type == "http" {
        if body.ssh_config.is_some() {
            return Err(AppError::ValidationError(
                "ssh_config is only valid when service_type is ssh".to_string(),
            ));
        }

        let base_url = body
            .base_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                AppError::ValidationError(
                    "base_url is required when service_type is http".to_string(),
                )
            })?;
        if base_url.len() > 2048 {
            return Err(AppError::ValidationError(
                "Input exceeds maximum length".to_string(),
            ));
        }

        // Preserve the original auth_type value before mapping
        let auth_type_original = body.auth_method.clone();

        // Map frontend auth_type values to backend auth_method values
        let auth_method = match body.auth_method.as_deref() {
            Some("api_key") => "header".to_string(),
            Some("oauth2") | Some("bearer") => "bearer".to_string(),
            Some("basic") => "basic".to_string(),
            Some("none") => "none".to_string(),
            Some(other) => other.to_string(),
            None => "header".to_string(),
        };

        let auth_key_name =
            body.auth_key_name
                .clone()
                .unwrap_or_else(|| match auth_method.as_str() {
                    "bearer" => "Authorization".to_string(),
                    "bot_bearer" => "Authorization".to_string(),
                    "basic" => "Authorization".to_string(),
                    "query" => "api_key".to_string(),
                    "path" => "bot".to_string(),
                    "none" => String::new(),
                    _ => "X-API-Key".to_string(),
                });

        let credential = body.credential.clone().unwrap_or_default();

        let valid_methods = [
            "header",
            "bearer",
            "bot_bearer",
            "query",
            "basic",
            "body",
            "token_exchange",
            "path",
            "oidc",
            "none",
        ];
        if !valid_methods.contains(&auth_method.as_str()) {
            return Err(AppError::ValidationError(format!(
                "auth_method must be one of: {}",
                valid_methods.join(", ")
            )));
        }

        // `body` auth has no sensible default for the field name -- the
        // proxy needs to know which key to inject into the JSON payload.
        // Fail at creation time instead of surfacing as a 500 on the first
        // proxied request.
        if auth_method == "body" && auth_key_name.is_empty() {
            return Err(AppError::ValidationError(
                "auth_key_name is required when auth_method is 'body' \
                 (e.g. 'app_secret' for custom body-auth services)"
                    .to_string(),
            ));
        }

        // `token_exchange` requires a declarative config at create time --
        // the proxy resolves outbound requests through that config, and a
        // service without one 500s on every proxy call. Admin-provided
        // configs are validated against the generic helpers so typos in
        // the template / injection format surface here, not at runtime.
        let token_exchange_config = if auth_method == "token_exchange" {
            let config = body.token_exchange_config.clone().ok_or_else(|| {
                AppError::ValidationError(
                    "token_exchange auth_method requires token_exchange_config \
                     (endpoint, request_template, token_response_path, injection, \
                     credential_fields)"
                        .to_string(),
                )
            })?;
            validate_token_exchange_config(&config)?;
            if !credential.is_empty() {
                // Round-trip the credential through parse_credential so
                // admins see a clear error if the JSON shape doesn't
                // match the declared fields.
                crate::services::provider_token_exchange_service::parse_credential(
                    &credential,
                    &config.credential_fields,
                )?;
            }
            Some(config)
        } else {
            None
        };

        validate_base_url(base_url)?;

        let (encrypted_cred, oauth_client_id) = if auth_method == "oidc" {
            let callback_url = format!("{}/callback", base_url.trim_end_matches('/'));
            let client_name = format!("{} OIDC Client", body.name);
            let (client, raw_secret) = oauth_client_service::create_client(
                &state.db,
                &client_name,
                &[callback_url],
                "confidential",
                &user_id_str,
                "",
                oauth_client_service::DEFAULT_ALLOWED_SCOPES,
            )
            .await?;

            let secret_to_encrypt = raw_secret.unwrap_or_default();
            let enc = state
                .encryption_keys
                .encrypt(secret_to_encrypt.as_bytes())
                .await?;

            (enc, Some(client.id))
        } else {
            let enc = state.encryption_keys.encrypt(credential.as_bytes()).await?;
            (enc, None)
        };

        let docs_metadata = api_docs_service::discover_service_docs(base_url, None, None).await;
        let service_category =
            derive_http_service_category(&auth_method, body.service_category.as_deref())?;
        let requires_user_credential = service_category == "connection";

        (
            base_url.to_string(),
            auth_method,
            auth_type_original,
            auth_key_name,
            encrypted_cred,
            oauth_client_id,
            docs_metadata.openapi_spec_url,
            docs_metadata.asyncapi_spec_url,
            docs_metadata.streaming_supported,
            None,
            service_category,
            requires_user_credential,
            token_exchange_config,
        )
    } else {
        if body.base_url.is_some()
            || body.auth_method.is_some()
            || body.auth_key_name.is_some()
            || body.credential.is_some()
        {
            return Err(AppError::ValidationError(
                "HTTP-specific fields cannot be used when service_type is ssh".to_string(),
            ));
        }

        let ssh_config = body.ssh_config.as_ref().ok_or_else(|| {
            AppError::ValidationError("ssh_config is required when service_type is ssh".to_string())
        })?;
        let built_ssh_config = ssh_service::build_ssh_config(
            &state.encryption_keys,
            &id,
            None,
            ssh_service::SshConfigInput {
                host: ssh_config.host.as_str(),
                port: ssh_config.port,
                certificate_auth_enabled: ssh_config.certificate_auth_enabled,
                certificate_ttl_minutes: ssh_config.certificate_ttl_minutes,
                allowed_principals: &ssh_config.allowed_principals,
            },
        )
        .await?;
        let service_category = derive_ssh_service_category(body.service_category.as_deref())?;

        (
            ssh_service::target_base_url(&built_ssh_config.host, built_ssh_config.port),
            "none".to_string(),
            Some("ssh".to_string()),
            String::new(),
            state.encryption_keys.encrypt(b"").await?,
            None,
            None,
            None,
            false,
            Some(built_ssh_config),
            service_category,
            false,
            None, // SSH services never use token_exchange
        )
    };

    // Validate metadata URL fields
    for (label, url_opt) in [
        ("homepage_url", &body.homepage_url),
        ("repository_url", &body.repository_url),
        ("issues_url", &body.issues_url),
    ] {
        if let Some(url) = url_opt.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
            validate_optional_spec_url(url).map_err(|_| {
                AppError::ValidationError(format!("{label} must be a valid HTTP(S) URL"))
            })?;
        }
    }
    if let Some(ref notes) = body.auth_notes
        && notes.len() > 4096
    {
        return Err(AppError::ValidationError(
            "auth_notes must not exceed 4096 characters".to_string(),
        ));
    }
    if let Some(ref lim) = body.known_limitations
        && lim.len() > 4096
    {
        return Err(AppError::ValidationError(
            "known_limitations must not exceed 4096 characters".to_string(),
        ));
    }
    if let Some(ref perms) = body.required_permissions {
        if perms.len() > 100 {
            return Err(AppError::ValidationError(
                "required_permissions must not exceed 100 entries".to_string(),
            ));
        }
        for p in perms {
            if p.len() > 256 {
                return Err(AppError::ValidationError(
                    "Each permission must not exceed 256 characters".to_string(),
                ));
            }
        }
    }

    let visibility = derive_visibility(&service_type, body.visibility.as_deref())?;

    // developer_app_ids causes cross-user auto-provisioning -- admin only,
    // and each referenced OAuth client must exist and be active.
    // Only meaningful on private services; on public services the app scoping
    // would be a no-op since public services auto-provision for everyone.
    if let Some(ref app_ids) = body.developer_app_ids {
        if !app_ids.is_empty() && visibility != "private" {
            return Err(AppError::ValidationError(
                "developer_app_ids can only be set on private services".to_string(),
            ));
        }
        validate_developer_app_ids(&state, &auth_user, app_ids).await?;
    }

    // Validate & normalize initial default_request_headers (NyxID#356).
    let default_request_headers = match body.default_request_headers.clone() {
        Some(list) => crate::models::default_request_header::validate_headers(list)?,
        None => None,
    };

    let new_service = DownstreamService {
        id: id.clone(),
        name: body.name.clone(),
        slug: slug.clone(),
        description: body.description.clone(),
        base_url,
        service_type: service_type.clone(),
        visibility,
        auth_method: auth_method.clone(),
        auth_type,
        auth_key_name,
        credential_encrypted: encrypted_cred,
        openapi_spec_url,
        asyncapi_spec_url,
        streaming_supported,
        ssh_config,
        oauth_client_id: oauth_client_id.clone(),
        service_category,
        requires_user_credential,
        is_active: true,
        created_by: user_id_str.clone(),
        identity_propagation_mode: "none".to_string(),
        identity_include_user_id: false,
        identity_include_email: false,
        identity_include_name: false,
        identity_jwt_audience: None,
        forward_access_token: body.forward_access_token,
        inject_delegation_token: false,
        delegation_token_scope: "llm:proxy".to_string(),
        provider_config_id: None,
        homepage_url: body.homepage_url.clone(),
        repository_url: body.repository_url.clone(),
        issues_url: body.issues_url.clone(),
        capabilities: body.capabilities.clone(),
        auth_notes: body.auth_notes.clone(),
        known_limitations: body.known_limitations.clone(),
        required_permissions: body.required_permissions.clone(),
        examples_url: body.examples_url.clone(),
        recommended_skills: body.recommended_skills.clone(),
        custom_user_agent: None,
        default_request_headers,
        developer_app_ids: body.developer_app_ids.clone(),
        token_exchange_config,
        created_at: now,
        updated_at: now,
    };

    state
        .db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .insert_one(&new_service)
        .await?;

    tracing::info!(service_id = %id, name = %body.name, created_by = %auth_user.user_id, "Service created");

    // CR-1: Audit log for service creation
    audit_service::log_async(
        state.db.clone(),
        Some(user_id_str.clone()),
        "service_created".to_string(),
        Some(serde_json::json!({ "service_id": &id, "name": &body.name })),
        None,
        None,
        None,
        None,
    );

    emit_event(
        state.telemetry.as_deref(),
        &auth_user.user_id.to_string(),
        auth_user.api_key_id.as_deref(),
        &tele,
        TelemetryEvent::AdminServiceCreated {
            slug: new_service.slug.clone(),
        },
    );

    Ok(Json(service_to_response(new_service)))
}

/// DELETE /api/v1/services/:service_id
///
/// Deactivate a downstream service. Requires admin or service creator.
#[utoipa::path(
    delete,
    path = "/api/v1/services/{service_id}",
    params(
        ("service_id" = String, Path, description = "Downstream service ID")
    ),
    responses(
        (status = 200, description = "Service deactivated", body = DeleteServiceResponse),
        (status = 403, description = "Forbidden", body = crate::errors::ErrorResponse),
        (status = 404, description = "Service not found", body = crate::errors::ErrorResponse)
    ),
    tag = "Services"
)]
pub async fn delete_service(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(service_id): Path<String>,
) -> AppResult<Json<DeleteServiceResponse>> {
    // CR-4: Use shared require_admin_or_creator helper instead of inline check
    let service = fetch_service(&state, &service_id).await?;
    require_admin_or_creator(&state, &auth_user, &service.created_by).await?;

    let now = Utc::now();
    state
        .db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .update_one(
            doc! { "_id": &service_id },
            doc! { "$set": {
                "is_active": false,
                "updated_at": bson::DateTime::from_chrono(now),
            }},
        )
        .await?;

    // SEC-M2: Cascade deactivation - wipe all user credentials for this service
    use crate::models::user_service_connection::{
        COLLECTION_NAME as CONNECTIONS, UserServiceConnection,
    };
    state
        .db
        .collection::<UserServiceConnection>(CONNECTIONS)
        .update_many(
            doc! { "service_id": &service_id, "is_active": true },
            doc! { "$set": {
                "is_active": false,
                "credential_encrypted": bson::Bson::Null,
                "credential_type": bson::Bson::Null,
                "credential_label": bson::Bson::Null,
                "updated_at": bson::DateTime::from_chrono(now),
            }},
        )
        .await?;

    // OIDC services auto-create an `oauth_clients` row at registration
    // time (see `create_service` -- the `auth_method == "oidc"` branch)
    // and store its id in `service.oauth_client_id`. The OAuth
    // authorize / token paths gate purely on `oauth_clients.is_active`,
    // not on the parent service state, so without this cascade the
    // supposedly-deleted service would keep accepting OAuth flows
    // until someone separately discovered the generated app and
    // deactivated it. The same dangling row would also block
    // `delete_org_user`, which treats `oauth_clients.is_active = true`
    // as a live blocker.
    if let Some(client_id) = service.oauth_client_id.as_deref() {
        state
            .db
            .collection::<OauthClient>(OAUTH_CLIENTS)
            .update_one(
                doc! { "_id": client_id },
                doc! { "$set": {
                    "is_active": false,
                    "updated_at": bson::DateTime::from_chrono(now),
                }},
            )
            .await?;
        tracing::info!(
            service_id = %service_id,
            oauth_client_id = %client_id,
            "OIDC OAuth client deactivated alongside service"
        );
    }

    tracing::info!(service_id = %service_id, deactivated_by = %auth_user.user_id, "Service deactivated");

    // CR-1: Audit log for service deletion
    audit_service::log_async(
        state.db.clone(),
        Some(auth_user.user_id.to_string()),
        "service_deleted".to_string(),
        Some(serde_json::json!({ "service_id": &service_id })),
        None,
        None,
        None,
        None,
    );

    // CR-16: Use typed response struct
    Ok(Json(DeleteServiceResponse {
        message: "Service deactivated".to_string(),
    }))
}

/// GET /api/v1/services/{service_id}
///
/// Get a single service by ID. Requires authentication.
#[utoipa::path(
    get,
    path = "/api/v1/services/{service_id}",
    params(
        ("service_id" = String, Path, description = "Downstream service ID")
    ),
    responses(
        (status = 200, description = "Downstream service", body = ServiceResponse),
        (status = 404, description = "Service not found", body = crate::errors::ErrorResponse)
    ),
    tag = "Services"
)]
pub async fn get_service(
    State(state): State<AppState>,
    _auth_user: AuthUser,
    Path(service_id): Path<String>,
) -> AppResult<Json<ServiceResponse>> {
    let service = fetch_service(&state, &service_id).await?;
    Ok(Json(service_to_response(service)))
}

/// PUT /api/v1/services/{service_id}
///
/// Update a downstream service. Requires admin or original creator.
#[utoipa::path(
    put,
    path = "/api/v1/services/{service_id}",
    params(
        ("service_id" = String, Path, description = "Downstream service ID")
    ),
    request_body = UpdateServiceRequest,
    responses(
        (status = 200, description = "Updated downstream service", body = ServiceResponse),
        (status = 400, description = "Validation error", body = crate::errors::ErrorResponse),
        (status = 403, description = "Forbidden", body = crate::errors::ErrorResponse),
        (status = 404, description = "Service not found", body = crate::errors::ErrorResponse)
    ),
    tag = "Services"
)]
pub async fn update_service(
    State(state): State<AppState>,
    auth_user: AuthUser,
    tele: TelemetryContext,
    Path(service_id): Path<String>,
    Json(body): Json<UpdateServiceRequest>,
) -> AppResult<Json<ServiceResponse>> {
    let service = fetch_service(&state, &service_id).await?;
    require_admin_or_creator(&state, &auth_user, &service.created_by).await?;

    // Build the $set document with only provided fields
    let mut set_doc = doc! {};
    let mut http_docs_refresh: Option<api_docs_service::ServiceDocumentationMetadata> = None;
    let mut explicit_openapi_spec_url: Option<Option<String>> = None;
    let mut explicit_asyncapi_spec_url: Option<Option<String>> = None;

    if let Some(ref name) = body.name {
        if name.is_empty() || name.len() > 200 {
            return Err(AppError::ValidationError(
                "name must be between 1 and 200 characters".to_string(),
            ));
        }
        set_doc.insert("name", name.as_str());
    }

    if let Some(ref description) = body.description {
        if description.len() > 500 {
            return Err(AppError::ValidationError(
                "description must not exceed 500 characters".to_string(),
            ));
        }
        set_doc.insert("description", description.as_str());
    }

    if let Some(is_active) = body.is_active {
        set_doc.insert("is_active", is_active);
    }

    if let Some(ref visibility) = body.visibility {
        match visibility.as_str() {
            "public" | "private" => {
                // Reject changing to public if the service has developer_app_ids
                // (unless they're being cleared in this same request).
                if visibility == "public" {
                    let clearing_app_ids = body
                        .developer_app_ids
                        .as_ref()
                        .is_some_and(|ids| ids.is_empty());
                    let has_app_ids = service
                        .developer_app_ids
                        .as_ref()
                        .is_some_and(|ids| !ids.is_empty());
                    if has_app_ids && !clearing_app_ids {
                        return Err(AppError::ValidationError(
                            "Cannot change visibility to public while developer_app_ids is set. \
                             Clear developer_app_ids first."
                                .to_string(),
                        ));
                    }
                }
                set_doc.insert("visibility", visibility.as_str());
            }
            other => {
                return Err(AppError::ValidationError(format!(
                    "Invalid visibility: {other}. Must be public or private"
                )));
            }
        }
    }

    match service.service_type.as_str() {
        "http" => {
            if body.ssh_config.is_some() {
                return Err(AppError::ValidationError(
                    "ssh_config is only valid for SSH services".to_string(),
                ));
            }

            let refresh_openapi_url = should_refresh_openapi_url(&service, &body);
            let refresh_asyncapi_url = should_refresh_asyncapi_url(&service, &body);

            let openapi_spec_url = body
                .openapi_spec_url
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            let asyncapi_spec_url = body
                .asyncapi_spec_url
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);

            if let Some(ref base_url) = body.base_url {
                validate_base_url(base_url)?;
                if base_url.len() > 2048 {
                    return Err(AppError::ValidationError(
                        "base_url must not exceed 2048 characters".to_string(),
                    ));
                }
                set_doc.insert("base_url", base_url.as_str());
            }

            // Track explicit spec URL values for resolve_spec_url_update,
            // but don't insert into set_doc here -- the http_docs_refresh block
            // below handles the final insert to avoid duplicate BSON keys.
            if body.openapi_spec_url.is_some() {
                explicit_openapi_spec_url = Some(openapi_spec_url.clone());
                if let Some(ref url) = openapi_spec_url {
                    validate_optional_spec_url(url)?;
                }
            }

            if body.asyncapi_spec_url.is_some() {
                explicit_asyncapi_spec_url = Some(asyncapi_spec_url.clone());
                if let Some(ref url) = asyncapi_spec_url {
                    validate_optional_spec_url(url)?;
                }
            }

            if let Some(ref mode) = body.identity_propagation_mode {
                let valid_modes = ["none", "headers", "jwt", "both"];
                if !valid_modes.contains(&mode.as_str()) {
                    return Err(AppError::ValidationError(format!(
                        "identity_propagation_mode must be one of: {}",
                        valid_modes.join(", ")
                    )));
                }
                set_doc.insert("identity_propagation_mode", mode.as_str());
            }
            if let Some(include_uid) = body.identity_include_user_id {
                set_doc.insert("identity_include_user_id", include_uid);
            }
            if let Some(include_email) = body.identity_include_email {
                set_doc.insert("identity_include_email", include_email);
            }
            if let Some(include_name) = body.identity_include_name {
                set_doc.insert("identity_include_name", include_name);
            }
            if let Some(ref audience) = body.identity_jwt_audience {
                if audience.len() > 2048 {
                    return Err(AppError::ValidationError(
                        "identity_jwt_audience must not exceed 2048 characters".to_string(),
                    ));
                }
                set_doc.insert("identity_jwt_audience", audience.as_str());
            }
            if let Some(forward) = body.forward_access_token {
                set_doc.insert("forward_access_token", forward);
            }
            if let Some(inject) = body.inject_delegation_token {
                set_doc.insert("inject_delegation_token", inject);
            }
            if let Some(ref scope) = body.delegation_token_scope {
                let scope = if scope.is_empty() {
                    "llm:proxy"
                } else {
                    scope.as_str()
                };

                let valid_scopes = ["llm:proxy", "proxy:*", "llm:status"];
                for s in scope.split_whitespace() {
                    if !valid_scopes.contains(&s) {
                        return Err(AppError::ValidationError(format!(
                            "Invalid delegation_token_scope '{}'. Must be one of: {}",
                            s,
                            valid_scopes.join(", ")
                        )));
                    }
                }

                set_doc.insert("delegation_token_scope", scope);
            }

            // Replace the declarative token exchange config. Only meaningful
            // on services whose `auth_method` is already `token_exchange`;
            // rejected otherwise so admins don't silently attach config to
            // services that will never read it.
            if let Some(ref new_config) = body.token_exchange_config {
                if service.auth_method != "token_exchange" {
                    return Err(AppError::ValidationError(
                        "token_exchange_config can only be set on services with \
                         auth_method = 'token_exchange'"
                            .to_string(),
                    ));
                }
                validate_token_exchange_config(new_config)?;
                set_doc.insert(
                    "token_exchange_config",
                    bson::to_bson(new_config).map_err(|e| {
                        AppError::Internal(format!("BSON serialization error: {e}"))
                    })?,
                );
            }

            let docs_base_url = body.base_url.as_deref().unwrap_or(&service.base_url);
            let explicit_openapi = if body.openapi_spec_url.is_some() {
                openapi_spec_url.clone()
            } else if refresh_openapi_url {
                None
            } else {
                service.openapi_spec_url.clone()
            };
            let explicit_asyncapi = if body.asyncapi_spec_url.is_some() {
                asyncapi_spec_url.clone()
            } else if refresh_asyncapi_url {
                None
            } else {
                service.asyncapi_spec_url.clone()
            };
            http_docs_refresh = Some(
                api_docs_service::discover_service_docs(
                    docs_base_url,
                    explicit_openapi,
                    explicit_asyncapi,
                )
                .await,
            );
        }
        "ssh" => {
            let has_http_only_updates = body.base_url.is_some()
                || body.openapi_spec_url.is_some()
                || body.asyncapi_spec_url.is_some()
                || body.identity_propagation_mode.is_some()
                || body.identity_include_user_id.is_some()
                || body.identity_include_email.is_some()
                || body.identity_include_name.is_some()
                || body.identity_jwt_audience.is_some()
                || body.forward_access_token.is_some()
                || body.inject_delegation_token.is_some()
                || body.delegation_token_scope.is_some();
            if has_http_only_updates {
                return Err(AppError::ValidationError(
                    "HTTP-specific fields cannot be updated on SSH services".to_string(),
                ));
            }

            if let Some(ref ssh_config) = body.ssh_config {
                let updated_ssh_config = ssh_service::build_ssh_config(
                    &state.encryption_keys,
                    &service_id,
                    service.ssh_config.as_ref(),
                    ssh_service::SshConfigInput {
                        host: ssh_config.host.as_str(),
                        port: ssh_config.port,
                        certificate_auth_enabled: ssh_config.certificate_auth_enabled,
                        certificate_ttl_minutes: ssh_config.certificate_ttl_minutes,
                        allowed_principals: &ssh_config.allowed_principals,
                    },
                )
                .await?;
                set_doc.insert(
                    "ssh_config",
                    bson::to_bson(&updated_ssh_config).map_err(|e| {
                        AppError::Internal(format!("BSON serialization error: {e}"))
                    })?,
                );
                set_doc.insert(
                    "base_url",
                    ssh_service::target_base_url(&updated_ssh_config.host, updated_ssh_config.port),
                );
            }
        }
        other => {
            return Err(AppError::Internal(format!(
                "Unsupported service type: {other}"
            )));
        }
    }

    // Rich metadata fields (apply to all service types).
    // Empty strings clear the field (set to null), matching openapi_spec_url pattern.
    if body.homepage_url.is_some() {
        let val = body
            .homepage_url
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty());
        if let Some(url) = val {
            validate_optional_spec_url(url)?;
            set_doc.insert("homepage_url", url);
        } else {
            set_doc.insert("homepage_url", bson::Bson::Null);
        }
    }
    if body.repository_url.is_some() {
        let val = body
            .repository_url
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty());
        if let Some(url) = val {
            validate_optional_spec_url(url)?;
            set_doc.insert("repository_url", url);
        } else {
            set_doc.insert("repository_url", bson::Bson::Null);
        }
    }
    if body.issues_url.is_some() {
        let val = body
            .issues_url
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty());
        if let Some(url) = val {
            validate_optional_spec_url(url)?;
            set_doc.insert("issues_url", url);
        } else {
            set_doc.insert("issues_url", bson::Bson::Null);
        }
    }
    if let Some(ref caps) = body.capabilities {
        let bson_caps = bson::to_bson(caps)
            .map_err(|e| AppError::Internal(format!("BSON serialization error: {e}")))?;
        set_doc.insert("capabilities", bson_caps);
    }
    if body.auth_notes.is_some() {
        let val = body
            .auth_notes
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty());
        if let Some(notes) = val {
            if notes.len() > 4096 {
                return Err(AppError::ValidationError(
                    "auth_notes must not exceed 4096 characters".to_string(),
                ));
            }
            set_doc.insert("auth_notes", notes);
        } else {
            set_doc.insert("auth_notes", bson::Bson::Null);
        }
    }
    if body.known_limitations.is_some() {
        let val = body
            .known_limitations
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty());
        if let Some(lim) = val {
            if lim.len() > 4096 {
                return Err(AppError::ValidationError(
                    "known_limitations must not exceed 4096 characters".to_string(),
                ));
            }
            set_doc.insert("known_limitations", lim);
        } else {
            set_doc.insert("known_limitations", bson::Bson::Null);
        }
    }
    if let Some(ref perms) = body.required_permissions {
        if perms.len() > 100 {
            return Err(AppError::ValidationError(
                "required_permissions must not exceed 100 entries".to_string(),
            ));
        }
        for p in perms {
            if p.len() > 256 {
                return Err(AppError::ValidationError(
                    "Each permission must not exceed 256 characters".to_string(),
                ));
            }
        }
        let bson_perms = bson::to_bson(perms)
            .map_err(|e| AppError::Internal(format!("BSON serialization error: {e}")))?;
        set_doc.insert("required_permissions", bson_perms);
    }
    if body.examples_url.is_some() {
        let val = body
            .examples_url
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty());
        if let Some(url) = val {
            validate_optional_spec_url(url)?;
            set_doc.insert("examples_url", url);
        } else {
            set_doc.insert("examples_url", bson::Bson::Null);
        }
    }
    if let Some(ref skills) = body.recommended_skills {
        if skills.len() > 50 {
            return Err(AppError::ValidationError(
                "recommended_skills must not exceed 50 entries".to_string(),
            ));
        }
        let bson_skills = bson::to_bson(skills)
            .map_err(|e| AppError::Internal(format!("BSON serialization error: {e}")))?;
        set_doc.insert("recommended_skills", bson_skills);
    }
    if let Some(ref app_ids) = body.developer_app_ids {
        // Effective visibility: use the value being set in this request,
        // or fall back to the service's current visibility.
        let effective_visibility = body.visibility.as_deref().unwrap_or(&service.visibility);
        if !app_ids.is_empty() && effective_visibility != "private" {
            return Err(AppError::ValidationError(
                "developer_app_ids can only be set on private services".to_string(),
            ));
        }
        validate_developer_app_ids(&state, &auth_user, app_ids).await?;
        let bson_ids = bson::to_bson(app_ids)
            .map_err(|e| AppError::Internal(format!("BSON serialization error: {e}")))?;
        set_doc.insert("developer_app_ids", bson_ids);
    }
    if let Some(ref ua) = body.custom_user_agent {
        let trimmed = ua.trim();
        if trimmed.is_empty() {
            set_doc.insert("custom_user_agent", bson::Bson::Null);
        } else {
            if trimmed.len() > 256 {
                return Err(AppError::ValidationError(
                    "custom_user_agent must not exceed 256 characters".to_string(),
                ));
            }
            if trimmed.bytes().any(|b| b < 0x20 && b != b'\t') {
                return Err(AppError::ValidationError(
                    "custom_user_agent must not contain control characters".to_string(),
                ));
            }
            set_doc.insert("custom_user_agent", trimmed);
        }
    }

    // default_request_headers (NyxID#356):
    //   None         => leave unchanged
    //   Some(None)   => explicit clear (Bson::Null)
    //   Some(Some()) => reconcile redaction placeholders against the
    //                    currently stored list, validate, then replace
    let default_headers_changed = body.default_request_headers.is_some();
    let default_headers_payload_names: Vec<String> = match &body.default_request_headers {
        Some(Some(list)) => list.iter().map(|h| h.name.clone()).collect(),
        _ => Vec::new(),
    };
    if let Some(drh) = body.default_request_headers.clone() {
        match drh {
            Some(list) => {
                // Before validation, restore stored values for any
                // entries the client submitted with the
                // `REDACTED_PLACEHOLDER`. Without this, a GET → edit →
                // PUT round trip would overwrite every sensitive value
                // with the literal placeholder string.
                let reconciled = crate::models::default_request_header::reconcile_with_stored(
                    list,
                    service.default_request_headers.as_deref(),
                );
                let normalized =
                    crate::models::default_request_header::validate_headers(reconciled)?;
                match normalized {
                    Some(norm) => {
                        let bson_val = bson::to_bson(&norm).map_err(|e| {
                            AppError::Internal(format!(
                                "Failed to serialize default_request_headers: {e}"
                            ))
                        })?;
                        set_doc.insert("default_request_headers", bson_val);
                    }
                    None => {
                        set_doc.insert("default_request_headers", bson::Bson::Null);
                    }
                }
            }
            None => {
                set_doc.insert("default_request_headers", bson::Bson::Null);
            }
        }
    }

    if set_doc.is_empty() {
        return Err(AppError::ValidationError(
            "At least one field must be provided for update".to_string(),
        ));
    }

    let now = Utc::now();
    set_doc.insert("updated_at", bson::DateTime::from_chrono(now));
    if let Some(docs_metadata) = http_docs_refresh {
        let refresh_openapi_url = should_refresh_openapi_url(&service, &body);
        let refresh_asyncapi_url = should_refresh_asyncapi_url(&service, &body);
        let next_openapi_spec_url = resolve_spec_url_update(
            explicit_openapi_spec_url,
            service.openapi_spec_url.as_ref(),
            docs_metadata.openapi_spec_url.as_ref(),
            refresh_openapi_url,
        );
        let next_asyncapi_spec_url = resolve_spec_url_update(
            explicit_asyncapi_spec_url,
            service.asyncapi_spec_url.as_ref(),
            docs_metadata.asyncapi_spec_url.as_ref(),
            refresh_asyncapi_url,
        );
        set_doc.insert(
            "openapi_spec_url",
            next_openapi_spec_url
                .clone()
                .map_or(bson::Bson::Null, bson::Bson::String),
        );
        set_doc.insert(
            "asyncapi_spec_url",
            next_asyncapi_spec_url
                .clone()
                .map_or(bson::Bson::Null, bson::Bson::String),
        );
        if refresh_openapi_url || refresh_asyncapi_url {
            set_doc.insert("streaming_supported", docs_metadata.streaming_supported);
        }
    }

    // Always unset the legacy api_spec_url alias to prevent duplicate-field
    // deserialization errors on documents created before the field was renamed.
    state
        .db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .update_one(
            doc! { "_id": &service_id },
            doc! {
                "$set": &set_doc,
                "$unset": { "api_spec_url": "" },
            },
        )
        .await?;

    // Propagate identity/forward_access_token changes to all UserService records
    // that were provisioned from this catalog service (auto or manual).
    let identity_changed = body.identity_propagation_mode.is_some()
        || body.identity_include_user_id.is_some()
        || body.identity_include_email.is_some()
        || body.identity_include_name.is_some()
        || body.identity_jwt_audience.is_some()
        || body.forward_access_token.is_some()
        || body.inject_delegation_token.is_some()
        || body.delegation_token_scope.is_some();

    if identity_changed {
        let mut user_svc_set = bson::Document::new();
        if let Some(ref mode) = body.identity_propagation_mode {
            user_svc_set.insert("identity_propagation_mode", mode.as_str());
        }
        if let Some(v) = body.identity_include_user_id {
            user_svc_set.insert("identity_include_user_id", v);
        }
        if let Some(v) = body.identity_include_email {
            user_svc_set.insert("identity_include_email", v);
        }
        if let Some(v) = body.identity_include_name {
            user_svc_set.insert("identity_include_name", v);
        }
        if let Some(ref aud) = body.identity_jwt_audience {
            user_svc_set.insert("identity_jwt_audience", aud.as_str());
        }
        if let Some(v) = body.forward_access_token {
            user_svc_set.insert("forward_access_token", v);
        }
        if let Some(v) = body.inject_delegation_token {
            user_svc_set.insert("inject_delegation_token", v);
        }
        if let Some(ref scope) = body.delegation_token_scope {
            let scope = if scope.is_empty() {
                "llm:proxy"
            } else {
                scope.as_str()
            };
            user_svc_set.insert("delegation_token_scope", scope);
        }

        if !user_svc_set.is_empty() {
            user_svc_set.insert("updated_at", bson::DateTime::from_chrono(Utc::now()));
            let db = state.db.clone();
            let sid = service_id.clone();
            tokio::spawn(async move {
                match db
                    .collection::<crate::models::user_service::UserService>(
                        crate::models::user_service::COLLECTION_NAME,
                    )
                    .update_many(
                        doc! { "catalog_service_id": &sid },
                        doc! { "$set": &user_svc_set },
                    )
                    .await
                {
                    Ok(result) => {
                        if result.modified_count > 0 {
                            tracing::info!(
                                catalog_service_id = %sid,
                                modified = result.modified_count,
                                "Propagated identity config to user services"
                            );
                        }
                    }
                    Err(e) => {
                        tracing::error!(
                            catalog_service_id = %sid,
                            error = %e,
                            "Failed to propagate identity config to user services"
                        );
                    }
                }
            });
        }
    }

    // If base_url changed and service has an OIDC client, update default redirect URI
    if service.service_type == "http"
        && let (Some(new_base_url), Some(oauth_client_id)) =
            (&body.base_url, &service.oauth_client_id)
    {
        let new_callback = format!("{}/callback", new_base_url.trim_end_matches('/'));
        oauth_client_service::update_redirect_uris(&state.db, oauth_client_id, &[new_callback])
            .await?;
    }

    tracing::info!(service_id = %service_id, updated_by = %auth_user.user_id, "Service updated");

    // CR-1: Audit log for service update
    audit_service::log_async(
        state.db.clone(),
        Some(auth_user.user_id.to_string()),
        "service_updated".to_string(),
        Some(serde_json::json!({ "service_id": &service_id })),
        None,
        None,
        None,
        None,
    );

    // Per-change audit for default_request_headers mutations. Values are
    // deliberately *never* logged — only the set of header names — so even
    // misconfigured "sensitive" defaults don't leak into the audit store.
    if default_headers_changed {
        audit_service::log_async(
            state.db.clone(),
            Some(auth_user.user_id.to_string()),
            "service_default_headers_updated".to_string(),
            Some(serde_json::json!({
                "service_id": &service_id,
                "header_names": default_headers_payload_names,
            })),
            None,
            None,
            None,
            None,
        );
    }

    // Re-fetch the updated service to return fresh data
    let updated = fetch_service(&state, &service_id).await?;

    emit_event(
        state.telemetry.as_deref(),
        &auth_user.user_id.to_string(),
        auth_user.api_key_id.as_deref(),
        &tele,
        TelemetryEvent::AdminServiceUpdated {
            slug: updated.slug.clone(),
        },
    );

    Ok(Json(service_to_response(updated)))
}

/// GET /api/v1/services/{service_id}/oidc-credentials
///
/// Retrieve OIDC client credentials. Admin only.
#[utoipa::path(
    get,
    path = "/api/v1/services/{service_id}/oidc-credentials",
    params(
        ("service_id" = String, Path, description = "Downstream service ID")
    ),
    responses(
        (status = 200, description = "OIDC client credentials", body = OidcCredentialsResponse),
        (status = 400, description = "Service is not an OIDC service", body = crate::errors::ErrorResponse),
        (status = 403, description = "Forbidden", body = crate::errors::ErrorResponse),
        (status = 404, description = "Service not found", body = crate::errors::ErrorResponse)
    ),
    tag = "Services"
)]
pub async fn get_oidc_credentials(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(service_id): Path<String>,
) -> AppResult<Json<OidcCredentialsResponse>> {
    let service = fetch_service(&state, &service_id).await?;
    require_admin_or_creator(&state, &auth_user, &service.created_by).await?;

    if service.auth_method != "oidc" {
        return Err(AppError::BadRequest(
            "Service is not an OIDC service".to_string(),
        ));
    }

    let oauth_client_id = service
        .oauth_client_id
        .ok_or_else(|| AppError::Internal("OIDC service missing oauth_client_id".to_string()))?;

    // Decrypt the client secret from credential_encrypted
    let decrypted_bytes = state
        .encryption_keys
        .decrypt(&service.credential_encrypted)
        .await?;
    let client_secret = String::from_utf8(decrypted_bytes)
        .map_err(|e| AppError::Internal(format!("Failed to decode decrypted secret: {e}")))?;

    // Fetch the OAuth client for redirect URIs and scopes
    let oauth_client = oauth_client_service::get_client(&state.db, &oauth_client_id).await?;

    // CR-7: redirect_uris is now Vec<String> on the model, no deserialization needed
    let redirect_uris = oauth_client.redirect_uris;

    // Build OIDC discovery endpoints from config base_url
    let base = state.config.base_url.trim_end_matches('/');

    tracing::info!(
        service_id = %service_id,
        accessed_by = %auth_user.user_id,
        "OIDC credentials accessed"
    );

    // CR-1/SEC-4: Audit log for credential access
    audit_service::log_async(
        state.db.clone(),
        Some(auth_user.user_id.to_string()),
        "oidc_credentials_accessed".to_string(),
        Some(serde_json::json!({ "service_id": &service_id })),
        None,
        None,
        None,
        None,
    );

    Ok(Json(OidcCredentialsResponse {
        client_id: oauth_client_id,
        client_secret,
        redirect_uris,
        allowed_scopes: oauth_client.allowed_scopes,
        delegation_scopes: oauth_client.delegation_scopes,
        issuer: state.config.jwt_issuer.clone(),
        authorization_endpoint: format!("{base}/oauth/authorize"),
        token_endpoint: format!("{base}/oauth/token"),
        userinfo_endpoint: format!("{base}/oauth/userinfo"),
        jwks_uri: format!("{base}/.well-known/jwks.json"),
    }))
}

/// PUT /api/v1/services/{service_id}/redirect-uris
///
/// Update redirect URIs for an OIDC service. Admin only.
#[utoipa::path(
    put,
    path = "/api/v1/services/{service_id}/redirect-uris",
    params(
        ("service_id" = String, Path, description = "Downstream service ID")
    ),
    request_body = UpdateRedirectUrisRequest,
    responses(
        (status = 200, description = "Updated redirect URIs", body = RedirectUrisResponse),
        (status = 400, description = "Validation error", body = crate::errors::ErrorResponse),
        (status = 403, description = "Forbidden", body = crate::errors::ErrorResponse),
        (status = 404, description = "Service not found", body = crate::errors::ErrorResponse)
    ),
    tag = "Services"
)]
pub async fn update_redirect_uris(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(service_id): Path<String>,
    Json(body): Json<UpdateRedirectUrisRequest>,
) -> AppResult<Json<RedirectUrisResponse>> {
    let service = fetch_service(&state, &service_id).await?;
    require_admin_or_creator(&state, &auth_user, &service.created_by).await?;

    if service.auth_method != "oidc" {
        return Err(AppError::BadRequest(
            "Service is not an OIDC service".to_string(),
        ));
    }

    let oauth_client_id = service
        .oauth_client_id
        .ok_or_else(|| AppError::Internal("OIDC service missing oauth_client_id".to_string()))?;

    if body.redirect_uris.is_empty() {
        return Err(AppError::ValidationError(
            "At least one redirect URI is required".to_string(),
        ));
    }

    // CR-13/SEC-8: Limit count and length of redirect URIs
    if body.redirect_uris.len() > 10 {
        return Err(AppError::ValidationError(
            "Maximum 10 redirect URIs allowed".to_string(),
        ));
    }

    // Validate each URI (SEC-1: restrict to http/https schemes)
    for uri in &body.redirect_uris {
        if uri.len() > 2048 {
            return Err(AppError::ValidationError(
                "Redirect URI exceeds max length of 2048 characters".to_string(),
            ));
        }
        let parsed = url::Url::parse(uri)
            .map_err(|_| AppError::ValidationError(format!("Invalid redirect URI: {uri}")))?;
        let scheme = parsed.scheme();
        if scheme != "https" && scheme != "http" {
            return Err(AppError::ValidationError(format!(
                "Redirect URI must use https or http scheme: {uri}"
            )));
        }
    }

    oauth_client_service::update_redirect_uris(&state.db, &oauth_client_id, &body.redirect_uris)
        .await?;

    // Touch updated_at on the service
    let now = Utc::now();
    state
        .db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .update_one(
            doc! { "_id": &service_id },
            doc! { "$set": { "updated_at": bson::DateTime::from_chrono(now) } },
        )
        .await?;

    tracing::info!(
        service_id = %service_id,
        updated_by = %auth_user.user_id,
        "Redirect URIs updated"
    );

    // CR-1: Audit log for redirect URI update
    audit_service::log_async(
        state.db.clone(),
        Some(auth_user.user_id.to_string()),
        "redirect_uris_updated".to_string(),
        Some(serde_json::json!({ "service_id": &service_id })),
        None,
        None,
        None,
        None,
    );

    Ok(Json(RedirectUrisResponse {
        redirect_uris: body.redirect_uris,
    }))
}

/// POST /api/v1/services/{service_id}/regenerate-secret
///
/// Regenerate the OIDC client secret. Admin only.
#[utoipa::path(
    post,
    path = "/api/v1/services/{service_id}/regenerate-secret",
    params(
        ("service_id" = String, Path, description = "Downstream service ID")
    ),
    responses(
        (status = 200, description = "Regenerated OIDC client secret", body = RegenerateSecretResponse),
        (status = 400, description = "Service is not an OIDC service", body = crate::errors::ErrorResponse),
        (status = 403, description = "Forbidden", body = crate::errors::ErrorResponse),
        (status = 404, description = "Service not found", body = crate::errors::ErrorResponse)
    ),
    tag = "Services"
)]
pub async fn regenerate_oidc_secret(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(service_id): Path<String>,
) -> AppResult<Json<RegenerateSecretResponse>> {
    let service = fetch_service(&state, &service_id).await?;
    require_admin_or_creator(&state, &auth_user, &service.created_by).await?;

    if service.auth_method != "oidc" {
        return Err(AppError::BadRequest(
            "Service is not an OIDC service".to_string(),
        ));
    }

    let oauth_client_id = service
        .oauth_client_id
        .ok_or_else(|| AppError::Internal("OIDC service missing oauth_client_id".to_string()))?;

    // Generate a new secret
    let new_secret = generate_random_token();
    let new_hash = hash_token(&new_secret);

    // SEC-5: These two updates are not wrapped in a MongoDB transaction. If the
    // server crashes after the first update but before the second, the system will
    // be in an inconsistent state (new hash stored but old encrypted secret remains).
    // A MongoDB multi-document transaction requires a replica set or sharded cluster.
    // TODO: Use a MongoDB session with start_transaction/commit_transaction when
    // running on a replica set.

    // Update the OauthClient with the new hash
    let now = Utc::now();
    state
        .db
        .collection::<OauthClient>(OAUTH_CLIENTS)
        .update_one(
            doc! { "_id": &oauth_client_id },
            doc! { "$set": {
                "client_secret_hash": &new_hash,
                "updated_at": bson::DateTime::from_chrono(now),
            }},
        )
        .await?;

    // Encrypt the new secret and update credential_encrypted on the service
    let encrypted = state.encryption_keys.encrypt(new_secret.as_bytes()).await?;

    state
        .db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .update_one(
            doc! { "_id": &service_id },
            doc! { "$set": {
                "credential_encrypted": bson::Binary { subtype: bson::spec::BinarySubtype::Generic, bytes: encrypted },
                "updated_at": bson::DateTime::from_chrono(now),
            }},
        )
        .await?;

    tracing::info!(
        service_id = %service_id,
        regenerated_by = %auth_user.user_id,
        "OIDC client secret regenerated"
    );

    // CR-1/SEC-4: Audit log for secret regeneration
    audit_service::log_async(
        state.db.clone(),
        Some(auth_user.user_id.to_string()),
        "oidc_secret_regenerated".to_string(),
        Some(serde_json::json!({ "service_id": &service_id })),
        None,
        None,
        None,
        None,
    );

    Ok(Json(RegenerateSecretResponse {
        client_secret: new_secret,
        message: "Previous secret is now invalidated. Store this secret securely.".to_string(),
    }))
}

#[cfg(test)]
mod tests {
    use super::{UpdateServiceRequest, resolve_spec_url_update};

    // Three wire shapes on the update request need to stay distinguishable
    // so the admin can clear the field without an empty-array workaround:
    //   omitted   -> None                 (leave unchanged)
    //   null      -> Some(None)           (explicit clear)
    //   array     -> Some(Some(vec![...]))(replace)
    // A plain `Option<Option<_>>` collapses omitted and null to None, so
    // we rely on `nullable_field::deserialize` for the outer layer.
    #[test]
    fn update_service_default_request_headers_tri_state_deser() {
        let omitted: UpdateServiceRequest = serde_json::from_str("{}").expect("parse");
        assert!(omitted.default_request_headers.is_none());

        let null_body: UpdateServiceRequest =
            serde_json::from_str(r#"{"default_request_headers": null}"#).expect("parse");
        assert_eq!(null_body.default_request_headers, Some(None));

        let array_body: UpdateServiceRequest = serde_json::from_str(
            r#"{"default_request_headers": [{"name":"x-scope","value":"a","overridable":false,"sensitive":false}]}"#,
        )
        .expect("parse");
        match array_body.default_request_headers {
            Some(Some(list)) => {
                assert_eq!(list.len(), 1);
                assert_eq!(list[0].name, "x-scope");
                assert_eq!(list[0].value, "a");
            }
            other => panic!("expected Some(Some(_)), got {other:?}"),
        }

        let empty_body: UpdateServiceRequest =
            serde_json::from_str(r#"{"default_request_headers": []}"#).expect("parse");
        match empty_body.default_request_headers {
            Some(Some(list)) => assert!(list.is_empty()),
            other => panic!("expected Some(Some([])), got {other:?}"),
        }
    }

    #[test]
    fn resolve_spec_url_update_prefers_explicit_url() {
        let current = "https://current.example/openapi.json".to_string();
        let discovered = "https://discovered.example/openapi.json".to_string();

        assert_eq!(
            resolve_spec_url_update(
                Some(Some("https://admin.example/openapi.json".to_string())),
                Some(&current),
                Some(&discovered),
                true,
            ),
            Some("https://admin.example/openapi.json".to_string())
        );
    }

    #[test]
    fn resolve_spec_url_update_respects_explicit_clear() {
        let current = "https://current.example/openapi.json".to_string();
        let discovered = "https://discovered.example/openapi.json".to_string();

        assert_eq!(
            resolve_spec_url_update(Some(None), Some(&current), Some(&discovered), true),
            None
        );
    }

    #[test]
    fn resolve_spec_url_update_uses_discovered_url_for_refreshes() {
        let current = "https://current.example/openapi.json".to_string();
        let discovered = "https://discovered.example/openapi.json".to_string();

        assert_eq!(
            resolve_spec_url_update(None, Some(&current), Some(&discovered), true),
            Some("https://discovered.example/openapi.json".to_string())
        );
    }

    // ─── validate_token_exchange_config ──────────────────────────────

    use super::validate_token_exchange_config;
    use crate::models::downstream_service::{CredentialFieldSpec, TokenExchangeConfig};

    fn lark_config() -> TokenExchangeConfig {
        TokenExchangeConfig {
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
        }
    }

    #[test]
    fn validate_token_exchange_config_accepts_lark() {
        validate_token_exchange_config(&lark_config()).expect("lark config must be valid");
    }

    #[test]
    fn validate_token_exchange_config_rejects_empty_endpoint() {
        let mut config = lark_config();
        config.endpoint = "".to_string();
        assert!(validate_token_exchange_config(&config).is_err());
    }

    #[test]
    fn validate_token_exchange_config_rejects_unknown_encoding() {
        let mut config = lark_config();
        config.request_encoding = "xml".to_string();
        assert!(validate_token_exchange_config(&config).is_err());
    }

    #[test]
    fn validate_token_exchange_config_rejects_non_object_template() {
        let mut config = lark_config();
        config.request_template = serde_json::json!("not an object");
        assert!(validate_token_exchange_config(&config).is_err());
    }

    #[test]
    fn validate_token_exchange_config_rejects_unknown_injection() {
        let mut config = lark_config();
        config.injection = "weird".to_string();
        assert!(validate_token_exchange_config(&config).is_err());
    }

    #[test]
    fn validate_token_exchange_config_accepts_custom_header_injection() {
        let mut config = lark_config();
        config.injection = "header:X-Api-Key".to_string();
        validate_token_exchange_config(&config).expect("custom header must be valid");
    }

    #[test]
    fn validate_token_exchange_config_rejects_empty_header_name() {
        let mut config = lark_config();
        config.injection = "header:".to_string();
        assert!(validate_token_exchange_config(&config).is_err());
    }

    #[test]
    fn validate_token_exchange_config_rejects_empty_token_path() {
        let mut config = lark_config();
        config.token_response_path = "".to_string();
        assert!(validate_token_exchange_config(&config).is_err());
    }

    #[test]
    fn validate_token_exchange_config_rejects_too_short_ttl() {
        let mut config = lark_config();
        config.default_ttl_secs = 30;
        assert!(validate_token_exchange_config(&config).is_err());
    }

    #[test]
    fn validate_token_exchange_config_rejects_empty_credential_fields() {
        let mut config = lark_config();
        config.credential_fields = vec![];
        assert!(validate_token_exchange_config(&config).is_err());
    }

    #[test]
    fn validate_token_exchange_config_rejects_duplicate_field_names() {
        let mut config = lark_config();
        config.credential_fields = vec![
            CredentialFieldSpec {
                name: "app_id".to_string(),
                label: "App ID".to_string(),
                placeholder: None,
                secret: false,
            },
            CredentialFieldSpec {
                name: "app_id".to_string(),
                label: "App ID again".to_string(),
                placeholder: None,
                secret: false,
            },
        ];
        assert!(validate_token_exchange_config(&config).is_err());
    }

    #[test]
    fn validate_token_exchange_config_rejects_undeclared_template_placeholder() {
        // Regression: a typo like `$app_scret` should fail admin validation
        // rather than producing a runtime 500 on every proxy call.
        let mut config = lark_config();
        config.request_template = serde_json::json!({
            "app_id": "$app_id",
            "app_secret": "$app_scret", // typo!
        });
        let err = validate_token_exchange_config(&config).unwrap_err();
        assert!(err.to_string().contains("app_scret"));
    }

    #[test]
    fn validate_token_exchange_config_checks_placeholders_in_nested_template() {
        // The walker must recurse into nested objects and arrays.
        let mut config = lark_config();
        config.request_template = serde_json::json!({
            "top": {
                "nested_array": ["$missing"],
            }
        });
        config.credential_fields = vec![CredentialFieldSpec {
            name: "top".to_string(),
            label: "top".to_string(),
            placeholder: None,
            secret: false,
        }];
        let err = validate_token_exchange_config(&config).unwrap_err();
        assert!(err.to_string().contains("missing"));
    }
}
