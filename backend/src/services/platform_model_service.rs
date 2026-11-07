//! Registry and metadata authorization only. This module cannot dispatch or decrypt.
use std::collections::{HashMap, HashSet};

use bson::{Document, doc};
use chrono::Utc;
use futures::TryStreamExt;
use mongodb::{Database, IndexModel, options::IndexOptions};
use uuid::Uuid;

use super::{
    audit_service::{self, AuditActor},
    feature_flag_service, platform_key_service, security_readiness_service, transaction_service,
};
use crate::{
    errors::{AppError, AppResult},
    models::{
        downstream_service::{DownstreamService, PlatformKeyPurpose},
        group::{COLLECTION_NAME as GROUPS, Group},
        platform_model::*,
        user::{COLLECTION_NAME as USERS, User},
    },
};

pub const FLAG: &str = "inference:platform-models";
/// Only the foundation capability is advertised by PR 1.
pub const REGISTRY_READINESS: &str = "inference:platform-model-registry-v1";
pub const EXECUTION_READINESS: &str = "inference:platform-models-v1";
pub const BILLING_READINESS: &str = "billing:model-prices-v1";
pub const ASSISTANT_READINESS: &str = "assistant:platform-models-v1";
const MAX_ROUTES: usize = 32;
const MAX_RULES: usize = 500;

pub fn unavailable() -> AppError {
    AppError::NotFound("Model not found".into())
}
fn invalid(message: &str) -> AppError {
    AppError::ValidationError(message.into())
}

pub async fn ensure_indexes(db: &Database) -> mongodb::error::Result<()> {
    for (collection, keys, name, unique) in [
        (
            COLLECTION_NAME,
            doc! {"public_id":1},
            "platform_model_public_id",
            true,
        ),
        (
            COLLECTION_NAME,
            doc! {"lifecycle":1,"enabled":1,"public_id":1},
            "platform_model_listing",
            false,
        ),
        (
            ROUTING_COLLECTION,
            doc! {"model_id":1,"model_revision":1},
            "platform_model_route_revision",
            true,
        ),
    ] {
        db.collection::<Document>(collection)
            .create_index(
                IndexModel::builder()
                    .keys(keys)
                    .options(
                        IndexOptions::builder()
                            .name(name.to_owned())
                            .unique(unique)
                            .build(),
                    )
                    .build(),
            )
            .await?;
    }
    Ok(())
}

/// Unlike require(), unknown/future capabilities must fail closed. PR 1 cannot
/// be activated even if someone writes a flag or fabricated future heartbeat.
pub async fn require_execution_ready(db: &Database) -> AppResult<()> {
    for capability in [REGISTRY_READINESS, EXECUTION_READINESS, BILLING_READINESS] {
        if !security_readiness_service::CAPABILITIES.contains(&capability)
            || !security_readiness_service::report(db, capability)
                .await?
                .ready
        {
            return Err(AppError::SecurityReadinessUnavailable);
        }
    }
    Ok(())
}

fn identifier(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"-._".contains(&b))
        && value.as_bytes()[0].is_ascii_alphanumeric()
}
fn uuid(value: &str) -> AppResult<()> {
    if Uuid::parse_str(value).is_ok_and(|id| id.to_string() == value) {
        Ok(())
    } else {
        Err(invalid("Expected a canonical UUID"))
    }
}
pub fn validate_public_id(public_id: &str, family: ModelFamily) -> AppResult<()> {
    let model = public_id
        .strip_prefix("nyxid/")
        .ok_or_else(|| invalid("Model IDs must start with nyxid/"))?;
    if !identifier(model, 120)
        || !match family {
            ModelFamily::Gemini => model.starts_with("gemini-"),
            ModelFamily::Claude => model.starts_with("claude-"),
            ModelFamily::Openai => {
                model.starts_with("gpt-")
                    || model.starts_with("o1")
                    || model.starts_with("o3")
                    || model.starts_with("o4")
            }
            ModelFamily::OpenModel => true,
        }
    {
        return Err(invalid(
            "Expected a real model name matching its model family",
        ));
    }
    Ok(())
}
fn validate_capabilities(value: &Capabilities) -> AppResult<()> {
    fn unique<T: PartialEq>(values: &[T], max: usize) -> bool {
        values.len() <= max
            && values
                .iter()
                .enumerate()
                .all(|(i, v)| !values[..i].contains(v))
    }
    if !unique(&value.protocols, 4)
        || !unique(&value.input_modalities, 4)
        || !unique(&value.output_modalities, 4)
        || value
            .context_tokens
            .is_some_and(|n| n == 0 || n > 100_000_000)
        || value
            .output_tokens
            .is_some_and(|n| n == 0 || n > 100_000_000)
        || matches!((value.context_tokens,value.output_tokens),(Some(c),Some(o)) if o>c)
    {
        return Err(invalid("Invalid model capability bounds"));
    }
    Ok(())
}
fn validate_text(name: &str, description: &str) -> AppResult<()> {
    if name.trim().is_empty()
        || name.len() > 160
        || description.len() > 2000
        || name.chars().any(char::is_control)
        || description.chars().any(|c| c.is_control() && c != '\n')
    {
        return Err(invalid("Invalid model display text"));
    }
    Ok(())
}

pub struct NewModel {
    pub public_id: String,
    pub display_name: String,
    pub description: String,
    pub family: ModelFamily,
    pub capabilities: Capabilities,
}

pub async fn create(
    db: &Database,
    actor: &AuditActor,
    input: NewModel,
) -> AppResult<PlatformModel> {
    validate_public_id(&input.public_id, input.family)?;
    validate_text(&input.display_name, &input.description)?;
    validate_capabilities(&input.capabilities)?;
    let now = Utc::now();
    let row = PlatformModel {
        id: Uuid::new_v4().to_string(),
        public_id: input.public_id,
        display_name: input.display_name,
        description: input.description,
        family: input.family,
        capabilities: input.capabilities,
        lifecycle: Lifecycle::Draft,
        enabled: false,
        revision: 1,
        authorization_revision: 1,
        routing_revision_id: None,
        billing_service_id: None,
        entitlements: Default::default(),
        created_at: now,
        updated_at: now,
    };
    persist(db, actor, None, &row, None, "platform_model_created").await?;
    Ok(row)
}

pub async fn get(db: &Database, id: &str) -> AppResult<PlatformModel> {
    db.collection::<PlatformModel>(COLLECTION_NAME)
        .find_one(doc! {"_id":id})
        .await?
        .ok_or_else(unavailable)
}
pub async fn list(db: &Database, after: Option<&str>, limit: u32) -> AppResult<Vec<PlatformModel>> {
    if limit == 0 || limit > 100 {
        return Err(invalid("limit must be between 1 and 100"));
    }
    let filter = after.map_or_else(Document::new, |id| doc! {"public_id":{"$gt":id}});
    Ok(db
        .collection::<PlatformModel>(COLLECTION_NAME)
        .find(filter)
        .sort(doc! {"public_id":1})
        .limit(i64::from(limit))
        .await?
        .try_collect()
        .await?)
}
pub async fn routing(db: &Database, model: &PlatformModel) -> AppResult<Option<RoutingRevision>> {
    let Some(id) = &model.routing_revision_id else {
        return Ok(None);
    };
    let revision = db
        .collection::<RoutingRevision>(ROUTING_COLLECTION)
        .find_one(doc! {"_id":id,"model_id":&model.id})
        .await?
        .ok_or_else(unavailable)?;
    Ok(Some(revision))
}
fn expect_revision(row: &PlatformModel, revision: i64) -> AppResult<()> {
    if revision <= 0 || row.revision != revision {
        return Err(AppError::Conflict(
            "Model changed; reload before saving".into(),
        ));
    }
    Ok(())
}
fn increment(row: &mut PlatformModel, authorization: bool) -> AppResult<()> {
    row.revision = row
        .revision
        .checked_add(1)
        .ok_or_else(|| invalid("Model revision exhausted"))?;
    if authorization {
        row.authorization_revision = row
            .authorization_revision
            .checked_add(1)
            .ok_or_else(|| invalid("Model authorization revision exhausted"))?;
    }
    row.updated_at = Utc::now();
    Ok(())
}

pub struct ModelUpdate {
    pub expected_revision: i64,
    pub display_name: String,
    pub description: String,
    pub capabilities: Capabilities,
    pub lifecycle: Lifecycle,
    pub enabled: bool,
    pub billing_service_id: Option<String>,
}
pub async fn update(
    db: &Database,
    actor: &AuditActor,
    id: &str,
    input: ModelUpdate,
) -> AppResult<PlatformModel> {
    let mut row = get(db, id).await?;
    expect_revision(&row, input.expected_revision)?;
    validate_text(&input.display_name, &input.description)?;
    validate_capabilities(&input.capabilities)?;
    if row.lifecycle == Lifecycle::Retired && input.lifecycle != Lifecycle::Retired {
        return Err(invalid("Retired model IDs cannot be reused"));
    }
    if input.enabled && input.lifecycle != Lifecycle::Active {
        return Err(invalid("Only an active model can be enabled"));
    }
    if input.lifecycle == Lifecycle::Active && (row.lifecycle != Lifecycle::Active || input.enabled)
    {
        require_execution_ready(db).await?;
    }
    if let Some(id) = &input.billing_service_id
        && (row.billing_service_id != input.billing_service_id || input.enabled)
    {
        uuid(id)?;
        let service = db
            .collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
            .find_one(doc! {"_id":id,"is_active":true})
            .await?
            .ok_or_else(|| invalid("Model offering is unavailable"))?;
        if !service
            .platform_key
            .as_ref()
            .is_some_and(|c| c.purpose == PlatformKeyPurpose::ModelOffering)
        {
            return Err(invalid(
                "Billing service must be a dedicated model offering",
            ));
        }
    }
    row.display_name = input.display_name;
    row.description = input.description;
    row.capabilities = input.capabilities;
    row.lifecycle = input.lifecycle;
    row.enabled = input.enabled;
    row.billing_service_id = input.billing_service_id;
    increment(&mut row, true)?;
    persist(
        db,
        actor,
        Some(input.expected_revision),
        &row,
        None,
        "platform_model_updated",
    )
    .await?;
    Ok(row)
}

async fn validate_entitlements(db: &Database, policy: &Entitlements) -> AppResult<()> {
    if policy.rules.len() > MAX_RULES {
        return Err(invalid("At most 500 model entitlement rules are allowed"));
    }
    let mut unique = HashSet::new();
    let mut people = Vec::new();
    let mut orgs = Vec::new();
    let mut groups = Vec::new();
    for rule in &policy.rules {
        uuid(&rule.subject_id)?;
        if !unique.insert((rule.subject_kind, &rule.subject_id)) {
            return Err(invalid("Duplicate model entitlement target"));
        }
        match rule.subject_kind {
            SubjectKind::Person => people.push(&rule.subject_id),
            SubjectKind::Org => orgs.push(&rule.subject_id),
            SubjectKind::Group => groups.push(&rule.subject_id),
        }
    }
    for (ids, kind) in [(people, "person"), (orgs, "org")] {
        if !ids.is_empty() {
            let mut query = doc! {"_id":{"$in":&ids},"is_active":true};
            if kind == "person" {
                query.insert("user_type", doc! {"$ne":"org"});
            } else {
                query.insert("user_type", "org");
            }
            if db
                .collection::<Document>(USERS)
                .count_documents(query)
                .await?
                != ids.len() as u64
            {
                return Err(invalid("Entitlement target is unavailable"));
            }
        }
    }
    if !groups.is_empty()
        && db
            .collection::<Document>(GROUPS)
            .count_documents(doc! {"_id":{"$in":&groups}})
            .await?
            != groups.len() as u64
    {
        return Err(invalid("Entitlement target is unavailable"));
    }
    Ok(())
}
pub async fn set_entitlements(
    db: &Database,
    actor: &AuditActor,
    id: &str,
    revision: i64,
    policy: Entitlements,
) -> AppResult<PlatformModel> {
    let mut row = get(db, id).await?;
    expect_revision(&row, revision)?;
    validate_entitlements(db, &policy).await?;
    row.entitlements = policy;
    increment(&mut row, true)?;
    persist(
        db,
        actor,
        Some(revision),
        &row,
        None,
        "platform_model_entitlements_updated",
    )
    .await?;
    Ok(row)
}

fn same_model(native: &str, public: &str) -> bool {
    // Publisher/date spelling may differ; arbitrary aliases and other models may
    // not. Additional version equivalences need a reviewed adapter mapping.
    if native == public {
        return true;
    }
    ["@", "-"].iter().any(|separator| {
        native
            .strip_prefix(format!("{public}{separator}").as_str())
            .is_some_and(|date| {
                date.len() == 8
                    && date.bytes().all(|b| b.is_ascii_digit())
                    && chrono::NaiveDate::parse_from_str(date, "%Y%m%d").is_ok()
            })
    })
}
pub fn validate_route(model: &PlatformModel, route: &ModelRoute) -> AppResult<()> {
    uuid(&route.id)?;
    uuid(&route.credential_service_id)?;
    if route.weight == 0 || route.weight > 1000 {
        return Err(invalid("Route weight must be between 1 and 1000"));
    }
    let public = model
        .public_id
        .strip_prefix("nyxid/")
        .ok_or_else(unavailable)?;
    let (native, location) = match &route.target {
        RouteTarget::VertexPublisher {
            publisher,
            project,
            location,
            native_model,
        } => {
            if !identifier(project, 63)
                || !identifier(location, 63)
                || !matches!(
                    (publisher, model.family),
                    (VertexPublisher::Google, ModelFamily::Gemini)
                        | (VertexPublisher::Anthropic, ModelFamily::Claude)
                )
            {
                return Err(invalid("Invalid publisher model route"));
            }
            (native_model, Some(location))
        }
        RouteTarget::VertexOpenapiMaas {
            project,
            location,
            publisher,
            native_model,
        } => {
            if !identifier(project, 63)
                || !identifier(location, 63)
                || !identifier(publisher, 63)
                || model.family != ModelFamily::OpenModel
            {
                return Err(invalid("Invalid open MaaS route"));
            }
            (native_model, Some(location))
        }
        RouteTarget::DirectProvider {
            provider,
            native_model,
        } => {
            if !matches!(
                (provider, model.family),
                (DirectProvider::Openai, ModelFamily::Openai)
                    | (DirectProvider::Anthropic, ModelFamily::Claude)
            ) {
                return Err(invalid("Direct provider does not serve this model family"));
            }
            (native_model, None)
        }
    };
    if !same_model(native, public) || location.is_some_and(|l| l.contains("..")) {
        return Err(invalid(
            "Every route must serve the same public model; aliases are not accepted",
        ));
    }
    Ok(())
}
async fn validate_route_credentials(db: &Database, routes: &[ModelRoute]) -> AppResult<()> {
    let ids: Vec<_> = routes.iter().map(|r| &r.credential_service_id).collect();
    let rows: Vec<DownstreamService> = db
        .collection(crate::models::downstream_service::COLLECTION_NAME)
        .find(doc! {"_id":{"$in":ids}})
        .await?
        .try_collect()
        .await?;
    let rows: HashMap<_, _> = rows.into_iter().map(|r| (r.id.clone(), r)).collect();
    for route in routes {
        let row = rows
            .get(&route.credential_service_id)
            .ok_or_else(|| invalid("Route credential is unavailable"))?;
        if !row.is_active
            || row.credential_encrypted.is_empty()
            || row.service_type != "http"
            || !row.destination_targets.is_empty()
            || row.provider_config_id.is_some()
            || !row
                .platform_key
                .as_ref()
                .is_some_and(|c| c.enabled && c.purpose == PlatformKeyPurpose::ModelRoute)
        {
            return Err(invalid(
                "Routes require an active route-only catalog master credential",
            ));
        }
        platform_key_service::validate_model_purpose(row)?;
        super::catalog_credential_record_service::validate(row)?;
    }
    Ok(())
}
pub async fn set_routes(
    db: &Database,
    actor: &AuditActor,
    id: &str,
    revision: i64,
    routes: Vec<ModelRoute>,
) -> AppResult<PlatformModel> {
    let mut row = get(db, id).await?;
    expect_revision(&row, revision)?;
    if row.lifecycle == Lifecycle::Retired {
        return Err(invalid("Retired model routes cannot be changed"));
    }
    if routes.len() > MAX_ROUTES {
        return Err(invalid("At most 32 routes are allowed"));
    }
    let mut ids = HashSet::new();
    for route in &routes {
        validate_route(&row, route)?;
        if !ids.insert(&route.id) {
            return Err(invalid("Duplicate route ID"));
        }
    }
    if !routes.is_empty() {
        validate_route_credentials(db, &routes).await?;
    }
    // An active route edit is a new contract publication, never an unchecked swap.
    if row.lifecycle == Lifecycle::Active {
        require_execution_ready(db).await?;
    }
    increment(&mut row, true)?;
    let route_revision = RoutingRevision {
        id: Uuid::new_v4().to_string(),
        model_id: row.id.clone(),
        model_revision: row.revision,
        routes,
        created_at: Utc::now(),
    };
    row.routing_revision_id = Some(route_revision.id.clone());
    persist(
        db,
        actor,
        Some(revision),
        &row,
        Some(&route_revision),
        "platform_model_routes_updated",
    )
    .await?;
    Ok(row)
}

async fn persist(
    db: &Database,
    actor: &AuditActor,
    expected: Option<i64>,
    row: &PlatformModel,
    routing: Option<&RoutingRevision>,
    event: &str,
) -> AppResult<()> {
    let key = audit_service::audit_chain_hmac_key()
        .ok_or_else(|| AppError::Internal("Audit chain unavailable".into()))?;
    let context = (
        db.clone(),
        actor.clone(),
        row.clone(),
        routing.cloned(),
        event.to_owned(),
        expected,
    );
    transaction_service::run_db_only(
        db,
        &context,
        |session, (db, actor, row, routing, event, expected)| Box::pin(async move {
            if !super::tools_manager_service::fence_role_authority(
                db, session, &actor.user_id, None
            ).await? {
                return Err(AppError::Forbidden("Admin authority revoked".into()));
            }
            let collection = db.collection::<PlatformModel>(COLLECTION_NAME);
            if let Some(expected) = expected {
                if collection.replace_one(doc! {"_id": &row.id, "revision": expected}, row)
                    .session(&mut *session).await?.matched_count != 1 {
                    return Err(AppError::Conflict("Model changed; reload before saving".into()));
                }
            } else {
                if collection.find_one(doc! {"public_id": &row.public_id})
                    .session(&mut *session).await?.is_some() {
                    return Err(AppError::Conflict("Model ID is already registered".into()));
                }
                if let Err(error) = collection.insert_one(row).session(&mut *session).await {
                    if matches!(error.kind.as_ref(),
                        mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(write))
                            if write.code == 11000) {
                        return Err(AppError::Conflict("Model ID is already registered".into()));
                    }
                    return Err(error.into());
                }
            }
            if let Some(routing) = routing {
                db.collection::<RoutingRevision>(ROUTING_COLLECTION)
                    .insert_one(routing).session(&mut *session).await?;
            }
            audit_service::log_actor_event_in_session(
                db, session, key, actor, event,
                serde_json::json!({
                    "model_id": row.id,
                    "revision": row.revision,
                    "authorization_revision": row.authorization_revision
                })
            ).await?;
            Ok(())
        })
    ).await
}

/// One request snapshot, never a positive cross-request ACL cache.
pub struct EntitlementSnapshot {
    person_id: String,
    grants: platform_key_service::OwnerGrants,
    groups: HashSet<String>,
}
impl EntitlementSnapshot {
    pub async fn load(db: &Database, person_id: &str) -> AppResult<Self> {
        let person = db
            .collection::<User>(USERS)
            .find_one(doc! {"_id":person_id,"is_active":true})
            .await?
            .filter(|p| p.user_type.is_person())
            .ok_or_else(unavailable)?;
        let grants = platform_key_service::OwnerGrants::load(db, person_id).await?;
        let groups: Vec<Group> = db
            .collection(GROUPS)
            .find(doc! {"_id":{"$in":&person.group_ids}})
            .await?
            .try_collect()
            .await?;
        Ok(Self {
            person_id: person_id.into(),
            grants,
            groups: groups.into_iter().map(|g| g.id).collect(),
        })
    }
    pub fn reason(&self, model: &PlatformModel) -> &'static str {
        if model.lifecycle != Lifecycle::Active {
            return "model_not_active";
        }
        if !model.enabled {
            return "model_disabled";
        }
        self.policy_reason(&model.entitlements)
    }
    pub fn policy_reason(&self, policy: &Entitlements) -> &'static str {
        let mut allowed = policy.all_people;
        for rule in &policy.rules {
            let matches = match rule.subject_kind {
                SubjectKind::Person => rule.subject_id == self.person_id,
                SubjectKind::Org => {
                    rule.subject_id != self.person_id
                        && self.grants.active_owner_ids().contains(&rule.subject_id)
                }
                SubjectKind::Group => self.groups.contains(&rule.subject_id),
            };
            if matches {
                if rule.effect == Effect::Deny {
                    return "explicit_deny";
                }
                allowed = true;
            }
        }
        if allowed { "allowed" } else { "no_entitlement" }
    }
}

/// Metadata authorization is deliberately not credential or egress authority.
/// Subsequent execution must re-read policy after waits and use a funded permit.
pub async fn authorized_model(
    db: &Database,
    person_id: &str,
    public_id: &str,
) -> AppResult<PlatformModel> {
    if !feature_flag_service::personal_flag_enabled(db, person_id, FLAG).await? {
        return Err(unavailable());
    }
    let snapshot = EntitlementSnapshot::load(db, person_id).await?;
    let model = db
        .collection::<PlatformModel>(COLLECTION_NAME)
        .find_one(doc! {"public_id":public_id})
        .await?
        .ok_or_else(unavailable)?;
    if snapshot.reason(&model) != "allowed" {
        return Err(unavailable());
    }
    Ok(model)
}

/// The logical offering retains normal connection/key/org resource ceilings.
/// This only discovers metadata; it is not permission to send a request.
pub async fn authorized_catalog_scope(
    db: &Database,
    snapshot: &EntitlementSnapshot,
    allow_all: bool,
    allowed: &[String],
    scopes: &crate::models::agent_operation_scope::OperationScopes,
) -> AppResult<Option<HashSet<String>>> {
    use super::org_service::OwnerAccess;
    use crate::models::org_membership::OrgRole;
    use crate::models::user_service::{COLLECTION_NAME as CONNECTIONS, UserService};
    let person = snapshot.person_id.as_str();
    if allow_all && scopes.is_empty() {
        return Ok(None);
    }
    let ids = if allow_all {
        scopes.keys().cloned().collect::<Vec<_>>()
    } else {
        allowed.to_vec()
    };
    let rows: Vec<UserService> = db
        .collection(CONNECTIONS)
        .find(doc! {"_id":{"$in":ids},"is_active":true,"deleted_at":bson::Bson::Null})
        .await?
        .try_collect()
        .await?;
    let grants = &snapshot.grants;
    let mut access = HashMap::new();
    let mut result = HashSet::new();
    for row in rows {
        let Some(catalog) = &row.catalog_service_id else {
            continue;
        };
        if !grants.active_owner_ids().contains(&row.user_id) {
            continue;
        }
        if let std::collections::hash_map::Entry::Vacant(entry) = access.entry(row.user_id.clone())
        {
            let owner = if row.user_id == person {
                OwnerAccess::Direct
            } else {
                let Some(member) = grants.memberships().iter().find(|m| {
                    m.org_user_id == row.user_id
                        && m.member_user_id == person
                        && m.revoked_at.is_none()
                        && m.role.can_proxy()
                }) else {
                    continue;
                };
                let scope =
                    super::org_role_scope_service::effective_scope_for_membership(db, member)
                        .await?;
                if member.role == OrgRole::Admin {
                    OwnerAccess::AsOrgAdmin {
                        org_user_id: row.user_id.clone(),
                        membership_id: member.id.clone(),
                        allowed_service_ids: scope,
                    }
                } else {
                    OwnerAccess::AsOrgMember {
                        org_user_id: row.user_id.clone(),
                        membership_id: member.id.clone(),
                        role: member.role,
                        allowed_service_ids: scope,
                    }
                }
            };
            entry.insert(owner);
        }
        let owner = &access[&row.user_id];
        if !owner.allows_resource(&row.id)
            || row.admin_only
                && !matches!(owner, OwnerAccess::Direct | OwnerAccess::AsOrgAdmin { .. })
        {
            continue;
        }
        if !["/v1/chat/completions", "/v1/responses", "/v1/messages"]
            .iter()
            .any(|path| {
                super::agent_operation_scope_service::route_visible(
                    scopes,
                    &row.id,
                    Some(catalog),
                    "POST",
                    path,
                )
            })
        {
            continue;
        }
        result.insert(catalog.clone());
    }
    Ok(Some(result))
}

/// Batch authorizes before pagination. Inaccessible model names never appear in
/// cursors, counts or projections, including when an entire page is denied.
pub async fn list_authorized(
    db: &Database,
    snapshot: &EntitlementSnapshot,
    scope: Option<&HashSet<String>>,
    after: Option<&str>,
    limit: u32,
) -> AppResult<Vec<PlatformModel>> {
    if limit == 0 || limit > 100 {
        return Err(invalid("limit must be between 1 and 100"));
    }
    let mut offering_query =
        doc! {"is_active":true,"platform_key.purpose":"model_offering","platform_key.enabled":true};
    if let Some(scope) = scope {
        offering_query.insert("_id", doc! {"$in":scope.iter().collect::<Vec<_>>()});
    }
    let offerings: Vec<Document> = db
        .collection::<Document>(crate::models::downstream_service::COLLECTION_NAME)
        .find(offering_query)
        .projection(doc! {"_id":1})
        .await?
        .try_collect()
        .await?;
    let offering_ids: Vec<_> = offerings
        .iter()
        .filter_map(|o| o.get_str("_id").ok())
        .collect();
    let subjects = vec![
        doc! {"subject_kind":"person","subject_id":&snapshot.person_id},
        doc! {"subject_kind":"org","subject_id":{"$in":snapshot.grants.active_owner_ids().iter().filter(|id| *id!=&snapshot.person_id).collect::<Vec<_>>()}},
        doc! {"subject_kind":"group","subject_id":{"$in":snapshot.groups.iter().collect::<Vec<_>>()}},
    ];
    let mut query = doc! {"lifecycle":"active","enabled":true,"billing_service_id":{"$in":offering_ids},
    "$and":[
        {"$or":[{"entitlements.all_people":true},{"entitlements.rules":{"$elemMatch":{"effect":"allow","$or":&subjects}}}]},
        {"entitlements.rules":{"$not":{"$elemMatch":{"effect":"deny","$or":subjects}}}}
    ]};
    if let Some(after) = after {
        query.insert("public_id", doc! {"$gt":after});
    }
    Ok(db
        .collection::<PlatformModel>(COLLECTION_NAME)
        .find(query)
        .sort(doc! {"public_id":1})
        .limit(i64::from(limit) + 1)
        .await?
        .try_collect()
        .await?)
}

/// A server-resolved metadata plan, not a credential/materialization permit.
/// Only the future funded executor may turn this into egress authority after
/// checking complete execution readiness and revalidating live caller fences.
pub struct AuthorizedRouteMetadata {
    model: PlatformModel,
    routing: RoutingRevision,
}
impl AuthorizedRouteMetadata {
    pub fn model_revision(&self) -> i64 {
        self.model.revision
    }
    pub fn route_count(&self) -> usize {
        self.routing.routes.iter().filter(|r| r.enabled).count()
    }
}
pub async fn resolve_route_metadata(
    db: &Database,
    person: &str,
    public_id: &str,
    scope: Option<&HashSet<String>>,
) -> AppResult<AuthorizedRouteMetadata> {
    let model = authorized_model(db, person, public_id).await?;
    let Some(offering) = &model.billing_service_id else {
        return Err(unavailable());
    };
    if scope.is_some_and(|s| !s.contains(offering)) {
        return Err(unavailable());
    }
    if db.collection::<Document>(crate::models::downstream_service::COLLECTION_NAME)
        .find_one(doc! {"_id":offering,"is_active":true,"platform_key.purpose":"model_offering","platform_key.enabled":true})
        .projection(doc! {"_id":1}).await?.is_none() {return Err(unavailable());}
    let routing = routing(db, &model).await?.ok_or_else(unavailable)?;
    let active: Vec<_> = routing
        .routes
        .iter()
        .filter(|r| r.enabled)
        .cloned()
        .collect();
    if active.is_empty() {
        return Err(unavailable());
    }
    for route in &active {
        validate_route(&model, route).map_err(|_| unavailable())?;
    }
    validate_route_credentials(db, &active)
        .await
        .map_err(|error| match error {
            AppError::DatabaseError(_) => error,
            _ => unavailable(),
        })?;
    Ok(AuthorizedRouteMetadata { model, routing })
}

/// Batch only already-authorized models. Uses the admission/explain price selector;
/// callers must project these private explanations into their public DTO.
pub async fn retail_explanations(
    db: &Database,
    models: &[PlatformModel],
    ttl: u64,
    billing_enabled: bool,
) -> AppResult<HashMap<String, Vec<super::billing::model_tariffs::Explanation>>> {
    use super::billing::model_tariffs;
    use crate::models::billing_rate_cache::{BillingRateCache, COLLECTION_NAME as RATES};
    let ids: Vec<&str> = models
        .iter()
        .filter_map(|m| m.billing_service_id.as_deref())
        .collect();
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let mut services: Vec<DownstreamService> = db
        .collection(crate::models::downstream_service::COLLECTION_NAME)
        .find(doc! {"_id":{"$in":ids}})
        .await?
        .try_collect()
        .await?;
    // A billing service can host more models than this authorized page. Do not
    // load their private tariffs/rates (or let a malformed unrelated row break discovery).
    let model_ids: HashSet<&str> = models.iter().map(|m| m.id.as_str()).collect();
    for service in &mut services {
        if let Some(billing) = service.billing.as_mut() {
            if let Some(lane) = billing.platform_key_pricing.as_mut() {
                lane.model_prices
                    .retain(|binding| model_ids.contains(binding.model_id.as_str()));
            }
            if let Some(lane) = billing.byok_pricing.as_mut() {
                lane.model_prices.clear();
            }
        }
    }
    let tariffs = model_tariffs::load_for(db, &services).await?;
    let rate_ids: Vec<String> = tariffs
        .iter()
        .flat_map(|t| &t.components)
        .map(|c| BillingRateCache::cache_id(&c.lago_metric_code, None))
        .collect();
    let rates = if rate_ids.is_empty() {
        HashMap::new()
    } else {
        db.collection::<BillingRateCache>(RATES)
            .find(doc! {"_id":{"$in":rate_ids}})
            .await?
            .try_collect::<Vec<_>>()
            .await?
            .into_iter()
            .map(|rate| (rate.lago_metric_code.clone(), rate))
            .collect()
    };
    let now = Utc::now();
    Ok(models
        .iter()
        .filter_map(|model| {
            let service = services
                .iter()
                .find(|s| Some(&s.id) == model.billing_service_id.as_ref())?;
            let prices =
                model_tariffs::explain(service, &tariffs, &rates, now, ttl, billing_enabled)
                    .into_iter()
                    .filter(|p| {
                        p.model_id == model.id
                            && p.lane
                                == crate::models::platform_model_tariff::TariffLane::PlatformKey
                    })
                    .collect();
            Some((model.id.clone(), prices))
        })
        .collect())
}

#[cfg(test)]
mod tests;
