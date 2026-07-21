use std::collections::{BTreeMap, BTreeSet, HashSet};

use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::doc;
use serde::Serialize;
use sha2::{Digest, Sha256};
use utoipa::ToSchema;

use crate::errors::{AppError, AppResult};
use crate::models::node::{COLLECTION_NAME as NODES, Node};
use crate::models::node_service_binding::{
    COLLECTION_NAME as NODE_SERVICE_BINDINGS, NodeServiceBinding,
};
use crate::models::user::{COLLECTION_NAME as USERS, User, UserType};
use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
use crate::services::{node_service, org_service, user_service_service};

pub const SCOPE_PLAN_AUTHORITY: &str = "nyxid";
pub const SCOPE_PLAN_CONTRACT_VERSION: &str = "1";
pub const SCOPE_PLAN_POLICY_VERSION: &str = "api-key-scope-v1";

#[derive(Clone, Copy)]
pub enum ScopeAuthorization<'a> {
    OwnerOnly,
    ActorPermissions { actor_user_id: &'a str },
}

impl<'a> ScopeAuthorization<'a> {
    pub fn for_actor(actor_user_id: Option<&'a str>) -> Self {
        match actor_user_id {
            Some(actor_user_id) => Self::ActorPermissions { actor_user_id },
            None => Self::OwnerOnly,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ScopePlanOwnerType {
    Personal,
    Organization,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, ToSchema)]
pub struct ScopePlanPrincipal {
    pub id: String,
    #[serde(rename = "type")]
    pub principal_type: ScopePlanOwnerType,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ScopePlanNodeGrant {
    NotRequired,
    Required { node_ids: Vec<String> },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, ToSchema)]
pub struct ScopePlanServiceGrant {
    pub user_service_id: String,
    pub resource_owner: ScopePlanPrincipal,
    pub node_grant: ScopePlanNodeGrant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ScopePlanRouteCandidateBasis {
    ActiveConfiguredRoutes,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, ToSchema)]
pub struct ScopePlanCompleteness {
    pub list_complete: bool,
    pub no_duplicates: bool,
    pub route_candidate_basis: ScopePlanRouteCandidateBasis,
    pub transient_node_state_excluded: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ScopePlanFreshnessMode {
    MutationRevalidatedSnapshot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ScopePlanPostCreationDrift {
    FailClosed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, ToSchema)]
pub struct ScopePlanFreshness {
    pub mode: ScopePlanFreshnessMode,
    pub precondition_field: String,
    pub post_creation_drift: ScopePlanPostCreationDrift,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, ToSchema)]
pub struct EffectiveScopePlan {
    pub authority: String,
    pub contract_version: String,
    pub policy_version: String,
    pub authenticated_actor: ScopePlanPrincipal,
    pub intended_key_owner: ScopePlanPrincipal,
    pub services: Vec<ScopePlanServiceGrant>,
    pub allowed_service_ids: Vec<String>,
    pub allowed_node_ids: Vec<String>,
    pub evaluated_at: String,
    pub normalized_grant_digest: String,
    pub freshness: ScopePlanFreshness,
    pub completeness: ScopePlanCompleteness,
}

fn service_scope_error(service_id: &str) -> AppError {
    AppError::ValidationError(format!(
        "UserService '{}' not found or not permitted for this API key",
        service_id
    ))
}

fn node_scope_error(node_id: &str) -> AppError {
    AppError::ValidationError(format!(
        "Node '{}' not found or not permitted for this API key",
        node_id
    ))
}

fn service_is_scopeable_for_actor(
    service_id: &str,
    entry: &user_service_service::UserServiceWithSource,
) -> bool {
    if entry.service.id != service_id || !entry.service.is_active {
        return false;
    }

    match &entry.source {
        user_service_service::CredentialSource::Personal => true,
        user_service_service::CredentialSource::Org { allowed, .. } => *allowed,
    }
}

/// Validate explicit service grants through the same owner/ACL authority used
/// by the scope-plan endpoint. `OwnerOnly` remains available for internal
/// provisioning flows that do not have an authenticated actor context.
pub async fn validate_service_ids(
    db: &mongodb::Database,
    key_owner_user_id: &str,
    service_ids: &[String],
    authorization: ScopeAuthorization<'_>,
) -> AppResult<()> {
    match authorization {
        ScopeAuthorization::ActorPermissions { actor_user_id }
            if actor_user_id == key_owner_user_id =>
        {
            let visible_services =
                user_service_service::list_user_services_with_sources(db, actor_user_id).await?;
            for sid in service_ids {
                if !visible_services
                    .iter()
                    .any(|entry| service_is_scopeable_for_actor(sid, entry))
                {
                    return Err(service_scope_error(sid));
                }
            }
            return Ok(());
        }
        ScopeAuthorization::ActorPermissions { actor_user_id } => {
            let access =
                org_service::resolve_owner_access(db, actor_user_id, key_owner_user_id).await?;
            if !access.can_write() {
                return Err(AppError::OrgRoleInsufficient(
                    "you must be an admin of the API key owner".to_string(),
                ));
            }
            for sid in service_ids {
                let exists = db
                    .collection::<UserService>(USER_SERVICES)
                    .find_one(doc! {
                        "_id": sid,
                        "user_id": key_owner_user_id,
                        "is_active": true,
                    })
                    .await?;
                if exists.is_none() || !access.allows_resource(sid) {
                    return Err(service_scope_error(sid));
                }
            }
            return Ok(());
        }
        ScopeAuthorization::OwnerOnly => {}
    }

    for sid in service_ids {
        let exists = db
            .collection::<UserService>(USER_SERVICES)
            .find_one(doc! {
                "_id": sid,
                "user_id": key_owner_user_id,
                "is_active": true,
            })
            .await?;
        if exists.is_none() {
            return Err(service_scope_error(sid));
        }
    }
    Ok(())
}

/// Validate explicit node grants through the node ACL used by API-key scope
/// creation and update. Org-owned keys remain confined to nodes owned by that
/// org; personal keys may include readable org nodes under #1133 semantics.
pub async fn validate_node_ids(
    db: &mongodb::Database,
    key_owner_user_id: &str,
    node_ids: &[String],
    authorization: ScopeAuthorization<'_>,
) -> AppResult<()> {
    for nid in node_ids {
        let node = db
            .collection::<Node>(NODES)
            .find_one(doc! { "_id": nid, "is_active": true })
            .await?;
        let Some(node) = node else {
            return Err(node_scope_error(nid));
        };

        match authorization {
            ScopeAuthorization::ActorPermissions { actor_user_id }
                if actor_user_id == key_owner_user_id =>
            {
                let access =
                    org_service::resolve_owner_access(db, actor_user_id, &node.user_id).await?;
                if !node_service::node_access_can_read(&access) {
                    return Err(node_scope_error(nid));
                }
            }
            ScopeAuthorization::ActorPermissions { actor_user_id } => {
                let owner_access =
                    org_service::resolve_owner_access(db, actor_user_id, key_owner_user_id).await?;
                if !owner_access.can_write() || node.user_id != key_owner_user_id {
                    return Err(node_scope_error(nid));
                }
            }
            ScopeAuthorization::OwnerOnly => {
                if node.user_id != key_owner_user_id {
                    return Err(node_scope_error(nid));
                }
            }
        }
    }
    Ok(())
}

async fn load_principal(db: &mongodb::Database, id: &str) -> AppResult<ScopePlanPrincipal> {
    let user = db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": id, "is_active": true })
        .await?
        .ok_or_else(|| {
            AppError::ApiKeyScopePlanNotFound(format!("scope-plan owner '{}' not found", id))
        })?;

    Ok(ScopePlanPrincipal {
        id: user.id,
        principal_type: match user.user_type {
            UserType::Person => ScopePlanOwnerType::Personal,
            UserType::Org => ScopePlanOwnerType::Organization,
        },
    })
}

async fn resolve_key_owner(
    db: &mongodb::Database,
    actor_user_id: &str,
    target_owner_id: Option<&str>,
) -> AppResult<(ScopePlanPrincipal, ScopePlanPrincipal)> {
    let actor = load_principal(db, actor_user_id).await?;
    if actor.principal_type != ScopePlanOwnerType::Personal {
        return Err(AppError::ApiKeyScopePlanOwnerUnsupported(
            "authenticated actor must be a personal subject".to_string(),
        ));
    }

    let Some(target_owner_id) = target_owner_id else {
        return Ok((actor.clone(), actor));
    };

    let owner = load_principal(db, target_owner_id).await?;
    if owner.principal_type != ScopePlanOwnerType::Organization {
        return Err(AppError::ApiKeyScopePlanOwnerUnsupported(
            "target_org_id must identify an organization owner".to_string(),
        ));
    }

    let access = org_service::resolve_owner_access(db, actor_user_id, target_owner_id).await?;
    if !access.can_write() {
        return Err(AppError::ApiKeyScopePlanDenied(
            "organization admin access is required for the intended key owner".to_string(),
        ));
    }

    Ok((actor, owner))
}

/// Resolve the intended storage owner through the same typed owner authority
/// used by scope planning. This keeps API-key creation from accepting an
/// owner shape that the planning contract would reject.
pub async fn resolve_scope_owner_id(
    db: &mongodb::Database,
    actor_user_id: &str,
    target_owner_id: Option<&str>,
) -> AppResult<String> {
    let (_, owner) = resolve_key_owner(db, actor_user_id, target_owner_id).await?;
    Ok(owner.id)
}

fn reject_duplicate_service_ids(service_ids: &[String]) -> AppResult<()> {
    let mut seen = HashSet::new();
    for service_id in service_ids {
        if !seen.insert(service_id.as_str()) {
            return Err(AppError::ValidationError(format!(
                "selected_service_ids contains duplicate UserService '{}'",
                service_id
            )));
        }
    }
    Ok(())
}

async fn selected_services_for_personal_key(
    db: &mongodb::Database,
    actor_user_id: &str,
    service_ids: &[String],
) -> AppResult<Vec<UserService>> {
    let visible = user_service_service::list_user_services_with_sources(db, actor_user_id).await?;
    let allowed: BTreeMap<&str, &UserService> = visible
        .iter()
        .filter(|entry| service_is_scopeable_for_actor(&entry.service.id, entry))
        .map(|entry| (entry.service.id.as_str(), &entry.service))
        .collect();

    let mut selected = Vec::with_capacity(service_ids.len());
    for service_id in service_ids {
        if let Some(service) = allowed.get(service_id.as_str()) {
            selected.push((*service).clone());
            continue;
        }

        let exists = db
            .collection::<UserService>(USER_SERVICES)
            .find_one(doc! { "_id": service_id, "is_active": true })
            .await?;
        return Err(if exists.is_some() {
            AppError::ApiKeyScopePlanDenied(format!(
                "UserService '{}' is not permitted for the authenticated actor",
                service_id
            ))
        } else {
            AppError::ApiKeyScopePlanNotFound(format!("UserService '{}' not found", service_id))
        });
    }
    Ok(selected)
}

async fn selected_services_for_org_key(
    db: &mongodb::Database,
    actor_user_id: &str,
    owner_user_id: &str,
    service_ids: &[String],
) -> AppResult<Vec<UserService>> {
    let access = org_service::resolve_owner_access(db, actor_user_id, owner_user_id).await?;
    if !access.can_write() {
        return Err(AppError::ApiKeyScopePlanDenied(
            "organization admin access is required for the intended key owner".to_string(),
        ));
    }

    let mut selected = Vec::with_capacity(service_ids.len());
    for service_id in service_ids {
        let service = db
            .collection::<UserService>(USER_SERVICES)
            .find_one(doc! { "_id": service_id, "is_active": true })
            .await?;
        let Some(service) = service else {
            return Err(AppError::ApiKeyScopePlanNotFound(format!(
                "UserService '{}' not found",
                service_id
            )));
        };
        if service.user_id != owner_user_id || !access.allows_resource(service_id) {
            return Err(AppError::ApiKeyScopePlanDenied(format!(
                "UserService '{}' is not permitted for the intended key owner",
                service_id
            )));
        }
        selected.push(service);
    }
    Ok(selected)
}

async fn configured_node_ids_for_service(
    db: &mongodb::Database,
    service: &UserService,
) -> AppResult<BTreeSet<String>> {
    let mut node_ids = BTreeSet::new();
    if let Some(node_id) = service.node_id.as_deref()
        && !node_id.is_empty()
    {
        node_ids.insert(node_id.to_string());
    }

    if let Some(catalog_service_id) = service.catalog_service_id.as_deref() {
        // Runtime routing resolves `UserService.node_id` by owner + catalog
        // service, not by the selected UserService ID. Include every active
        // matching row so a multi-connection catalog service cannot expose a
        // node candidate that is absent from the authorization plan.
        let peer_services: Vec<UserService> = db
            .collection::<UserService>(USER_SERVICES)
            .find(doc! {
                "user_id": &service.user_id,
                "catalog_service_id": catalog_service_id,
                "is_active": true,
            })
            .await?
            .try_collect()
            .await?;
        for peer in peer_services {
            if let Some(node_id) = peer.node_id
                && !node_id.is_empty()
            {
                node_ids.insert(node_id);
            }
        }

        let bindings: Vec<NodeServiceBinding> = db
            .collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
            .find(doc! {
                "user_id": &service.user_id,
                "service_id": catalog_service_id,
                "is_active": true,
            })
            .await?
            .try_collect()
            .await?;
        for binding in bindings {
            if binding.node_id.is_empty() {
                return Err(AppError::ApiKeyScopePlanRouteUnresolved(format!(
                    "UserService '{}' has an active binding without a node ID",
                    service.id
                )));
            }
            node_ids.insert(binding.node_id);
        }
    }

    Ok(node_ids)
}

async fn validate_configured_nodes(
    db: &mongodb::Database,
    actor_user_id: &str,
    service: &UserService,
    node_ids: &BTreeSet<String>,
) -> AppResult<()> {
    for node_id in node_ids {
        let node = db
            .collection::<Node>(NODES)
            .find_one(doc! { "_id": node_id })
            .await?;
        let Some(node) = node else {
            return Err(AppError::ApiKeyScopePlanRouteUnresolved(format!(
                "UserService '{}' references missing node '{}'",
                service.id, node_id
            )));
        };
        if !node.is_active || node.user_id != service.user_id {
            return Err(AppError::ApiKeyScopePlanRouteUnresolved(format!(
                "UserService '{}' references an inactive or differently owned node '{}'",
                service.id, node_id
            )));
        }

        let access = org_service::resolve_owner_access(db, actor_user_id, &node.user_id).await?;
        if !node_service::node_access_can_read(&access) {
            return Err(AppError::ApiKeyScopePlanDenied(format!(
                "node '{}' is not permitted for the authenticated actor",
                node_id
            )));
        }
    }
    Ok(())
}

fn digest_field(hasher: &mut Sha256, value: &str) {
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value.as_bytes());
}

fn digest_count(hasher: &mut Sha256, count: usize) {
    hasher.update((count as u64).to_be_bytes());
}

fn normalized_grant_digest(
    actor: &ScopePlanPrincipal,
    owner: &ScopePlanPrincipal,
    services: &[ScopePlanServiceGrant],
    allowed_service_ids: &[String],
    allowed_node_ids: &[String],
) -> String {
    let mut hasher = Sha256::new();
    digest_field(&mut hasher, "nyxid-api-key-scope-plan-digest");
    for value in [
        SCOPE_PLAN_AUTHORITY,
        SCOPE_PLAN_CONTRACT_VERSION,
        SCOPE_PLAN_POLICY_VERSION,
        &actor.id,
        match actor.principal_type {
            ScopePlanOwnerType::Personal => "personal",
            ScopePlanOwnerType::Organization => "organization",
        },
        &owner.id,
        match owner.principal_type {
            ScopePlanOwnerType::Personal => "personal",
            ScopePlanOwnerType::Organization => "organization",
        },
    ] {
        digest_field(&mut hasher, value);
    }
    digest_field(&mut hasher, "services");
    digest_count(&mut hasher, services.len());
    for service in services {
        digest_field(&mut hasher, "service");
        digest_field(&mut hasher, &service.user_service_id);
        digest_field(&mut hasher, &service.resource_owner.id);
        match &service.node_grant {
            ScopePlanNodeGrant::NotRequired => digest_field(&mut hasher, "not_required"),
            ScopePlanNodeGrant::Required { node_ids } => {
                digest_field(&mut hasher, "required");
                digest_count(&mut hasher, node_ids.len());
                for node_id in node_ids {
                    digest_field(&mut hasher, node_id);
                }
            }
        }
    }
    digest_field(&mut hasher, "allowed_service_ids");
    digest_count(&mut hasher, allowed_service_ids.len());
    for service_id in allowed_service_ids {
        digest_field(&mut hasher, service_id);
    }
    digest_field(&mut hasher, "allowed_node_ids");
    digest_count(&mut hasher, allowed_node_ids.len());
    for node_id in allowed_node_ids {
        digest_field(&mut hasher, node_id);
    }
    format!("sha256:{}", hex::encode(hasher.finalize()))
}

/// Build the complete, caller-scoped configured-route snapshot for a selected
/// set of `UserService` resources. Node liveness and WS connectivity are
/// intentionally excluded: a route that is offline now may become selectable
/// later and therefore still needs an authorization grant.
pub async fn build_scope_plan(
    db: &mongodb::Database,
    actor_user_id: &str,
    target_owner_id: Option<&str>,
    selected_service_ids: &[String],
) -> AppResult<EffectiveScopePlan> {
    reject_duplicate_service_ids(selected_service_ids)?;
    let (actor, owner) = resolve_key_owner(db, actor_user_id, target_owner_id).await?;

    let mut selected = if owner.id == actor.id {
        selected_services_for_personal_key(db, actor_user_id, selected_service_ids).await?
    } else {
        selected_services_for_org_key(db, actor_user_id, &owner.id, selected_service_ids).await?
    };
    selected.sort_by(|left, right| left.id.cmp(&right.id));

    let mut services = Vec::with_capacity(selected.len());
    let mut allowed_service_ids = Vec::with_capacity(selected.len());
    let mut allowed_node_ids = BTreeSet::new();
    for service in selected {
        let configured_node_ids = configured_node_ids_for_service(db, &service).await?;
        validate_configured_nodes(db, actor_user_id, &service, &configured_node_ids).await?;
        allowed_service_ids.push(service.id.clone());
        allowed_node_ids.extend(configured_node_ids.iter().cloned());

        let resource_owner = ScopePlanPrincipal {
            id: service.user_id.clone(),
            principal_type: if service.user_id == actor.id {
                ScopePlanOwnerType::Personal
            } else {
                ScopePlanOwnerType::Organization
            },
        };
        services.push(ScopePlanServiceGrant {
            user_service_id: service.id,
            resource_owner,
            node_grant: if configured_node_ids.is_empty() {
                ScopePlanNodeGrant::NotRequired
            } else {
                ScopePlanNodeGrant::Required {
                    node_ids: configured_node_ids.into_iter().collect(),
                }
            },
        });
    }

    let allowed_node_ids: Vec<String> = allowed_node_ids.into_iter().collect();
    let normalized_grant_digest = normalized_grant_digest(
        &actor,
        &owner,
        &services,
        &allowed_service_ids,
        &allowed_node_ids,
    );

    Ok(EffectiveScopePlan {
        authority: SCOPE_PLAN_AUTHORITY.to_string(),
        contract_version: SCOPE_PLAN_CONTRACT_VERSION.to_string(),
        policy_version: SCOPE_PLAN_POLICY_VERSION.to_string(),
        authenticated_actor: actor,
        intended_key_owner: owner,
        services,
        allowed_service_ids,
        allowed_node_ids,
        evaluated_at: Utc::now().to_rfc3339(),
        normalized_grant_digest,
        freshness: ScopePlanFreshness {
            mode: ScopePlanFreshnessMode::MutationRevalidatedSnapshot,
            precondition_field: "scope_plan_digest".to_string(),
            post_creation_drift: ScopePlanPostCreationDrift::FailClosed,
        },
        completeness: ScopePlanCompleteness {
            list_complete: true,
            no_duplicates: true,
            route_candidate_basis: ScopePlanRouteCandidateBasis::ActiveConfiguredRoutes,
            transient_node_state_excluded: true,
        },
    })
}

fn normalized_set(values: &[String], field_name: &str) -> AppResult<Vec<String>> {
    let mut normalized = values.to_vec();
    normalized.sort();
    let original_len = normalized.len();
    normalized.dedup();
    if normalized.len() != original_len {
        return Err(AppError::ValidationError(format!(
            "{} must not contain duplicates",
            field_name
        )));
    }
    Ok(normalized)
}

/// Recompute and enforce a scope-plan snapshot before an API-key mutation.
/// The submitted grants must be the exact sets from the plan; otherwise the
/// mutation fails closed with a typed stale-plan conflict.
#[allow(clippy::too_many_arguments)]
pub async fn verify_scope_plan_precondition(
    db: &mongodb::Database,
    actor_user_id: &str,
    key_owner_user_id: &str,
    allowed_service_ids: &[String],
    allowed_node_ids: &[String],
    allow_all_services: bool,
    allow_all_nodes: bool,
    expected_digest: &str,
) -> AppResult<()> {
    if allow_all_services || allow_all_nodes {
        return Err(AppError::ApiKeyScopePlanStale(
            "scope_plan_digest requires allow_all_services=false and allow_all_nodes=false"
                .to_string(),
        ));
    }

    let target_owner_id = (key_owner_user_id != actor_user_id).then_some(key_owner_user_id);
    let plan = build_scope_plan(db, actor_user_id, target_owner_id, allowed_service_ids).await?;
    let submitted_services = normalized_set(allowed_service_ids, "allowed_service_ids")?;
    let submitted_nodes = normalized_set(allowed_node_ids, "allowed_node_ids")?;

    if plan.normalized_grant_digest != expected_digest
        || plan.allowed_service_ids != submitted_services
        || plan.allowed_node_ids != submitted_nodes
    {
        return Err(AppError::ApiKeyScopePlanStale(
            "scope plan no longer matches the current authorization or route configuration"
                .to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::node::{NodeMetrics, NodeStatus};
    use crate::models::org_membership::{
        COLLECTION_NAME as ORG_MEMBERSHIPS, OrgMembership, OrgRole,
    };
    use crate::services::key_service;
    use crate::test_utils::{connect_test_database, test_membership, test_user, test_user_service};
    use uuid::Uuid;

    fn principal(id: &str, principal_type: ScopePlanOwnerType) -> ScopePlanPrincipal {
        ScopePlanPrincipal {
            id: id.to_string(),
            principal_type,
        }
    }

    fn test_node(id: &str, owner_id: &str, status: NodeStatus) -> Node {
        let now = Utc::now();
        Node {
            id: id.to_string(),
            user_id: owner_id.to_string(),
            name: format!("node-{}", &id[..8]),
            status,
            auth_token_hash: "auth-hash".to_string(),
            signing_secret_encrypted: None,
            signing_secret_hash: "signing-hash".to_string(),
            last_heartbeat_at: None,
            connected_at: None,
            metadata: None,
            metrics: NodeMetrics::default(),
            is_active: true,
            created_at: now,
            updated_at: now,
        }
    }

    fn test_binding(
        id: &str,
        owner_id: &str,
        node_id: &str,
        catalog_service_id: &str,
    ) -> NodeServiceBinding {
        let now = Utc::now();
        NodeServiceBinding {
            id: id.to_string(),
            node_id: node_id.to_string(),
            user_id: owner_id.to_string(),
            service_id: catalog_service_id.to_string(),
            is_active: true,
            priority: 0,
            created_at: now,
            updated_at: now,
        }
    }

    async fn insert_user(db: &mongodb::Database, id: &str, user_type: UserType) {
        db.collection::<User>(USERS)
            .insert_one(test_user(id, user_type))
            .await
            .expect("insert user");
    }

    #[test]
    fn digest_is_stable_for_normalized_grants() {
        let actor = principal("actor", ScopePlanOwnerType::Personal);
        let owner = actor.clone();
        let services = vec![ScopePlanServiceGrant {
            user_service_id: "service-a".to_string(),
            resource_owner: owner.clone(),
            node_grant: ScopePlanNodeGrant::Required {
                node_ids: vec!["node-a".to_string(), "node-b".to_string()],
            },
        }];
        let first = normalized_grant_digest(
            &actor,
            &owner,
            &services,
            &["service-a".to_string()],
            &["node-a".to_string(), "node-b".to_string()],
        );
        let second = normalized_grant_digest(
            &actor,
            &owner,
            &services,
            &["service-a".to_string()],
            &["node-a".to_string(), "node-b".to_string()],
        );
        assert_eq!(first, second);
        assert!(first.starts_with("sha256:"));
        assert_eq!(first.len(), 71);
    }

    #[test]
    fn digest_encodes_collection_boundaries_unambiguously() {
        let actor = principal("actor", ScopePlanOwnerType::Personal);
        let owner = principal("owner", ScopePlanOwnerType::Organization);
        let embedded_boundary = vec![ScopePlanServiceGrant {
            user_service_id: "service-a".to_string(),
            resource_owner: owner.clone(),
            node_grant: ScopePlanNodeGrant::Required {
                node_ids: vec![
                    "service-b".to_string(),
                    "owner".to_string(),
                    "not_required".to_string(),
                ],
            },
        }];
        let separate_service = vec![
            ScopePlanServiceGrant {
                user_service_id: "service-a".to_string(),
                resource_owner: owner.clone(),
                node_grant: ScopePlanNodeGrant::Required { node_ids: vec![] },
            },
            ScopePlanServiceGrant {
                user_service_id: "service-b".to_string(),
                resource_owner: owner.clone(),
                node_grant: ScopePlanNodeGrant::NotRequired,
            },
        ];
        let service_ids = vec!["service-a".to_string(), "service-b".to_string()];

        assert_ne!(
            normalized_grant_digest(&actor, &owner, &embedded_boundary, &service_ids, &[]),
            normalized_grant_digest(&actor, &owner, &separate_service, &service_ids, &[])
        );
    }

    #[test]
    fn normalized_set_rejects_duplicates_and_sorts() {
        assert_eq!(
            normalized_set(&["b".to_string(), "a".to_string()], "ids").unwrap(),
            vec!["a".to_string(), "b".to_string()]
        );
        assert!(normalized_set(&["a".to_string(), "a".to_string()], "ids").is_err());
    }

    #[tokio::test]
    async fn plan_is_complete_for_configured_routes_and_create_accepts_only_exact_grants() {
        let Some(db) = connect_test_database("api_key_scope_plan_complete").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let actor_id = Uuid::new_v4().to_string();
        let routed_service_id = Uuid::new_v4().to_string();
        let direct_service_id = Uuid::new_v4().to_string();
        let node_a_id = Uuid::new_v4().to_string();
        let node_b_id = Uuid::new_v4().to_string();
        let extra_node_id = Uuid::new_v4().to_string();
        let catalog_service_id = Uuid::new_v4().to_string();
        insert_user(&db, &actor_id, UserType::Person).await;

        let routed = test_user_service(
            &routed_service_id,
            &actor_id,
            "routed",
            "endpoint-routed",
            Some(&catalog_service_id),
            Some(&node_a_id),
        );
        let direct = test_user_service(
            &direct_service_id,
            &actor_id,
            "direct",
            "endpoint-direct",
            None,
            None,
        );
        db.collection::<UserService>(USER_SERVICES)
            .insert_many([routed, direct])
            .await
            .expect("insert services");
        db.collection::<Node>(NODES)
            .insert_many([
                test_node(&node_a_id, &actor_id, NodeStatus::Offline),
                test_node(&node_b_id, &actor_id, NodeStatus::Draining),
                test_node(&extra_node_id, &actor_id, NodeStatus::Online),
            ])
            .await
            .expect("insert nodes");
        let duplicate_binding_id = Uuid::new_v4().to_string();
        let fallback_binding_id = Uuid::new_v4().to_string();
        db.collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
            .insert_many([
                test_binding(
                    &duplicate_binding_id,
                    &actor_id,
                    &node_a_id,
                    &catalog_service_id,
                ),
                test_binding(
                    &fallback_binding_id,
                    &actor_id,
                    &node_b_id,
                    &catalog_service_id,
                ),
            ])
            .await
            .expect("insert bindings");

        let selected = vec![routed_service_id.clone(), direct_service_id.clone()];
        let plan = build_scope_plan(&db, &actor_id, None, &selected)
            .await
            .expect("build plan");

        assert_eq!(plan.authority, SCOPE_PLAN_AUTHORITY);
        assert_eq!(plan.contract_version, SCOPE_PLAN_CONTRACT_VERSION);
        assert_eq!(plan.policy_version, SCOPE_PLAN_POLICY_VERSION);
        assert_eq!(plan.authenticated_actor.id, actor_id);
        assert_eq!(plan.intended_key_owner.id, actor_id);
        let mut expected_service_ids = vec![direct_service_id.clone(), routed_service_id.clone()];
        expected_service_ids.sort();
        assert_eq!(plan.allowed_service_ids, expected_service_ids);
        let mut expected_node_ids = vec![node_a_id.clone(), node_b_id.clone()];
        expected_node_ids.sort();
        assert_eq!(plan.allowed_node_ids, expected_node_ids);
        assert!(plan.completeness.list_complete);
        assert!(plan.completeness.no_duplicates);
        assert!(plan.completeness.transient_node_state_excluded);

        let direct_grant = plan
            .services
            .iter()
            .find(|grant| grant.user_service_id == direct_service_id)
            .expect("direct grant");
        assert!(matches!(
            direct_grant.node_grant,
            ScopePlanNodeGrant::NotRequired
        ));
        let routed_grant = plan
            .services
            .iter()
            .find(|grant| grant.user_service_id == routed_service_id)
            .expect("routed grant");
        assert!(matches!(
            &routed_grant.node_grant,
            ScopePlanNodeGrant::Required { node_ids }
                if node_ids == &expected_node_ids
        ));

        let created = key_service::create_api_key_with_scope_authorization(
            &db,
            &actor_id,
            Some(&actor_id),
            "planned-agent",
            "proxy",
            None,
            None,
            Some(&plan.allowed_service_ids),
            Some(&plan.allowed_node_ids),
            Some(false),
            Some(false),
            None,
            None,
            Some("generic"),
            None,
            Some(&plan.normalized_grant_digest),
        )
        .await
        .expect("create key from exact plan");
        assert_eq!(created.allowed_service_ids, plan.allowed_service_ids);
        assert_eq!(created.allowed_node_ids, plan.allowed_node_ids);
        let updated = key_service::update_api_key_scope_with_scope_authorization(
            &db,
            &actor_id,
            Some(&actor_id),
            &created.id,
            Some("planned-agent-updated"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(&plan.normalized_grant_digest),
        )
        .await
        .expect("update revalidates the same plan");
        assert_eq!(updated.name, "planned-agent-updated");

        let mut overbroad_nodes = plan.allowed_node_ids.clone();
        overbroad_nodes.push(extra_node_id);
        let err = verify_scope_plan_precondition(
            &db,
            &actor_id,
            &actor_id,
            &plan.allowed_service_ids,
            &overbroad_nodes,
            false,
            false,
            &plan.normalized_grant_digest,
        )
        .await
        .expect_err("unplanned node grant must be rejected");
        assert!(matches!(err, AppError::ApiKeyScopePlanStale(_)));

        db.collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
            .update_one(
                doc! { "_id": &fallback_binding_id },
                doc! { "$set": { "is_active": false } },
            )
            .await
            .expect("deactivate binding");
        let changed = build_scope_plan(&db, &actor_id, None, &selected)
            .await
            .expect("rebuild after binding deletion");
        assert_eq!(changed.allowed_node_ids, vec![node_a_id]);
        assert_ne!(
            changed.normalized_grant_digest,
            plan.normalized_grant_digest
        );
        let err = key_service::update_api_key_scope_with_scope_authorization(
            &db,
            &actor_id,
            Some(&actor_id),
            &created.id,
            Some("must-not-commit"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(&plan.normalized_grant_digest),
        )
        .await
        .expect_err("binding deletion must invalidate the old plan");
        assert!(matches!(err, AppError::ApiKeyScopePlanStale(_)));
    }

    #[tokio::test]
    async fn personal_plan_follows_org_permission_narrowing() {
        let Some(db) = connect_test_database("api_key_scope_plan_permission").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let actor_id = Uuid::new_v4().to_string();
        let org_id = Uuid::new_v4().to_string();
        let service_id = Uuid::new_v4().to_string();
        insert_user(&db, &actor_id, UserType::Person).await;
        insert_user(&db, &org_id, UserType::Org).await;
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(test_user_service(
                &service_id,
                &org_id,
                "org-service",
                "endpoint-org",
                None,
                None,
            ))
            .await
            .expect("insert org service");
        let membership = test_membership(
            &org_id,
            &actor_id,
            OrgRole::Member,
            Some(vec![service_id.clone()]),
        );
        let membership_id = membership.id.clone();
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .insert_one(membership)
            .await
            .expect("insert membership");

        let plan = build_scope_plan(&db, &actor_id, None, std::slice::from_ref(&service_id))
            .await
            .expect("permitted org service is planable");
        assert_eq!(plan.services[0].resource_owner.id, org_id);

        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .update_one(
                doc! { "_id": membership_id },
                doc! { "$set": { "allowed_service_ids": Vec::<String>::new() } },
            )
            .await
            .expect("narrow membership");
        let err = build_scope_plan(&db, &actor_id, None, std::slice::from_ref(&service_id))
            .await
            .expect_err("narrowed permission must deny the service");
        assert!(matches!(err, AppError::ApiKeyScopePlanDenied(_)));
    }

    #[tokio::test]
    async fn org_plan_reports_typed_owner_resource_and_route_errors() {
        let Some(db) = connect_test_database("api_key_scope_plan_errors").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let actor_id = Uuid::new_v4().to_string();
        let org_id = Uuid::new_v4().to_string();
        let other_person_id = Uuid::new_v4().to_string();
        let org_service_id = Uuid::new_v4().to_string();
        let personal_service_id = Uuid::new_v4().to_string();
        insert_user(&db, &actor_id, UserType::Person).await;
        insert_user(&db, &org_id, UserType::Org).await;
        insert_user(&db, &other_person_id, UserType::Person).await;
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .insert_one(test_membership(&org_id, &actor_id, OrgRole::Admin, None))
            .await
            .expect("insert admin membership");
        db.collection::<UserService>(USER_SERVICES)
            .insert_many([
                test_user_service(
                    &org_service_id,
                    &org_id,
                    "org-service",
                    "endpoint-org",
                    None,
                    None,
                ),
                test_user_service(
                    &personal_service_id,
                    &actor_id,
                    "personal-service",
                    "endpoint-personal",
                    None,
                    None,
                ),
            ])
            .await
            .expect("insert services");

        let plan = build_scope_plan(
            &db,
            &actor_id,
            Some(&org_id),
            std::slice::from_ref(&org_service_id),
        )
        .await
        .expect("org admin can plan org-owned key");
        assert_eq!(
            plan.intended_key_owner.principal_type,
            ScopePlanOwnerType::Organization
        );

        let err = build_scope_plan(
            &db,
            &actor_id,
            Some(&org_id),
            std::slice::from_ref(&personal_service_id),
        )
        .await
        .expect_err("org key cannot include personal resource");
        assert!(matches!(err, AppError::ApiKeyScopePlanDenied(_)));

        let err = build_scope_plan(&db, &actor_id, Some(&other_person_id), &[])
            .await
            .expect_err("person cannot be target_org_id");
        assert!(matches!(err, AppError::ApiKeyScopePlanOwnerUnsupported(_)));
        let missing_id = Uuid::new_v4().to_string();
        let err = build_scope_plan(&db, &actor_id, None, std::slice::from_ref(&missing_id))
            .await
            .expect_err("missing service is typed");
        assert!(matches!(err, AppError::ApiKeyScopePlanNotFound(_)));

        let missing_node_id = Uuid::new_v4().to_string();
        db.collection::<UserService>(USER_SERVICES)
            .update_one(
                doc! { "_id": &org_service_id },
                doc! { "$set": { "node_id": &missing_node_id } },
            )
            .await
            .expect("set unresolved node route");
        let err = build_scope_plan(
            &db,
            &actor_id,
            Some(&org_id),
            std::slice::from_ref(&org_service_id),
        )
        .await
        .expect_err("missing configured node is unresolved");
        assert!(matches!(err, AppError::ApiKeyScopePlanRouteUnresolved(_)));
    }
}
