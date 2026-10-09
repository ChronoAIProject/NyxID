//! Specialist operation policy: management compiles once; execution only matches.
use std::collections::{BTreeMap, BTreeSet};

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
        api_key::{ApiKey, ApiKeyPurpose, COLLECTION_NAME as API_KEYS},
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

/// Whether any saved scope applies to this execution identity.
pub fn is_scoped(scopes: &OperationScopes, service_id: &str, catalog_id: Option<&str>) -> bool {
    applicable(scopes, service_id, catalog_id).next().is_some()
}

/// A selection reviewed against a contract digest must still compile to it.
fn ensure_reviewed(
    input: &OperationSelection,
    scope: Option<&AgentOperationScope>,
) -> AppResult<()> {
    if input
        .contract_digest
        .as_deref()
        .is_some_and(|expected| expected != contract_digest(scope))
    {
        return Err(AppError::Conflict(
            "Operations changed since this selection was reviewed; review it again".into(),
        ));
    }
    Ok(())
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

/// Names of a template's `{variable}` segments (custom-method suffixes ignored).
fn template_variables(template: &str) -> BTreeSet<&str> {
    template
        .split('/')
        .filter_map(|segment| segment.strip_prefix('{')?.split_once('}'))
        .map(|(name, _)| name)
        .collect()
}

/// Value limits of the matched operations, checked against the final request.
/// Call after `authorize` wherever the query and body are known. A request
/// passes a scope when any operation it matches accepts its inputs.
#[allow(clippy::too_many_arguments)]
pub fn check_inputs(
    scopes: &OperationScopes,
    service_id: &str,
    catalog_id: Option<&str>,
    endpoint_id: Option<&str>,
    method: &str,
    path: &CanonicalPath,
    query: Option<&str>,
    body: &[u8],
    content_type: Option<&str>,
) -> AppResult<()> {
    for scope in applicable(scopes, service_id, catalog_id) {
        let mut refusal = None;
        let accepted = scope
            .operations
            .iter()
            .filter(|operation| {
                endpoint_id.is_none_or(|id| {
                    operation
                        .endpoint_id
                        .as_deref()
                        .is_none_or(|selected| selected == id)
                }) && proxy_authorization::rule_matches(&operation.rule, method, path)
            })
            .any(|operation| {
                let Some(inputs) = &operation.inputs else {
                    return true;
                };
                let arguments = proxy_authorization::match_path_arguments(&operation.rule, path)
                    .unwrap_or_default();
                match inputs.check(&arguments, query, body, content_type) {
                    Ok(()) => true,
                    Err(error) => {
                        refusal = Some(error);
                        false
                    }
                }
            });
        if !accepted {
            return Err(AppError::ApiKeyScopeForbidden(match refusal {
                Some(error) => format!("Operation scope input limit: {error}"),
                None => "Operation scope denied this call".into(),
            }));
        }
    }
    Ok(())
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

/// The whole compiled operation is the contract: an identical entry covers;
/// otherwise only replacing a single-use, unconstrained variable with a literal
/// it matches narrows. Renaming variables, changing parameter grammars or
/// narrowing a repeated variable can widen what matches, so it never covers.
fn operation_covers(old: &ScopedOperation, new: &ScopedOperation) -> bool {
    if old == new {
        return true;
    }
    if old.endpoint_id != new.endpoint_id
        || old.rule.method != new.rule.method
        || old.rule.target_id != new.rule.target_id
        || !old.rule.path_parameter_constraints.is_empty()
        || !new.rule.path_parameter_constraints.is_empty()
        || old.risk != new.risk
        || old.destructive != new.destructive
        || old.changes_existing != new.changes_existing
        // Adding input limits narrows; changing or removing existing ones may widen.
        || (old.inputs.is_some() && old.inputs != new.inputs)
    {
        return false;
    }
    let old_segments: Vec<_> = old.rule.path_template.split('/').collect();
    let new_segments: Vec<_> = new.rule.path_template.split('/').collect();
    let uses = |segment: &str| old_segments.iter().filter(|s| **s == segment).count();
    old_segments.len() == new_segments.len()
        && old_segments
            .iter()
            .zip(new_segments)
            .all(|(previous, next)| {
                *previous == next
                    || (previous.starts_with('{')
                        && previous.ends_with('}')
                        && uses(previous) == 1
                        && !(next.starts_with('{') && next.ends_with('}'))
                        && (CanonicalPath::from_mcp_literal(next).is_ok_and(|path| {
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

/// What a scope holder (specialist or Agent Key) may scope: its granted
/// instances and platform catalog services, and the service's current revision.
struct Holder<'a> {
    service_ids: &'a [String],
    platform_service_ids: &'a [String],
    revision: i64,
}

async fn compile(
    db: &Database,
    agent: &AssistantAgent,
    service: &str,
    input: &OperationSelection,
    session: &mut ClientSession,
) -> AppResult<Option<AgentOperationScope>> {
    let holder = Holder {
        service_ids: &agent.grants.service_ids,
        platform_service_ids: &agent.grants.platform_service_ids,
        revision: revision(agent, service),
    };
    Box::pin(compile_for(db, &holder, service, input, session)).await
}

async fn compile_for(
    db: &Database,
    holder: &Holder<'_>,
    service: &str,
    input: &OperationSelection,
    session: &mut ClientSession,
) -> AppResult<Option<AgentOperationScope>> {
    if !holder
        .service_ids
        .iter()
        .chain(holder.platform_service_ids)
        .any(|id| id == service)
    {
        return Err(AppError::ValidationError(
            "Operation scopes require an existing service grant".into(),
        ));
    }
    if input.expected_revision != holder.revision {
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
    let catalog_service_id = if holder.platform_service_ids.iter().any(|id| id == service) {
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
        let rule = proxy_authorization::rule_from_endpoint(
            &endpoint.method,
            &endpoint.path,
            endpoint.parameters.as_ref(),
        )?;
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
        let rule = policy.rules[0].clone();
        let inputs = match input.inputs.get(id) {
            Some(inputs) if !inputs.is_empty() => {
                inputs
                    .validate(&rule.method, &template_variables(&rule.path_template))
                    .map_err(|error| AppError::ValidationError(error.to_string()))?;
                Some(inputs.clone())
            }
            _ => None,
        };
        operations.push(ScopedOperation {
            endpoint_id: Some(id.clone()),
            rule,
            risk: endpoint.risk,
            destructive: marks.destructive,
            changes_existing: marks.changes_existing,
            inputs,
        });
    }
    if input.inputs.keys().any(|id| !ids.contains(id)) {
        return Err(AppError::ValidationError(
            "Input limits must name selected endpoint IDs".into(),
        ));
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

/// Digest of the compiled contract a selection resolves to, excluding its
/// revision. Owner cards carry it so endpoint edits cannot ride an approval.
pub fn contract_digest(scope: Option<&AgentOperationScope>) -> String {
    use sha2::{Digest, Sha256};
    let value = scope.map(|scope| (&scope.catalog_service_id, &scope.operations));
    let bytes = serde_json::to_vec(&value).unwrap_or_default();
    hex::encode(Sha256::digest(bytes))
}

/// Compile a selection without saving it, returning the digest an owner card binds.
pub async fn preview_digest(
    db: &Database,
    owner: &str,
    agent_id: &str,
    service: &str,
    input: &OperationSelection,
) -> AppResult<String> {
    let agent = team::maintained_agent(db, owner, agent_id).await?;
    preview_agent_digest(db, &agent, service, input).await
}

/// Server-internal preview for a specialist's own permission request; the
/// decision path re-authorizes the decider before anything is applied.
pub async fn preview_request_digest(
    db: &Database,
    agent_id: &str,
    service: &str,
    input: &OperationSelection,
) -> AppResult<String> {
    let agent = db
        .collection::<AssistantAgent>(AGENTS)
        .find_one(doc! {"_id": agent_id, "kind": "specialist", "destroyed_at": bson::Bson::Null})
        .await?
        .ok_or_else(|| AppError::NotFound("Specialist not found".into()))?;
    preview_agent_digest(db, &agent, service, input).await
}

async fn preview_agent_digest(
    db: &Database,
    agent: &AssistantAgent,
    service: &str,
    input: &OperationSelection,
) -> AppResult<String> {
    let mut session = db.client().start_session().await?;
    let scope = Box::pin(compile(db, agent, service, input, &mut session)).await?;
    Ok(contract_digest(scope.as_ref()))
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
    /// Saved value limits per selected endpoint ID; clients send them back.
    pub inputs: std::collections::BTreeMap<String, nyxid_permissions::values::InputRules>,
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
    let service_ids = agent.grants.service_ids.clone();
    let platform_ids = agent.grants.platform_service_ids.clone();
    Box::pin(build_options(
        db,
        &instances,
        &service_ids,
        &platform_ids,
        &agent.operation_scopes,
        |service| revision(&agent, service),
    ))
    .await
}

async fn build_options(
    db: &Database,
    instances: &[UserService],
    service_ids: &[String],
    platform_ids: &[String],
    scopes: &OperationScopes,
    revision_of: impl Fn(&str) -> i64,
) -> AppResult<Vec<ServiceOptions>> {
    let catalog_ids: Vec<&str> = platform_ids
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
    let endpoint_owners: Vec<&str> = service_ids
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
    for service_id in service_ids.iter().chain(platform_ids) {
        let instance = instances.iter().find(|row| &row.id == service_id);
        let catalog_id = instance
            .and_then(|row| row.catalog_service_id.as_deref())
            .or_else(|| {
                platform_ids
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
        let scope = scopes.get(service_id);
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
            revision: revision_of(service_id),
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
            inputs: scope
                .map(|s| {
                    s.operations
                        .iter()
                        .filter_map(|op| Some((op.endpoint_id.clone()?, op.inputs.clone()?)))
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
///
/// Conversation keys are checked whether or not the service is scoped: guest
/// and webhook limits are turn authority, not a property of the scope. Other
/// keys carry only their operation scope, which `authorize` enforces, and
/// acquire no database reads here.
pub async fn check_non_mcp_context(
    db: &Database,
    auth: &crate::mw::auth::AuthUser,
    scoped: bool,
    service: &str,
    catalog: Option<&str>,
    method: &str,
    path: Option<&CanonicalPath>,
) -> AppResult<bool> {
    if let Some(chat) = auth.assistant_chat.as_deref() {
        return check_non_mcp_chat_context(
            db,
            chat,
            &auth.assistant_operation_scopes,
            service,
            catalog,
            method,
            path,
        )
        .await;
    }
    if !scoped || (auth.assistant_agent_owner_id.is_none() && auth.assistant_group_id.is_none()) {
        return Ok(false);
    }
    let path = path.ok_or_else(|| {
        AppError::ApiKeyScopeForbidden("Scoped operations require a canonical path".into())
    })?;
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
    let Some(chat) =
        super::assistant_acknowledgement_service::for_key_with_access(db, actor, key, access)
            .await?
    else {
        return Err(AppError::ApiKeyScopeForbidden(
            "Specialist operation authority requires a live conversation".into(),
        ));
    };
    check_non_mcp_chat_context(db, &chat, scopes, service, catalog, method, Some(path)).await
}

/// Whether an assistant key's live conversation is a guest turn. Used where
/// guests are refused outright, so no operation effects are needed.
pub async fn key_is_guest(
    db: &Database,
    actor: &str,
    key: Option<&str>,
    access: Option<&std::sync::Arc<super::org_agent_service::RequestAccess>>,
) -> AppResult<bool> {
    Ok(
        super::assistant_acknowledgement_service::for_key_with_access(db, actor, key, access)
            .await?
            .is_some_and(|chat| chat.guest),
    )
}

async fn check_non_mcp_chat_context(
    db: &Database,
    chat: &super::assistant_acknowledgement_service::ChatAuthority,
    scopes: &OperationScopes,
    service: &str,
    catalog: Option<&str>,
    method: &str,
    path: Option<&CanonicalPath>,
) -> AppResult<bool> {
    use crate::models::assistant_agent::GuestAccess;
    let method_value = reqwest::Method::from_bytes(method.as_bytes())
        .map_err(|_| AppError::BadRequest("Invalid method".into()))?;
    let mut matched: Option<mcp_service::OperationEffects> = None;
    if let Some(path) = path {
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
                matched = Some(intersect_effects(matched, effects));
            }
        }
    }
    let effect_limited = chat.guest || chat.confirmation_policy.is_some();
    let effects = match matched {
        Some(effects) => effects,
        // Unscoped services keep their guest and webhook limits: classify from
        // the stored endpoint contract, or conservatively from the method.
        None if effect_limited => {
            Box::pin(unscoped_effects(db, service, catalog, &method_value, path)).await?
        }
        None => mcp_service::operation_effects(&method_value, Default::default()),
    };
    let (reads, uses, destructive) = (effects.reads, effects.uses, effects.destructive);
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
        chat,
        reads,
        destructive,
    ) {
        return Err(AppError::ApiKeyScopeForbidden(
            "Webhook service changes must use MCP and its owner action card".into(),
        ));
    }
    Ok(chat.guest)
}

fn intersect_effects(
    current: Option<mcp_service::OperationEffects>,
    next: mcp_service::OperationEffects,
) -> mcp_service::OperationEffects {
    match current {
        None => next,
        Some(current) => mcp_service::OperationEffects {
            reads: current.reads && next.reads,
            uses: current.uses && next.uses,
            destructive: current.destructive || next.destructive,
        },
    }
}

/// Effects of a raw call on a service without a saved scope. Stored endpoint
/// rows are catalog contracts; with no matching row the method decides, which
/// never treats a POST as a read.
async fn unscoped_effects(
    db: &Database,
    service: &str,
    catalog: Option<&str>,
    method: &reqwest::Method,
    path: Option<&CanonicalPath>,
) -> AppResult<mcp_service::OperationEffects> {
    let fallback = mcp_service::operation_effects(method, Default::default());
    let Some(path) = path else {
        return Ok(fallback);
    };
    let owners: Vec<&str> = std::iter::once(service).chain(catalog).collect();
    let endpoints: Vec<ServiceEndpoint> = db
        .collection::<ServiceEndpoint>(ENDPOINTS)
        .find(doc! {"service_id": {"$in": &owners}, "is_active": true})
        .await?
        .try_collect()
        .await?;
    let candidates: Vec<&ServiceEndpoint> = endpoints
        .iter()
        .filter(|row| row.method.eq_ignore_ascii_case(method.as_str()))
        .filter(|row| {
            proxy_authorization::rule_from_endpoint(&row.method, &row.path, row.parameters.as_ref())
                .is_ok_and(|rule| proxy_authorization::rule_matches(&rule, method.as_str(), path))
        })
        .collect();
    if candidates.is_empty() {
        return Ok(fallback);
    }
    let catalog_slug = db
        .collection::<bson::Document>(crate::models::downstream_service::COLLECTION_NAME)
        .find_one(doc! {"_id": catalog.unwrap_or(service)})
        .projection(doc! {"slug": 1})
        .await?
        .and_then(|row| row.get_str("slug").ok().map(str::to_owned));
    let mut effects = None;
    for endpoint in candidates {
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
        effects = Some(intersect_effects(
            effects,
            mcp_service::operation_effects(
                method,
                mcp_service::McpDurableEndpointMetadata {
                    risk: endpoint.risk,
                    catalog_contract: true,
                    destructive: marks.destructive,
                    changes_existing: marks.changes_existing,
                    ..Default::default()
                },
            ),
        ));
    }
    Ok(effects.unwrap_or(fallback))
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
    ensure_reviewed(input, scope.as_ref())?;
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
    let limits = limit_changes(current, selection);
    Ok(format!(
        "{}: {} → {} (revision {}).{}",
        current.service_name, before, after, selection.expected_revision, limits
    ))
}

/// Spell out value-limit changes so an owner never approves an opaque digest.
pub(crate) fn limit_changes(current: &ServiceOptions, selection: &OperationSelection) -> String {
    const MAX_VALUE: usize = 300;
    let kept = |id: &String| !selection.all_operations && selection.endpoint_ids.contains(id);
    let ids: BTreeSet<&String> = current
        .inputs
        .keys()
        .chain(selection.inputs.keys())
        .collect();
    let mut changes = Vec::new();
    for id in ids {
        // Empty limits compile away, so they read as no limits.
        let before = current.inputs.get(id).filter(|rules| !rules.is_empty());
        let after = selection
            .inputs
            .get(id)
            .filter(|rules| kept(id) && !rules.is_empty());
        if before == after {
            continue;
        }
        let label = current
            .operations
            .iter()
            .find(|op| &op.endpoint_id == id)
            .map_or_else(|| id.clone(), |op| format!("{} {}", op.method, op.path));
        let change = match after {
            None => "limits removed".to_owned(),
            Some(rules) => {
                let mut value = serde_json::to_string(rules).unwrap_or_default();
                if value.len() > MAX_VALUE {
                    let mut end = MAX_VALUE;
                    while !value.is_char_boundary(end) {
                        end -= 1;
                    }
                    value.truncate(end);
                    value.push('…');
                }
                format!(
                    "limits {} {value}",
                    if before.is_some() {
                        "changed to"
                    } else {
                        "set to"
                    }
                )
            }
        };
        changes.push(format!("{label}: {change}"));
    }
    if changes.is_empty() {
        String::new()
    } else {
        format!(" Value limits — {}.", changes.join("; "))
    }
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

// Agent Key operation scopes. The same compiled scopes as specialists, stored
// on the key itself; the owner is a human, so widening needs no action card.

/// Ordinary general-purpose keys only. Conversation keys mirror their agent's
/// scopes and permission-bound keys carry their own immutable policy.
fn ensure_scopable_key(key: &ApiKey) -> AppResult<()> {
    if !key.is_active
        || key.purpose != ApiKeyPurpose::General
        || key.assistant_agent_owner_id.is_some()
        || key.assistant_group_id.is_some()
        || crate::mw::auth::is_assistant_conversation_key_candidate(key)
    {
        return Err(AppError::ValidationError(
            "Operation scopes can only be set on ordinary Agent Keys".into(),
        ));
    }
    Ok(())
}

async fn accessible_key(
    db: &Database,
    actor: &str,
    key_id: &str,
    write: bool,
) -> AppResult<(ApiKey, super::org_service::OwnerAccess)> {
    let key = db
        .collection::<ApiKey>(API_KEYS)
        .find_one(doc! {"_id": key_id})
        .await?
        .ok_or_else(|| AppError::NotFound("API key not found".into()))?;
    let access = super::org_service::resolve_owner_access(db, actor, &key.user_id).await?;
    if !access.can_read() {
        return Err(AppError::NotFound("API key not found".into()));
    }
    if write && !access.can_write() {
        return Err(AppError::OrgRoleInsufficient(
            "you do not have permission to change operations on this API key".into(),
        ));
    }
    ensure_scopable_key(&key)?;
    Ok((key, access))
}

/// Services a key may scope: its effective allowlist (including auto-connected
/// rows), or every live service of its owner when it allows all services.
async fn key_instances(
    db: &Database,
    key: &ApiKey,
    access: &super::org_service::OwnerAccess,
) -> AppResult<Vec<UserService>> {
    let filter = if key.allow_all_services {
        doc! {"user_id": &key.user_id, "is_active": true}
    } else {
        let ids =
            super::key_service::effective_allowed_service_ids_with_access(db, key, None).await?;
        doc! {"_id": {"$in": ids}, "is_active": true}
    };
    let mut instances: Vec<UserService> = db
        .collection::<UserService>(SERVICES)
        .find(filter)
        .await?
        .try_collect()
        .await?;
    // SSH services have no HTTP operations; the allowlist governs them.
    instances.retain(|row| row.service_type != "ssh" && access.allows_resource(&row.id));
    instances.sort_by(|a, b| a.slug.cmp(&b.slug));
    Ok(instances)
}

/// Per-service revision counters kept beside the key's scopes. They survive a
/// return to all operations, so a writer who last saw revision 0 cannot
/// overwrite a newer change. Read separately: `ApiKey` does not carry them.
const KEY_REVISIONS: &str = "operation_scope_revisions";

async fn key_revisions(db: &Database, key_id: &str) -> AppResult<BTreeMap<String, i64>> {
    let row = db
        .collection::<bson::Document>(API_KEYS)
        .find_one(doc! {"_id": key_id})
        .projection(doc! {KEY_REVISIONS: 1})
        .await?;
    Ok(row
        .and_then(|row| row.get_document(KEY_REVISIONS).ok().cloned())
        .map(|revisions| {
            revisions
                .iter()
                .filter_map(|(service, value)| Some((service.clone(), value.as_i64()?)))
                .collect()
        })
        .unwrap_or_default())
}

fn key_revision(key: &ApiKey, revisions: &BTreeMap<String, i64>, service: &str) -> i64 {
    revisions.get(service).copied().unwrap_or_else(|| {
        key.assistant_operation_scopes
            .get(service)
            .map_or(0, |scope| scope.revision)
    })
}

pub async fn key_options(
    db: &Database,
    actor: &str,
    key_id: &str,
) -> AppResult<Vec<ServiceOptions>> {
    let (key, access) = accessible_key(db, actor, key_id, false).await?;
    let revisions = key_revisions(db, &key.id).await?;
    let instances = key_instances(db, &key, &access).await?;
    let ids: Vec<String> = instances.iter().map(|row| row.id.clone()).collect();
    Box::pin(build_options(
        db,
        &instances,
        &ids,
        &[],
        &key.assistant_operation_scopes,
        |service| key_revision(&key, &revisions, service),
    ))
    .await
}

/// Save one service's selection on an Agent Key. The stored revision fences
/// concurrent writers; requests already admitted keep their auth snapshot.
pub async fn set_key(
    db: &Database,
    actor: &str,
    key_id: &str,
    service: &str,
    input: &OperationSelection,
) -> AppResult<(ApiKey, i64)> {
    Box::pin(require_configuration_enabled(db, actor)).await?;
    let (key, access) = accessible_key(db, actor, key_id, true).await?;
    let instances = key_instances(db, &key, &access).await?;
    let ids: Vec<String> = instances.iter().map(|row| row.id.clone()).collect();
    let revisions = key_revisions(db, &key.id).await?;
    let current = key_revision(&key, &revisions, service);
    let holder = Holder {
        service_ids: &ids,
        platform_service_ids: &[],
        revision: current,
    };
    let mut session = db.client().start_session().await?;
    let scope = Box::pin(compile_for(db, &holder, service, input, &mut session)).await?;
    ensure_reviewed(input, scope.as_ref())?;
    if scope.is_some()
        && !key.assistant_operation_scopes.contains_key(service)
        && key.assistant_operation_scopes.len() >= 256
    {
        return Err(AppError::ValidationError(
            "At most 256 scoped services per key".into(),
        ));
    }
    // Service IDs come from stored rows, but never let one address another field.
    if service.contains(['.', '$']) {
        return Err(AppError::ValidationError("Invalid service ID".into()));
    }
    let field = format!("assistant_operation_scopes.{service}");
    let revision_field = format!("{KEY_REVISIONS}.{service}");
    let mut filter = doc! {"_id": &key.id, "user_id": &key.user_id, "is_active": true};
    if revisions.contains_key(service) {
        filter.insert(&revision_field, current);
    } else {
        filter.insert(&revision_field, doc! {"$exists": false});
        if current == 0 {
            filter.insert(&field, doc! {"$exists": false});
        } else {
            filter.insert(format!("{field}.revision"), current);
        }
    }
    let next = current
        .checked_add(1)
        .ok_or_else(|| AppError::Conflict("Operation revision exhausted".into()))?;
    let update = match &scope {
        Some(scope) => doc! {"$set": {
            &field: bson::to_bson(scope)
                .map_err(|_| AppError::Internal("Operation scope encoding failed".into()))?,
            &revision_field: next,
            "updated_at": bson::DateTime::now(),
        }},
        None => doc! {
            "$unset": {&field: ""},
            "$set": {&revision_field: next, "updated_at": bson::DateTime::now()},
        },
    };
    let result = db
        .collection::<ApiKey>(API_KEYS)
        .update_one(filter, update)
        .await?;
    if result.matched_count != 1 {
        return Err(AppError::Conflict(
            "Operation scope changed; reload its revision".into(),
        ));
    }
    let mut updated = key;
    match scope {
        Some(scope) => {
            updated
                .assistant_operation_scopes
                .insert(service.to_owned(), scope);
        }
        None => {
            updated.assistant_operation_scopes.remove(service);
        }
    }
    super::audit_service::log_actor_event(
        db.clone(),
        &super::audit_service::AuditActor {
            user_id: actor.into(),
            ip_address: None,
            user_agent: None,
            api_key_id: None,
            api_key_name: None,
        },
        "api_key_operations_changed",
        Some(serde_json::json!({
            "owner_id": updated.user_id,
            "api_key_id": updated.id,
            "service_id": service,
            "revision": next,
            "operation_count": updated
                .assistant_operation_scopes
                .get(service)
                .map(|scope| scope.operations.len()),
        })),
    )
    .await?;
    Ok((updated, next))
}
