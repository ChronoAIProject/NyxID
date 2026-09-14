//! Server-owned credential recipients and operation-first HTTP destination selection.
use std::collections::BTreeMap;

use super::proxy_authorization::{CanonicalPath, rule_forwarding_path};
use super::proxy_service::ProxyTarget;
use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::{DownstreamService, ProxyOperationPolicy};

pub fn workspace_targets() -> BTreeMap<String, String> {
    [
        ("docs", "https://docs.googleapis.com"),
        ("sheets", "https://sheets.googleapis.com"),
        ("slides", "https://slides.googleapis.com"),
    ]
    .into_iter()
    .map(|(id, origin)| (id.into(), origin.into()))
    .collect()
}

fn invalid(message: &str) -> AppError {
    AppError::ValidationError(message.to_string())
}

pub fn normalize_origin(value: &str) -> AppResult<String> {
    super::url_validation::validate_base_url(value)?;
    let url = url::Url::parse(value).map_err(|_| invalid("Invalid destination origin"))?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
        || url.host_str().is_none_or(|host| host.contains('*'))
    {
        return Err(invalid(
            "Destination targets must be exact HTTPS origins without userinfo, paths, queries, fragments, or wildcards",
        ));
    }
    Ok(url.origin().ascii_serialization())
}

pub fn validate_target_id(id: &str) -> AppResult<()> {
    if id.is_empty()
        || id.len() > 64
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(invalid(
            "Destination target IDs must contain 1-64 ASCII letters, digits, underscores, or hyphens",
        ));
    }
    Ok(())
}

pub fn normalize_targets(
    slug: &str,
    auth_method: &str,
    service_type: &str,
    targets: BTreeMap<String, String>,
    policy: Option<&ProxyOperationPolicy>,
) -> AppResult<BTreeMap<String, String>> {
    if !targets.is_empty() && (auth_method != "bearer" || service_type != "http") {
        return Err(invalid(
            "Destination targets require bearer HTTP authentication; token_exchange is unsupported",
        ));
    }
    if targets.len() > 32 {
        return Err(invalid("A service may have at most 32 destination targets"));
    }
    let google =
        super::google_workspace::GoogleProduct::from_slug(slug).is_some() || slug == "api-google";
    let mut normalized = BTreeMap::new();
    for (id, origin) in targets {
        validate_target_id(&id)?;
        let origin = normalize_origin(&origin)?;
        if google
            && !workspace_targets()
                .values()
                .any(|allowed| allowed == &origin)
        {
            return Err(invalid(
                "Google destination targets must be docs.googleapis.com, sheets.googleapis.com, or slides.googleapis.com on HTTPS port 443",
            ));
        }
        if normalized.values().any(|existing| existing == &origin) {
            return Err(invalid("Destination origins must have unique target IDs"));
        }
        normalized.insert(id, origin);
    }
    if let Some(policy) = policy {
        for rule in &policy.rules {
            if let Some(id) = &rule.target_id {
                validate_target_id(id)?;
                if !normalized.contains_key(id) {
                    return Err(invalid(
                        "Operation target is absent from the service destination map",
                    ));
                }
            }
        }
    }
    if !normalized.is_empty() && policy.is_none() {
        return Err(invalid("Destination targets require an operation policy"));
    }
    Ok(normalized)
}

/// The origin is selected only from trusted catalog metadata, never an instance spec.
/// Multiple matching rules may not disagree about the credential recipient.
pub fn select_target(
    service: &DownstreamService,
    method: &str,
    path: &CanonicalPath,
) -> AppResult<(String, Option<String>)> {
    let forwarding = super::proxy_authorization::authorize_proxy_operation(service, method, path)?;
    let mut selected: Option<Option<String>> = None;
    if let Some(policy) = &service.proxy_operation_policy {
        for rule in &policy.rules {
            if rule_forwarding_path(rule, method, path).is_some() {
                if selected.as_ref().is_some_and(|old| old != &rule.target_id) {
                    return Err(invalid("Ambiguous operation destination"));
                }
                selected = Some(rule.target_id.clone());
            }
        }
    }
    if !service.destination_targets.is_empty() && selected.is_none() {
        return Err(AppError::NotFound("Service operation not found".into()));
    }
    let target_id = selected.flatten();
    if let Some(id) = &target_id {
        let origin = service.destination_targets.get(id).ok_or_else(|| {
            invalid("Operation target is absent from the service destination map")
        })?;
        if normalize_origin(origin)? != *origin {
            return Err(invalid("Destination origin is not normalized"));
        }
    }
    Ok((forwarding, target_id))
}

pub fn resolve_target(
    target: &mut ProxyTarget,
    method: &str,
    path: &CanonicalPath,
    selected_endpoint_target: Option<Option<&str>>,
) -> AppResult<String> {
    if target.workspace_destinations_pending && workspace_editor_operation(method, path) {
        return Err(AppError::WorkspaceDestinationsNotActivated);
    }
    let (path, target_id) = select_target(&target.service, method, path)?;
    if selected_endpoint_target.is_some_and(|selected| selected != target_id.as_deref()) {
        return Err(invalid(
            "Endpoint target does not match the authorized operation",
        ));
    }
    if let Some(id) = &target_id {
        if target.auth_method != "bearer"
            && !(target.auth_method == "none"
                && target.service.requires_user_credential
                && target.service.provider_config_id.is_some())
        {
            return Err(invalid("Destination targets require bearer authentication"));
        }
        target.base_url = target.service.destination_targets[id].clone();
    }
    target.target_id = target_id;
    Ok(path)
}

pub fn reject_websocket(target: &ProxyTarget) -> AppResult<()> {
    if target.target_id.is_some() {
        return Err(AppError::BadRequest(
            "Target-selected operations support HTTP only".into(),
        ));
    }
    Ok(())
}

/// A credential override may inherit recipients only from the same provider or
/// an existing connection with the identical server-owned destination map.
pub async fn validate_override_recipient(
    db: &mongodb::Database,
    user_service_id: &str,
    credential: &crate::models::user_api_key::UserApiKey,
) -> AppResult<()> {
    use crate::models::{downstream_service, user_service};
    use futures::TryStreamExt;
    use mongodb::bson::doc;
    let service = db
        .collection::<user_service::UserService>(user_service::COLLECTION_NAME)
        .find_one(doc! {"_id": user_service_id})
        .await?
        .ok_or_else(|| invalid("Override service is unavailable"))?;
    let Some(catalog_id) = service.catalog_service_id.as_deref() else {
        return Ok(());
    };
    let Some(catalog) = db
        .collection::<DownstreamService>(downstream_service::COLLECTION_NAME)
        .find_one(doc! {"_id": catalog_id})
        .await?
    else {
        return Ok(());
    };
    if catalog.destination_targets.is_empty() {
        return Ok(());
    }
    normalize_targets(
        &catalog.slug,
        &service.auth_method,
        &catalog.service_type,
        catalog.destination_targets.clone(),
        catalog.proxy_operation_policy.as_ref(),
    )?;
    if catalog.provider_config_id.is_some()
        && catalog.provider_config_id == credential.provider_config_id
        && credential.credential_type == "oauth2"
    {
        return Ok(());
    }
    let sources: Vec<user_service::UserService> = db.collection::<user_service::UserService>(user_service::COLLECTION_NAME)
        .find(doc! {"api_key_id": &credential.id, "user_id": &credential.user_id, "is_active": true, "auth_method": "bearer"})
        .await?.try_collect().await?;
    for source in sources {
        if let Some(source_id) = source.catalog_service_id
            && let Some(source_catalog) = db
                .collection::<DownstreamService>(downstream_service::COLLECTION_NAME)
                .find_one(doc! {"_id":source_id})
                .await?
            && source_catalog.destination_targets == catalog.destination_targets
        {
            return Ok(());
        }
    }
    Err(invalid(
        "Override credential has no authority for this service's destination recipients",
    ))
}

pub async fn validate_selected_override(
    db: &mongodb::Database,
    user_id: &str,
    api_key_id: &str,
    user_service_id: &str,
    target: &ProxyTarget,
) -> AppResult<()> {
    if target.target_id.is_none() {
        return Ok(());
    }
    let Some(id) = super::agent_binding_service::resolve_credential_override(
        db,
        api_key_id,
        user_service_id,
        user_id,
    )
    .await?
    else {
        return Ok(());
    };
    let credential = db
        .collection::<crate::models::user_api_key::UserApiKey>(
            crate::models::user_api_key::COLLECTION_NAME,
        )
        .find_one(mongodb::bson::doc! {"_id": id, "user_id": user_id})
        .await?
        .ok_or_else(|| invalid("Override credential is unavailable"))?;
    validate_override_recipient(db, user_service_id, &credential).await
}

/// Google catalog rows retain `auth_method = none`: their bearer injection is
/// owned by ServiceProviderRequirement. Never rewrite that legacy contract.
pub async fn effective_catalog_auth(
    db: &mongodb::Database,
    service: &DownstreamService,
) -> AppResult<String> {
    use crate::models::service_provider_requirement::{
        COLLECTION_NAME, ServiceProviderRequirement,
    };
    use futures::TryStreamExt;
    let requirements: Vec<ServiceProviderRequirement> = db
        .collection(COLLECTION_NAME)
        .find(mongodb::bson::doc! {"service_id": &service.id})
        .await?
        .try_collect()
        .await?;
    if requirements.iter().any(|requirement| {
        requirement.injection_method != "bearer"
            || Some(&requirement.provider_config_id) != service.provider_config_id.as_ref()
    }) {
        return Err(invalid(
            "Destination targets require bearer credentials from the service provider",
        ));
    }
    Ok(super::unified_key_service::derive_effective_auth(service, requirements.first()).0)
}

pub fn validate_node_outbound_destination(
    target: &ProxyTarget,
    method: &str,
    path: &str,
    delegated: &[super::delegation_service::DelegatedCredential],
) -> AppResult<()> {
    validate_outbound_injection(target, method, path, delegated, true)
}

pub fn validate_outbound_destination(
    target: &ProxyTarget,
    method: &str,
    path: &str,
    delegated: &[super::delegation_service::DelegatedCredential],
) -> AppResult<()> {
    validate_outbound_injection(target, method, path, delegated, false)
}

fn validate_outbound_injection(
    target: &ProxyTarget,
    method: &str,
    path: &str,
    delegated: &[super::delegation_service::DelegatedCredential],
    node_local_bearer: bool,
) -> AppResult<()> {
    if target.service.destination_targets.is_empty() && target.target_id.is_none() {
        return Ok(());
    }
    let canonical = CanonicalPath::from_mcp_built(path)?;
    let (_, id) = select_target(&target.service, method, &canonical)?;
    if id != target.target_id
        || id
            .as_ref()
            .is_some_and(|id| target.service.destination_targets.get(id) != Some(&target.base_url))
    {
        return Err(invalid(
            "Resolved destination does not match the authorized operation",
        ));
    }
    if target.target_id.is_some()
        && (delegated
            .iter()
            .any(|credential| credential.injection_method != "bearer")
            || (target.auth_method != "bearer"
                && !(target.auth_method == "none" && (!delegated.is_empty() || node_local_bearer))))
    {
        return Err(invalid("Destination targets require bearer authentication"));
    }
    Ok(())
}

/// Metadata-only lifecycle events use the ordinary chained audit append path.
pub struct DestinationAudit {
    context: Option<(
        mongodb::Database,
        super::audit_service::AuditActor,
        serde_json::Value,
    )>,
    dispatched: bool,
    completed: bool,
}

impl DestinationAudit {
    pub fn new(
        db: &mongodb::Database,
        actor: super::audit_service::AuditActor,
        target: &ProxyTarget,
    ) -> Self {
        let context = target.target_id.as_ref().and_then(|id| {
            let origin = normalize_origin(&target.base_url).ok()?;
            Some((
                db.clone(),
                actor,
                serde_json::json!({
                    "service_id": target.service.id, "target_id": id, "destination_origin": origin,
                }),
            ))
        });
        Self {
            context,
            dispatched: false,
            completed: false,
        }
    }

    fn record(&self, event: &str, status: Option<u16>, outcome: &str) {
        if let Some((db, actor, metadata)) = &self.context {
            let mut data = metadata.clone();
            data["outcome"] = outcome.into();
            if let Some(status) = status {
                data["response_status"] = status.into();
            }
            super::audit_service::log_async(
                db.clone(),
                Some(actor.user_id.clone()),
                event.into(),
                Some(data),
                actor.ip_address.clone(),
                actor.user_agent.clone(),
                actor.api_key_id.clone(),
                actor.api_key_name.clone(),
            );
        }
    }

    pub fn dismiss(&mut self) {
        self.completed = true;
    }

    pub fn denied(&mut self) {
        self.record("proxy_target_denied", None, "not_dispatched");
        self.completed = true;
    }

    pub fn dispatch(&mut self) {
        self.completed = false;
        self.dispatched = true;
        self.record("proxy_target_dispatch", None, "dispatch_requested");
    }

    pub fn complete(&mut self, status: u16) {
        self.completed = true;
        self.record("proxy_target_complete", Some(status), "response");
    }
}

impl Drop for DestinationAudit {
    fn drop(&mut self) {
        if !self.completed {
            self.record(
                if self.dispatched {
                    "proxy_target_complete"
                } else {
                    "proxy_target_denied"
                },
                None,
                if self.dispatched {
                    "no_response"
                } else {
                    "not_dispatched"
                },
            );
        }
    }
}

/// Temporary upgrade state, derived from the exact pre-B default rather than
/// process configuration. All replicas agree once the shared catalog activates.
pub fn workspace_destinations_pending(service: &DownstreamService) -> bool {
    if service.slug != "api-google-workspace" || !service.destination_targets.is_empty() {
        return false;
    }
    let Ok(mut policy) = super::google_workspace::GoogleProduct::Workspace.operation_policy()
    else {
        return false;
    };
    policy.rules.retain(|rule| rule.target_id.is_none());
    service.proxy_operation_policy.as_ref() == Some(&policy)
}

pub fn workspace_editor_operation(method: &str, path: &CanonicalPath) -> bool {
    [
        super::google_workspace::GoogleProduct::Docs,
        super::google_workspace::GoogleProduct::Sheets,
        super::google_workspace::GoogleProduct::Slides,
    ]
    .into_iter()
    .any(|product| {
        product.operation_policy().is_ok_and(|policy| {
            policy
                .rules
                .iter()
                .any(|rule| rule_forwarding_path(rule, method, path).is_some())
        })
    })
}

#[cfg(test)]
#[path = "destination_routing_tests.rs"]
pub(crate) mod tests;
