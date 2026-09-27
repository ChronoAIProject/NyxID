use super::*;
use crate::models::{
    agent_service_binding::{AgentServiceBinding, COLLECTION_NAME as BINDINGS},
    downstream_service::{COLLECTION_NAME as CATALOG, DownstreamService},
    user_api_key::{COLLECTION_NAME as CONNECTIONS, UserApiKey},
};
use mongodb::{ClientSession, bson::Document};
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Serialize, ToSchema)]
#[schema(as = LoginEffectiveService)]
pub struct EffectiveService {
    pub id: String,
    pub label: String,
    pub owner_id: String,
    pub auto_connected: bool,
    pub credential_binding: String,
    pub slug: String,
    pub catalog_service_slug: Option<String>,
    pub is_active: bool,
    pub status: String,
    pub connection_status: Option<String>,
    pub credential_missing: bool,
    pub expires_at: Option<String>,
    pub node_id: Option<String>,
    pub granted_scopes: Option<Vec<String>>,
    pub permission_snapshot: String,
}

pub(super) fn auto_connected(service: &UserService) -> bool {
    service.source.as_deref() == Some(crate::models::user_service::AUTO_PROVISION_SOURCE)
        || service.credential_binding.as_deref() == Some("platform")
}

fn digest(value: &impl Serialize) -> AppResult<String> {
    let bytes = serde_json::to_vec(value)
        .map_err(|_| AppError::Internal("Invalid login permission summary".into()))?;
    Ok(hex::encode(Sha256::digest(bytes)))
}
fn changed() -> AppError {
    AppError::Conflict("The key or connection access changed. Refresh and review the updated permissions before approving.".into())
}

async fn read<T: DeserializeOwned + Send + Sync>(
    db: &Database,
    collection: &str,
    filter: Document,
    session: Option<&mut ClientSession>,
) -> AppResult<Option<T>> {
    let collection = db.collection::<T>(collection);
    let query = collection.find_one(filter);
    Ok(match session {
        Some(session) => query.session(session).await?,
        None => query.await?,
    })
}

// A write fence makes a concurrent change to a reviewed resource conflict with
// the approval transaction. This counter does not change credential_epoch:
// approval does not replace credential material.
async fn fence(
    db: &Database,
    collection: &str,
    id: &str,
    session: &mut ClientSession,
) -> AppResult<()> {
    let result = db
        .collection::<Document>(collection)
        .update_one(
            doc! {"_id": id},
            doc! {"$inc": {"login_consent_fence": 1_i64}},
        )
        .session(session)
        .await?;
    if result.matched_count != 1 {
        return Err(changed());
    }
    Ok(())
}

pub(super) async fn connection(
    db: &Database,
    service: &UserService,
    parent: Option<&ApiKey>,
    mut session: Option<&mut ClientSession>,
) -> AppResult<EffectiveService> {
    let platform = crate::services::platform_key_service::binding(service) == "platform"
        && (service.auth_method != "none"
            || service.credential_binding.as_deref() == Some("platform"));
    // Execution selects catalog platform credentials before agent overrides.
    let binding: Option<AgentServiceBinding> = if let Some(parent) = parent.filter(|_| !platform) {
        read(db, BINDINGS, doc! {"api_key_id": &parent.id, "user_service_id": &service.id, "user_id": &parent.user_id}, session.as_deref_mut()).await?
    } else {
        None
    };
    let credential_id = binding
        .as_ref()
        .map(|b| b.user_api_key_id.as_str())
        .or(service.api_key_id.as_deref())
        .filter(|_| !platform);
    let owner = binding
        .as_ref()
        .map_or(service.user_id.as_str(), |b| b.user_id.as_str());
    let credential: Option<UserApiKey> = match credential_id {
        Some(id) => {
            read(
                db,
                CONNECTIONS,
                doc! {"_id": id, "user_id": owner},
                session.as_deref_mut(),
            )
            .await?
        }
        None => None,
    };
    let catalog: Option<DownstreamService> = match &service.catalog_service_id {
        Some(id) => read(db, CATALOG, doc! {"_id": id}, session.as_deref_mut()).await?,
        None => None,
    };
    // Legacy OAuth can sync another provider-token row at execution time. Its
    // provider access is deliberately unreported until it has a bound connection;
    // stale UserApiKey.token_scopes must never produce an exact permission match.
    let granted_scopes = credential
        .as_ref()
        .filter(|c| {
            !(c.credential_type == "oauth2"
                && c.connection_id.is_none()
                && c.provider_config_id.is_some())
        })
        .and_then(|c| c.token_scopes.as_ref())
        .and_then(|raw| {
            let mut scopes: Vec<String> = raw
                .split(|c: char| c.is_whitespace() || c == ',')
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect();
            scopes.sort();
            scopes.dedup();
            (!scopes.is_empty()).then_some(scopes)
        });
    let material = match credential.as_ref() {
        Some(c)
            if service.node_id.is_some()
                && binding.is_none()
                && matches!(
                    c.credential_type.as_str(),
                    "node_managed" | "ssh_certificate"
                ) =>
        {
            true
        }
        Some(c) => crate::services::proxy_service::credential_is_materializable(db, c).await?,
        None => false,
    };
    let platform_ready = if platform {
        match &catalog {
            Some(catalog)
                if service.node_id.is_none()
                    && crate::services::platform_key_service::available(
                        db,
                        catalog,
                        &service.user_id,
                    )
                    .await? =>
            {
                match crate::services::platform_key_service::effective_auth(db, catalog).await {
                    Ok(_) => true,
                    Err(AppError::ValidationError(_)) => false,
                    Err(error) => return Err(error),
                }
            }
            _ => false,
        }
    } else {
        false
    };
    let no_auth = !platform && credential_id.is_none() && service.auth_method == "none";
    let catalog_ready =
        if service.source.as_deref() == Some(crate::models::user_service::AUTO_PROVISION_SOURCE) {
            match crate::services::proxy_service::verify_auto_provision_eligibility(
                db,
                service,
                &service.user_id,
            )
            .await
            {
                Ok(()) => true,
                Err(AppError::NotFound(_)) => false,
                Err(error) => return Err(error),
            }
        } else {
            true
        };
    let mut output = EffectiveService {
        id: service.id.clone(),
        owner_id: service.user_id.clone(),
        auto_connected: auto_connected(service),
        credential_binding: crate::services::platform_key_service::binding(service).into(),
        label: credential.as_ref().map_or_else(
            || {
                catalog
                    .as_ref()
                    .map_or_else(|| service.slug.clone(), |c| c.name.clone())
            },
            |c| c.label.clone(),
        ),
        slug: service.slug.clone(),
        catalog_service_slug: catalog.as_ref().map(|c| c.slug.clone()),
        is_active: service.is_active && catalog_ready,
        status: credential.as_ref().map_or_else(
            || {
                if no_auth || platform_ready {
                    "active"
                } else {
                    "missing"
                }
                .into()
            },
            |c| c.status.clone(),
        ),
        credential_missing: !no_auth && !(if platform { platform_ready } else { material }),
        // OAuth access-token refresh is not an authority change. The shared
        // connection status determines whether that connection can still run.
        connection_status: credential
            .as_ref()
            .and_then(crate::services::unified_key_service::oauth_connection_status),
        expires_at: credential
            .as_ref()
            .filter(|c| !matches!(c.credential_type.as_str(), "oauth2" | "gcp_service_account"))
            .and_then(|c| c.expires_at)
            .map(|t| t.to_rfc3339()),
        node_id: service.node_id.clone(),
        granted_scopes,
        permission_snapshot: String::new(),
    };
    output.permission_snapshot = digest(&serde_json::json!({
        "version": 1, "view": &output,
        "service_owner": service.user_id, "endpoint_id": service.endpoint_id,
        "auth_method": service.auth_method, "auth_key_name": service.auth_key_name,
        "service_updated_at": service.updated_at,
        "platform_authority": if platform { catalog.as_ref().map(|c| serde_json::json!({
            "id": c.id, "updated_at": c.updated_at, "platform_key": c.platform_key,
            "provider_config_id": c.provider_config_id, "auth_method": c.auth_method,
            "auth_key_name": c.auth_key_name,
        })) } else { None },
        "binding": binding.as_ref().map(|b| (&b.id, &b.user_api_key_id, b.updated_at)),
        "credential": credential.as_ref().map(|c| (&c.id, c.credential_epoch, &c.credential_type, &c.connection_id, c.last_authorized_at)),
    }))?;
    if let Some(session) = session {
        fence(db, SERVICES, &service.id, &mut *session).await?;
        if let Some(credential) = credential {
            fence(db, CONNECTIONS, &credential.id, &mut *session).await?;
        }
        if let Some(binding) = binding {
            fence(db, BINDINGS, &binding.id, &mut *session).await?;
        }
        if let Some(catalog) = catalog {
            fence(db, CATALOG, &catalog.id, &mut *session).await?;
        }
    }
    Ok(output)
}

pub(super) async fn key_access(
    db: &Database,
    key: &ApiKey,
    mut session: Option<&mut ClientSession>,
) -> AppResult<(Vec<EffectiveService>, String)> {
    let mut ids = key_service::effective_allowed_service_ids(db, key).await?;
    if key.allow_all_services {
        ids = user_service_service::list_user_services_with_sources(db, &key.user_id)
            .await?
            .into_iter()
            .filter(|e| match &e.source {
                user_service_service::CredentialSource::Personal => true,
                user_service_service::CredentialSource::Org { allowed, .. } => *allowed,
            })
            .map(|e| e.service.id)
            .collect();
    }
    ids.sort();
    ids.dedup();
    let mut services = Vec::new();
    for id in &ids {
        if let Some(service) =
            read::<UserService>(db, SERVICES, doc! {"_id": id}, session.as_deref_mut()).await?
        {
            services.push(connection(db, &service, Some(key), session.as_deref_mut()).await?);
        }
    }
    let snapshot = digest(&serde_json::json!({
        "version": 1, "key_id": key.id, "owner_id": key.user_id, "state_version": key.state_version,
        "scopes": key.scopes, "allowed_service_ids": key.allowed_service_ids, "allowed_node_ids": key.allowed_node_ids,
        "allow_all_services": key.allow_all_services, "allow_auto_connected_services": key.allow_auto_connected_services, "allow_all_nodes": key.allow_all_nodes,
        "expires_at": key.expires_at, "is_active": key.is_active, "purpose": key.purpose,
        "rate_limit_per_second": key.rate_limit_per_second, "rate_limit_burst": key.rate_limit_burst,
        "platform": key.platform, "services": services,
    }))?;
    Ok((services, snapshot))
}

pub(super) async fn validate(
    db: &Database,
    parent: &ApiKey,
    selection: &Selection,
    session: &mut ClientSession,
) -> AppResult<()> {
    match selection {
        Selection::Existing {
            permission_snapshot: Some(expected),
            ..
        } => {
            let (_, actual) = key_access(db, parent, Some(&mut *session)).await?;
            if expected.len() != 64 || expected != &actual {
                return Err(changed());
            }
            // Binding CRUD uses the same parent authority version in its transaction.
            mutations::update_one(db, doc! {"_id": &parent.id}, doc! {}, Some(&mut *session))
                .await?;
        }
        Selection::New(input) if input.connection_snapshots.is_some() => {
            let snapshots = input.connection_snapshots.as_ref().expect("checked");
            let (services, _) = key_access(db, parent, Some(&mut *session)).await?;
            if snapshots.len() != services.len()
                || snapshots.iter().any(|s| s.permission_snapshot.len() != 64)
                || services.iter().any(|s| {
                    snapshots
                        .iter()
                        .filter(|v| {
                            v.service_id == s.id && v.permission_snapshot == s.permission_snapshot
                        })
                        .count()
                        != 1
                })
            {
                return Err(changed());
            }
        }
        _ => {} // Installed clients predate the additive consent fence.
    }
    Ok(())
}
