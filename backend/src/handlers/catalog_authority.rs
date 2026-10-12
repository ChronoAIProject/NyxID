//! Who may write providers and catalog services, and what a Catalog Manager
//! may change.
//!
//! Admins keep full authority. An Operator holding the Catalog Manager role
//! may create providers and HTTP catalog services, edit the rows they created,
//! and set the first OAuth client on a provider that has none. The field
//! lists below destructure every request field, so adding a field fails to
//! compile until it is classified here.

use mongodb::bson::doc;

use crate::AppState;
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
}

impl CatalogAuthority {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::CatalogManager => "catalog_manager",
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
/// they may only set the first OAuth client, never replace or clear one.
pub fn check_provider_update(
    caller_id: &str,
    provider: &ProviderConfig,
    body: &UpdateProviderRequest,
) -> AppResult<()> {
    if provider.created_by == caller_id {
        return Ok(());
    }
    let has_client =
        provider.client_id_encrypted.is_some() || provider.client_secret_encrypted.is_some();
    if has_client {
        return Err(AppError::Forbidden(
            "A Catalog Manager can only change providers they created, and can only set the OAuth client on a provider that has none; ask an Admin".to_string(),
        ));
    }
    forbid_fields(
        "change on a provider they did not create",
        provider_update_fields_except_client(body),
    )?;
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
    Ok(())
}

fn provider_update_fields_except_client(body: &UpdateProviderRequest) -> Vec<&'static str> {
    let UpdateProviderRequest {
        name,
        description,
        is_active,
        authorization_url,
        token_url,
        revocation_url,
        revocation,
        default_scopes,
        client_id: _,
        client_secret: _,
        supports_pkce,
        device_code_url,
        device_token_url,
        device_verification_url,
        hosted_callback_url,
        api_key_instructions,
        api_key_url,
        icon_url,
        documentation_url,
        credential_mode,
        token_endpoint_auth_method,
        token_request_encoding,
        oauth_request_headers,
        supports_oauth_scopes,
        extra_auth_params,
        device_code_format,
        client_id_param_name,
        clear_client_credentials,
    } = body;
    [
        name.as_ref().map(|_| "name"),
        description.as_ref().map(|_| "description"),
        is_active.map(|_| "is_active"),
        authorization_url.as_ref().map(|_| "authorization_url"),
        token_url.as_ref().map(|_| "token_url"),
        revocation_url.as_ref().map(|_| "revocation_url"),
        revocation.as_ref().map(|_| "revocation"),
        default_scopes.as_ref().map(|_| "default_scopes"),
        supports_pkce.map(|_| "supports_pkce"),
        device_code_url.as_ref().map(|_| "device_code_url"),
        device_token_url.as_ref().map(|_| "device_token_url"),
        device_verification_url
            .as_ref()
            .map(|_| "device_verification_url"),
        hosted_callback_url.as_ref().map(|_| "hosted_callback_url"),
        api_key_instructions
            .as_ref()
            .map(|_| "api_key_instructions"),
        api_key_url.as_ref().map(|_| "api_key_url"),
        icon_url.as_ref().map(|_| "icon_url"),
        documentation_url.as_ref().map(|_| "documentation_url"),
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
        supports_oauth_scopes.map(|_| "supports_oauth_scopes"),
        extra_auth_params.as_ref().map(|_| "extra_auth_params"),
        device_code_format.as_ref().map(|_| "device_code_format"),
        client_id_param_name
            .as_ref()
            .map(|_| "client_id_param_name"),
        clear_client_credentials.map(|_| "clear_client_credentials"),
    ]
    .into_iter()
    .flatten()
    .collect()
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
        auth_key_name: _,
        credential,
        service_category,
        openapi_spec_url: _,
        visibility,
        ssh_config,
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
        forward_access_token,
        token_exchange_config,
        default_request_headers,
        ws_frame_injections,
        anonymous_endpoints,
        proxy_operation_policy,
    } = body;
    let denied: Vec<&'static str> = [
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
    .collect();
    forbid_fields("set", denied)
}

/// On rows they created, Catalog Managers edit descriptive metadata, the
/// destination URL and the active flag.
pub fn check_service_update(body: &UpdateServiceRequest) -> AppResult<()> {
    let UpdateServiceRequest {
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
        homepage_url: _,
        repository_url: _,
        issues_url: _,
        capabilities: _,
        billing,
        inference,
        platform_key,
        credential,
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
        custom_user_agent: _,
        default_request_headers,
        token_exchange_config,
        ws_frame_injections,
        anonymous_endpoints,
        proxy_operation_policy,
    } = body;
    let denied: Vec<&'static str> = [
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
        recommended_skills.as_ref().map(|_| "recommended_skills"),
        skills_revision.map(|_| "skills_revision"),
        skills_request_id.as_ref().map(|_| "skills_request_id"),
        clear_skill_refs.then_some("clear_skill_refs"),
        recommended_skill_refs
            .as_ref()
            .map(|_| "recommended_skill_refs"),
        developer_app_ids.as_ref().map(|_| "developer_app_ids"),
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
    .collect();
    forbid_fields("change", denied)
}
