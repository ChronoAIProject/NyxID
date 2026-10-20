//! Live machine policy. The editor flag selects defaults for new assignments;
//! admission and renewal always enforce stored policy, live ACL and local ceilings.
use crate::{
    errors::{AppError, AppResult},
    models::{
        assistant_agent::{AssistantAgent, COLLECTION_NAME as AGENTS},
        machine_access::{self as model, Assignment, Policy, Selection},
        node::{COLLECTION_NAME as NODES, Node},
    },
    services::{
        api_key_mutation_service as transactions, assistant_acknowledgement_service::ChatAuthority,
    },
};
use chrono::Utc;
use futures::TryStreamExt;
use mongodb::{
    ClientSession, Database, IndexModel,
    bson::{self, Document, doc},
};
use nyxid_machine::{
    Operation,
    authority::{Authority, Capabilities},
    context::{self, Choice, Mode},
};

pub const FLAG: &str = "assistant:machine-capabilities";
pub const CONTEXT_FLAG: &str = "assistant:machine-contexts";
/// Leaves room for the maximum 64 local jobs plus concurrent short operations.
pub(super) const MAX_NODE_LEASES: u64 = 128;
const RENEW_INTERVAL_SECS: u64 = 10;
const SETTINGS: &str = "platform_settings";
fn encode<T: serde::Serialize>(value: &T) -> AppResult<bson::Bson> {
    bson::to_bson(value).map_err(|_| AppError::Internal("Machine authority encoding failed".into()))
}
fn refused() -> AppError {
    AppError::MachinePermissionRevoked
}

/// Advertisements alone never authorize separated execution. Re-evaluate the
/// current protocol and every observed prerequisite, including credential custody.
pub fn separated_support(profile: &nyxid_machine::MachineProfile) -> context::Support {
    let evidence = profile.separation_evidence.as_ref();
    let reason = if !profile.authority_v2() {
        Some("machine_authority_upgrade_required")
    } else {
        match evidence {
            None => Some("separated_capabilities_not_reported"),
            Some(evidence) => evidence.unavailable_reason().or_else(|| {
                (!profile.separated.as_ref().is_some_and(|s| s.available))
                    .then_some("separated_context_unavailable")
            }),
        }
    };
    context::Support {
        available: reason.is_none(),
        landlock_abi: evidence.and_then(|e| e.landlock_abi),
        reason: reason.map(str::to_owned),
    }
}

pub fn supports_separated(profile: &nyxid_machine::MachineProfile) -> bool {
    separated_support(profile).available
}

/// Current reports are mandatory for a new separated assignment. Existing
/// assignments are never passed through this selector or reinterpreted.
pub fn select_profile(
    profile: &nyxid_machine::MachineProfile,
    choice: Option<Choice>,
) -> Result<Mode, &'static str> {
    let choice = choice.ok_or("machine_selection_required")?;
    let mode = context::select_new(choice, profile.separation_evidence.as_ref())?;
    if mode == Mode::Separated && !profile.authority_v2() {
        return Err("machine_authority_upgrade_required");
    }
    Ok(mode)
}

pub fn selection_message(reason: &str) -> String {
    if reason == "machine_context_setup_not_enabled" {
        return "machine_selection_required: The recorded choice requires separated contexts, but context setup is not enabled for this account. Ask the administrator to enable it, or explicitly approve shared legacy access in Assistant → Machines after reviewing the warning. No shared fallback has been selected.".into();
    }
    if reason == "machine_selection_required" {
        return "machine_selection_required: This assignment needs an owner separation selection before execution. Open Assistant → Machines to confirm supported automatic/required separation or explicitly approve shared legacy access after reviewing its warning.".into();
    }
    format!(
        "machine_selection_required: {} Choose automatic or require-separated after confirming support, or explicitly approve shared legacy access after reviewing its warning in Assistant → Machines.",
        context::unavailable_message(reason)
    )
}

pub fn pending_selection_reason(
    profile: &nyxid_machine::MachineProfile,
    contexts: bool,
) -> &'static str {
    match select_profile(profile, profile.separation_choice) {
        Err(reason) => reason,
        Ok(Mode::Separated) if !contexts => "machine_context_setup_not_enabled",
        Ok(_) => "machine_selection_required",
    }
}

/// A denied placeholder makes an unresolved choice visible without granting
/// execution. Capability editing still defaults new permissions to off.
pub(super) fn selected_assignment(
    profile: &nyxid_machine::MachineProfile,
    revision: i64,
    editing: bool,
    contexts: bool,
) -> Assignment {
    let mode = select_profile(profile, profile.separation_choice)
        .ok()
        .filter(|mode| *mode != Mode::Separated || contexts);
    Assignment {
        mode: match mode {
            Some(Mode::Separated) => "separated",
            Some(Mode::SharedLegacy) => "shared_legacy",
            None => model::SELECTION_REQUIRED,
        }
        .into(),
        capabilities: if mode.is_some() && !editing {
            Capabilities::legacy(profile)
        } else {
            Capabilities::default()
        },
        legacy: mode == Some(Mode::SharedLegacy) && !editing,
        revision,
        saved_login_ids: (mode == Some(Mode::Separated)).then(Vec::new),
    }
}

pub fn unresolved_assignment(revision: i64) -> Assignment {
    Assignment {
        mode: model::SELECTION_REQUIRED.into(),
        revision,
        ..Default::default()
    }
}

/// Freeze actual machine enablement and org management reachability, rather
/// than inferring either from the creation time of a node or membership.
/// The marker and snapshot commit together; a restart never extends the set.
async fn ensure_separation_cutover(db: &Database) -> AppResult<()> {
    // MongoDB cannot implicitly create a collection inside a transaction.
    // Its unique _id index is also the only index these snapshot reads need.
    db.collection::<Document>(model::SEPARATION_COMPATIBILITY)
        .update_one(
            doc! {"_id":"schema-v1"},
            doc! {"$setOnInsert":{"version":1}},
        )
        .upsert(true)
        .await?;
    let mut session = db.client().start_session().await?;
    let db = db.clone();
    session.start_transaction().and_run2(async move |session| {
        let result = Box::pin(async {
            if let Some(marker) = db.collection::<Document>(SETTINGS)
                .find_one(doc! {"_id":model::SEPARATION_CUTOVER})
                .session(&mut *session).await? {
                if marker.get_i32("snapshot_version").ok() != Some(1) || marker.get_datetime("created_at").is_err() {
                    return Err(AppError::Internal("Machine separation cutover is unreadable".into()));
                }
                return Ok(());
            }
            db.collection::<Document>(SETTINGS).insert_one(doc! {
                "_id":model::SEPARATION_CUTOVER,"created_at":bson::DateTime::now(),"snapshot_version":1,
            }).session(&mut *session).await?;
            let orgs: Vec<Document> = db.collection::<Document>(crate::models::user::COLLECTION_NAME)
                .find(doc! {"user_type":"org","is_active":true}).projection(doc! {"_id":1})
                .session(&mut *session).await?.stream(&mut *session).try_collect().await?;
            let org_ids: Vec<&str> = orgs.iter().filter_map(|org| org.get_str("_id").ok()).collect();
            let mut memberships = db.collection::<Document>(crate::models::org_membership::COLLECTION_NAME)
                .find(doc! {"org_user_id":{"$in":org_ids},"role":"admin","revoked_at":bson::Bson::Null})
                .batch_size(100).session(&mut *session).await?;
            let mut snapshots = Vec::with_capacity(100);
            loop {
                let member = memberships.stream(&mut *session).try_next().await?;
                let Some(member) = member else { break };
                let (Ok(org),Ok(actor)) = (member.get_str("org_user_id"),member.get_str("member_user_id")) else {continue};
                snapshots.push(doc! {"_id":format!("{org}:{actor}"),"owner_id":org,"actor_id":actor});
                if snapshots.len() == 100 {
                    db.collection::<Document>(model::SEPARATION_COMPATIBILITY)
                        .insert_many(std::mem::take(&mut snapshots)).session(&mut *session).await?;
                }
            }
            let mut nodes = db.collection::<Node>(NODES)
                .find(doc! {"is_active":true,"machine":{"$ne":bson::Bson::Null}})
                .batch_size(100).session(&mut *session).await?;
            loop {
                let node = nodes.stream(&mut *session).try_next().await?;
                let Some(node) = node else { break };
                if let Some(profile) = node.machine.filter(|p| p.enabled()) {
                    snapshots.push(doc! {
                        "_id":node.id,"owner_id":node.user_id,"capabilities":encode(&Capabilities::legacy(&profile))?,
                    });
                    if snapshots.len() == 100 {
                        db.collection::<Document>(model::SEPARATION_COMPATIBILITY)
                            .insert_many(std::mem::take(&mut snapshots)).session(&mut *session).await?;
                    }
                }
            }
            if !snapshots.is_empty() {
                db.collection::<Document>(model::SEPARATION_COMPATIBILITY)
                    .insert_many(snapshots).session(&mut *session).await?;
            }
            Ok(())
        }).await;
        transactions::transaction_result(result)
    }).await.map_err(transactions::map_transaction_error)
}

async fn compatibility_capabilities(
    db: &Database,
    actor: &str,
    nodes: &[Node],
    session: &mut ClientSession,
) -> AppResult<std::collections::BTreeMap<String, Capabilities>> {
    let ids: Vec<String> = nodes
        .iter()
        .flat_map(|node| [node.id.clone(), format!("{}:{actor}", node.user_id)])
        .collect();
    let snapshots: Vec<Document> = db
        .collection::<Document>(model::SEPARATION_COMPATIBILITY)
        .find(doc! {"_id":{"$in":ids}})
        .session(&mut *session)
        .await?
        .stream(&mut *session)
        .try_collect()
        .await?;
    let rows: std::collections::BTreeMap<&str, &Document> = snapshots
        .iter()
        .filter_map(|row| row.get_str("_id").ok().map(|id| (id, row)))
        .collect();
    Ok(nodes
        .iter()
        .filter_map(|node| {
            let snapshot = rows.get(node.id.as_str())?;
            if snapshot.get_str("owner_id").ok() != Some(node.user_id.as_str())
                || (node.user_id != actor
                    && !rows.contains_key(format!("{}:{actor}", node.user_id).as_str()))
            {
                return None;
            }
            let caps = bson::from_bson(snapshot.get("capabilities")?.clone()).ok()?;
            Some((node.id.clone(), caps))
        })
        .collect())
}

pub async fn ensure_indexes(db: &Database) -> AppResult<()> {
    db.collection::<Document>(SETTINGS)
        .update_one(
            doc! {"_id":model::CUTOVER},
            doc! {"$setOnInsert":{"created_at":bson::DateTime::now()}},
        )
        .upsert(true)
        .await?;
    ensure_separation_cutover(db).await?;
    // The former unconditional index treated absent Assistant IDs as a shared
    // null identity. External contexts have their own typed grant identity.
    // Build the narrower replacement first, then retire the old definition.
    db.collection::<Document>(model::CONTEXTS)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"node_id":1,"owner_id":1,"agent_id":1,"actor_id":1,"group_id":1})
                .options(
                    mongodb::options::IndexOptions::builder()
                        .name(Some("assistant_context_identity".to_string()))
                        .unique(true)
                        .partial_filter_expression(doc! {"agent_id":{"$type":"string"}})
                        .build(),
                )
                .build(),
        )
        .await?;
    let old_index = "node_id_1_owner_id_1_agent_id_1_actor_id_1_group_id_1";
    let existing: Vec<mongodb::IndexModel> = db
        .collection::<Document>(model::CONTEXTS)
        .list_indexes()
        .await?
        .try_collect()
        .await?;
    if existing
        .iter()
        .any(|i| i.options.as_ref().and_then(|o| o.name.as_deref()) == Some(old_index))
    {
        db.collection::<Document>(model::CONTEXTS)
            .drop_index(old_index)
            .await?;
    }
    for (collection, keys) in [
        (
            model::LEASES,
            doc! {"authority.agent_id":1,"node_id":1,"authority.revision":1},
        ),
        (
            model::LEASES,
            doc! {"node_id":1,"authority.expires_at_ms":1},
        ),
        (model::OUTBOX, doc! {"pending":1,"claim_until":1}),
        (model::OUTBOX, doc! {"agent_id":1,"pending":1,"node_id":1}),
        (
            model::CONTEXTS,
            doc! {"login_ids":1,"mode":1,"agent_id":1,"node_id":1},
        ),
    ] {
        db.collection::<Document>(collection)
            .create_index(IndexModel::builder().keys(keys).build())
            .await?;
    }
    db.collection::<Document>(model::LEASES)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"expires_at":1})
                .options(
                    mongodb::options::IndexOptions::builder()
                        .expire_after(std::time::Duration::ZERO)
                        .build(),
                )
                .build(),
        )
        .await?;
    Ok(())
}

/// Runs inside the same transaction as admission/membership writes. The agent
/// write conflicts with all modern writers; pre-cutover data alone may inherit.
pub async fn migrate_in_session(
    db: &Database,
    agent: &mut AssistantAgent,
    session: &mut ClientSession,
) -> AppResult<()> {
    if agent.machine_access.is_some() {
        return Ok(());
    }
    let cutoff = db
        .collection::<Document>(SETTINGS)
        .find_one(doc! {"_id":model::CUTOVER})
        .session(&mut *session)
        .await?
        .and_then(|d| d.get_datetime("created_at").ok().copied())
        .ok_or_else(|| AppError::Internal("Machine authority migration not initialized".into()))?;
    let mut policy = Policy::default();
    if agent.created_at <= cutoff.to_chrono() {
        let owners = if agent.is_nyxbot() {
            let mut owners = super::machine_service::usable_owners(db, &agent.user_id).await?;
            // A delayed migration must not inherit organizations joined after
            // the cutover. Live ACL checks still apply on every execution.
            let memberships: Vec<Document> = db
                .collection::<Document>(crate::models::org_membership::COLLECTION_NAME)
                .find(
                    doc! {"member_user_id": &agent.user_id, "org_user_id": {"$in": &owners},
                    "created_at": {"$lte": cutoff}, "revoked_at": bson::Bson::Null},
                )
                .projection(doc! {"org_user_id": 1})
                .session(&mut *session)
                .await?
                .stream(&mut *session)
                .try_collect()
                .await?;
            owners.retain(|owner| {
                owner == &agent.user_id
                    || memberships
                        .iter()
                        .any(|m| m.get_str("org_user_id").ok() == Some(owner.as_str()))
            });
            owners
        } else {
            vec![]
        };
        let mut filter = doc! {"created_at":{"$lte":cutoff},"machine":{"$ne":bson::Bson::Null}};
        if agent.is_nyxbot() {
            filter.insert("user_id", doc! {"$in":owners});
        } else {
            filter.insert("_id", doc! {"$in":&agent.machine_node_ids});
        }
        // Discovery's 500-row page is not an authority bound. Stream the full
        // pre-cutover snapshot in bounded driver batches: legacy rosters keep
        // working even when larger than the new-assignment/editor page limits.
        let mut nodes = db
            .collection::<Node>(NODES)
            .find(filter)
            .batch_size(100)
            .session(&mut *session)
            .await?;
        let contexts =
            super::feature_flag_service::personal_flag_enabled(db, &agent.user_id, CONTEXT_FLAG)
                .await?;
        loop {
            let mut batch = Vec::with_capacity(100);
            while batch.len() < 100 {
                match nodes.stream(&mut *session).try_next().await? {
                    Some(node) => batch.push(node),
                    None => break,
                }
            }
            if batch.is_empty() {
                break;
            }
            let compatibility = if agent.is_nyxbot() {
                compatibility_capabilities(db, &agent.user_id, &batch, session).await?
            } else {
                Default::default()
            };
            for node in batch {
                if let Some(profile) = node.machine.as_ref() {
                    let assignment = if agent.is_nyxbot() {
                        if profile.separation_choice.is_none()
                            && compatibility.contains_key(&node.id)
                        {
                            Assignment {
                                capabilities: compatibility[&node.id],
                                legacy: true,
                                ..Default::default()
                            }
                        } else {
                            selected_assignment(profile, 1, false, contexts)
                        }
                    } else {
                        Assignment {
                            capabilities: Capabilities::legacy(profile),
                            legacy: true,
                            ..Default::default()
                        }
                    };
                    policy.assignments.insert(node.id, assignment);
                }
            }
        }
    }
    agent.machine_access = Some(Box::new(policy));
    db.collection::<Document>(AGENTS)
        .update_one(
            doc! {"_id":&agent.id},
            doc! {"$set":{"machine_access":encode(&agent.machine_access)?}},
        )
        .session(session)
        .await?;
    Ok(())
}

pub async fn policy(db: &Database, id: &str) -> AppResult<Policy> {
    // Every call reads live policy. Already-migrated rows need no transaction
    // or full agent payload; admission supplies its own transactional fence.
    let row = db
        .collection::<Document>(AGENTS)
        .find_one(doc! {"_id": id, "destroyed_at": bson::Bson::Null})
        .projection(doc! {"machine_access": 1})
        .await?
        .ok_or_else(refused)?;
    if let Some(value) = row
        .get("machine_access")
        .filter(|v| **v != bson::Bson::Null)
    {
        return bson::from_bson(value.clone()).map_err(|_| refused());
    }
    let mut session = db.client().start_session().await?;
    let db = db.clone();
    let id = id.to_owned();
    session
        .start_transaction()
        .and_run2(async move |session| {
            let result = Box::pin(async {
                let mut agent = db
                    .collection::<AssistantAgent>(AGENTS)
                    .find_one(doc! {"_id":&id,"destroyed_at":bson::Bson::Null})
                    .session(&mut *session)
                    .await?
                    .ok_or_else(refused)?;
                Box::pin(migrate_in_session(&db, &mut agent, session)).await?;
                Ok(*agent.machine_access.expect("migrated"))
            })
            .await;
            transactions::transaction_result(result)
        })
        .await
        .map_err(transactions::map_transaction_error)
}

/// Called only with machine selections already authorized by the grant flow.
/// New authority always uses recorded intent and positively observed support.
pub async fn new_assignments(
    db: &Database,
    actor: &str,
    nodes: &[String],
    revision: i64,
) -> AppResult<std::collections::BTreeMap<String, Assignment>> {
    let mut assignments = nodes
        .iter()
        .map(|id| (id.clone(), unresolved_assignment(revision)))
        .collect::<std::collections::BTreeMap<_, _>>();
    if nodes.is_empty() {
        return Ok(assignments);
    }
    let editing = super::feature_flag_service::personal_flag_enabled(db, actor, FLAG).await?;
    let contexts =
        super::feature_flag_service::personal_flag_enabled(db, actor, CONTEXT_FLAG).await?;
    let mut rows = db
        .collection::<Node>(NODES)
        .find(doc! {"_id":{"$in":nodes},"is_active":true})
        .batch_size(100)
        .await?;
    while let Some(node) = rows.try_next().await? {
        if let Some(profile) = node.machine {
            assignments.insert(
                node.id,
                selected_assignment(&profile, revision, editing, contexts),
            );
        }
    }
    Ok(assignments)
}

/// Materialize only missing NyxBot assignments, after the caller's live node ACL.
/// Discovery supplies one authorized batch, so this never adds a per-row query.
async fn legacy_nyxbot_assignments(
    db: &Database,
    chat: &ChatAuthority,
    agent: &AssistantAgent,
    nodes: &[Node],
    policy: Policy,
) -> AppResult<Policy> {
    if policy.version != 2 {
        return Err(refused());
    }
    if !agent.is_nyxbot()
        || nodes.iter().all(|n| policy.assignments.contains_key(&n.id))
        || super::feature_flag_service::personal_flag_enabled(db, &chat.user_id, FLAG).await?
    {
        return Ok(policy);
    }
    let db = db.clone();
    let agent_id = agent.id.clone();
    let actor = chat.user_id.clone();
    let nodes = nodes.to_vec();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let result = Box::pin(async {
            let mut agent = db.collection::<AssistantAgent>(AGENTS)
                .find_one(doc! {"_id":&agent_id,"user_id":&actor,"kind":"nyxbot","destroyed_at":bson::Bson::Null})
                .session(&mut *session).await?.ok_or_else(refused)?;
            Box::pin(migrate_in_session(&db, &mut agent, session)).await?;
            let policy = agent.machine_access.as_mut().expect("migrated");
            if policy.version != 2 { return Err(refused()); }
            // A concurrent editor write wins through this transaction's agent-row fence.
            if super::feature_flag_service::personal_flag_enabled(&db, &actor, FLAG).await? {
                return Ok((**policy).clone());
            }
            let owners = super::machine_service::usable_owners(&db, &actor).await?;
            let cutoff = db.collection::<Document>(SETTINGS)
                .find_one(doc! {"_id":model::SEPARATION_CUTOVER}).session(&mut *session).await?
                .and_then(|d| d.get_datetime("created_at").ok().copied())
                .ok_or_else(|| AppError::Internal("Machine separation migration not initialized".into()))?;
            let contexts = super::feature_flag_service::personal_flag_enabled(&db, &actor, CONTEXT_FLAG).await?;
            let mut changed = false;
            let revision = policy.revision.checked_add(1).ok_or_else(refused)?;
            // Re-read the authorized batch in this transaction. Discovery's
            // batched projection never becomes one database lookup per node.
            let ids: Vec<&str> = nodes.iter().map(|node| node.id.as_str()).collect();
            let live_nodes: Vec<Node> = db.collection::<Node>(NODES)
                .find(doc! {"_id":{"$in":ids},"is_active":true,"user_id":{"$in":&owners}})
                .session(&mut *session).await?.stream(&mut *session).try_collect().await?;
            let compatibility = compatibility_capabilities(&db,&actor,&live_nodes,session).await?;
            for node in &live_nodes {
                if !policy.assignments.contains_key(&node.id)
                    && let Some(profile) = node.machine.as_ref()
                {
                    let preexisting = agent.created_at <= cutoff.to_chrono()
                        && compatibility.contains_key(&node.id);
                    let assignment = if preexisting && profile.separation_choice.is_none() {
                        Assignment { capabilities: compatibility[&node.id], legacy:true, revision, ..Default::default() }
                    } else {
                        selected_assignment(profile, revision, false, contexts)
                    };
                    policy.assignments.insert(node.id.clone(), assignment);
                    changed = true;
                }
            }
            if changed {
                policy.revision = revision;
                db.collection::<Document>(AGENTS).update_one(doc! {"_id":&agent_id},
                    doc! {"$set":{"machine_access":encode(&agent.machine_access)?}})
                    .session(session).await?;
            }
            Ok(*agent.machine_access.expect("migrated"))
        }).await;
        transactions::transaction_result(result)
    }).await.map_err(transactions::map_transaction_error)
}

pub async fn assignment(db: &Database, chat: &ChatAuthority, node: &Node) -> AppResult<Assignment> {
    super::machine_service::caller(chat)?;
    super::remote_machine_manager::require_execution(db, node).await?;
    let agent = super::org_agent_service::chat_agent(db, chat).await?;
    if agent.destroyed_at.is_some()
        || (agent.user_id != chat.user_id && node.user_id != agent.user_id)
        || (agent.user_id == chat.user_id
            && !super::org_service::resolve_owner_access(db, &chat.user_id, &node.user_id)
                .await?
                .can_write())
    {
        return Err(refused());
    }
    let policy = Box::pin(policy(db, &agent.id)).await?;
    let policy = Box::pin(legacy_nyxbot_assignments(
        db,
        chat,
        &agent,
        std::slice::from_ref(node),
        policy,
    ))
    .await?;
    if policy.version != 2 {
        return Err(refused());
    }
    let assignment = policy
        .assignments
        .get(&node.id)
        .cloned()
        .unwrap_or_else(|| unresolved_assignment(policy.revision));
    if !agent.is_nyxbot() && !agent.machine_node_ids.contains(&node.id) {
        return Err(refused());
    }
    match assignment.mode.as_str() {
        model::SELECTION_REQUIRED => {
            let contexts =
                super::feature_flag_service::personal_flag_enabled(db, &chat.user_id, CONTEXT_FLAG)
                    .await?;
            let reason = node
                .machine
                .as_ref()
                .map(|p| pending_selection_reason(p, contexts))
                .unwrap_or("machine_selection_required");
            return Err(AppError::ValidationError(selection_message(reason)));
        }
        "shared_legacy" => {}
        "separated"
            if !assignment.legacy && node.machine.as_ref().is_some_and(supports_separated) => {}
        _ => return Err(AppError::MachineAuthorityUnsupported),
    }
    if !node.machine.as_ref().is_some_and(|p| p.authority_v2()) && !assignment.legacy {
        return Err(AppError::MachineAuthorityUnsupported);
    }
    Ok(assignment)
}

pub async fn authorize(
    db: &Database,
    chat: &ChatAuthority,
    node: &Node,
    operation: Operation,
    args: &serde_json::Value,
) -> AppResult<Assignment> {
    if operation == Operation::JobCancel {
        super::machine_service::caller(chat)?;
        let job = args["job_id"]
            .as_str()
            .ok_or(AppError::MachineJobNotFound)?;
        super::machine_service::job(db, chat, &node.id, job).await?;
        let policy = Box::pin(policy(db, &chat.agent_id)).await?;
        return Ok(policy
            .assignments
            .get(&node.id)
            .cloned()
            .unwrap_or_else(|| Assignment {
                revision: policy.revision,
                legacy: !node.machine.as_ref().is_some_and(|p| p.authority_v2()),
                ..Default::default()
            }));
    }
    let access = Box::pin(assignment(db, chat, node)).await?;
    if !access.capabilities.allows(operation, args) {
        return Err(refused());
    }
    if operation == Operation::FillLogin
        && access.saved_login_ids.as_ref().is_some_and(|ids| {
            args["login"]
                .as_str()
                .is_none_or(|id| !ids.iter().any(|v| v == id))
        })
    {
        return Err(refused());
    }
    Ok(access)
}

/// Preserve existing policy; new selections follow the acting person's editor flag.
/// Removed assignments retain a bounded all-off preference: a later Grants
/// selection or implicit discovery must not resurrect revoked authority.
pub async fn membership_changed(
    db: &Database,
    actor: &str,
    agent: &mut AssistantAgent,
    before: &[String],
    logins_changed: bool,
    session: &mut ClientSession,
) -> AppResult<()> {
    let after = agent.machine_node_ids.clone();
    agent.machine_node_ids = before.to_vec();
    Box::pin(migrate_in_session(db, agent, session)).await?;
    agent.machine_node_ids = after;
    let policy = agent.machine_access.as_mut().expect("migrated");
    if policy.version != 2 {
        return Err(AppError::MachineAuthorityUnsupported);
    }
    if before == agent.machine_node_ids && !logins_changed {
        return Ok(());
    }
    policy.revision = policy.revision.checked_add(1).ok_or_else(refused)?;
    for node in before
        .iter()
        .filter(|id| !agent.machine_node_ids.contains(id))
    {
        let separated = policy
            .assignments
            .get(node)
            .is_some_and(|a| a.mode == "separated");
        if separated {
            let assignment = policy
                .assignments
                .get_mut(node)
                .expect("existing assignment");
            assignment.capabilities = Capabilities::default();
            assignment.revision = policy.revision;
            assignment.legacy = false;
            db.collection::<Document>(model::CONTEXTS)
                .update_many(
                    doc! {"agent_id":&agent.id,"node_id":node,"mode":"separated"},
                    doc! {"$inc":{"generation":1},"$set":{"login_ids":[]}},
                )
                .session(&mut *session)
                .await?;
        } else if let Some(assignment) = policy.assignments.get_mut(node) {
            assignment.capabilities = Capabilities::default();
            assignment.revision = policy.revision;
            // Preserve the mode and denial so implicit NyxBot discovery cannot
            // recreate a revoked assignment, including pre-cutover nodes.
        }
        enqueue(db, &agent.id, node, policy.revision, session).await?;
        if separated {
            mark_profile_reset(db, &agent.id, node, policy.revision, session).await?;
        }
    }
    let added: Vec<String> = agent
        .machine_node_ids
        .iter()
        .filter(|node| !policy.assignments.contains_key(*node))
        .cloned()
        .collect();
    if !added.is_empty() && added.len() > 64_usize.saturating_sub(policy.assignments.len()) {
        return Err(AppError::ValidationError(
            "Machine assignment history is full. At most 64 new choices, including removed assignments, may be retained. Reuse a previously selected machine or create another agent.".into(),
        ));
    }
    policy
        .assignments
        .extend(Box::pin(new_assignments(db, actor, &added, policy.revision)).await?);
    if logins_changed {
        db.collection::<Document>(model::CONTEXTS)
            .update_many(
                doc! {"agent_id":&agent.id,"mode":"separated"},
                doc! {"$inc":{"generation":1},"$set":{"login_ids":[]}},
            )
            .session(&mut *session)
            .await?;
        for (node, assignment) in &mut policy.assignments {
            assignment.revision = policy.revision;
            enqueue(db, &agent.id, node, policy.revision, session).await?;
            mark_profile_reset(db, &agent.id, node, policy.revision, session).await?;
        }
    }
    Ok(())
}
/// Destruction commits its durable cancellation records with the agent tombstone.
pub async fn destroyed_in_session(
    db: &Database,
    agent: &mut AssistantAgent,
    session: &mut ClientSession,
) -> AppResult<()> {
    Box::pin(migrate_in_session(db, agent, session)).await?;
    let policy = agent.machine_access.as_mut().expect("migrated");
    if policy.version != 2 {
        return Err(AppError::MachineAuthorityUnsupported);
    }
    policy.revision = policy.revision.checked_add(1).ok_or_else(refused)?;
    for node in policy.assignments.keys() {
        enqueue(db, &agent.id, node, policy.revision, session).await?;
    }
    policy.assignments.clear();
    db.collection::<Document>(AGENTS)
        .update_one(
            doc! {"_id": &agent.id},
            doc! {"$set": {"machine_access": encode(&agent.machine_access)?}},
        )
        .session(session)
        .await?;
    Ok(())
}

async fn enqueue(
    db: &Database,
    agent: &str,
    node: &str,
    revision: i64,
    session: &mut ClientSession,
) -> AppResult<()> {
    db.collection::<Document>(model::OUTBOX)
        .insert_one(doc! {"_id":uuid::Uuid::new_v4().to_string(),
        "agent_id":agent,"node_id":node,"revision":revision,"pending":true,
        "created_at":bson::DateTime::now(),
        "claim_until":bson::DateTime::from_millis(0)})
        .session(&mut *session)
        .await?;
    Ok(())
}

async fn mark_profile_reset(
    db: &Database,
    agent: &str,
    node: &str,
    revision: i64,
    session: &mut ClientSession,
) -> AppResult<()> {
    db.collection::<Document>(model::OUTBOX)
        .update_many(
            doc! {"agent_id":agent,"node_id":node,"revision":revision,"pending":true},
            doc! {"$set":{"quarantine_profiles":true}},
        )
        .session(session)
        .await?;
    Ok(())
}

/// Runs in the saved-login deletion transaction. Recording a fill writes that
/// same login row, so deletion cannot miss a concurrently admitted binding.
pub async fn quarantine_login_in_session(
    db: &Database,
    login: &str,
    session: &mut ClientSession,
) -> AppResult<()> {
    let mut rows = db
        .collection::<Document>(model::CONTEXTS)
        .find(doc! {"login_ids":login,"mode":"separated"})
        .sort(doc! {"agent_id":1,"node_id":1})
        .projection(doc! {"agent_id":1,"node_id":1})
        .batch_size(100)
        .session(&mut *session)
        .await?;
    let mut last = None;
    loop {
        let row = rows.stream(&mut *session).try_next().await?;
        let Some(row) = row else { break };
        let agent_id = row.get_str("agent_id").map_err(|_| refused())?;
        let node_id = row.get_str("node_id").map_err(|_| refused())?;
        let key = (agent_id.to_owned(), node_id.to_owned());
        if last.as_ref() == Some(&key) {
            continue;
        }
        last = Some(key);
        let Some(mut agent) = db
            .collection::<AssistantAgent>(AGENTS)
            .find_one(doc! {"_id":agent_id})
            .session(&mut *session)
            .await?
        else {
            continue;
        };
        let Some(policy) = agent.machine_access.as_mut() else {
            continue;
        };
        policy.revision = policy.revision.checked_add(1).ok_or_else(refused)?;
        if let Some(assignment) = policy.assignments.get_mut(node_id) {
            assignment.revision = policy.revision;
            if let Some(ids) = &mut assignment.saved_login_ids {
                ids.retain(|id| id != login);
            }
        }
        let revision = policy.revision;
        db.collection::<Document>(AGENTS)
            .update_one(
                doc! {"_id":agent_id},
                doc! {"$set":{"machine_access":encode(&agent.machine_access)?}},
            )
            .session(&mut *session)
            .await?;
        db.collection::<Document>(model::CONTEXTS)
            .update_many(
                doc! {"agent_id":agent_id,"node_id":node_id,"mode":"separated"},
                doc! {"$inc":{"generation":1},"$set":{"login_ids":[]}},
            )
            .session(&mut *session)
            .await?;
        enqueue(db, agent_id, node_id, revision, session).await?;
        mark_profile_reset(db, agent_id, node_id, revision, session).await?;
    }
    Ok(())
}

pub async fn configure(
    db: &Database,
    actor: &str,
    agent_id: &str,
    node_id: &str,
    selection: Selection,
) -> AppResult<Policy> {
    let editing = super::feature_flag_service::personal_flag_enabled(db, actor, FLAG).await?;
    let contexts =
        super::feature_flag_service::personal_flag_enabled(db, actor, CONTEXT_FLAG).await?;
    let context_action = selection.mode.is_some();
    if !editing && !context_action {
        return Err(AppError::ValidationError(
            "Machine capability configuration is not enabled. Existing restrictions still apply."
                .into(),
        ));
    }
    if selection.mode.as_deref().is_some_and(|mode| {
        !matches!(
            mode,
            "shared_legacy" | "separated" | "automatic" | "require_separated"
        )
    }) {
        return Err(AppError::ValidationError(
            "Choose automatic, require_separated or shared_legacy".into(),
        ));
    }
    if selection
        .mode
        .as_deref()
        .is_some_and(|mode| mode != "shared_legacy")
        && !contexts
    {
        return Err(AppError::ValidationError(
            "Machine context setup is not enabled; existing separation still applies".into(),
        ));
    }
    if !selection.capabilities.valid()
        || selection.saved_login_ids.as_ref().is_some_and(|ids| {
            ids.len() > 64 || ids.iter().any(|id| uuid::Uuid::parse_str(id).is_err())
        })
    {
        return Err(AppError::ValidationError(
            "Computer and developer browser require browser; at most 64 saved logins are allowed"
                .into(),
        ));
    }
    let agent = super::assistant_team_service::maintained_agent(db, actor, agent_id).await?;
    let node = super::node_service::get_node_by_id(db, node_id)
        .await?
        .ok_or_else(refused)?;
    if !super::org_service::resolve_owner_access(db, actor, &node.user_id)
        .await?
        .can_write()
        || (agent.user_id != actor && node.user_id != agent.user_id)
    {
        return Err(refused());
    }
    let profile = node
        .machine
        .as_ref()
        .ok_or(AppError::MachineCapabilityDisabled)?
        .clone();
    if !profile.authority_v2()
        && (selection.mode.as_deref() != Some("shared_legacy")
            || selection.capabilities != Capabilities::legacy(&profile)
            || selection
                .saved_login_ids
                .as_ref()
                .is_some_and(|ids| !ids.is_empty()))
    {
        return Err(AppError::MachineAuthorityUnsupported);
    }
    if !selection
        .capabilities
        .subset_of(Capabilities::legacy(&profile))
    {
        return Err(AppError::MachineCapabilityDisabled);
    }
    if let Some(ids) = &selection.saved_login_ids
        && ((agent.user_id != actor && !ids.is_empty())
            || (!agent.is_nyxbot() && ids.iter().any(|id| !agent.saved_login_ids.contains(id))))
    {
        return Err(refused());
    }
    let mut session = db.client().start_session().await?;
    let db = db.clone();
    let agent_id = agent_id.to_owned();
    let node_id = node_id.to_owned();
    let actor = actor.to_owned();
    session.start_transaction().and_run2(async move |session| {
        let result=Box::pin(async {
            let mut agent=db.collection::<AssistantAgent>(AGENTS).find_one(doc!{"_id":&agent_id,"destroyed_at":bson::Bson::Null})
                .session(&mut *session).await?.ok_or_else(refused)?;
            Box::pin(super::org_agent_service::require_maintain(&db, &actor, &agent)).await?;
            if !super::org_service::resolve_owner_access(&db,&actor,&node.user_id).await?.can_write() {return Err(refused());}
            if actor != agent.user_id {
                let fenced = db.collection::<Document>(crate::models::user::COLLECTION_NAME).update_one(doc! {"_id": &agent.user_id, "is_active": true},doc! {"$inc": {"agent_team_fence":1}}).session(&mut *session).await?;
                if fenced.matched_count != 1 {return Err(refused());}
            }
            Box::pin(migrate_in_session(&db,&mut agent,session)).await?;
            let policy=agent.machine_access.as_mut().expect("migrated");
            if policy.version != 2 { return Err(AppError::MachineAuthorityUnsupported); }
            if policy.revision!=selection.expected_revision {return Err(AppError::Conflict("Machine access changed; reload before saving".into()));}
            if !policy.assignments.contains_key(&node_id) && policy.assignments.len()>=64 {return Err(AppError::ValidationError("At most 64 new machine assignments".into()));}
            let old = policy.assignments.get(&node_id).cloned().unwrap_or_else(|| selected_assignment(&profile, policy.revision, editing, contexts));
            let mode = if let Some(requested) = selection.mode.as_deref() {
                let choice = match requested {
                    "shared_legacy" => Choice::SharedLegacy,
                    "automatic" => Choice::Automatic,
                    _ => Choice::RequireSeparated,
                };
                match select_profile(&profile, Some(choice)).map_err(|reason| AppError::ValidationError(selection_message(reason)))? {
                    Mode::Separated => "separated",
                    Mode::SharedLegacy => "shared_legacy",
                }
            } else { &old.mode };
            if mode == model::SELECTION_REQUIRED {
                return Err(AppError::ValidationError(selection_message("machine_selection_required")));
            }
            if mode == "separated" && selection.saved_login_ids.is_none() {
                return Err(AppError::ValidationError("Separate workspace and browser requires an explicit saved-login selection (empty is allowed)".into()));
            }
            let reset = mode != old.mode || old.saved_login_ids != selection.saved_login_ids;
            if reset {
                db.collection::<Document>(model::CONTEXTS).update_many(doc!{"agent_id":&agent_id,"node_id":&node_id}, doc!{"$set":{"mode":mode,"login_ids":[]},"$inc":{"generation":1}}).session(&mut *session).await?;
            }
            policy.revision=policy.revision.checked_add(1).ok_or_else(refused)?;
            policy.assignments.insert(node_id.clone(),Assignment {capabilities:selection.capabilities,mode:mode.into(),revision:policy.revision,legacy:!profile.authority_v2(),saved_login_ids:selection.saved_login_ids.clone()});
            if !agent.machine_node_ids.contains(&node_id) {agent.machine_node_ids.push(node_id.clone());}
            db.collection::<Document>(AGENTS).update_one(doc!{"_id":&agent.id},doc!{"$set":{
                "machine_access":encode(&agent.machine_access)?,"machine_node_ids":&agent.machine_node_ids}}).session(&mut *session).await?;
            Box::pin(enqueue(&db,&agent.id,&node_id,agent.machine_access.as_ref().expect("policy").revision,session)).await?;
            if reset { mark_profile_reset(&db, &agent.id, &node_id, agent.machine_access.as_ref().expect("policy").revision, session).await?; }
            let rows:Vec<crate::models::assistant_conversation::AssistantConversation>=db.collection(crate::models::assistant_conversation::COLLECTION_NAME)
                .find(doc!{"agent_id":&agent.id}).session(&mut *session).await?.stream(&mut *session).try_collect().await?;
            let ids:Vec<&str>=rows.iter().map(|r|r.id.as_str()).collect();
            db.collection::<Document>(crate::models::assistant_acknowledgement::COLLECTION_NAME)
                .update_many(doc!{"conversation_id":{"$in":ids},"status":{"$in":["pending","allowed"]},"$or":[{"kind":"machine","service_id":&node_id},{"tool_name":{"$regex":"^(nyx__machine_|nyxid__machine_)"}}]},doc!{"$set":{"status":"expired"}}).session(&mut *session).await?;
            Box::pin(super::assistant_team_service::sync_thread_authority(&db,&agent,&rows,session)).await?;
            Ok(*agent.machine_access.expect("policy"))
        }).await; transactions::transaction_result(result)
    }).await.map_err(transactions::map_transaction_error)
}

/// Serialize grant writes and admission by writing the same agent row. Every
/// v2 operation has a distinct renewable lease; no request-local ACL is cached.
pub fn admit<'a>(
    db: &'a Database,
    chat: &'a ChatAuthority,
    node: &'a Node,
    operation: Operation,
    args: &'a serde_json::Value,
    job_id: Option<&'a str>,
) -> futures::future::BoxFuture<'a, AppResult<Option<Box<Authority>>>> {
    Box::pin(async move {
        let access = Box::pin(authorize(db, chat, node, operation, args)).await?;
        let login_binding = (operation == Operation::FillLogin && access.mode == "separated")
            .then(|| args["login"].as_str().map(str::to_owned))
            .flatten();
        let profile = node
            .machine
            .as_ref()
            .ok_or(AppError::MachineCapabilityDisabled)?;
        if !profile.authority_v2() {
            return Ok(None);
        }
        let agent = super::org_agent_service::chat_agent(db, chat).await?;
        let conversation =
            super::assistant_nyxagent::get(db, &chat.user_id, &chat.conversation_id).await?;
        let context_id = uuid::Uuid::new_v4().to_string();
        let authority = Box::new(Authority {
            require_v2: !access.legacy,
            context_id: context_id.clone(),
            generation: 1,
            mode: access.mode.clone(),
            agent_id: agent.id.clone(),
            owner_id: agent.user_id.clone(),
            actor_id: chat.user_id.clone(),
            group_id: conversation.group_id,
            runtime_id: profile.runtime_id.clone(),
            conversation_id: chat.conversation_id.clone(),
            turn_id: chat
                .turn_id
                .clone()
                .filter(|_| !chat.turn_stopped)
                .ok_or(AppError::MachineTurnStopped)?,
            lease_id: uuid::Uuid::new_v4().to_string(),
            revision: access.revision,
            expires_at_ms: Utc::now().timestamp_millis() + nyxid_machine::authority::LEASE_MS,
            capabilities: access.capabilities,
        });
        let lease = model::Lease {
            id: authority.lease_id.clone(),
            node_id: node.id.clone(),
            api_key_id: chat.api_key_id.clone(),
            authority: authority.clone(),
            job_id: job_id.map(str::to_owned),
            expires_at: Utc::now() + chrono::Duration::hours(25),
        };
        let context = model::Context {
            id: context_id,
            node_id: node.id.clone(),
            agent_id: agent.id,
            owner_id: agent.user_id,
            actor_id: chat.user_id.clone(),
            group_id: authority.group_id.clone(),
            generation: 1,
            mode: access.mode.clone(),
        };
        let db = db.clone();
        let node = node.clone();
        let mut session = db.client().start_session().await?;
        let authority = session.start_transaction().and_run2(async move |session| {
        let result=Box::pin(async {
            let mut lease = lease.clone();
            let runtime = super::remote_machine_runtime::fence_execution_in_session(&db, &node, session).await?;
            if let Some(cutoff) = runtime.funding_cutoff {
                lease.authority.expires_at_ms = lease.authority.expires_at_ms.min(cutoff.timestamp_millis());
            }
            if lease.authority.expires_at_ms <= Utc::now().timestamp_millis() { return Err(refused()); }
            let capacity=db.collection::<Document>(NODES).update_one(doc!{"_id":&lease.node_id,"is_active":true,"machine.runtime_id":&lease.authority.runtime_id},doc!{"$inc":{"machine_authority_admission_fence":1}}).session(&mut *session).await?;
            if capacity.matched_count!=1 {return Err(AppError::MachineAuthorityStale);}
            // v1 returns before this transaction. Idle sockets create no leases;
            // expired leases and other nodes never consume this node's quota.
            if operation != Operation::JobCancel && db.collection::<Document>(model::LEASES)
                .count_documents(doc!{"node_id":&lease.node_id,"authority.expires_at_ms":{"$gt":Utc::now().timestamp_millis()}})
                .limit(MAX_NODE_LEASES).session(&mut *session).await? >= MAX_NODE_LEASES {return Err(AppError::MachineAuthorityBusy);}
            let revision_field = format!("machine_access.assignments.{}.revision",lease.node_id);
            let mut fence = doc! {"_id":&lease.authority.agent_id,"destroyed_at":bson::Bson::Null};
            if operation == Operation::JobCancel {
                fence.insert("$or", vec![doc! {&revision_field:lease.authority.revision},doc! {&revision_field:{"$exists":false},"machine_access.revision":lease.authority.revision}]);
            } else { fence.insert(revision_field,lease.authority.revision); }
            let fenced=db.collection::<Document>(AGENTS).update_one(fence,
                doc!{"$inc":{"machine_admission_fence":1}}).session(&mut *session).await?;
            if fenced.matched_count!=1 {return Err(refused());}
            let turn=db.collection::<Document>("assistant_conversations").update_one(doc!{"_id":&lease.authority.conversation_id,"active_turn.turn_id":&lease.authority.turn_id,"active_turn.stop_requested":false},
                doc!{"$addToSet":{"active_turn.machine_node_ids":&lease.node_id}}).session(&mut *session).await?;
            if turn.matched_count!=1 {return Err(AppError::MachineTurnStopped);}
            let persisted = db.collection::<model::Context>(model::CONTEXTS).find_one_and_update(
                doc!{"node_id":&context.node_id,"owner_id":&context.owner_id,"agent_id":&context.agent_id,"actor_id":&context.actor_id,"group_id":&context.group_id},
                doc!{"$setOnInsert":bson::to_document(&context).map_err(|_|refused())?})
                .upsert(true).return_document(mongodb::options::ReturnDocument::After)
                .session(&mut *session).await?.ok_or_else(refused)?;
            if persisted.mode != lease.authority.mode { return Err(AppError::MachineAuthorityStale); }
            if let Some(login) = &login_binding {
                let fenced = db.collection::<Document>(crate::models::saved_login::COLLECTION_NAME).update_one(doc!{"_id":login},doc!{"$inc":{"machine_fill_fence":1}}).session(&mut *session).await?;
                if fenced.matched_count != 1 { return Err(refused()); }
                db.collection::<Document>(model::CONTEXTS).update_one(doc!{"_id":&persisted.id},doc!{"$addToSet":{"login_ids":login}}).session(&mut *session).await?;
            }
            lease.authority.context_id = persisted.id;
            lease.authority.generation = if persisted.mode == "separated" { u64::try_from(persisted.generation).ok().filter(|g| *g > 0).ok_or_else(refused)? } else { 1 };
            db.collection::<model::Lease>(model::LEASES).insert_one(&lease).session(session).await?;
            Ok(lease.authority)
        }).await; transactions::transaction_result(result)
    }).await.map_err(transactions::map_transaction_error)?;
        Ok(Some(authority))
    })
}

pub(super) async fn fence_renewal_in_session(
    db: &Database,
    lease: &model::Lease,
    session: &mut ClientSession,
) -> AppResult<()> {
    // Serialize the deadline reservation with grant mutation. A renewal that
    // read the old policy may not issue a deadline after its revocation commits.
    let agent_fence = db.collection::<Document>(AGENTS).update_one(
        doc! {"_id":&lease.authority.agent_id,"destroyed_at":bson::Bson::Null,
            format!("machine_access.assignments.{}.revision", lease.node_id):lease.authority.revision},
        doc! {"$inc":{"machine_admission_fence":1_i64}},
    ).session(&mut *session).await?;
    let node_fence = db.collection::<Document>(NODES).update_one(
        doc! {"_id":&lease.node_id,"is_active":true,"machine.runtime_id":&lease.authority.runtime_id},
        doc! {"$inc":{"machine_authority_admission_fence":1_i64}},
    ).session(&mut *session).await?;
    if agent_fence.matched_count != 1 || node_fence.matched_count != 1 {
        return Err(refused());
    }
    Ok(())
}

/// Keep renewal independent of the requesting HTTP connection for background
/// jobs. Authentication is rebuilt from MongoDB on every renewal.
pub(super) async fn renew_one(state: &crate::AppState, mut lease: model::Lease) -> AppResult<()> {
    let chat = super::assistant_acknowledgement_service::for_key(
        &state.db,
        &lease.authority.actor_id,
        Some(&lease.api_key_id),
    )
    .await?
    .ok_or_else(refused)?;
    if chat.agent_id != lease.authority.agent_id || chat.guest {
        return Err(refused());
    }
    let node = super::node_service::get_node_by_id(&state.db, &lease.node_id)
        .await?
        .filter(|n| n.is_active)
        .ok_or_else(refused)?;
    let access = Box::pin(assignment(&state.db, &chat, &node)).await?;
    if access.revision != lease.authority.revision
        || access.mode != lease.authority.mode
        || access.capabilities != lease.authority.capabilities
        || node
            .machine
            .as_ref()
            .is_none_or(|p| p.runtime_id != lease.authority.runtime_id)
    {
        return Err(refused());
    }
    if let Some(job) = &lease.job_id {
        let job = super::machine_service::job(&state.db, &chat, &node.id, job).await?;
        if job.state != "running" || job.expires_at <= Utc::now() {
            return Err(refused());
        }
        Box::pin(super::machine_gateway_service::job_auth(&state.db, &job)).await?;
    } else if chat.turn_stopped || chat.turn_id.as_ref() != Some(&lease.authority.turn_id) {
        return Err(refused());
    }
    lease.authority.expires_at_ms =
        Utc::now().timestamp_millis() + nyxid_machine::authority::LEASE_MS;
    // Retry only the database reservation, under the sweep's existing 2s
    // timeout. Same-node/agent renewals deliberately contend on these fences.
    // Keep the proposed expiry fixed across retries; never resend control as
    // part of a transaction retry or extend an expired/revised/runtime-old lease.
    let db = state.db.clone();
    let node_tx = node.clone();
    let reservation = lease.clone();
    let mut session = state.db.client().start_session().await?;
    lease.authority = session
        .start_transaction()
        .write_concern(mongodb::options::WriteConcern::majority())
        .and_run2(async move |session| {
            let result = Box::pin(async {
                let mut lease = reservation.clone();
                #[cfg(test)]
                super::machine_access_tests::renewal_snapshot_loaded(&db, &lease, session).await?;
                // Recheck managed lifecycle/Billing and both live fences in
                // every attempt, including after a transient snapshot conflict.
                let runtime = super::remote_machine_runtime::fence_execution_in_session(
                    &db, &node_tx, session,
                )
                .await?;
                if let Some(cutoff) = runtime.funding_cutoff {
                    lease.authority.expires_at_ms =
                        lease.authority.expires_at_ms.min(cutoff.timestamp_millis());
                }
                if lease.authority.expires_at_ms <= Utc::now().timestamp_millis() {
                    return Err(refused());
                }
                fence_renewal_in_session(&db, &lease, session).await?;
                let fenced = db.collection::<Document>(model::LEASES).update_one(
                    doc! {"_id":&lease.id,"authority.expires_at_ms":{"$gt":Utc::now().timestamp_millis()}},
                    doc! {"$inc":{"renewal_fence":1_i64}},
                ).session(session).await?;
                if fenced.matched_count != 1 {
                    return Err(refused());
                }
                Ok(lease.authority)
            })
            .await;
            transactions::transaction_result(result)
        })
        .await
        .map_err(transactions::map_transaction_error)?;
    let expires = lease.authority.expires_at_ms;
    let result = send_control(state, &node, Operation::AuthorityRenew, lease.authority).await?;
    if result["accepted"] != true {
        // Renewal may overtake initial dispatch. Retry while the last signed
        // deadline is live; a node never resurrects an expired/stopped lease.
        return Err(AppError::MachineAuthorityStale);
    }
    state
        .db
        .collection::<Document>(model::LEASES)
        .update_one(
            doc! {"_id":&lease.id,"authority.expires_at_ms":{"$lt":expires}},
            doc! {"$set":{"authority.expires_at_ms":expires}},
        )
        .await?;
    Ok(())
}

async fn send_control(
    state: &crate::AppState,
    node: &Node,
    operation: Operation,
    authority: Box<Authority>,
) -> AppResult<serde_json::Value> {
    dispatch_control(
        state,
        node,
        operation,
        serde_json::json!({}),
        Some(authority),
    )
    .await
}

async fn dispatch_control(
    state: &crate::AppState,
    node: &Node,
    operation: Operation,
    parameters: serde_json::Value,
    authority: Option<Box<Authority>>,
) -> AppResult<serde_json::Value> {
    super::machine_service::capable(node, operation)?;
    let secret =
        super::node_service::signing_secret_from_node(&state.encryption_keys, node).await?;
    let mut request = nyxid_machine::Request {
        version: if authority.is_some() { 2 } else { 1 },
        authority,
        external_authority: None,
        request_id: uuid::Uuid::new_v4().to_string(),
        node_id: node.id.clone(),
        operation,
        parameters,
        timestamp: Utc::now().timestamp(),
        nonce: uuid::Uuid::new_v4().to_string(),
        signature: String::new(),
    };
    request.signature = nyxid_machine::signing::sign(&request, &secret);
    Ok(state
        .node_dispatch
        .machine_request_with_node(request, node)
        .await?
        .result)
}

/// Start one conservative grace only after the committed row is observable.
/// Enqueue/created_at can precede a slow transaction commit, during which older
/// renewal writers can still read the old grant. Legacy rows also lack a
/// trustworthy creation time. Include the older worker's 2s in-flight timeout;
/// never infer an ACK or delete delivery/quarantine work on expiry.
async fn initialize_revocation_deadlines(db: &Database, mut filter: Document) -> AppResult<()> {
    filter.insert("pending", true);
    filter.insert("execution_expires_at", doc! {"$exists":false});
    // Observe committed identities before asking MongoDB to start the grace. An
    // update_many query could otherwise include rows committed while queued.
    let observed: Vec<Document> = db
        .collection::<Document>(model::OUTBOX)
        .find(filter)
        .projection(doc! {"_id":1})
        .limit(128)
        .await?
        .try_collect()
        .await?;
    let ids = observed
        .iter()
        .filter_map(|row| row.get_str("_id").ok().map(str::to_owned))
        .collect::<Vec<_>>();
    initialize_observed_revocations(db, &ids).await
}

pub(super) async fn initialize_observed_revocations(
    db: &Database,
    ids: &[String],
) -> AppResult<()> {
    if ids.is_empty() {
        return Ok(());
    }
    // Initialization and status comparison share MongoDB's clock. Different
    // application replicas must not shorten the grace through opposing skew.
    db.collection::<Document>(model::OUTBOX).update_many(
        doc! {"_id":{"$in":ids},"pending":true,"execution_expires_at":{"$exists":false}},
        vec![doc! {"$set":{"execution_expires_at":{"$add":["$$NOW",nyxid_machine::authority::LEASE_MS+2_000]},"expiry_observed_at":"$$NOW"}}]).await?;
    Ok(())
}

/// Aggregate before bounding results: hundreds of changes on one machine must
/// never hide a pending revocation on another visible machine.
pub async fn revocation_status(
    db: &Database,
    agent: &str,
    nodes: &[String],
) -> AppResult<std::collections::BTreeMap<String, Document>> {
    if nodes.is_empty() {
        return Ok(Default::default());
    }
    let filter = doc! {"agent_id":agent,"node_id":{"$in":nodes},"pending":true};
    initialize_revocation_deadlines(db, filter.clone()).await?;
    let rows: Vec<Document> = db.collection::<Document>(model::OUTBOX).aggregate(vec![
        doc! {"$match":filter},
        doc! {"$group":{"_id":"$node_id","execution_pending":{"$max":{"$cond":[{"$eq":[{"$type":"$execution_expires_at"},"date"]},{"$gt":["$execution_expires_at","$$NOW"]},true]}},"initialization_pending":{"$max":{"$eq":[{"$type":"$execution_expires_at"},"missing"]}},"invalid_deadline":{"$max":{"$and":[{"$ne":[{"$type":"$execution_expires_at"},"missing"]},{"$ne":[{"$type":"$execution_expires_at"},"date"]}]}},"quarantine_pending":{"$max":{"$eq":["$quarantine_profiles",true]}},"expires_at":{"$max":"$execution_expires_at"}}},
    ]).await?.try_collect().await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| Some((row.get_str("_id").ok()?.to_owned(), row)))
        .collect())
}

pub(super) async fn revoke_one(state: &crate::AppState, row: Document) -> AppResult<()> {
    if row
        .get_str("kind")
        .is_ok_and(|kind| matches!(kind, "external" | "external_lease" | "owner_job"))
    {
        return super::machine_external_service::revoke_outbox(state, row).await;
    }
    let id = row.get_str("_id").map_err(|_| refused())?;
    initialize_revocation_deadlines(&state.db, doc! {"_id":id}).await?;
    let claim = uuid::Uuid::new_v4().to_string();
    let claimed=state.db.collection::<Document>(model::OUTBOX).find_one_and_update(doc!{"_id":id,"pending":true,"claim_until":{"$lte":bson::DateTime::now()}},
        doc!{"$set":{"claim":&claim,"claim_until":bson::DateTime::from_chrono(Utc::now()+chrono::Duration::seconds(10))}})
        .return_document(mongodb::options::ReturnDocument::After).await?;
    let Some(row) = claimed else {
        return Ok(());
    };
    let node_id = row.get_str("node_id").map_err(|_| refused())?;
    let node = super::node_service::get_node_by_id(&state.db, node_id)
        .await?
        .ok_or_else(refused)?;
    let revision = row.get_i64("revision").map_err(|_| refused())?;
    let agent_id = row.get_str("agent_id").map_err(|_| refused())?;
    if node.machine.as_ref().is_some_and(|p| p.authority_v2()) {
        let contexts: Vec<model::Context> = state
            .db
            .collection(model::CONTEXTS)
            .find(doc! {"node_id":node_id,"agent_id":agent_id})
            .limit(1)
            .await?
            .try_collect()
            .await?;
        // Revisions are per-agent; one existing context fences every lease.
        let fallback = model::Context {
            id: uuid::Uuid::new_v4().to_string(),
            node_id: node_id.into(),
            agent_id: agent_id.into(),
            owner_id: node.user_id.clone(),
            actor_id: node.user_id.clone(),
            group_id: None,
            generation: 1,
            mode: "shared_legacy".into(),
        };
        {
            let context = contexts.first().unwrap_or(&fallback);
            let placeholder = uuid::Uuid::new_v4().to_string();
            let authority = Box::new(Authority {
                require_v2: true,
                context_id: context.id.clone(),
                generation: 1,
                mode: "shared_legacy".into(),
                agent_id: agent_id.into(),
                owner_id: context.owner_id.clone(),
                actor_id: context.actor_id.clone(),
                group_id: context.group_id.clone(),
                runtime_id: node.machine.as_ref().expect("profile").runtime_id.clone(),
                conversation_id: placeholder.clone(),
                turn_id: placeholder.clone(),
                lease_id: placeholder,
                revision,
                expires_at_ms: Utc::now().timestamp_millis() + nyxid_machine::authority::LEASE_MS,
                capabilities: Capabilities::default(),
            });
            if dispatch_control(state, &node, Operation::AuthorityRevoke, serde_json::json!({"quarantine_profiles":row.get_bool("quarantine_profiles").unwrap_or(false)}), Some(authority)).await?["accepted"]
                != true
            {
                return Err(refused());
            }
        }
    } else {
        // v1 has no capability/revision cancellation. Preserve its shared wire
        // protocol and stop affected conversations using its existing command.
        let mut filter = doc! {"agent_id":agent_id};
        if let Ok(after) = row.get_str("conversation_cursor") {
            filter.insert("_id", doc! {"$gt":after});
        }
        let rows: Vec<Document> = state
            .db
            .collection::<Document>("assistant_conversations")
            .find(filter)
            .sort(doc! {"_id":1})
            .projection(doc! {"_id":1})
            .limit(20)
            .await?
            .try_collect()
            .await?;
        let full_batch = rows.len() == 20;
        for row in rows {
            let conversation = row.get_str("_id").map_err(|_| refused())?;
            let result = dispatch_control(
                state,
                &node,
                Operation::Cancel,
                serde_json::json!({"conversation_id":conversation}),
                None,
            )
            .await?;
            if result.get("error").is_some() {
                return Err(refused());
            }
            // Persist each acknowledgement before yielding; an interrupted worker
            // resumes at the last confirmed conversation rather than the first page.
            let advanced = state.db.collection::<Document>(model::OUTBOX).update_one(
                doc! {"_id":id,"claim":&claim,"pending":true,"claim_until":{"$gt":bson::DateTime::now()}},
                doc! {"$set":{"conversation_cursor":conversation}},
            ).await?;
            if advanced.matched_count != 1 {
                return Err(AppError::MachineAuthorityStale);
            }
        }
        if full_batch {
            state.db.collection::<Document>(model::OUTBOX).update_one(
                doc! {"_id":id,"claim":&claim,"pending":true,"claim_until":{"$gt":bson::DateTime::now()}},
                doc! {"$set":{"claim_until":bson::DateTime::from_millis(0)}},
            ).await?;
            return Ok(());
        }
    }
    state
        .db
        .collection::<Document>(model::OUTBOX)
        .update_one(
            doc! {"_id":id,"claim":&claim,"pending":true,"claim_until":{"$gt":bson::DateTime::now()}},
            doc! {"$set":{"pending":false,"acknowledged_at":bson::DateTime::now()}},
        )
        .await?;
    Ok(())
}

async fn renew_leases(state: &crate::AppState) -> AppResult<()> {
    use super::coordination_service::{LeaseStore, cluster_lease_runtime};
    let runtime = cluster_lease_runtime();
    let Some(token) = runtime
        .acquire(&state.db, "machine-authority-renewal")
        .await?
    else {
        return Ok(());
    };
    let task = Box::pin(async {
        // Stream bounded batches through every node; there is no fleet-wide
        // first-page limit that could starve leases on later nodes.
        let leases = state
            .db
            .collection::<model::Lease>(model::LEASES)
            .find(doc! {"authority":{"$exists":true},"kind":{"$ne":"external"}})
            .batch_size(128)
            .await?;
        // Bounded concurrency avoids one offline node delaying every lease.
        leases
            .try_for_each_concurrent(64, |lease| async move {
                let id = lease.id.clone();
                let last_deadline = lease.authority.expires_at_ms;
                let expired = last_deadline <= Utc::now().timestamp_millis();
                let result = if expired {
                    None
                } else {
                    Some(
                        tokio::time::timeout(
                            std::time::Duration::from_secs(2),
                            Box::pin(renew_one(state, lease)),
                        )
                        .await,
                    )
                };
                if expired
                    || matches!(result, Some(Ok(Err(AppError::MachinePermissionRevoked))))
                    || (!matches!(result, Some(Ok(Ok(()))))
                        && Utc::now().timestamp_millis() >= last_deadline)
                {
                    let _ = state
                        .db
                        .collection::<Document>(model::LEASES)
                        .delete_one(doc! {"_id":id})
                        .await;
                }
                Ok::<_, mongodb::error::Error>(())
            })
            .await?;
        Ok::<_, AppError>(())
    });
    let result = runtime.run_while_renewed(&state.db, &token, task).await;
    LeaseStore::release(&state.db, &token).await?;
    result.unwrap_or(Ok(()))
}

async fn tick(state: &crate::AppState) -> AppResult<()> {
    use super::coordination_service::{LeaseStore, cluster_lease_runtime};
    let runtime = cluster_lease_runtime();
    let Some(token) = runtime
        .acquire(&state.db, "machine-authority-dispatch")
        .await?
    else {
        return Ok(());
    };
    let task = Box::pin(async {
        use futures::StreamExt;
        let rows: Vec<Document> = state
            .db
            .collection::<Document>(model::OUTBOX)
            .find(doc! {"pending":true,"claim_until":{"$lte":bson::DateTime::now()}})
            .limit(100)
            .await?
            .try_collect()
            .await?;
        futures::stream::iter(rows)
            .for_each_concurrent(32, |row| async move {
                let _ = tokio::time::timeout(
                    std::time::Duration::from_millis(500),
                    Box::pin(revoke_one(state, row)),
                )
                .await;
            })
            .await;
        Ok::<_, AppError>(())
    });
    let result = runtime.run_while_renewed(&state.db, &token, task).await;
    LeaseStore::release(&state.db, &token).await?;
    result.unwrap_or(Ok(()))
}
async fn migrate_pending(state: &crate::AppState) -> AppResult<()> {
    use super::coordination_service::{LeaseStore, cluster_lease_runtime};
    let runtime = cluster_lease_runtime();
    let Some(token) = runtime
        .acquire(&state.db, "machine-authority-migration")
        .await?
    else {
        return Ok(());
    };
    let result = runtime
        .run_while_renewed(
            &state.db,
            &token,
            Box::pin(async {
                let agents: Vec<Document> = state
                    .db
                    .collection::<Document>(AGENTS)
                    .find(doc! {"machine_access":bson::Bson::Null,"destroyed_at":bson::Bson::Null})
                    .projection(doc! {"_id":1})
                    .limit(25)
                    .await?
                    .try_collect()
                    .await?;
                for agent in agents {
                    let _ =
                        Box::pin(policy(&state.db, agent.get_str("_id").unwrap_or_default())).await;
                }
                Ok::<_, AppError>(())
            }),
        )
        .await;
    LeaseStore::release(&state.db, &token).await?;
    result.unwrap_or(Ok(()))
}

async fn sweep_external_sources(
    state: &crate::AppState,
    cursor: &mut Option<String>,
) -> AppResult<()> {
    use super::coordination_service::{LeaseStore, cluster_lease_runtime};
    let runtime = cluster_lease_runtime();
    let Some(token) = runtime
        .acquire(&state.db, "machine-external-source-cleanup")
        .await?
    else {
        return Ok(());
    };
    let after = cursor.clone();
    let result = runtime
        .run_while_renewed(
            &state.db,
            &token,
            Box::pin(async {
                let batch = super::machine_external_grant_service::sweep_invalid_sources(
                    &state.db,
                    after.as_deref(),
                    100,
                )
                .await?;
                super::machine_external_source_service::sweep_expired_children(&state.db, 100)
                    .await?;
                Ok::<_, AppError>(batch.next_after)
            }),
        )
        .await;
    LeaseStore::release(&state.db, &token).await?;
    if let Some(result) = result {
        *cursor = result?;
    }
    Ok(())
}
/// Change streams provide prompt delivery; the leased sweep remains authoritative
/// when streams disconnect or a replica restarts. Per-row claims fence duplicates.
async fn watch_revocations(state: crate::AppState) {
    loop {
        let result = Box::pin(async {
            let mut events = state
                .db
                .collection::<Document>(model::OUTBOX)
                .watch()
                .pipeline(vec![doc! {"$match": {"operationType": "insert"}}])
                .await?;
            while let Some(event) = events.try_next().await? {
                if let Some(row) = event.full_document {
                    let _ = tokio::time::timeout(
                        std::time::Duration::from_millis(500),
                        Box::pin(revoke_one(&state, row)),
                    )
                    .await;
                }
            }
            Ok::<_, mongodb::error::Error>(())
        })
        .await;
        // No payload or database error details are logged. The one-second sweep
        // does not depend on this stream or on its resume token.
        let _ = result;
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }
}

pub fn spawn(state: crate::AppState) {
    tokio::spawn(Box::pin(watch_revocations(state.clone())));
    let renewal_state = state.clone();
    tokio::spawn(async move {
        let mut timer = tokio::time::interval(std::time::Duration::from_secs(RENEW_INTERVAL_SECS));
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            timer.tick().await;
            if Box::pin(renew_leases(&renewal_state)).await.is_err() {
                tracing::warn!("Machine authority renewal deferred; offline grace remains bounded");
            }
            if Box::pin(super::machine_external_service::maintenance(&renewal_state))
                .await
                .is_err()
            {
                tracing::warn!("External machine maintenance deferred; authority remains bounded");
            }
        }
    });
    let migration_state = state.clone();
    tokio::spawn(async move {
        let mut timer = tokio::time::interval(std::time::Duration::from_secs(10));
        loop {
            timer.tick().await;
            let _ = Box::pin(migrate_pending(&migration_state)).await;
        }
    });
    let source_state = state.clone();
    tokio::spawn(async move {
        let mut cursor = None;
        let mut timer = tokio::time::interval(std::time::Duration::from_secs(10));
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            timer.tick().await;
            if Box::pin(sweep_external_sources(&source_state, &mut cursor))
                .await
                .is_err()
            {
                tracing::warn!(
                    "External Machine source cleanup deferred; live authority remains fenced"
                );
            }
        }
    });
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            if Box::pin(tick(&state)).await.is_err() {
                tracing::warn!("Machine authority reconciliation deferred; leases fail closed");
            }
        }
    });
}

/// Discovery retains stored assignments and derives current support only on the
/// returned report. One agent/policy lookup plus the existing batched node ACL
/// query; never one query per node.
pub async fn visible_assignments(
    db: &Database,
    chat: &ChatAuthority,
) -> AppResult<Vec<(Node, Assignment)>> {
    super::machine_service::caller(chat)?;
    let agent = super::org_agent_service::chat_agent(db, chat).await?;
    let policy = Box::pin(policy(db, &agent.id)).await?;
    let nodes = super::machine_service::visible_nodes(db, chat).await?;
    let policy = Box::pin(legacy_nyxbot_assignments(db, chat, &agent, &nodes, policy)).await?;
    if policy.version != 2 {
        return Err(refused());
    }
    Ok(nodes
        .into_iter()
        .filter_map(|mut node| {
            if !agent.is_nyxbot() && !agent.machine_node_ids.contains(&node.id) {
                return None;
            }
            let access = policy
                .assignments
                .get(&node.id)
                .cloned()
                .unwrap_or_else(|| unresolved_assignment(policy.revision));
            if access.mode == model::SELECTION_REQUIRED {
                return Some((node, access));
            }
            if access.mode == "separated" && !access.legacy {
                // Keep reviewed assignments discoverable during an upgrade or
                // failed observation. This projection never rewrites storage.
                let profile = node.machine.as_mut()?;
                if !supports_separated(profile) {
                    profile.separated = Some(separated_support(profile));
                }
                return Some((node, access));
            }
            let supported = access.mode == "shared_legacy";
            if !supported
                || (!access.legacy && !node.machine.as_ref().is_some_and(|p| p.authority_v2()))
            {
                return None;
            }
            Some((node, access))
        })
        .collect())
}
pub async fn definitions(
    db: &Database,
    chat: &ChatAuthority,
) -> AppResult<Vec<super::mcp_service::McpToolDefinition>> {
    let rows = Box::pin(visible_assignments(db, chat)).await?;
    Ok(super::machine_tools::definitions()
        .into_iter()
        .filter(|tool| {
            let Some(op) = super::machine_tools::operation(&tool.name) else {
                return true;
            };
            if op == Operation::JobCancel {
                return true;
            }
            rows.iter().any(|(node, access)| {
                node.machine.as_ref().is_some_and(|p| {
                    op.allowed(p)
                        && (access.mode == "shared_legacy"
                            || (access.mode == "separated" && supports_separated(p)))
                }) && access.capabilities.allows(op, &serde_json::json!({}))
            })
        })
        .collect())
}
