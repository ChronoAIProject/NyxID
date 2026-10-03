//! Specialist operation policy: management compiles once; execution only matches.
use std::collections::BTreeSet;

use futures::TryStreamExt;
use mongodb::{
    ClientSession, Database,
    bson::{self, doc},
};
use serde::Serialize;

use crate::{
    errors::{AppError, AppResult},
    models::{
        agent_operation_scope::{
            AgentOperationScope, OperationScopes, OperationSelection, ScopedOperation,
        },
        assistant_agent::{AssistantAgent, COLLECTION_NAME as AGENTS},
        assistant_conversation::{AssistantConversation, COLLECTION_NAME as CONVERSATIONS},
        downstream_service::{ProxyOperationPolicy, ProxyOperationRule},
        service_endpoint::{COLLECTION_NAME as ENDPOINTS, ServiceEndpoint},
        user_service::{COLLECTION_NAME as SERVICES, UserService},
    },
    services::{
        api_key_mutation_service as transactions, assistant_team_service as team, mcp_service,
        proxy_authorization::{self, CanonicalPath},
    },
};

/// No authority is inferred from aliases when an exact instance is resolved.
/// Catalog-only routes conservatively intersect all scopes bound to that catalog.
pub fn applicable<'a>(
    scopes: &'a OperationScopes,
    service_id: &'a str,
    catalog_id: Option<&'a str>,
) -> impl Iterator<Item = &'a AgentOperationScope> {
    scopes.iter().filter_map(move |(id, scope)| {
        (id == service_id
            || catalog_id == Some(id.as_str())
            || (catalog_id.is_none() && scope.catalog_service_id.as_deref() == Some(service_id)))
        .then_some(scope)
    })
}

fn refused(scope: &AgentOperationScope) -> AppError {
    const MAX_BYTES: usize = 2048;
    const DISCOVERY: &str = ". Use nyx__search_tools for the full allowed list.";
    let mut message =
        "Specialist operation scope denied this call. Allowed operations: ".to_owned();
    let mut shown = 0;
    for operation in scope.operations.iter().take(20) {
        // Keep IDs intact; shorten long UTF-8 templates within the remaining
        // byte budget, reserving space for the count and discovery pointer.
        let id = operation
            .endpoint_id
            .as_ref()
            .map(|id| format!(" ({id})"))
            .unwrap_or_default();
        let prefix = format!(
            "{}{} ",
            if shown == 0 { "" } else { ", " },
            operation.rule.method
        );
        let remaining = MAX_BYTES
            .saturating_sub(message.len() + prefix.len() + id.len() + DISCOVERY.len() + 48);
        if remaining < 4 {
            break;
        }
        message.push_str(&prefix);
        let path = &operation.rule.path_template;
        if path.len() <= remaining {
            message.push_str(path);
        } else {
            let mut end = remaining - 3;
            while !path.is_char_boundary(end) {
                end -= 1;
            }
            message.push_str(&path[..end]);
            message.push_str("...");
        }
        message.push_str(&id);
        shown += 1;
    }
    if scope.operations.is_empty() {
        message.push_str("none");
    } else if shown < scope.operations.len() {
        message.push_str(&format!(
            "{}and {} more",
            if shown == 0 { "" } else { "; " },
            scope.operations.len() - shown
        ));
    }
    message.push_str(DISCOVERY);
    AppError::ApiKeyScopeForbidden(message)
}

/// Configuration only: never consult this flag on execution/auth paths.
pub async fn require_configuration_enabled(db: &Database, owner: &str) -> AppResult<()> {
    if !super::feature_flag_service::personal_flag_enabled(
        db,
        owner,
        super::feature_flag_service::AGENT_OPERATION_SCOPES_FLAG_KEY,
    )
    .await?
    {
        // Keep this separate from Forbidden, which the native tool translates
        // into an owner widening card.
        return Err(AppError::ValidationError(
            "Operation scope configuration is not enabled yet. Existing operation limits still apply.".into(),
        ));
    }
    Ok(())
}

/// Match the final downstream route, never a gateway/pool alias. The returned
/// path must be forwarded so authorization and transport agree on encoding.
#[allow(clippy::too_many_arguments)]
pub fn authorize(
    scopes: &OperationScopes,
    service_id: &str,
    catalog_id: Option<&str>,
    endpoint_id: Option<&str>,
    method: &str,
    path: &CanonicalPath,
    method_override: bool,
    websocket: bool,
) -> AppResult<Option<String>> {
    let mut forwarding = None;
    for scope in applicable(scopes, service_id, catalog_id) {
        if method_override || websocket {
            return Err(refused(scope));
        }
        let operation = scope
            .operations
            .iter()
            .find(|operation| {
                endpoint_id.is_none_or(|id| {
                    operation
                        .endpoint_id
                        .as_deref()
                        .is_none_or(|selected| selected == id)
                }) && proxy_authorization::rule_matches(&operation.rule, method, path)
            })
            .ok_or_else(|| refused(scope))?;
        forwarding = proxy_authorization::rule_forwarding_path(&operation.rule, method, path);
    }
    Ok(forwarding)
}

pub fn mcp_catalog_id(service: &mcp_service::McpToolService) -> Option<&str> {
    match &service.source {
        mcp_service::McpToolSource::UserManaged {
            catalog_service_id, ..
        } => catalog_service_id.as_deref(),
        _ => None,
    }
}

pub fn endpoint_visible(
    scopes: &OperationScopes,
    service: &mcp_service::McpToolService,
    endpoint: &mcp_service::McpToolEndpoint,
) -> bool {
    applicable(scopes, &service.service_id, mcp_catalog_id(service)).all(|scope| {
        scope.operations.iter().any(|operation| {
            // Generic discovery has no operation identity; each concrete call
            // still checks the complete compiled selection before dispatch.
            if service.is_generic_proxy {
                true
            } else if let Some(id) = &operation.endpoint_id {
                (id == &endpoint.endpoint_id
                    || mcp_service::producer_operation_generation(service, endpoint).is_none())
                    && operation.rule.method == endpoint.method
                    && operation.rule.path_template == endpoint.path
            } else {
                operation.rule.method == endpoint.method
                    && operation.rule.path_template == endpoint.path
            }
        })
    })
}

pub fn filter_catalog(scopes: &OperationScopes, services: &mut Vec<mcp_service::McpToolService>) {
    if scopes.is_empty() {
        return;
    }
    for service in services.iter_mut() {
        let endpoints = std::mem::take(&mut service.endpoints);
        service.endpoints = endpoints
            .into_iter()
            .filter(|endpoint| endpoint_visible(scopes, service, endpoint))
            .collect();
    }
    services.retain(|service| !service.endpoints.is_empty());
}

/// Only demonstrable subsets narrow without a card. Endpoint identity and
/// effect marks stay pinned; a variable may be narrowed to one literal segment.
pub fn widens(old: Option<&AgentOperationScope>, new: Option<&AgentOperationScope>) -> bool {
    match (old, new) {
        (None, _) => false,
        (Some(_), None) => true,
        (Some(old), Some(new)) => new.operations.iter().any(|entry| {
            old.catalog_service_id != new.catalog_service_id
                || !old
                    .operations
                    .iter()
                    .any(|previous| operation_covers(previous, entry))
        }),
    }
}

fn operation_covers(old: &ScopedOperation, new: &ScopedOperation) -> bool {
    if old.endpoint_id != new.endpoint_id
        || old.rule.method != new.rule.method
        || old.risk != new.risk
        || old.destructive != new.destructive
        || old.changes_existing != new.changes_existing
    {
        return false;
    }
    let old_segments: Vec<_> = old.rule.path_template.split('/').collect();
    let new_segments: Vec<_> = new.rule.path_template.split('/').collect();
    old_segments.len() == new_segments.len()
        && old_segments
            .iter()
            .zip(new_segments)
            .all(|(previous, next)| {
                *previous == next
                    || (previous.starts_with('{')
                        && previous.ends_with('}')
                        && ((next.starts_with('{') && next.ends_with('}'))
                            || CanonicalPath::from_mcp_literal(next).is_ok_and(|path| {
                                proxy_authorization::rule_matches(
                                    &ProxyOperationRule {
                                        method: old.rule.method.clone(),
                                        path_template: format!("/{previous}"),
                                        ..Default::default()
                                    },
                                    &old.rule.method,
                                    &path,
                                )
                            })))
            })
}

fn revision(agent: &AssistantAgent, service: &str) -> i64 {
    agent
        .operation_scope_revisions
        .get(service)
        .copied()
        .or_else(|| {
            agent
                .operation_scopes
                .get(service)
                .map(|scope| scope.revision)
        })
        .unwrap_or(0)
}

async fn compile(
    db: &Database,
    agent: &AssistantAgent,
    service: &str,
    input: &OperationSelection,
    session: &mut ClientSession,
) -> AppResult<Option<AgentOperationScope>> {
    if !agent
        .grants
        .service_ids
        .iter()
        .chain(&agent.grants.platform_service_ids)
        .any(|id| id == service)
    {
        return Err(AppError::ValidationError(
            "Operation scopes require an existing service grant".into(),
        ));
    }
    if input.expected_revision != revision(agent, service) {
        return Err(AppError::Conflict(
            "Operation scope changed; reload its revision".into(),
        ));
    }
    if input.endpoint_ids.len() + input.rules.len() > 256 || input.expected_revision < 0 {
        return Err(AppError::ValidationError(
            "Select at most 256 operations and a valid revision".into(),
        ));
    }
    if input.all_operations {
        if !input.endpoint_ids.is_empty() || !input.rules.is_empty() {
            return Err(AppError::ValidationError(
                "All operations cannot include a selection".into(),
            ));
        }
        return Ok(None);
    }
    let catalog_service_id = if agent
        .grants
        .platform_service_ids
        .iter()
        .any(|id| id == service)
    {
        Some(service.to_owned())
    } else {
        db.collection::<UserService>(SERVICES)
            .find_one(doc! {"_id":service})
            .session(&mut *session)
            .await?
            .ok_or_else(|| AppError::NotFound("Service not found".into()))?
            .catalog_service_id
    };
    let mut owners = vec![service.to_owned()];
    if let Some(id) = &catalog_service_id {
        owners.push(id.clone());
    }
    let mut cursor = db
        .collection::<ServiceEndpoint>(ENDPOINTS)
        .find(doc! {"service_id":{"$in":owners}})
        .session(&mut *session)
        .await?;
    let endpoints: Vec<ServiceEndpoint> = cursor.stream(&mut *session).try_collect().await?;
    if !input.rules.is_empty() && !endpoints.is_empty() {
        return Err(AppError::ValidationError(
            "Select stable endpoint IDs when endpoint rows exist".into(),
        ));
    }
    let catalog_slug = if let Some(id) = &catalog_service_id {
        db.collection::<bson::Document>(crate::models::downstream_service::COLLECTION_NAME)
            .find_one(doc! {"_id": id})
            .session(&mut *session)
            .await?
            .and_then(|row| row.get_str("slug").ok().map(str::to_owned))
    } else {
        None
    };
    let mut operations = Vec::new();
    let mut ids = BTreeSet::new();
    for id in &input.endpoint_ids {
        if !ids.insert(id) {
            continue;
        }
        let endpoint = endpoints
            .iter()
            .find(|row| &row.id == id && row.is_active)
            .ok_or_else(|| {
                AppError::ValidationError(
                    "Selected endpoint is unavailable for this service".into(),
                )
            })?;
        let rule = ProxyOperationRule {
            method: endpoint.method.clone(),
            path_template: endpoint.path.clone(),
            ..Default::default()
        };
        let policy =
            proxy_authorization::normalize_policy(ProxyOperationPolicy { rules: vec![rule] })?;
        let marks = catalog_slug
            .as_deref()
            .map(|slug| {
                super::catalog_spec_registry::operation_marks(
                    slug,
                    &endpoint.method,
                    &endpoint.path,
                    &endpoint.name,
                )
            })
            .unwrap_or_default();
        operations.push(ScopedOperation {
            endpoint_id: Some(id.clone()),
            rule: policy.rules[0].clone(),
            risk: endpoint.risk,
            destructive: marks.destructive,
            changes_existing: marks.changes_existing,
        });
    }
    for rule in &input.rules {
        if rule.target_id.is_some() || !rule.path_parameter_constraints.is_empty() {
            return Err(AppError::ValidationError(
                "Operation scope rules cannot select targets or parameter grammars".into(),
            ));
        }
    }
    let policy = proxy_authorization::normalize_policy(ProxyOperationPolicy {
        rules: input.rules.clone(),
    })?;
    for rule in policy.rules {
        let operation = ScopedOperation {
            endpoint_id: None,
            rule,
            ..Default::default()
        };
        if !operations.contains(&operation) {
            operations.push(operation);
        }
    }
    Ok(Some(AgentOperationScope {
        revision: input
            .expected_revision
            .checked_add(1)
            .ok_or_else(|| AppError::Conflict("Operation revision exhausted".into()))?,
        catalog_service_id,
        operations,
    }))
}

/// `allow_widening` is supplied only by a human handler or after a digest-bound
/// owner action card. Agent writes always use false until that card is consumed.
pub async fn set(
    db: &Database,
    owner: &str,
    agent_id: &str,
    service: &str,
    input: &OperationSelection,
    allow_widening: bool,
) -> AppResult<AssistantAgent> {
    let mut session = db.client().start_session().await?;
    let db_owned = db.clone();
    let owner_owned = owner.to_owned();
    let agent_id = agent_id.to_owned();
    let service_owned = service.to_owned();
    let input = input.clone();
    let agent = session
        .start_transaction()
        .and_run2(async move |session| {
            let operation = async {
                Box::pin(apply_in_session(
                    &db_owned,
                    &owner_owned,
                    &agent_id,
                    &service_owned,
                    &input,
                    allow_widening,
                    session,
                ))
                .await
            }
            .await;
            transactions::transaction_result(operation)
        })
        .await
        .map_err(transactions::map_transaction_error)?;
    super::audit_service::log_actor_event(db.clone(), &super::audit_service::AuditActor {
        user_id:owner.into(), ip_address:None, user_agent:None, api_key_id:None, api_key_name:None },
        "assistant_agent_operations_changed", Some(serde_json::json!({"owner_id":agent.user_id,"agent_id":agent.id,"service_id":service,
            "revision":revision(&agent, service),"operation_count":agent.operation_scopes.get(service).map(|scope| scope.operations.len())}))).await?;
    Ok(agent)
}

#[derive(Serialize)]
pub struct OperationOption {
    pub endpoint_id: String,
    pub method: String,
    pub path: String,
    pub summary: Option<String>,
    pub read_only: bool,
    pub changes_existing: bool,
}
#[derive(Serialize)]
pub struct ServiceOptions {
    pub service_id: String,
    pub service_slug: String,
    pub service_name: String,
    pub revision: i64,
    pub all_operations: bool,
    pub allows_explicit_rules: bool,
    pub endpoint_ids: Vec<String>,
    pub rules: Vec<ProxyOperationRule>,
    pub operations: Vec<OperationOption>,
}

pub async fn options(
    db: &Database,
    _nodes: &super::node_ws_manager::NodeWsManager,
    owner: &str,
    agent_id: &str,
) -> AppResult<Vec<ServiceOptions>> {
    let mut agent = team::live_specialist(db, owner, agent_id).await?;
    super::org_agent_service::require_maintain(db, owner, &agent).await?;
    if agent.user_id != owner {
        let acl = super::org_agent_service::access(db, owner, &agent.user_id).await?;
        agent
            .grants
            .service_ids
            .retain(|id| acl.allows_resource(id));
    }
    // Management reads durable rows, including offline services. Loading the
    // execution catalog here would fetch remote specs and hide offline grants.
    let mut instances: Vec<UserService> = db
        .collection::<UserService>(SERVICES)
        .find(doc! {"_id":{"$in":&agent.grants.service_ids}})
        .await?
        .try_collect()
        .await?;
    if agent.user_id != owner {
        let acl = super::org_agent_service::access(db, owner, &agent.user_id).await?;
        instances
            .retain(|row| row.user_id == agent.user_id && (!row.admin_only || acl.can_write()));
        agent
            .grants
            .service_ids
            .retain(|id| instances.iter().any(|row| &row.id == id));
    }
    let catalog_ids: Vec<&str> = agent
        .grants
        .platform_service_ids
        .iter()
        .map(String::as_str)
        .chain(
            instances
                .iter()
                .filter_map(|row| row.catalog_service_id.as_deref()),
        )
        .collect();
    let catalogs: Vec<bson::Document> = db
        .collection::<bson::Document>(crate::models::downstream_service::COLLECTION_NAME)
        .find(doc! {"_id":{"$in":catalog_ids}})
        .projection(doc! {"_id":1,"slug":1,"name":1})
        .await?
        .try_collect()
        .await?;
    let endpoint_owners: Vec<&str> = agent
        .grants
        .service_ids
        .iter()
        .map(String::as_str)
        .chain(catalogs.iter().filter_map(|row| row.get_str("_id").ok()))
        .collect();
    let all_endpoints: Vec<ServiceEndpoint> = db
        .collection::<ServiceEndpoint>(ENDPOINTS)
        .find(doc! {"service_id":{"$in":endpoint_owners}})
        .await?
        .try_collect()
        .await?;
    let mut result = Vec::new();
    for service_id in agent
        .grants
        .service_ids
        .iter()
        .chain(&agent.grants.platform_service_ids)
    {
        let instance = instances.iter().find(|row| &row.id == service_id);
        let catalog_id = instance
            .and_then(|row| row.catalog_service_id.as_deref())
            .or_else(|| {
                agent
                    .grants
                    .platform_service_ids
                    .contains(service_id)
                    .then_some(service_id.as_str())
            });
        let catalog = catalogs
            .iter()
            .find(|row| row.get_str("_id").ok() == catalog_id);
        let catalog_slug = catalog.and_then(|row| row.get_str("slug").ok());
        let service_slug = instance
            .map(|row| row.slug.as_str())
            .or(catalog_slug)
            .unwrap_or(service_id);
        let service_name = catalog
            .and_then(|row| row.get_str("name").ok())
            .unwrap_or(service_slug);
        let scope = agent.operation_scopes.get(service_id);
        let endpoints: Vec<&ServiceEndpoint> = all_endpoints
            .iter()
            .filter(|row| {
                &row.service_id == service_id || Some(row.service_id.as_str()) == catalog_id
            })
            .collect();
        let allows_explicit_rules = endpoints.is_empty();
        let mut operations = Vec::new();
        for endpoint in endpoints.into_iter().filter(|row| row.is_active) {
            let marks = catalog_slug
                .map(|slug| {
                    super::catalog_spec_registry::operation_marks(
                        slug,
                        &endpoint.method,
                        &endpoint.path,
                        &endpoint.name,
                    )
                })
                .unwrap_or_default();
            let metadata = mcp_service::McpDurableEndpointMetadata {
                risk: endpoint.risk,
                catalog_contract: true,
                destructive: marks.destructive,
                changes_existing: marks.changes_existing,
                ..Default::default()
            };
            let method = reqwest::Method::from_bytes(endpoint.method.as_bytes())
                .map_err(|_| AppError::Internal("Invalid endpoint method".into()))?;
            let effects = mcp_service::operation_effects(&method, metadata);
            operations.push(OperationOption {
                endpoint_id: endpoint.id.clone(),
                method: endpoint.method.clone(),
                path: endpoint.path.clone(),
                summary: endpoint.description.clone(),
                read_only: effects.reads,
                changes_existing: !effects.uses,
            });
        }
        result.push(ServiceOptions {
            revision: revision(&agent, service_id),
            service_id: service_id.clone(),
            service_slug: service_slug.to_owned(),
            service_name: service_name.to_owned(),
            all_operations: scope.is_none(),
            allows_explicit_rules,
            endpoint_ids: scope
                .map(|s| {
                    s.operations
                        .iter()
                        .filter_map(|op| op.endpoint_id.clone())
                        .collect()
                })
                .unwrap_or_default(),
            rules: scope
                .map(|s| {
                    s.operations
                        .iter()
                        .filter(|op| op.endpoint_id.is_none())
                        .map(|op| op.rule.clone())
                        .collect()
                })
                .unwrap_or_default(),
            operations,
        });
    }
    Ok(result)
}

/// Raw requests retain the same guest/effect classification. Webhook calls
/// requiring an action card must use the MCP card protocol.
pub async fn check_non_mcp_context(
    db: &Database,
    auth: &crate::mw::auth::AuthUser,
    service: &str,
    catalog: Option<&str>,
    method: &str,
    path: &CanonicalPath,
) -> AppResult<bool> {
    check_non_mcp_key_context(
        db,
        &auth.user_id.to_string(),
        auth.api_key_id.as_deref(),
        auth.org_agent_access.as_ref(),
        &auth.assistant_operation_scopes,
        service,
        catalog,
        method,
        path,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn check_non_mcp_key_context(
    db: &Database,
    actor: &str,
    key: Option<&str>,
    access: Option<&std::sync::Arc<super::org_agent_service::RequestAccess>>,
    scopes: &OperationScopes,
    service: &str,
    catalog: Option<&str>,
    method: &str,
    path: &CanonicalPath,
) -> AppResult<bool> {
    use crate::models::assistant_agent::GuestAccess;
    let Some(chat) =
        super::assistant_acknowledgement_service::for_key_with_access(db, actor, key, access)
            .await?
    else {
        return Err(AppError::ApiKeyScopeForbidden(
            "Specialist operation authority requires a live conversation".into(),
        ));
    };
    let method_value = reqwest::Method::from_bytes(method.as_bytes())
        .map_err(|_| AppError::BadRequest("Invalid method".into()))?;
    let mut reads = true;
    let mut uses = true;
    let mut destructive = false;
    for scope in applicable(scopes, service, catalog) {
        for operation in scope
            .operations
            .iter()
            .filter(|op| proxy_authorization::rule_matches(&op.rule, method, path))
        {
            let effects = mcp_service::operation_effects(
                &method_value,
                mcp_service::McpDurableEndpointMetadata {
                    risk: operation.risk,
                    destructive: operation.destructive,
                    changes_existing: operation.changes_existing,
                    catalog_contract: operation.endpoint_id.is_some(),
                    ..Default::default()
                },
            );
            reads &= effects.reads;
            uses &= effects.uses;
            destructive |= effects.destructive;
        }
    }
    if chat.guest {
        let agent = team::agent(db, &chat.user_id, &chat.agent_id).await?;
        let access = agent
            .guest_access
            .get(service)
            .or_else(|| catalog.and_then(|id| agent.guest_access.get(id)))
            .copied()
            .unwrap_or_default();
        let allowed = match access {
            GuestAccess::All => true,
            GuestAccess::Use => uses,
            GuestAccess::Read => reads && uses,
        };
        if !allowed {
            return Err(AppError::ApiKeyScopeForbidden(
                "Guest access does not permit this operation".into(),
            ));
        }
    }
    if super::assistant_acknowledgement_service::webhook_confirmation_required(
        &chat,
        reads,
        destructive,
    ) {
        return Err(AppError::ApiKeyScopeForbidden(
            "Webhook service changes must use MCP and its owner action card".into(),
        ));
    }
    Ok(chat.guest)
}

/// Guests share a thread key, never the owner's approval grants or ability to
/// request approval. Call this before any approval notification or forwarding.
pub fn check_guest_approval(
    guest: bool,
    outcome: &super::approval_service::ApprovalOutcome,
) -> AppResult<()> {
    if guest
        && !matches!(
            outcome,
            super::approval_service::ApprovalOutcome::Allowed { required: false }
        )
    {
        return Err(AppError::ApiKeyScopeForbidden(
            "Guest calls cannot request or use owner approvals".into(),
        ));
    }
    Ok(())
}

pub(crate) async fn apply_in_session(
    db: &Database,
    owner: &str,
    agent_id: &str,
    service: &str,
    input: &OperationSelection,
    allow_widening: bool,
    session: &mut ClientSession,
) -> AppResult<AssistantAgent> {
    Box::pin(require_configuration_enabled(db, owner)).await?;
    let current = team::maintained_agent(db, owner, agent_id).await?;
    super::org_agent_service::authorize_service(db, owner, Some(&current.user_id), Some(service))
        .await?;
    let filter = doc! {"_id":agent_id,"user_id":&current.user_id,"kind":"specialist","destroyed_at":bson::Bson::Null};
    let mut agent = db
        .collection::<AssistantAgent>(AGENTS)
        .find_one(filter.clone())
        .session(&mut *session)
        .await?
        .ok_or_else(|| AppError::NotFound("Specialist not found".into()))?;
    let scope = Box::pin(compile(db, &agent, service, input, session)).await?;
    if !allow_widening && widens(agent.operation_scopes.get(service), scope.as_ref()) {
        return Err(AppError::Forbidden(
            "Widening operation access requires an owner action card".into(),
        ));
    }
    let next = input
        .expected_revision
        .checked_add(1)
        .ok_or_else(|| AppError::Conflict("Operation revision exhausted".into()))?;
    agent
        .operation_scope_revisions
        .insert(service.to_owned(), next);
    match scope {
        Some(scope) => {
            agent.operation_scopes.insert(service.to_owned(), scope);
        }
        None => {
            agent.operation_scopes.remove(service);
        }
    }
    if agent.operation_scope_revisions.len() > 256 {
        return Err(AppError::ValidationError(
            "At most 256 scoped services per agent".into(),
        ));
    }
    let encode = |value| {
        bson::to_bson(value)
            .map_err(|_| AppError::Internal("Operation scope encoding failed".into()))
    };
    db.collection::<AssistantAgent>(AGENTS).update_one(filter, doc! {"$set":{
                "operation_scopes":encode(&agent.operation_scopes)?,
                "operation_scope_revisions":bson::to_bson(&agent.operation_scope_revisions).map_err(|_| AppError::Internal("Operation revision encoding failed".into()))?,
                "updated_at":bson::DateTime::now()}}).session(&mut *session).await?;
    let mut cursor = db
        .collection::<AssistantConversation>(CONVERSATIONS)
        .find(doc! {"agent_id":&agent.id})
        .session(&mut *session)
        .await?;
    let rows: Vec<AssistantConversation> = cursor.stream(&mut *session).try_collect().await?;
    team::sync_thread_authority(db, &agent, &rows, session).await?;
    Ok(agent)
}

pub fn route_visible(
    scopes: &OperationScopes,
    id: &str,
    catalog: Option<&str>,
    method: &str,
    path: &str,
) -> bool {
    applicable(scopes, id, catalog).all(|scope| {
        scope.operations.iter().any(|operation| {
            operation.rule.method == method && operation.rule.path_template == path
        })
    })
}

pub async fn selection_summary(
    db: &Database,
    nodes: &super::node_ws_manager::NodeWsManager,
    owner: &str,
    agent: &str,
    service: &str,
    selection: &OperationSelection,
) -> AppResult<String> {
    let services = options(db, nodes, owner, agent).await?;
    let current = services
        .iter()
        .find(|row| row.service_id == service)
        .ok_or_else(|| AppError::NotFound("Granted service is unavailable".into()))?;
    let before = if current.all_operations {
        "all operations".to_owned()
    } else {
        format!(
            "{} selected operations",
            current.endpoint_ids.len() + current.rules.len()
        )
    };
    let after = if selection.all_operations {
        "all operations".to_owned()
    } else {
        let mut labels = Vec::new();
        for id in &selection.endpoint_ids {
            let endpoint = current
                .operations
                .iter()
                .find(|op| &op.endpoint_id == id)
                .ok_or_else(|| {
                    AppError::ValidationError("Selected operation is unavailable".into())
                })?;
            labels.push(format!("{} {}", endpoint.method, endpoint.path));
        }
        labels.extend(
            selection
                .rules
                .iter()
                .map(|rule| format!("{} {}", rule.method, rule.path_template)),
        );
        if labels.is_empty() {
            "no operations".to_owned()
        } else {
            labels.join(", ")
        }
    };
    Ok(format!(
        "{}: {} → {} (revision {}).",
        current.service_name, before, after, selection.expected_revision
    ))
}

/// Defaults and credential injection must not change the authorized verb.
/// Inspect names with a fixed sentinel, never copy credential material.
pub fn validate_target(target: &super::proxy_service::ProxyTarget, method: &str) -> AppResult<()> {
    let mut headers = axum::http::HeaderMap::new();
    for name in target
        .catalog_default_headers
        .iter()
        .chain(&target.user_service_default_headers)
        .map(|header| header.name.as_str())
        .chain(std::iter::once(target.auth_key_name.as_str()))
    {
        if let Ok(name) = axum::http::HeaderName::from_bytes(name.as_bytes()) {
            headers.insert(name, axum::http::HeaderValue::from_static("DELETE"));
        }
    }
    let base = url::Url::parse(&target.base_url)
        .map_err(|_| AppError::BadRequest("Invalid service URL".into()))?;
    if mcp_service::http_carries_method_override(method, &headers, base.query(), &[])
        || (target.auth_method == "query"
            && mcp_service::http_carries_method_override(
                method,
                &axum::http::HeaderMap::new(),
                Some(&format!(
                    "{}={}",
                    target.auth_key_name,
                    if method == "DELETE" { "GET" } else { "DELETE" }
                )),
                &[],
            ))
    {
        return Err(AppError::ApiKeyScopeForbidden(
            "Specialist operation scopes forbid injected method overrides".into(),
        ));
    }
    Ok(())
}

/// An ungranted inference instance must not evade a catalog's saved scopes by
/// using the assistant model exception. Explicitly granted instances stay independent.
pub fn execution_identity<'a>(
    auth: &crate::mw::auth::AuthUser,
    instance: Option<&'a str>,
    catalog: &'a str,
) -> (&'a str, Option<&'a str>) {
    match instance {
        Some(id)
            if auth.allowed_service_ids.iter().any(|allowed| allowed == id)
                || auth.allow_all_services =>
        {
            (id, Some(catalog))
        }
        _ => (catalog, None),
    }
}
