use super::*;
use crate::services::platform_key_service;

/// Provision an authenticated owner-bound platform connection using the same
/// endpoint/service storage and uniqueness checks as every unified connection.
#[allow(clippy::too_many_arguments)]
pub async fn create_platform_key(
    db: &mongodb::Database,
    owner_id: &str,
    actor_id: &str,
    slug: &str,
    label: &str,
    slug_override: Option<&str>,
    admin_only: bool,
    reserved_id: Option<&str>,
) -> AppResult<CreateKeyResult> {
    let catalog = db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find_one(doc! { "slug": slug, "is_active": true })
        .await?
        .ok_or_else(|| AppError::NotFound("Service is no longer available".to_string()))?;
    if catalog_spec_sync::is_platform_vendor_service(&catalog) {
        return Err(AppError::NotFound(
            "Service is no longer available".to_string(),
        ));
    }
    platform_key_service::require(db, &catalog, owner_id).await?;
    let access = crate::services::org_service::resolve_owner_access(db, actor_id, owner_id).await?;
    if !access.can_write() {
        return Err(AppError::NotFound(
            "Service is no longer available".to_string(),
        ));
    }
    let (auth_method, auth_key_name) = platform_key_service::effective_auth(db, &catalog).await?;
    let slug = resolve_unique_slug(
        db,
        owner_id,
        slug_override.unwrap_or(slug),
        if slug_override.is_some() {
            SlugCollisionStrategy::PreserveExact
        } else {
            SlugCollisionStrategy::AutoDisambiguate
        },
    )
    .await?;
    let endpoint = user_endpoint_service::create_endpoint(
        db,
        owner_id,
        label,
        &catalog.base_url,
        Some(&catalog.id),
        catalog.openapi_spec_url.as_deref(),
    )
    .await?;
    let result = user_service_service::create_user_service_with_id(
        db,
        owner_id,
        actor_id,
        &slug,
        &endpoint.id,
        None,
        &auth_method,
        &auth_key_name,
        Some(&catalog.id),
        None,
        0,
        "http",
        SshAuthMode::ProxyOnly,
        Some("platform_key"),
        Some(&endpoint.id),
        None,
        &identity_config_from_downstream_service(&catalog),
        None,
        admin_only,
        reserved_id,
    )
    .await;
    let service = match result {
        Ok(service) => service,
        Err(error) => {
            cleanup_auto_provision_endpoint(db, owner_id, &endpoint.id).await;
            return Err(error);
        }
    };
    Ok(CreateKeyResult {
        endpoint,
        api_key: None,
        service,
        ssh_host: None,
        ssh_port: None,
        ssh_ca_public_key: None,
        ssh_allowed_principals: None,
        ssh_certificate_ttl_minutes: None,
    })
}

#[allow(clippy::too_many_arguments)]
pub async fn switch_credential_binding(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    owner_id: &str,
    service_id: &str,
    use_platform_key: bool,
    credential: Option<&str>,
    oauth_client_credentials: OauthClientCredentialsInput<'_>,
) -> AppResult<()> {
    let service = user_service_service::get_user_service(db, owner_id, service_id).await?;
    let catalog_id = service.catalog_service_id.as_ref().ok_or_else(|| {
        AppError::ValidationError("Credential binding requires a catalog service".to_string())
    })?;
    let catalog = db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find_one(doc! { "_id": catalog_id })
        .await?
        .ok_or_else(|| AppError::NotFound("Service is no longer available".to_string()))?;
    let (auth_method, auth_key_name) = platform_key_service::effective_auth(db, &catalog).await?;
    if use_platform_key {
        if credential.is_some()
            || service.node_id.is_some()
            || !matches!(oauth_client_credentials, OauthClientCredentialsInput::None)
        {
            return Err(AppError::ValidationError(
                "Platform keys cannot be combined with a credential, OAuth app, or node routing"
                    .to_string(),
            ));
        }
        platform_key_service::require(db, &catalog, owner_id).await?;
    } else {
        if let Some(value) = credential {
            if value.trim().is_empty() {
                return Err(AppError::ValidationError(
                    "Credential must not be empty".to_string(),
                ));
            }
            validate_token_exchange_catalog_credential(&catalog, value)?;
        } else {
            let provider = match catalog.provider_config_id.as_deref() {
                Some(id) => {
                    db.collection::<ProviderConfig>(PROVIDER_CONFIGS)
                        .find_one(doc! { "_id": id, "is_active": true })
                        .await?
                }
                None => None,
            };
            if !provider
                .is_some_and(|p| matches!(p.provider_type.as_str(), "oauth2" | "device_code"))
            {
                return Err(AppError::ValidationError(
                    "Switching to your own key requires a credential or an OAuth provider"
                        .to_string(),
                ));
            }
        }
        ensure_user_api_key_for_update(
            db,
            encryption_keys,
            owner_id,
            service_id,
            Some(&auth_method),
            credential,
            None,
            &catalog.name,
            oauth_client_credentials,
        )
        .await?;
    }
    // Retain api_key_id when selecting platform. It remains a user-owned row;
    // final resolution ignores it until the binding changes back to user.
    crate::services::service_history::collection::<UserService>(
        db,
        crate::models::user_service::COLLECTION_NAME,
    )
    .update_one(
        doc! { "_id": service_id, "user_id": owner_id },
        doc! { "$set": {
            "credential_binding": if use_platform_key { "platform" } else { "user" },
            "auth_method": &auth_method, "auth_key_name": &auth_key_name,
            "source": bson::Bson::Null, "source_id": bson::Bson::Null,
            "updated_at": bson::DateTime::from_chrono(Utc::now()),
        }, "$inc": { "state_version": 1_i64 } },
    )
    .await?;
    Ok(())
}

/// Lifecycle of an explicitly selected platform connection. Catalog authorization
/// is checked again on enable; disabling remains possible after revocation.
pub async fn set_platform_connection_active(
    db: &mongodb::Database,
    owner_id: &str,
    service_id: &str,
    active: bool,
) -> AppResult<()> {
    let service = user_service_service::get_user_service(db, owner_id, service_id).await?;
    if active {
        let catalog = db
            .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .find_one(doc! { "_id": service.catalog_service_id.as_deref().unwrap_or("") })
            .await?
            .ok_or_else(|| AppError::NotFound("Service is no longer available".to_string()))?;
        platform_key_service::require(db, &catalog, owner_id).await?;
    }
    crate::services::service_history::collection::<UserService>(db, crate::models::user_service::COLLECTION_NAME)
        .update_one(doc! { "_id": service_id, "user_id": owner_id, "credential_binding": "platform" },
            doc! { "$set": { "is_active": active, "updated_at": bson::DateTime::from_chrono(Utc::now()) }, "$inc": { "state_version": 1_i64 } }).await?;
    Ok(())
}

/// Only presentation and per-user header settings can be changed with a
/// platform binding. Endpoint/auth/routing remain catalog-owned.
#[allow(clippy::too_many_arguments)]
pub async fn update_platform_connection_cosmetics(
    db: &mongodb::Database,
    owner_id: &str,
    actor_id: &str,
    service_id: &str,
    label: Option<&str>,
    skills: Option<Vec<String>>,
    active: Option<bool>,
    admin_only: Option<bool>,
    user_agent: Option<&str>,
    headers: Option<&Option<Vec<crate::models::default_request_header::DefaultRequestHeader>>>,
) -> AppResult<()> {
    let service = user_service_service::get_user_service(db, owner_id, service_id).await?;
    user_service_service::ensure_service_fields_editable(&service, &[])?;
    if let Some(active) = active
        && label.is_none()
        && skills.is_none()
        && admin_only.is_none()
        && user_agent.is_none()
        && headers.is_none()
    {
        return super::set_platform_connection_active(db, owner_id, service_id, active).await;
    }
    let skills = match skills {
        None => user_endpoint_service::RecommendedSkillsUpdate::Leave,
        Some(skills) => user_endpoint_service::RecommendedSkillsUpdate::Set(skills),
    };
    // Validate endpoint presentation before any service write.
    user_endpoint_service::build_endpoint_update(
        None,
        label,
        user_endpoint_service::OpenApiSpecUrlUpdate::Leave,
        skills.clone(),
    )?;
    user_service_service::update_user_service(
        db, owner_id, actor_id, service_id, None, None, None, None, active, None, user_agent,
        headers, None, admin_only,
    )
    .await?;
    user_endpoint_service::update_endpoint(
        db,
        owner_id,
        &service.endpoint_id,
        None,
        label,
        user_endpoint_service::OpenApiSpecUrlUpdate::Leave,
        skills,
    )
    .await
}
