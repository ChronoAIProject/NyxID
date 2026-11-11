//! Who may write providers and catalog services, and what a Catalog Manager
//! may change.
//!
//! Admins keep full authority. An Operator holding the Catalog Manager role
//! may create providers and HTTP catalog services and edit the rows they
//! created. On shared catalog rows they did not create, including seeded
//! ones, they may edit a provider's scopes, URLs and OAuth client and a
//! service's descriptive fields and operations. An OAuth client someone else
//! set is protected: changing where it is sent, or replacing it, requires
//! re-entering that client. The field lists below destructure every request
//! field, so adding a field fails to compile until it is classified here.

use mongodb::bson::doc;
use subtle::ConstantTimeEq;

use crate::AppState;
use crate::crypto::aes::EncryptionKeys;
use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::DownstreamService;
use crate::models::provider_config::ProviderConfig;
use crate::models::user::{COLLECTION_NAME as USERS, User};
use crate::mw::auth::AuthUser;
use crate::services::{provider_service, role_service};

use super::providers::{CreateProviderRequest, UpdateProviderRequest};
use super::services::{CreateServiceRequest, UpdateServiceRequest};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CatalogAuthority {
    Admin,
    CatalogManager,
    ToolsManager,
}

impl CatalogAuthority {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::CatalogManager => "catalog_manager",
            Self::ToolsManager => "tools_manager",
        }
    }
}

/// Resolve the caller's catalog write authority, or `None` when they have none.
pub async fn catalog_authority(
    state: &AppState,
    auth_user: &AuthUser,
) -> AppResult<Option<CatalogAuthority>> {
    let user_id = auth_user.user_id.to_string();
    let user = state
        .db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": &user_id })
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".to_string()))?;
    let platform_role = role_service::resolve_platform_role(&state.db, &user).await?;
    if platform_role.is_admin() {
        return Ok(Some(CatalogAuthority::Admin));
    }
    if role_service::can_manage_catalog(&state.db, &user, platform_role).await? {
        return Ok(Some(CatalogAuthority::CatalogManager));
    }
    Ok(None)
}

pub async fn require_catalog_writer(
    state: &AppState,
    auth_user: &AuthUser,
) -> AppResult<CatalogAuthority> {
    super::login_client_context::require_first_party_human(auth_user)?;
    catalog_authority(state, auth_user)
        .await?
        .ok_or_else(|| AppError::Forbidden("Admin or Catalog Manager access required".to_string()))
}

/// Gate for writes to a catalog row or its operations, applied after the
/// creator check. Rows a Catalog Manager created need catalog authority, so a
/// creator who loses the role loses write access. Every other row keeps the
/// plain creator rule unchanged and returns `None`.
pub async fn require_public_row_writer(
    state: &AppState,
    auth_user: &AuthUser,
    service: &DownstreamService,
) -> AppResult<Option<CatalogAuthority>> {
    if !service.catalog_manager_created || service.visibility == "private" {
        return Ok(None);
    }
    require_catalog_writer(state, auth_user).await.map(Some)
}

/// Write gate for a catalog row and its operations. Admins pass. A Catalog
/// Manager passes on every shared catalog row and on rows they created;
/// callers then apply the Catalog Manager field rules. Anyone else keeps the
/// creator rule: `None` means a creator without catalog authority writing a
/// row that predates the role.
pub async fn require_catalog_row_writer(
    state: &AppState,
    auth_user: &AuthUser,
    service: &DownstreamService,
) -> AppResult<Option<CatalogAuthority>> {
    if service.catalog_manager_created && service.visibility != "private" {
        super::login_client_context::require_first_party_human(auth_user)?;
    }
    if service.owner_user_id.is_some() {
        super::services_helpers::require_admin(state, auth_user).await?;
        return Ok(Some(CatalogAuthority::Admin));
    }
    let authority = catalog_authority(state, auth_user).await?;
    match authority {
        Some(CatalogAuthority::Admin) => return Ok(authority),
        Some(CatalogAuthority::CatalogManager)
            if crate::services::catalog_mutation_service::is_shared_catalog_row(service) =>
        {
            super::login_client_context::require_first_party_human(auth_user)?;
            return Ok(authority);
        }
        _ => {}
    }
    if service.created_by != auth_user.user_id.to_string() {
        return Err(AppError::Forbidden(
            "Only admins or the service creator can perform this action".to_string(),
        ));
    }
    if service.catalog_manager_created && service.visibility != "private" {
        return authority.map(Some).ok_or_else(|| {
            AppError::Forbidden("Admin or Catalog Manager access required".to_string())
        });
    }
    Ok(None)
}

/// Admin-only sub-resources of a catalog row (for example anonymous no-auth
/// proxy rules): a creator without Admin rights may not write them on a
/// public row.
pub async fn deny_catalog_manager(
    state: &AppState,
    auth_user: &AuthUser,
    service: &DownstreamService,
    what: &str,
) -> AppResult<()> {
    if require_public_row_writer(state, auth_user, service).await?
        == Some(CatalogAuthority::CatalogManager)
    {
        return Err(AppError::Forbidden(format!(
            "A Catalog Manager cannot change {what}; ask an Admin"
        )));
    }
    Ok(())
}

/// A Catalog Manager may move a row's destination only while nothing an
/// Admin attached would follow it: no shared credential, no platform key,
/// and no provider someone else created.
pub async fn check_destination_change(
    state: &AppState,
    caller_id: &str,
    service: &DownstreamService,
    base_url: Option<&str>,
) -> AppResult<()> {
    if base_url.is_none_or(|url| url == service.base_url) {
        return Ok(());
    }
    if !service.credential_encrypted.is_empty() || service.platform_key.is_some() {
        return Err(AppError::Forbidden(
            "This service carries an Admin-set credential, so only an Admin can change base_url"
                .to_string(),
        ));
    }
    if let Some(provider_id) = service.provider_config_id.as_deref() {
        check_owned_provider(state, caller_id, provider_id)
            .await
            .map_err(|_| {
                AppError::Forbidden(
                    "This service uses a provider you did not create, so only an Admin can change base_url"
                        .to_string(),
                )
            })?;
    }
    Ok(())
}

fn forbid_fields(action: &str, fields: Vec<&'static str>) -> AppResult<()> {
    if fields.is_empty() {
        return Ok(());
    }
    Err(AppError::Forbidden(format!(
        "A Catalog Manager cannot {action} these fields; ask an Admin: {}",
        fields.join(", ")
    )))
}

/// Catalog Managers may create OAuth, device-code and API-key providers.
pub fn check_provider_create(body: &CreateProviderRequest) -> AppResult<()> {
    if matches!(
        body.provider_type.as_str(),
        "oauth2" | "device_code" | "api_key"
    ) {
        return Ok(());
    }
    Err(AppError::Forbidden(format!(
        "A Catalog Manager cannot create {} providers; ask an Admin",
        body.provider_type
    )))
}

/// A Catalog Manager may edit providers they created. On any other provider
/// they may edit the name, description, scopes, documentation, API key
/// instructions and link, and icon. The
/// URLs, credential mode and token request settings decide where client
/// secrets, authorization codes and users' tokens are sent, so a Catalog
/// Manager may change them only before anyone has connected through the
/// provider, and, once someone has set an OAuth client, only by re-entering
/// that client in the same request. Replacing someone else's client, clearing
/// it, and deactivating the provider stay with Admins.
pub async fn check_provider_update(
    state: &AppState,
    caller_id: &str,
    provider: &ProviderConfig,
    body: &UpdateProviderRequest,
) -> AppResult<()> {
    if provider.created_by == caller_id {
        return Ok(());
    }
    let ProviderUpdateFields {
        admin_only,
        client_bound,
    } = provider_update_fields(body);
    forbid_fields("change on a provider they did not create", admin_only)?;
    if !client_bound.is_empty() && provider_has_connections(&state.db, &provider.id).await? {
        return Err(AppError::Forbidden(format!(
            "People have connected through this provider, so their own OAuth clients and tokens follow these settings. A Catalog Manager cannot change {}; ask an Admin",
            client_bound.join(", ")
        )));
    }
    let encryption_keys = state.encryption_keys.as_ref();

    let sets_client = body.client_id.is_some() || body.client_secret.is_some();
    let has_client =
        provider.client_id_encrypted.is_some() || provider.client_secret_encrypted.is_some();
    if has_client {
        if (sets_client || !client_bound.is_empty())
            && !reenters_current_client(encryption_keys, provider, body).await?
        {
            return Err(AppError::Forbidden(format!(
                "This provider already has an OAuth client. A Catalog Manager can change {} only by re-entering its current client_id and client_secret, and cannot replace it; ask an Admin",
                if client_bound.is_empty() {
                    "the client".to_string()
                } else {
                    client_bound.join(", ")
                }
            )));
        }
        return Ok(());
    }

    if sets_client {
        let secret_required = provider.provider_type == "oauth2";
        if body.client_id.as_deref().is_none_or(str::is_empty)
            || (secret_required && body.client_secret.as_deref().is_none_or(str::is_empty))
        {
            return Err(AppError::ValidationError(if secret_required {
                "Set both client_id and client_secret".to_string()
            } else {
                "Set client_id".to_string()
            }));
        }
    }
    Ok(())
}

/// Whether any person or org holds a connection, token or own OAuth client
/// for this provider. Those credentials are sent to the provider's URLs, so
/// re-entering the platform client cannot vouch for them.
async fn provider_has_connections(db: &mongodb::Database, provider_id: &str) -> AppResult<bool> {
    let filter = doc! { "provider_config_id": provider_id };
    for collection in [
        crate::models::user_provider_token::COLLECTION_NAME,
        crate::models::user_provider_credentials::COLLECTION_NAME,
        crate::models::user_api_key::COLLECTION_NAME,
    ] {
        if db
            .collection::<mongodb::bson::Document>(collection)
            .find_one(filter.clone())
            .projection(doc! { "_id": 1 })
            .await?
            .is_some()
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Whether the request carries the provider's stored client unchanged. A
/// client without a stored secret cannot be proven this way, because client
/// IDs are not secret.
async fn reenters_current_client(
    encryption_keys: &EncryptionKeys,
    provider: &ProviderConfig,
    body: &UpdateProviderRequest,
) -> AppResult<bool> {
    let (Some(stored_id), Some(stored_secret), Some(given_id), Some(given_secret)) = (
        provider.client_id_encrypted.as_ref(),
        provider.client_secret_encrypted.as_ref(),
        body.client_id.as_deref(),
        body.client_secret.as_deref(),
    ) else {
        return Ok(false);
    };
    let stored_id =
        crate::services::credential_record_service::CredentialRecord::provider(provider)
            .decrypt(encryption_keys, "client_id_encrypted", stored_id)
            .await?;
    let stored_secret =
        crate::services::credential_record_service::CredentialRecord::provider(provider)
            .decrypt(encryption_keys, "client_secret_encrypted", stored_secret)
            .await?;
    if stored_secret.iter().all(u8::is_ascii_whitespace) {
        return Ok(false);
    }
    let id_matches = stored_id.as_slice().ct_eq(given_id.as_bytes());
    let secret_matches = stored_secret.as_slice().ct_eq(given_secret.as_bytes());
    Ok(bool::from(id_matches & secret_matches))
}

/// Provider fields a Catalog Manager may not change on a provider someone
/// else created (`admin_only`), and fields they may change there only while
/// no foreign OAuth client or user credential would follow them
/// (`client_bound`). Every other field is descriptive (including the API key
/// instructions and link shown to users, like the documentation URL) or
/// scope selection and is always allowed.
struct ProviderUpdateFields {
    admin_only: Vec<&'static str>,
    client_bound: Vec<&'static str>,
}

fn provider_update_fields(body: &UpdateProviderRequest) -> ProviderUpdateFields {
    let UpdateProviderRequest {
        name: _,
        description: _,
        is_active,
        authorization_url,
        token_url,
        revocation_url,
        revocation,
        default_scopes: _,
        client_id: _,
        client_secret: _,
        supports_pkce,
        authorization_nonce_required,
        device_code_url,
        device_token_url,
        device_verification_url,
        hosted_callback_url,
        api_key_instructions: _,
        api_key_url: _,
        icon_url: _,
        documentation_url: _,
        credential_mode,
        token_endpoint_auth_method,
        token_request_encoding,
        oauth_request_headers,
        supports_oauth_scopes: _,
        extra_auth_params,
        device_code_format,
        client_id_param_name,
        clear_client_credentials,
    } = body;
    let admin_only = [
        is_active.map(|_| "is_active"),
        clear_client_credentials.map(|_| "clear_client_credentials"),
    ]
    .into_iter()
    .flatten()
    .collect();
    let client_bound = [
        authorization_url.as_ref().map(|_| "authorization_url"),
        token_url.as_ref().map(|_| "token_url"),
        revocation_url.as_ref().map(|_| "revocation_url"),
        revocation.as_ref().map(|_| "revocation"),
        supports_pkce.map(|_| "supports_pkce"),
        authorization_nonce_required.map(|_| "authorization_nonce_required"),
        device_code_url.as_ref().map(|_| "device_code_url"),
        device_token_url.as_ref().map(|_| "device_token_url"),
        device_verification_url
            .as_ref()
            .map(|_| "device_verification_url"),
        hosted_callback_url.as_ref().map(|_| "hosted_callback_url"),
        credential_mode.as_ref().map(|_| "credential_mode"),
        token_endpoint_auth_method
            .as_ref()
            .map(|_| "token_endpoint_auth_method"),
        token_request_encoding
            .as_ref()
            .map(|_| "token_request_encoding"),
        oauth_request_headers
            .as_ref()
            .map(|_| "oauth_request_headers"),
        extra_auth_params.as_ref().map(|_| "extra_auth_params"),
        device_code_format.as_ref().map(|_| "device_code_format"),
        client_id_param_name
            .as_ref()
            .map(|_| "client_id_param_name"),
    ]
    .into_iter()
    .flatten()
    .collect();
    ProviderUpdateFields {
        admin_only,
        client_bound,
    }
}

/// A Catalog Manager may only link services to providers they created, so a
/// new service cannot borrow the OAuth client of an existing provider.
pub async fn check_owned_provider(
    state: &AppState,
    caller_id: &str,
    provider_id: &str,
) -> AppResult<()> {
    let provider = provider_service::get_provider(&state.db, provider_id).await?;
    if provider.created_by == caller_id {
        return Ok(());
    }
    Err(AppError::Forbidden(
        "A Catalog Manager can only link services to providers they created; ask an Admin"
            .to_string(),
    ))
}

const MANAGER_AUTH_METHODS: &[&str] = &[
    "bearer",
    "oauth2",
    "header",
    "api_key",
    "query",
    "basic",
    "bot_bearer",
];

/// Catalog Managers create public HTTP AI services with descriptive metadata
/// only. Credential injection, identity forwarding, billing, inference and
/// access-policy fields stay with Admins.
pub fn check_service_create(body: &CreateServiceRequest) -> AppResult<()> {
    forbid_fields("set", service_create_denied_fields(body))
}

fn service_create_denied_fields(body: &CreateServiceRequest) -> Vec<&'static str> {
    let CreateServiceRequest {
        twin_of_service_id,
        asyncapi_spec_url: _,
        custom_user_agent: _,
        offering_kind,
        topics: _,
        supplier: _,
        import_source,
        destination_targets,
        provider_config_id: _,
        name: _,
        slug: _,
        description: _,
        service_type,
        base_url: _,
        auth_method,
        auth_scheme: _,
        auth_key_name: _,
        credential,
        service_category,
        openapi_spec_url: _,
        visibility,
        ssh_config,
        icon_url: _,
        homepage_url: _,
        repository_url: _,
        issues_url: _,
        capabilities: _,
        billing,
        inference,
        platform_key,
        auth_notes: _,
        known_limitations: _,
        required_permissions: _,
        examples_url: _,
        recommended_skills,
        skills_request_id: _,
        recommended_skill_refs,
        developer_app_ids,
        introspection_client_ids,
        forward_access_token,
        token_exchange_config,
        default_request_headers,
        ws_frame_injections,
        anonymous_endpoints,
        proxy_operation_policy,
    } = body;
    [
        twin_of_service_id.as_ref().map(|_| "twin_of_service_id"),
        offering_kind
            .as_ref()
            .filter(|kind| **kind == crate::models::downstream_service::OfferingKind::Tool)
            .map(|_| "offering_kind"),
        import_source.as_ref().map(|_| "import_source"),
        (!destination_targets.is_empty()).then_some("destination_targets"),
        service_type
            .as_deref()
            .filter(|kind| *kind != "http")
            .map(|_| "service_type"),
        auth_method
            .as_deref()
            .filter(|method| !MANAGER_AUTH_METHODS.contains(method))
            .map(|_| "auth_method"),
        credential.as_ref().map(|_| "credential"),
        service_category.as_ref().map(|_| "service_category"),
        visibility
            .as_deref()
            .filter(|value| *value != "public")
            .map(|_| "visibility"),
        ssh_config.as_ref().map(|_| "ssh_config"),
        billing.as_ref().map(|_| "billing"),
        inference.as_ref().map(|_| "inference"),
        platform_key.as_ref().map(|_| "platform_key"),
        recommended_skills.as_ref().map(|_| "recommended_skills"),
        recommended_skill_refs
            .as_ref()
            .map(|_| "recommended_skill_refs"),
        developer_app_ids.as_ref().map(|_| "developer_app_ids"),
        introspection_client_ids
            .as_ref()
            .map(|_| "introspection_client_ids"),
        forward_access_token.then_some("forward_access_token"),
        token_exchange_config
            .as_ref()
            .map(|_| "token_exchange_config"),
        default_request_headers
            .as_ref()
            .map(|_| "default_request_headers"),
        (!ws_frame_injections.is_empty()).then_some("ws_frame_injections"),
        (!anonymous_endpoints.is_empty()).then_some("anonymous_endpoints"),
        proxy_operation_policy
            .as_ref()
            .map(|_| "proxy_operation_policy"),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// Ingress authority `PUT /services/{id}` grants over `service` for `body`:
/// the row gate (`require_tool_writer` for Tools; otherwise Admin when the
/// body converts the row to a Tool, else `require_catalog_row_writer`), then
/// the Tools/Catalog Manager field classifier. `None` is the plain creator
/// rule. Manager checks that compare against stored values stay with the
/// caller. Identity resync passes a body setting every identity field.
pub async fn require_service_update_authority(
    state: &AppState,
    auth_user: &AuthUser,
    service: &DownstreamService,
    body: &UpdateServiceRequest,
) -> AppResult<Option<CatalogAuthority>> {
    use crate::models::downstream_service::OfferingKind;
    if service.offering_kind == OfferingKind::Tool {
        let authority = require_tool_writer(state, auth_user, Some(service)).await?;
        if authority == CatalogAuthority::ToolsManager {
            check_tool_update(body)?;
        }
        return Ok(Some(authority));
    }
    if body.offering_kind == Some(OfferingKind::Tool) {
        super::services_helpers::require_admin(state, auth_user).await?;
        return Ok(Some(CatalogAuthority::Admin));
    }
    let authority = require_catalog_row_writer(state, auth_user, service).await?;
    if authority == Some(CatalogAuthority::CatalogManager) {
        check_service_update(body)?;
    }
    Ok(authority)
}

/// On rows they created, Catalog Managers edit descriptive metadata, the
/// destination URL and the active flag.
pub fn check_service_update(body: &UpdateServiceRequest) -> AppResult<()> {
    forbid_fields("change", service_update_denied_fields(body))
}

fn service_update_denied_fields(body: &UpdateServiceRequest) -> Vec<&'static str> {
    let UpdateServiceRequest {
        auth_scheme: _,
        service_category,
        offering_kind,
        topics: _,
        supplier: _,
        import_source,
        destination_targets,
        name: _,
        description: _,
        base_url: _,
        is_active: _,
        visibility,
        openapi_spec_url: _,
        asyncapi_spec_url: _,
        identity_propagation_mode,
        identity_include_user_id,
        identity_include_email,
        identity_include_name,
        identity_jwt_audience,
        forward_access_token,
        inject_delegation_token,
        delegation_token_scope,
        ssh_config,
        icon_url: _,
        homepage_url: _,
        repository_url: _,
        issues_url: _,
        capabilities: _,
        billing,
        inference,
        platform_key,
        credential,
        clear_credential,
        auth_notes: _,
        known_limitations: _,
        required_permissions: _,
        examples_url: _,
        recommended_skills,
        skills_revision,
        skills_request_id,
        clear_skill_refs,
        recommended_skill_refs,
        developer_app_ids,
        introspection_client_ids,
        custom_user_agent: _,
        default_request_headers,
        token_exchange_config,
        ws_frame_injections,
        anonymous_endpoints,
        proxy_operation_policy,
    } = body;
    [
        service_category.as_ref().map(|_| "service_category"),
        offering_kind
            .as_ref()
            .filter(|kind| **kind == crate::models::downstream_service::OfferingKind::Tool)
            .map(|_| "offering_kind"),
        import_source.as_ref().map(|_| "import_source"),
        destination_targets.as_ref().map(|_| "destination_targets"),
        visibility.as_ref().map(|_| "visibility"),
        identity_propagation_mode
            .as_ref()
            .map(|_| "identity_propagation_mode"),
        identity_include_user_id.map(|_| "identity_include_user_id"),
        identity_include_email.map(|_| "identity_include_email"),
        identity_include_name.map(|_| "identity_include_name"),
        identity_jwt_audience
            .as_ref()
            .map(|_| "identity_jwt_audience"),
        forward_access_token.map(|_| "forward_access_token"),
        inject_delegation_token.map(|_| "inject_delegation_token"),
        delegation_token_scope
            .as_ref()
            .map(|_| "delegation_token_scope"),
        ssh_config.as_ref().map(|_| "ssh_config"),
        billing.as_ref().map(|_| "billing"),
        inference.as_ref().map(|_| "inference"),
        platform_key.as_ref().map(|_| "platform_key"),
        credential.as_ref().map(|_| "credential"),
        clear_credential.then_some("clear_credential"),
        recommended_skills.as_ref().map(|_| "recommended_skills"),
        skills_revision.map(|_| "skills_revision"),
        skills_request_id.as_ref().map(|_| "skills_request_id"),
        clear_skill_refs.then_some("clear_skill_refs"),
        recommended_skill_refs
            .as_ref()
            .map(|_| "recommended_skill_refs"),
        developer_app_ids.as_ref().map(|_| "developer_app_ids"),
        introspection_client_ids
            .as_ref()
            .map(|_| "introspection_client_ids"),
        default_request_headers
            .as_ref()
            .map(|_| "default_request_headers"),
        token_exchange_config
            .as_ref()
            .map(|_| "token_exchange_config"),
        ws_frame_injections.as_ref().map(|_| "ws_frame_injections"),
        anonymous_endpoints.as_ref().map(|_| "anonymous_endpoints"),
        proxy_operation_policy
            .as_ref()
            .map(|_| "proxy_operation_policy"),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// Select authority by offering, so callers holding both roles cannot cross lanes.
pub async fn require_service_creator(
    state: &AppState,
    auth: &AuthUser,
    tool: bool,
) -> AppResult<CatalogAuthority> {
    if !tool {
        return require_catalog_writer(state, auth).await;
    }
    require_tool_writer(state, auth, None).await
}

pub async fn require_tool_writer(
    state: &AppState,
    auth: &AuthUser,
    service: Option<&DownstreamService>,
) -> AppResult<CatalogAuthority> {
    super::login_client_context::require_first_party_human(auth)?;
    let id = auth.user_id.to_string();
    let user = state
        .db
        .collection::<User>(USERS)
        .find_one(doc! {"_id": &id})
        .await?
        .ok_or_else(|| AppError::Forbidden("Tool management requires a human account".into()))?;
    let role = role_service::resolve_platform_role(&state.db, &user).await?;
    if role.is_admin() {
        return Ok(CatalogAuthority::Admin);
    }
    let eligible_owner = if let Some(s) = service {
        s.tools_manager_created
            && s.created_by == id
            && s.owner_user_id.as_ref().is_none_or(|owner| owner == &id)
            && crate::services::tools_manager_service::eligible_manager_tool_in_db(
                &state.db, s, None,
            )
            .await?
    } else {
        true
    };
    if !role_service::can_manage_tools(&state.db, &user, role).await? || !eligible_owner {
        return Err(AppError::Forbidden(
            "Admin or owning Tools Manager access required".into(),
        ));
    }
    Ok(CatalogAuthority::ToolsManager)
}

// Compose the exhaustive catalog classifier with a tighter Tool allowlist. New
// request fields must still be classified in check_service_create/update.
pub fn check_tool_create(body: &CreateServiceRequest) -> AppResult<()> {
    let mut denied = service_create_denied_fields(body);
    denied.retain(|field| {
        ![
            "twin_of_service_id",
            "offering_kind",
            "service_category",
            "auth_method",
        ]
        .contains(field)
    });
    if body.provider_config_id.is_some() {
        denied.push("provider_config_id");
    }
    if body.auth_scheme.is_some() {
        denied.push("auth_scheme");
    }
    forbid_tool_fields(denied)?;
    if body
        .service_category
        .as_deref()
        .is_some_and(|v| v != "internal")
        || body.offering_kind != Some(crate::models::downstream_service::OfferingKind::Tool)
        || body.asyncapi_spec_url.is_some()
        || body.custom_user_agent.is_some()
        || body
            .auth_method
            .as_deref()
            .is_some_and(|m| !["none", "bearer", "header", "query", "basic"].contains(&m))
    {
        return Err(AppError::Forbidden(
            "Tools Manager may create only provider-less internal HTTP Tools with supported auth"
                .into(),
        ));
    }
    if body.auth_key_name.as_ref().is_some_and(|name| {
        name.len() > 256
            || (body.auth_method.as_deref() != Some("none")
                && (name.is_empty()
                    || !name
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"-_".contains(&c))))
    }) {
        return Err(AppError::ValidationError(
            "Invalid Tool auth key name".into(),
        ));
    }
    Ok(())
}

pub fn check_tool_update(body: &UpdateServiceRequest) -> AppResult<()> {
    let mut denied = service_update_denied_fields(body);
    if body.auth_scheme.is_some() {
        denied.push("auth_scheme");
    }
    denied.retain(|field| *field != "credential");
    forbid_tool_fields(denied)?;
    if body.offering_kind.is_some()
        || body.is_active.is_some()
        || body.asyncapi_spec_url.is_some()
        || body.custom_user_agent.is_some()
    {
        return Err(AppError::Forbidden(
            "Tool shape, activation and injection configuration require an Admin".into(),
        ));
    }
    Ok(())
}

fn forbid_tool_fields(fields: Vec<&'static str>) -> AppResult<()> {
    if fields.is_empty() {
        return Ok(());
    }
    Err(AppError::Forbidden(format!(
        "A Tools Manager cannot set these fields; ask an Admin: {}",
        fields.join(", ")
    )))
}

#[cfg(test)]
mod authorization_nonce_tests {
    use super::*;

    #[test]
    fn introspection_trust_is_admin_only_for_catalog_and_tools_managers() {
        for ids in [
            serde_json::json!([]),
            serde_json::json!(["resource-server"]),
        ] {
            let create: CreateServiceRequest = serde_json::from_value(serde_json::json!({
                "name": "Resource", "introspection_client_ids": ids,
            }))
            .unwrap();
            let update: UpdateServiceRequest = serde_json::from_value(serde_json::json!({
                "introspection_client_ids": ids,
            }))
            .unwrap();
            for result in [
                check_service_create(&create),
                check_service_update(&update),
                check_tool_create(&create),
                check_tool_update(&update),
            ] {
                assert!(matches!(
                    result,
                    Err(AppError::Forbidden(message)) if message.contains("introspection_client_ids")
                ));
            }
        }
        let create = serde_json::from_value(serde_json::json!({"name": "Resource"})).unwrap();
        let update =
            serde_json::from_value(serde_json::json!({"description": "Metadata"})).unwrap();
        assert!(check_service_create(&create).is_ok());
        assert!(check_service_update(&update).is_ok());
    }

    #[tokio::test]
    async fn authorization_nonce_preserves_shared_provider_client_and_connection_permissions() {
        use mongodb::bson::doc;
        let db = crate::test_utils::connect_test_database("auth_nonce_permissions")
            .await
            .expect("MongoDB required");
        let state = crate::test_utils::test_app_state(db.clone());
        let mut provider: ProviderConfig = mongodb::bson::from_document(doc! {
            "_id": "provider", "slug": "provider", "name": "Provider", "provider_type": "oauth2",
            "is_active": true, "created_by": "admin", "created_at": mongodb::bson::DateTime::now(), "updated_at": mongodb::bson::DateTime::now(),
        }).unwrap();
        provider.client_id_encrypted =
            Some(state.encryption_keys.encrypt(b"client").await.unwrap());
        provider.client_secret_encrypted =
            Some(state.encryption_keys.encrypt(b"secret").await.unwrap());
        for required in [true, false] {
            let mut request: UpdateProviderRequest = serde_json::from_value(
                serde_json::json!({"authorization_nonce_required":required}),
            )
            .unwrap();
            assert!(matches!(
                check_provider_update(&state, "manager", &provider, &request).await,
                Err(AppError::Forbidden(_))
            ));
            request.client_id = Some("client".into());
            request.client_secret = Some("secret".into());
            check_provider_update(&state, "manager", &provider, &request)
                .await
                .unwrap();
            db.collection::<mongodb::bson::Document>(crate::models::user_api_key::COLLECTION_NAME)
                .insert_one(doc! {"_id":"foreign-key", "provider_config_id": &provider.id})
                .await
                .unwrap();
            assert!(matches!(
                check_provider_update(&state, "manager", &provider, &request).await,
                Err(AppError::Forbidden(_))
            ));
            let descriptive: UpdateProviderRequest =
                serde_json::from_value(serde_json::json!({"name":"Allowed metadata"})).unwrap();
            check_provider_update(&state, "manager", &provider, &descriptive)
                .await
                .unwrap();
            check_provider_update(&state, "admin", &provider, &request)
                .await
                .unwrap();
            db.collection::<mongodb::bson::Document>(crate::models::user_api_key::COLLECTION_NAME)
                .delete_one(doc! {"_id":"foreign-key"})
                .await
                .unwrap();
        }
    }

    #[test]
    fn authorization_nonce_is_client_bound_including_explicit_disable() {
        for required in [true, false] {
            let request: UpdateProviderRequest = serde_json::from_value(serde_json::json!({
                "authorization_nonce_required": required,
            }))
            .unwrap();
            let fields = provider_update_fields(&request);
            assert!(fields.admin_only.is_empty());
            assert_eq!(fields.client_bound, ["authorization_nonce_required"]);
        }
        let request: UpdateProviderRequest =
            serde_json::from_value(serde_json::json!({"name": "Unrelated edit"})).unwrap();
        assert!(request.authorization_nonce_required.is_none());
        assert!(provider_update_fields(&request).client_bound.is_empty());
    }
}
