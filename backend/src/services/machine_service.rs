//! Live machine authorization and server-issued job authority.
use super::node_service;
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        assistant_conversation::{AssistantConversation, COLLECTION_NAME as CONVERSATIONS},
        machine_job::{COLLECTION_NAME as JOBS, MachineJob},
        node::{COLLECTION_NAME as NODES, Node, NodeStatus},
    },
    services::{assistant_acknowledgement_service::ChatAuthority, org_service},
};
use chrono::{Duration, Utc};
use futures::{StreamExt, TryStreamExt};
use mongodb::{
    Database,
    bson::{self, doc},
};
use nyxid_machine::authority::Authority;
use nyxid_machine::{Confirmation, Operation, Request};
use serde_json::{Value, json};

/// A setup/control notification and its watch settle atomically. The ordinary
/// assistant retry runner can wake a queued event after any replica crashes.
pub async fn settle_watch(
    db: &Database,
    watch: &crate::models::nyxbot_channel::NyxbotWatch,
    kind: &str,
    text: String,
    error: Option<&str>,
) -> AppResult<bool> {
    use super::api_key_mutation_service as transactions;
    let event = bson::to_bson(&super::nyxbot_watch_service::watch_event(watch, kind, text))
        .map_err(|_| AppError::Internal("Could not encode machine event".into()))?;
    let mut session = db.client().start_session().await?;
    let db = db.clone();
    let watch = watch.clone();
    let error = error.map(str::to_owned);
    session
        .start_transaction()
        .and_run2(async move |session| {
            let result: AppResult<bool> = async {
                let update = db
                    .collection::<bson::Document>(
                        crate::models::nyxbot_channel::WATCHES_COLLECTION_NAME,
                    )
                    .update_one(
                        doc! {"_id":&watch.id,"status":"pending"},
                        doc! {
                            "$set":{
                                "status":if error.is_some(){
                                    "failed"
                                }else{
                                    "done"
                                },
                                "last_error":&error
                            }
                        },
                    )
                    .session(&mut *session)
                    .await?;
                if update.modified_count == 0 {
                    return Ok(false);
                }
                db.collection::<bson::Document>(
                    crate::models::assistant_conversation::COLLECTION_NAME,
                )
                .update_one(
                    doc! {"_id":&watch.conversation_id,"user_id":&watch.user_id},
                    doc! {
                        "$push":{
                            "pending_events":{
                                "$each":[event.clone()],
                                "$slice":-(super::assistant_nyxagent::MAX_PENDING_EVENTS as i64)
                            }
                        }
                    },
                )
                .session(&mut *session)
                .await?;
                Ok(true)
            }
            .await;
            transactions::transaction_result(result)
        })
        .await
        .map_err(transactions::map_transaction_error)
}

pub fn caller(chat: &ChatAuthority) -> AppResult<()> {
    if chat.guest {
        return Err(AppError::MachineNotAllowed);
    }
    Ok(())
}

/// One membership snapshot and one node query; never a lookup per node.
pub async fn visible_nodes(db: &Database, chat: &ChatAuthority) -> AppResult<Vec<Node>> {
    caller(chat)?;
    let owners = if chat.is_orchestrator() {
        // NyxBot is always personal; retain its existing batched read budget.
        usable_owners(db, &chat.user_id).await?
    } else {
        let agent = super::org_agent_service::chat_agent(db, chat).await?;
        if agent.user_id != chat.user_id {
            vec![agent.user_id]
        } else {
            usable_owners(db, &chat.user_id).await?
        }
    };
    Ok(db
        .collection::<Node>(NODES)
        .find(doc! {
            "user_id":{
                "$in":owners
            },
            "is_active":true,
            "machine.version":nyxid_machine::PROTOCOL_VERSION as i64,
            "$or":[{
                "machine.shell":true
            },{
                "machine.files":true
            },{
                "machine.computer":true
            },{"machine.browser":true}]
        })
        .sort(doc! {"name":1,"_id":1})
        .limit(500)
        .await?
        .try_collect()
        .await?)
}

pub fn granted(chat: &ChatAuthority, node: &Node) -> bool {
    chat.is_orchestrator() || chat.machine_node_ids.contains(&node.id)
}

pub fn capable(node: &Node, operation: Operation) -> AppResult<()> {
    if node.status != NodeStatus::Online {
        return Err(AppError::NodeOffline(
            "Machine is offline; ask the owner to start its daemon".into(),
        ));
    }
    if !node
        .machine
        .as_ref()
        .is_some_and(|profile| operation.allowed(profile))
    {
        return Err(AppError::MachineCapabilityDisabled);
    }
    Ok(())
}

pub fn changing(operation: Operation, parameters: &Value) -> bool {
    match operation {
        Operation::ListFiles | Operation::ReadFile | Operation::ShareFile | Operation::Job => false,
        Operation::Browser => !matches!(
            parameters["action"].as_str(),
            Some("snapshot" | "tabs" | "wait" | "console" | "network" | "screenshot")
        ),
        Operation::Computer => {
            static TOOLS: std::sync::LazyLock<Vec<Value>> = std::sync::LazyLock::new(|| {
                serde_json::from_str(nyxid_machine::CUA_TOOLS).expect("embedded cua contract")
            });
            let tools = &*TOOLS;
            !tools
                .iter()
                .any(|t| t["name"] == parameters["tool"] && t["read_only"] == true)
        }
        _ => true,
    }
}

pub fn confirmation(node: &Node, operation: Operation, parameters: &Value) -> bool {
    node.machine_confirm == Confirmation::All
        || (node.machine_confirm == Confirmation::Changes && changing(operation, parameters))
}

pub fn metadata(node: &Node) -> Value {
    json!({
        "id":node.id,
        "name":node.name,
        "status":node.status,
        "machine":node.machine,
        "machine_confirm":node.machine_confirm,
        "agent_version":super::machine_update_service::current(node),
        "supported_version":super::machine_update_service::TARGET,
        "update_available":super::machine_update_service::update_available(node),
        "updater":super::machine_update_service::companion_status(node),
        "allow_single_user_saved_logins":node.allow_single_user_saved_logins
    })
}

#[cfg(test)]
pub fn issue_job<'a>(
    db: &'a Database,
    chat: &'a ChatAuthority,
    node: &'a Node,
    timeout: u64,
    services: Vec<crate::models::machine_job::DeclaredService>,
) -> futures::future::BoxFuture<'a, AppResult<MachineJob>> {
    issue_job_with_id(
        db,
        chat,
        node,
        timeout,
        services,
        uuid::Uuid::new_v4().to_string(),
    )
}

pub fn issue_job_with_id<'a>(
    db: &'a Database,
    chat: &'a ChatAuthority,
    node: &'a Node,
    timeout: u64,
    services: Vec<crate::models::machine_job::DeclaredService>,
    id: String,
) -> futures::future::BoxFuture<'a, AppResult<MachineJob>> {
    Box::pin(async move {
        caller(chat)?;
        let now = Utc::now();
        let runtime_id = node
            .machine
            .as_ref()
            .map(|profile| profile.runtime_id.clone())
            .filter(|id| uuid::Uuid::parse_str(id).is_ok())
            .ok_or_else(|| AppError::NodeOffline("Machine runtime has not connected".into()))?;
        let runtime_generation = if super::remote_machine_runtime::is_managed(node) {
            if timeout == 0 || timeout > 3600 {
                return Err(AppError::ValidationError(
                    "Managed command timeout must be 1 to 3600 seconds".into(),
                ));
            }
            let machine = super::remote_machine_service::managed_machine_for_node(db, &node.id)
                .await?
                .ok_or(AppError::MachineNotAllowed)?;
            Some(
                u64::try_from(machine.runtime_generation)
                    .map_err(|_| AppError::MachineAuthorityStale)?,
            )
        } else {
            None
        };
        let job = MachineJob {
            id,
            user_id: chat.user_id.clone(),
            node_id: node.id.clone(),
            runtime_id,
            binding: chat
                .turn_id
                .as_ref()
                .filter(|_| !chat.turn_stopped)
                .map(
                    |turn_id| crate::models::machine_job::MachineJobBinding::Assistant {
                        version: 1,
                        conversation_id: chat.conversation_id.clone(),
                        turn_id: turn_id.clone(),
                        api_key_id: chat.api_key_id.clone(),
                        agent_id: chat.agent_id.clone(),
                    },
                ),
            runtime_generation,
            conversation_id: chat.conversation_id.clone(),
            issuing_turn_id: chat.turn_id.clone(),
            api_key_id: chat.api_key_id.clone(),
            agent_id: chat.agent_id.clone(),
            state: "running".into(),
            exit_code: None,
            stop_reason: None,
            duration_ms: None,
            cancellation_requested_by: None,
            cancellation_requested_at: None,
            services,
            report_to: chat
                .report_to
                .clone()
                .filter(|_| chat.turn_live && !chat.turn_stopped),
            report_provenance: if chat.turn_live && !chat.turn_stopped {
                crate::models::assistant_conversation::REPORT_PROVENANCE_V1
            } else {
                0
            },
            created_at: now,
            expires_at: now + Duration::seconds(timeout.min(86400) as i64 + 15),
            finished_at: None,
            retain_until: None,
        };
        // Job publication and Stop contend on the exact active-turn write. A
        // revocation capturing jobs cannot miss a concurrent admitted publication.
        let mut session = db.client().start_session().await?;
        let db = db.clone();
        let chat = chat.clone();
        let node = node.clone();
        session
            .start_transaction()
            .and_run2(async move |session| {
                let result: AppResult<_> = async {
                    fence_turn_operation_in_session(&db, session, &chat, &node).await?;
                    db.collection::<MachineJob>(JOBS)
                        .insert_one(&job)
                        .session(&mut *session)
                        .await?;
                    Ok(job.clone())
                }
                .await;
                super::api_key_mutation_service::transaction_result(result)
            })
            .await
            .map_err(super::api_key_mutation_service::map_transaction_error)
    })
}

pub async fn job(
    db: &Database,
    chat: &ChatAuthority,
    node: &str,
    id: &str,
) -> AppResult<MachineJob> {
    caller(chat)?;
    db.collection::<MachineJob>(JOBS)
        .find_one(doc! {
            "_id":id,
            "node_id":node,
            "conversation_id":&chat.conversation_id,
            "api_key_id":&chat.api_key_id,
            "user_id":&chat.user_id
        })
        .await?
        .ok_or_else(|| AppError::MachineJobNotFound)
}
/// One indexed binding lookup; all identity is recovered from this server row.
pub async fn gateway_job(
    db: &Database,
    node: &str,
    runtime: &str,
    conversation: &str,
    id: &str,
) -> AppResult<MachineJob> {
    let job = db
        .collection::<MachineJob>(JOBS)
        .find_one(doc! {"_id":id,"node_id":node,"runtime_id":runtime,
        "state":"running","expires_at":{"$gt":bson::DateTime::now()}})
        .await?
        .ok_or_else(|| {
            AppError::Forbidden("Machine service call has no live server-issued job".into())
        })?;
    if !matches!(
        job.binding,
        Some(crate::models::machine_job::MachineJobBinding::External { .. })
    ) && (conversation.is_empty() || job.conversation_id != conversation)
    {
        return Err(AppError::MachinePermissionRevoked);
    }
    Ok(job)
}

pub async fn finish(db: &Database, id: &str) -> AppResult<()> {
    db.collection::<MachineJob>(JOBS)
        .update_one(
            doc! {"_id":id,"state":"running"},
            doc! {"$set":{"state":"finished","finished_at":bson::DateTime::now(),"retain_until":bson::DateTime::from_chrono(Utc::now()+Duration::days(30))}},
        )
        .await?;
    Ok(())
}

/// Resolve optional picker/tool grants without changing omitted fields.
pub async fn resolve_grant_change(
    db: &Database,
    owner: &str,
    machines: Option<Vec<String>>,
    logins: Option<Vec<String>>,
    base: super::assistant_team_service::GrantChange,
    mode: super::assistant_team_service::MachineGrantMode,
) -> AppResult<super::assistant_team_service::GrantChange> {
    if machines.is_none() && logins.is_none() {
        return Ok(base);
    }
    let owners = usable_owners(db, owner).await?;
    let resolve =
        |requested: Vec<String>, candidates: Vec<(String, String)>| -> AppResult<Vec<String>> {
            if requested.len() > 64 {
                return Err(AppError::ValidationError(
                    "At most 64 machine or login grants are allowed".into(),
                ));
            }
            let mut ids = Vec::new();
            for name in requested {
                // Revocation must work after deletion, disablement or loss of
                // membership. Removing a UUID never grants new authority.
                if matches!(
                    mode,
                    super::assistant_team_service::MachineGrantMode::Remove
                ) && uuid::Uuid::parse_str(&name).is_ok()
                {
                    if !ids.contains(&name) {
                        ids.push(name);
                    }
                    continue;
                }
                let mut found = candidates
                    .iter()
                    .filter(|(id, label)| *id == name || *label == name);
                let id = found
                    .next()
                    .ok_or_else(|| {
                        AppError::NotFound("Machine or saved login not found or not usable".into())
                    })?
                    .0
                    .clone();
                if found.next().is_some() {
                    return Err(AppError::ValidationError(
                        "Ambiguous name; use the ID".into(),
                    ));
                }
                if !ids.contains(&id) {
                    ids.push(id);
                }
            }
            Ok(ids)
        };
    let machines = if let Some(names) = machines {
        let nodes: Vec<Node> = db
            .collection::<Node>(NODES)
            .find(doc! {
                "user_id":{
                    "$in":&owners
                },
                "is_active":true,
                "machine.version":nyxid_machine::PROTOCOL_VERSION as i64
            })
            .await?
            .try_collect()
            .await?;
        Some(resolve(
            names,
            nodes
                .into_iter()
                .filter(|n| n.machine.as_ref().is_some_and(|p| p.enabled()))
                .map(|n| (n.id, n.name))
                .collect(),
        )?)
    } else {
        None
    };
    let logins = if let Some(names) = logins {
        let rows: Vec<crate::models::saved_login::SavedLogin> = db
            .collection(crate::models::saved_login::COLLECTION_NAME)
            .find(doc! {"user_id":{"$in":&owners}})
            .await?
            .try_collect()
            .await?;
        Some(resolve(
            names,
            rows.into_iter().map(|r| (r.id, r.label)).collect(),
        )?)
    } else {
        None
    };
    Ok(super::assistant_team_service::GrantChange::Machine {
        base: Box::new(base),
        machines,
        logins,
        mode,
    })
}

/// Equivalent write-owner membership gate, resolved in batches for machine pickers.
pub async fn usable_owners(db: &Database, actor: &str) -> AppResult<Vec<String>> {
    let memberships = org_service::list_memberships_for_member(db, actor, false).await?;
    let orgs: Vec<_> = memberships
        .into_iter()
        .filter(|m| m.role.can_admin())
        .map(|m| m.org_user_id)
        .collect();
    let mut owners = vec![actor.to_owned()];
    if !orgs.is_empty() {
        let rows: Vec<bson::Document> = db
            .collection::<bson::Document>(crate::models::user::COLLECTION_NAME)
            .find(doc! {"_id":{"$in":orgs},"user_type":"org","is_active":true})
            .projection(doc! {"_id":1})
            .await?
            .try_collect()
            .await?;
        owners.extend(
            rows.iter()
                .filter_map(|row| row.get_str("_id").ok().map(str::to_owned)),
        );
    }
    Ok(owners)
}

pub async fn dispatch(
    state: &AppState,
    node: &Node,
    operation: Operation,
    parameters: Value,
) -> AppResult<Value> {
    dispatch_authorized(state, node, operation, parameters, None).await
}
pub(crate) async fn dispatch_authorized(
    state: &AppState,
    node: &Node,
    operation: Operation,
    parameters: Value,
    authority: Option<Box<Authority>>,
) -> AppResult<Value> {
    let request = signed_request(state, node, operation, parameters, authority).await?;
    Ok(state
        .node_dispatch
        .machine_request_with_node(request, node)
        .await?
        .result)
}

pub(crate) async fn signed_request(
    state: &AppState,
    node: &Node,
    operation: Operation,
    parameters: Value,
    authority: Option<Box<Authority>>,
) -> AppResult<Request> {
    capable(node, operation)?;
    let secret = node_service::signing_secret_from_node(&state.encryption_keys, node).await?;
    let mut request = Request {
        external_authority: None,
        version: if authority.is_some() { 2 } else { 1 },
        authority,
        request_id: uuid::Uuid::new_v4().to_string(),
        node_id: node.id.clone(),
        operation,
        parameters,
        timestamp: Utc::now().timestamp(),
        nonce: uuid::Uuid::new_v4().to_string(),
        signature: String::new(),
    };
    request.signature = nyxid_machine::signing::sign(&request, &secret);
    Ok(request)
}

/// Persist the issue-time runtime beside the exact turn before dispatch. Stop
/// writes this same conversation, so capture cannot miss an admitted operation.
pub(crate) async fn admit_turn_operation(
    db: &Database,
    chat: &ChatAuthority,
    node: &Node,
) -> AppResult<()> {
    let mut session = db.client().start_session().await?;
    let db = db.clone();
    let chat = chat.clone();
    let node = node.clone();
    session
        .start_transaction()
        .and_run2(async move |session| {
            let result = fence_turn_operation_in_session(&db, session, &chat, &node).await;
            super::api_key_mutation_service::transaction_result(result)
        })
        .await
        .map_err(super::api_key_mutation_service::map_transaction_error)
}

async fn fence_turn_operation_in_session(
    db: &Database,
    session: &mut mongodb::ClientSession,
    chat: &ChatAuthority,
    node: &Node,
) -> AppResult<()> {
    use crate::models::assistant_conversation::{
        AssistantConversation, COLLECTION_NAME, MachineTurnScope,
    };
    let turn_id = chat
        .turn_id
        .as_deref()
        .ok_or(AppError::AssistantTurnRequired)?;
    let scope = node
        .machine
        .as_ref()
        .filter(|profile| uuid::Uuid::parse_str(&profile.runtime_id).is_ok())
        .map(|profile| MachineTurnScope {
            node_id: node.id.clone(),
            runtime_id: profile.runtime_id.clone(),
        });
    if chat.platform_thread && scope.is_none() {
        return Err(AppError::NodeOffline(
            "Machine runtime has not connected".into(),
        ));
    }
    if let Some(scope) = &scope {
        let node_fence = db
            .collection::<bson::Document>(NODES)
            .update_one(
                doc! {"_id":&node.id,"machine.runtime_id":&scope.runtime_id,"status":"online"},
                doc! {"$inc":{"machine_issue_generation":1_i64}},
            )
            .session(&mut *session)
            .await?;
        if node_fence.matched_count != 1 {
            return Err(AppError::MachineTurnStopped);
        }
    }
    let mut add = doc! {"active_turn.machine_node_ids":&node.id};
    if let Some(scope) = &scope {
        add.insert(
            "active_turn.machine_scopes",
            bson::to_bson(scope).map_err(|_| AppError::MachineTurnStopped)?,
        );
    }
    let row=db.collection::<AssistantConversation>(COLLECTION_NAME).find_one_and_update(
        doc! {"_id":&chat.conversation_id,"user_id":&chat.user_id,"credential_api_key_id":&chat.api_key_id,
            "agent_id":&chat.agent_id,"active_turn.turn_id":turn_id,"active_turn.stop_requested":false},
        doc! {"$addToSet":add,"$inc":{"machine_issue_generation":1_i64}},
    ).session(&mut *session).await?.ok_or(AppError::MachineTurnStopped)?;
    if super::assistant_nyxagent::live_turn(&row, Utc::now()).is_none() {
        return Err(AppError::MachineTurnStopped);
    }
    if let Some(origin) = row
        .active_turn
        .as_ref()
        .and_then(|turn| turn.platform_origin.as_ref())
    {
        let origin = super::platform_personal_turn_service::contract_origin(origin)?;
        let turn = super::platform_manager_contract::TurnRef {
            conversation_id: row.id.clone(),
            turn_id: turn_id.to_owned(),
        };
        let outcome = super::platform_personal_turn_service::fence_execution_in_session(
            db,
            session,
            &super::platform_personal_binding_service::IdentityProducer,
            &chat.user_id,
            &chat.api_key_id,
            &turn,
            &origin,
        )
        .await?;
        if !matches!(
            outcome,
            super::platform_manager_contract::FenceOutcome::Authorized
        ) {
            return Err(AppError::MachineTurnStopped);
        }
    }
    Ok(())
}

pub(crate) async fn cancel_conversation(state: &AppState, user: &str, id: &str) -> AppResult<()> {
    let row = state
        .db
        .collection::<AssistantConversation>(CONVERSATIONS)
        .find_one_and_update(
            doc! {"_id":id,"user_id":user,"active_turn":{"$ne":null}},
            doc! {"$set":{"active_turn.stop_requested":true}},
        )
        .return_document(mongodb::options::ReturnDocument::After)
        .await?;
    let mut nodes: std::collections::HashSet<String> = row
        .as_ref()
        .and_then(|r| r.active_turn.as_ref())
        .map(|t| t.machine_node_ids.iter().cloned().collect())
        .unwrap_or_default();
    let mut jobs = state
        .db
        .collection::<MachineJob>(JOBS)
        .find(doc! {"conversation_id":id,"user_id":user,"state":"running"})
        .await?;
    while let Some(job) = jobs.next().await {
        nodes.insert(job?.node_id);
    }
    state
        .db
        .collection::<MachineJob>(JOBS)
        .update_many(
            doc! {"conversation_id":id,"user_id":user,"state":"running"},
            doc! {"$set":{"state":"finished","finished_at":bson::DateTime::now()}},
        )
        .await?;
    // Cancelling every job in this conversation also covers background jobs
    // started by an earlier turn. New turns have new node admission scopes.
    let args = json!({"conversation_id":id,"turn_id":row.as_ref().and_then(|r|r.active_turn.as_ref()).map(|t|t.turn_id.as_str()).unwrap_or_default()});
    futures::stream::iter(nodes)
        .for_each_concurrent(16, |node| {
            let args = args.clone();
            async move {
                send_cancel(state, &node, args).await;
            }
        })
        .await;
    Ok(())
}

pub(crate) async fn send_cancel(state: &AppState, id: &str, args: serde_json::Value) {
    let work = async {
        if let Some(node) = state
            .db
            .collection::<Node>(NODES)
            .find_one(doc! {"_id":id})
            .await?
        {
            dispatch(state, &node, nyxid_machine::Operation::Cancel, args).await?;
        }
        AppResult::Ok(())
    };
    if !tokio::time::timeout(std::time::Duration::from_millis(800), work)
        .await
        .is_ok_and(|r| r.is_ok())
    {
        tracing::warn!(
            node_id = id,
            "Machine stop delivery pending or node offline"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn changes_confirm_all_commands_and_only_mutating_computer_tools() {
        for op in [
            Operation::Exec,
            Operation::JobCancel,
            Operation::WriteFile,
            Operation::EditFile,
            Operation::SaveAttachment,
            Operation::FillLogin,
        ] {
            assert!(changing(op, &json!({})));
        }
        for op in [
            Operation::Job,
            Operation::ReadFile,
            Operation::ListFiles,
            Operation::ShareFile,
        ] {
            assert!(!changing(op, &json!({})));
        }
        assert!(!changing(
            Operation::Computer,
            &json!({"tool":"get_window_state"})
        ));
        assert!(changing(Operation::Computer, &json!({"tool":"click"})));
        assert!(changing(Operation::Computer, &json!({"tool":"unknown"})));
    }
}
