use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use mongodb::{
    Database,
    bson::{self, doc},
};
use nyxid_permissions::{
    Engine, Error, Policy, ResourceBoundary, Transport, adapter_for_policy,
    hooks::{
        HookContext, HookDecision, HookDefinition, HookFailure, HookRegistry, HookStage,
        PermissionHook,
    },
};
use serde_json::Value;

use crate::{
    crypto::aes::EncryptionKeys,
    errors::{AppError, AppResult},
    models::{
        api_key::{ApiKey, ApiKeyPurpose, COLLECTION_NAME as KEYS},
        permission_policy::{COLLECTION_NAME as POLICIES, PermissionPolicy},
        user_api_key::{COLLECTION_NAME as CREDENTIALS, UserApiKey},
        user_service::{COLLECTION_NAME as SERVICES, UserService},
    },
    services::{
        api_key_mutation_service as mutations, execution_authority, key_service,
        proxy_service::{self, ProxyTarget, UserServiceResolution},
    },
};

fn unavailable() -> AppError {
    AppError::Forbidden(
        "Permission key or its bound Google connection is unavailable or has changed".into(),
    )
}

struct NoTransport;
#[async_trait]
impl Transport for NoTransport {
    async fn send(
        &self,
        _: &nyxid_permissions::Request,
    ) -> Result<nyxid_permissions::Response, Error> {
        Err(Error::Verification)
    }
}

pub fn validate_policy(policy: &Policy) -> AppResult<()> {
    // Native tenants may configure response checks, never host filesystem access.
    // Two slots are reserved for mandatory, host-owned live-authority hooks.
    if policy.hooks.len() > 6
        || policy
            .hooks
            .iter()
            .any(|h| h.handler != "response_markers" || h.name.starts_with("nyxid-"))
    {
        return Err(AppError::ValidationError(
            "Native policies support up to six response_markers hooks; nyxid- names are reserved"
                .into(),
        ));
    }
    Engine::new(
        policy.clone(),
        adapter_for_policy(policy).map_err(|e| AppError::ValidationError(e.to_string()))?,
        Arc::new(NoTransport),
    )
    .map_err(|e| AppError::ValidationError(e.to_string()))?;
    Ok(())
}

/// Also checked on the final selected target immediately before ordinary proxy gates.
pub fn validate_target(target: &ProxyTarget) -> AppResult<()> {
    let service = &target.service;
    let supported_auth = match target.auth_method.as_str() {
        "bearer" => target.auth_key_name.eq_ignore_ascii_case("authorization"),
        "header" => target.auth_key_name.eq_ignore_ascii_case("x-goog-api-key"),
        "query" => target.auth_key_name == "key",
        _ => false,
    };
    if google_origin(&target.base_url).is_none()
        || !supported_auth
        || service.service_type != "http"
        || service.forward_access_token
        || service.inject_delegation_token
        || service.identity_propagation_mode != "none"
        || service.token_exchange_config.is_some()
        || !target.catalog_default_headers.is_empty()
        || !target.user_service_default_headers.is_empty()
    {
        return Err(unavailable());
    }
    Ok(())
}

fn google_origin(base: &str) -> Option<String> {
    let parsed = url::Url::parse(base).ok()?;
    let origin = parsed.origin().ascii_serialization();
    if !nyxid_permissions::google::is_google_api_origin(&origin)
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.path().contains(['%', '\\'])
        || format!("{origin}{}", parsed.path()).trim_end_matches('/') != base.trim_end_matches('/')
    {
        return None;
    }
    Some(origin)
}

pub fn authority(resolved: &UserServiceResolution) -> AppResult<String> {
    validate_target(&resolved.target)?;
    if resolved.catalog_service_slug.is_none()
        || resolved.node_id.is_some()
        || resolved.org_routing.is_some()
        || resolved.pool_selection.is_some()
        || resolved.master_credential
        || !resolved.has_server_credential
        || resolved.api_key_id.is_none()
    {
        return Err(unavailable());
    }
    // Include routing declarations as well as the shared credential/epoch projection.
    Ok(super::mcp_service::canonical_sha256(serde_json::json!({
        "contract": "nyxid-permission-binding.v1",
        "authority": execution_authority::build_projection(resolved, None, vec![]),
        "catalog_id": resolved.target.service.id,
        "catalog_slug": resolved.catalog_service_slug,
        "destinations": resolved.target.service.destination_targets,
        "policy": resolved.target.service.proxy_operation_policy,
    })))
}

/// Policies can only narrow the exact connection's configured recipients.
/// Actual operation-to-recipient routing is checked again on the final target.
pub fn validate_policy_connection(
    policy: &Policy,
    resolved: &UserServiceResolution,
) -> AppResult<()> {
    let base = google_origin(&resolved.target.base_url).ok_or_else(unavailable)?;
    match &policy.resource {
        ResourceBoundary::GoogleDriveFolder { .. } => {
            if resolved.target.base_url.trim_end_matches('/') != "https://www.googleapis.com"
                || resolved.target.auth_method != "bearer"
                || !matches!(
                    resolved.catalog_service_slug.as_deref(),
                    Some("api-google-drive" | "api-google-workspace")
                )
            {
                return Err(unavailable());
            }
        }
        ResourceBoundary::GoogleApi { operations } => {
            for op in operations {
                if op.origin != base
                    && !resolved
                        .target
                        .service
                        .destination_targets
                        .values()
                        .any(|origin| origin == &op.origin)
                {
                    return Err(AppError::ValidationError(
                        "Policy API origin is absent from the bound connection's destinations"
                            .into(),
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Called after catalog routing. A generic policy never authorizes a host
/// chosen by the request; even another Google origin must match the plan.
pub fn validate_execution_target(
    target: &ProxyTarget,
    expected_origin: Option<&str>,
) -> AppResult<()> {
    validate_target(target)?;
    let expected = expected_origin.unwrap_or("https://www.googleapis.com");
    if google_origin(&target.base_url).as_deref() != Some(expected) {
        return Err(unavailable());
    }
    Ok(())
}

pub async fn validate_connection(db: &Database, owner: &str, connection: &str) -> AppResult<()> {
    let owner_active = db
        .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
        .find_one(doc! {"_id": owner, "is_active": true, "user_type": {"$ne": "org"}})
        .await?
        .is_some();
    if !owner_active {
        return Err(unavailable());
    }
    let service = db
        .collection::<UserService>(SERVICES)
        .find_one(doc! {
            "_id": connection, "user_id": owner, "is_active": true, "deleted_at": null,
        })
        .await?
        .ok_or_else(unavailable)?;
    if service.node_id.is_some() || service.credential_binding.as_deref() == Some("platform") {
        return Err(unavailable());
    }
    let credential = service.api_key_id.ok_or_else(unavailable)?;
    if db
        .collection::<UserApiKey>(CREDENTIALS)
        .find_one(doc! {
            "_id": credential, "user_id": owner, "credential_type": {"$in": ["oauth2", "api_key"]}, "status": "active",
        })
        .await?
        .is_none()
    {
        return Err(unavailable());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn create(
    db: &Database,
    encryption: &EncryptionKeys,
    owner: &str,
    name: &str,
    connection: &str,
    expires_at: DateTime<Utc>,
    policy: Policy,
) -> AppResult<(key_service::CreatedApiKey, PermissionPolicy)> {
    validate_policy(&policy)?;
    if expires_at <= Utc::now() || expires_at > Utc::now() + chrono::Duration::days(90) {
        return Err(AppError::ValidationError(
            "Permission keys require an expiry within 90 days".into(),
        ));
    }
    validate_connection(db, owner, connection).await?;
    let resolved = proxy_service::read_proxy_authority_snapshot_by_user_service_id(
        db, encryption, owner, connection, None,
    )
    .await?
    .ok_or_else(unavailable)?;
    validate_policy_connection(&policy, &resolved)?;
    let now = Utc::now();
    let binding = PermissionPolicy {
        id: uuid::Uuid::new_v4().to_string(),
        user_id: owner.into(),
        user_service_id: connection.into(),
        catalog_service_id: resolved.target.service.id.clone(),
        execution_authority_digest: authority(&resolved)?,
        policy,
        paused: false,
        revision: 1,
        created_at: now,
        updated_at: now,
    };
    let db = db.clone();
    let name = name.to_owned();
    let bound = binding.clone();
    let mut session = db.client().start_session().await?;
    let created = session
        .start_transaction()
        .and_run2(async move |session| {
            let result: AppResult<_> = async {
                let services = [bound.user_service_id.clone()];
                let key = key_service::create_api_key_with_security_class_and_id(
                    &db,
                    &bound.user_id,
                    Some(&bound.user_id),
                    Some(&bound.id),
                    &name,
                    "proxy",
                    Some(expires_at),
                    None,
                    Some(&services),
                    Some(&[]),
                    Some(false),
                    Some(false),
                    Some(false),
                    None,
                    None,
                    None,
                    None,
                    None,
                    ApiKeyPurpose::PermissionBound,
                    false,
                    Some(&mut *session),
                )
                .await?;
                db.collection::<PermissionPolicy>(POLICIES)
                    .insert_one(&bound)
                    .session(&mut *session)
                    .await?;
                Ok(key)
            }
            .await;
            mutations::transaction_result(result)
        })
        .await
        .map_err(mutations::map_transaction_error)?;
    Ok((created, binding))
}

pub async fn get(db: &Database, owner: &str, id: &str) -> AppResult<PermissionPolicy> {
    db.collection::<PermissionPolicy>(POLICIES)
        .find_one(doc! {"_id": id, "user_id": owner})
        .await?
        .ok_or_else(|| AppError::NotFound("Permission key not found".into()))
}

pub async fn set_paused(
    db: &Database,
    owner: &str,
    id: &str,
    revision: i64,
    paused: bool,
) -> AppResult<PermissionPolicy> {
    let updated = db.collection::<PermissionPolicy>(POLICIES).find_one_and_update(
        doc! {"_id": id, "user_id": owner, "revision": revision},
        doc! {"$set": {"paused": paused, "updated_at": bson::DateTime::now()}, "$inc": {"revision": 1_i64}},
    ).return_document(mongodb::options::ReturnDocument::After).await?;
    updated.ok_or_else(|| {
        AppError::Conflict(
            "Permission key missing or revision changed; reload before updating".into(),
        )
    })
}

pub async fn check_live(db: &Database, binding: &PermissionPolicy) -> AppResult<()> {
    if db.collection::<PermissionPolicy>(POLICIES).find_one(doc! {
        "_id": &binding.id, "user_id": &binding.user_id, "revision": binding.revision, "paused": false,
    }).await?.is_none() { return Err(unavailable()); }
    let key = db
        .collection::<ApiKey>(KEYS)
        .find_one(doc! {
            "_id": &binding.id, "user_id": &binding.user_id, "is_active": true,
        })
        .await?
        .ok_or_else(unavailable)?;
    if key.purpose != ApiKeyPurpose::PermissionBound
        || key.expires_at.is_none_or(|v| v <= Utc::now())
        || key.scopes != "proxy"
        || key.allow_all_services
        || key.allow_all_nodes
        || key.allow_auto_connected_services
        || !key.allowed_node_ids.is_empty()
        || key.allowed_service_ids != [binding.user_service_id.clone()]
    {
        return Err(unavailable());
    }
    validate_connection(db, &binding.user_id, &binding.user_service_id).await
}

struct LiveBinding {
    db: Database,
    binding: PermissionPolicy,
}
#[async_trait]
impl PermissionHook for LiveBinding {
    fn validate(&self, _: HookStage, config: &Value) -> Result<(), Error> {
        if config.is_null() {
            Ok(())
        } else {
            Err(Error::Policy("live binding configuration is host-owned"))
        }
    }
    async fn check(&self, _: HookContext<'_>, _: &Value) -> Result<HookDecision, HookFailure> {
        match check_live(&self.db, &self.binding).await {
            Ok(()) => Ok(HookDecision::Allow),
            Err(AppError::Forbidden(_)) => Ok(HookDecision::Deny),
            Err(_) => Err(HookFailure),
        }
    }
}

pub fn engine(
    db: Database,
    binding: PermissionPolicy,
    transport: Arc<dyn Transport>,
) -> AppResult<Engine> {
    validate_policy(&binding.policy)?;
    let mut policy = binding.policy.clone();
    let mut registry = HookRegistry::builtins();
    registry
        .register("nyxid_live_binding", Arc::new(LiveBinding { db, binding }))
        .map_err(|_| AppError::Internal("Unable to register permission guard".into()))?;
    for (name, stage) in [
        ("nyxid-before", HookStage::BeforeExecute),
        ("nyxid-after", HookStage::AfterResponse),
    ] {
        policy.hooks.push(HookDefinition {
            name: name.into(),
            handler: "nyxid_live_binding".into(),
            stage,
            operations: vec![],
            timeout_ms: 2000,
            config: Value::Null,
        });
    }
    let adapter =
        adapter_for_policy(&policy).map_err(|e| AppError::ValidationError(e.to_string()))?;
    Engine::new_with_hooks(policy, adapter, transport, registry)
        .map_err(|e| AppError::ValidationError(e.to_string()))
}

#[cfg(test)]
#[path = "permission_policy_service_tests.rs"]
mod tests;
