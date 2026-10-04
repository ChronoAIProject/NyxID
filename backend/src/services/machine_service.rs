//! Live machine authorization and server-issued job authority.
use crate::{
    errors::{AppError, AppResult},
    models::{
        machine_job::{COLLECTION_NAME as JOBS, MachineJob},
        node::{COLLECTION_NAME as NODES, Node, NodeStatus},
    },
    services::{assistant_acknowledgement_service::ChatAuthority, org_service},
};
use chrono::{Duration, Utc};
use futures::TryStreamExt;
use mongodb::{
    Database,
    bson::{self, doc},
};
use nyxid_machine::{Confirmation, Operation};
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
    let event = bson::to_bson(&super::assistant_team_service::event(kind, text, None))
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

pub async fn issue_job(
    db: &Database,
    chat: &ChatAuthority,
    node: &Node,
    timeout: u64,
    services: Vec<crate::models::machine_job::DeclaredService>,
) -> AppResult<MachineJob> {
    caller(chat)?;
    let now = Utc::now();
    let runtime_id = node
        .machine
        .as_ref()
        .map(|profile| profile.runtime_id.clone())
        .filter(|id| uuid::Uuid::parse_str(id).is_ok())
        .ok_or_else(|| AppError::NodeOffline("Machine runtime has not connected".into()))?;
    let job = MachineJob {
        id: uuid::Uuid::new_v4().to_string(),
        user_id: chat.user_id.clone(),
        node_id: node.id.clone(),
        runtime_id,
        conversation_id: chat.conversation_id.clone(),
        api_key_id: chat.api_key_id.clone(),
        agent_id: chat.agent_id.clone(),
        state: "running".into(),
        services,
        created_at: now,
        expires_at: now + Duration::seconds(timeout.min(86400) as i64 + 15),
        finished_at: None,
    };
    db.collection::<MachineJob>(JOBS).insert_one(&job).await?;
    Ok(job)
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
    db.collection::<MachineJob>(JOBS).find_one(doc!{"_id":id,"node_id":node,"runtime_id":runtime,
        "conversation_id":conversation,"state":"running","expires_at":{"$gt":bson::DateTime::now()}})
        .await?.ok_or_else(||AppError::Forbidden("Machine service call has no live server-issued job".into()))
}

pub async fn finish(db: &Database, id: &str) -> AppResult<()> {
    db.collection::<MachineJob>(JOBS)
        .update_one(
            doc! {"_id":id,"state":"running"},
            doc! {"$set":{"state":"finished","finished_at":bson::DateTime::now()}},
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
