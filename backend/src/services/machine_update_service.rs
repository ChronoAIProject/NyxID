//! Durable update policy, admission, reconnect fencing and thread watches.
use crate::{
    errors::{AppError, AppResult},
    models::{
        machine_update::{
            ATTEMPTS_COLLECTION_NAME, COLLECTION_NAME, MachineUpdate, MachineUpdateAttempt,
        },
        node::{COLLECTION_NAME as NODES, Node},
    },
};
use chrono::{Duration, Utc};
use mongodb::{
    Database,
    bson::{self, doc},
    options::ReturnDocument,
};
use nyxid_machine::update::{Installation, Phase, Progress};

pub const TARGET: &str = env!("CARGO_PKG_VERSION");

pub fn current(node: &Node) -> &str {
    node.metadata
        .as_ref()
        .and_then(|m| m.agent_version.as_deref())
        .unwrap_or("unknown")
}

pub fn update_available(node: &Node) -> bool {
    match (
        nyxid_machine::update::version(current(node)),
        nyxid_machine::update::version(TARGET),
    ) {
        (Ok(current), Ok(target)) => current < target,
        _ => false,
    }
}

pub fn installation(node: &Node) -> Installation {
    node.machine
        .as_ref()
        .and_then(|m| m.installation)
        .unwrap_or_else(|| {
            if node
                .machine
                .as_ref()
                .is_some_and(|m| m.os == "linux" && m.roots == ["/workspace"] && m.browser_isolated)
            {
                Installation::Container
            } else {
                Installation::Native
            }
        })
}

pub async fn owner_node(db: &Database, actor: &str, id: &str) -> AppResult<Node> {
    let node = super::node_service::get_node_by_id(db, id)
        .await?
        .filter(|n| n.is_active && n.machine.as_ref().is_some_and(|m| m.enabled()))
        .ok_or_else(|| AppError::NodeNotFound("Machine not found".into()))?;
    if !super::org_service::resolve_owner_access(db, actor, &node.user_id)
        .await?
        .can_write()
    {
        return Err(AppError::MachineNotAllowed);
    }
    Ok(node)
}

pub async fn get(db: &Database, node: &Node) -> AppResult<MachineUpdate> {
    Ok(db
        .collection::<MachineUpdate>(COLLECTION_NAME)
        .find_one(doc! {"_id":&node.id,"user_id":&node.user_id})
        .await?
        .unwrap_or_else(|| MachineUpdate::new(&node.id, &node.user_id)))
}

pub async fn policy(db: &Database, node: &Node, automatic: bool) -> AppResult<()> {
    // Ownership transfer cannot inherit an earlier owner's automatic-update consent.
    if let Some(mut previous) = db
        .collection::<MachineUpdate>(COLLECTION_NAME)
        .find_one(doc! {"_id": &node.id, "user_id": {"$ne": &node.user_id}})
        .await?
    {
        previous.phase = "failed".into();
        previous.code = Some("machine_ownership_changed".into());
        archive(db, &previous).await?;
        db.collection::<MachineUpdate>(COLLECTION_NAME)
            .delete_one(doc! {"_id": &node.id, "user_id": &previous.user_id})
            .await?;
    }
    let mut initial = bson::to_document(&MachineUpdate::new(&node.id, &node.user_id))
        .map_err(|_| AppError::Internal("Could not encode machine update policy".into()))?;
    for field in ["automatic", "updated_at"] {
        initial.remove(field);
    }
    db.collection::<MachineUpdate>(COLLECTION_NAME).update_one(doc!{"_id":&node.id,"user_id":&node.user_id},
        doc!{"$setOnInsert":initial,"$set":{"automatic":automatic,"updated_at":bson::DateTime::now()}}).upsert(true).await?;
    Ok(())
}

pub async fn idle(db: &Database, node: &str) -> AppResult<bool> {
    use crate::models::{assistant_conversation, machine_job};
    if db
        .collection::<bson::Document>(machine_job::COLLECTION_NAME)
        .find_one(doc! {"node_id":node,"state":"running"})
        .await?
        .is_some()
    {
        return Ok(false);
    }
    if db
        .collection::<bson::Document>(assistant_conversation::COLLECTION_NAME)
        .find_one(doc! {"active_turn.machine_node_ids":node})
        .await?
        .is_some()
    {
        return Ok(false);
    }
    for display in [
        nyxid_machine::desktop::Display::Secure,
        nyxid_machine::desktop::Display::Dev,
    ] {
        if super::machine_desktop_service::get(db, &display.key(node))
            .await?
            .is_some_and(|d| {
                !((d.status == "closed"
                    || d.status == "agent" && d.updated_at < Utc::now() - Duration::seconds(40))
                    && d.controller.is_none())
            })
        {
            return Ok(false);
        }
    }
    Ok(true)
}

pub async fn begin(
    db: &Database,
    node: &Node,
    actor: &str,
    conversation: Option<&str>,
    automatic: bool,
    manual: bool,
) -> AppResult<MachineUpdate> {
    if automatic && !idle(db, &node.id).await? {
        return Err(AppError::Conflict("Machine is busy".into()));
    }
    let row = get(db, node).await?;
    if row.pending() {
        return Err(AppError::Conflict(
            "A machine update is already in progress".into(),
        ));
    }
    policy(db, node, row.automatic).await?;
    use super::api_key_mutation_service as transactions;
    let now = Utc::now();
    let attempt = uuid::Uuid::new_v4().to_string();
    let deadline = now + Duration::minutes(if manual { 30 } else { 20 });
    let filter = doc! {
        "_id": &node.id,
        "user_id": &node.user_id,
        "phase": {"$nin": ["manual_step", "queued", "verifying", "downloading", "restarting"]},
    };
    let update = doc! {"$set": {
        "attempt_id": &attempt,
        "requested_by": actor,
        "previous_version": current(node),
        "target_version": TARGET,
        "phase": if manual {"manual_step"} else {"queued"},
        "code": null,
        "notify_pending": true,
        "requested_at": bson::DateTime::from_chrono(now),
        "deadline": bson::DateTime::from_chrono(deadline),
        "updated_at": bson::DateTime::from_chrono(now),
    }};
    let watch = conversation.map(|conversation| {
        doc! {
            "_id": uuid::Uuid::new_v4().to_string(),
            "user_id": actor,
            "kind": "machine_update",
            "conversation_id": conversation,
            "connect_link_id": &attempt,
            "status": "pending",
            "created_at": bson::DateTime::from_chrono(now),
            "expires_at": bson::DateTime::from_chrono(deadline + Duration::hours(1)),
        }
    });
    let mut session = db.client().start_session().await?;
    let database = db.clone();
    let previous_attempt = row.attempt_id.clone();
    let row = session
        .start_transaction()
        .and_run2(async move |session| {
            let result: AppResult<MachineUpdate> = async {
                if let Some(id) = &previous_attempt {
                    // Archive in the same transaction as replacement, so a delayed
                    // change-stream watcher always finds the terminal outcome.
                    if let Some(previous) = database
                        .collection::<MachineUpdate>(COLLECTION_NAME)
                        .find_one(doc! {"attempt_id": id})
                        .session(&mut *session)
                        .await?
                    {
                        let snapshot = MachineUpdateAttempt {
                            id: id.clone(),
                            user_id: previous.user_id.clone(),
                            state: previous,
                        };
                        database
                            .collection::<MachineUpdateAttempt>(ATTEMPTS_COLLECTION_NAME)
                            .replace_one(doc! {"_id": id}, snapshot)
                            .upsert(true)
                            .session(&mut *session)
                            .await?;
                    }
                }
                let row = database
                    .collection::<MachineUpdate>(COLLECTION_NAME)
                    .find_one_and_update(filter.clone(), update.clone())
                    .return_document(ReturnDocument::After)
                    .session(&mut *session)
                    .await?
                    .ok_or_else(|| {
                        AppError::Conflict("A machine update was already admitted".into())
                    })?;
                if let Some(watch) = &watch {
                    database
                        .collection::<bson::Document>(
                            crate::models::nyxbot_channel::WATCHES_COLLECTION_NAME,
                        )
                        .insert_one(watch)
                        .session(&mut *session)
                        .await?;
                }
                Ok(row)
            }
            .await;
            transactions::transaction_result(result)
        })
        .await
        .map_err(transactions::map_transaction_error)?;
    super::audit_service::log_async(
        db.clone(),
        Some(actor.into()),
        "machine_update_requested".into(),
        Some(
            serde_json::json!({"node_id":node.id,"target_version":TARGET,"automatic":automatic,"manual_step":manual}),
        ),
        None,
        None,
        None,
        None,
    );
    Ok(row)
}

pub async fn finish(
    db: &Database,
    row: &MachineUpdate,
    phase: &str,
    code: Option<&str>,
) -> AppResult<()> {
    db.collection::<MachineUpdate>(COLLECTION_NAME).update_one(
        doc!{"_id":&row.node_id,"attempt_id":&row.attempt_id,"phase":{"$in":["manual_step","queued","verifying","downloading","restarting"]}},
        doc!{"$set":{"phase":phase,"code":code,"notify_pending":true,"updated_at":bson::DateTime::now()}}
    ).await?;
    Ok(())
}

pub async fn observe(db: &Database, node: &Node, report: Option<&Progress>) -> AppResult<()> {
    let row = get(db, node).await?;
    if !row.pending() {
        return Ok(());
    }
    if let Some(report) =
        report.filter(|p| Some(p.target.as_str()) == row.target_version.as_deref())
    {
        // A stale status file from an older attempt cannot settle a new one.
        let fresh = row.requested_at.is_some_and(|at| {
            report.started_at_ms.saturating_add(1000) >= at.timestamp_millis() as u64
        });
        if fresh {
            match report.phase {
                Phase::Failed | Phase::RolledBack => {
                    return finish(
                        db,
                        &row,
                        if report.phase == Phase::RolledBack {
                            "rolled_back"
                        } else {
                            "failed"
                        },
                        Some(
                            report
                                .code
                                .as_deref()
                                .filter(|code| {
                                    nyxid_machine::update::failure_guidance(code).is_some()
                                })
                                .unwrap_or("update_failed_or_rolled_back"),
                        ),
                    )
                    .await;
                }
                Phase::Queued | Phase::Verifying | Phase::Downloading | Phase::Restarting => {
                    let phase = match report.phase {
                        Phase::Queued => "queued",
                        Phase::Verifying => "verifying",
                        Phase::Downloading => "downloading",
                        _ => "restarting",
                    };
                    db.collection::<MachineUpdate>(COLLECTION_NAME)
                        .update_one(
                            doc! {"_id":&node.id,"attempt_id":&row.attempt_id},
                            doc! {"$set":{"phase":phase,"updated_at":bson::DateTime::now()}},
                        )
                        .await?;
                }
                Phase::Connected => {}
            }
        }
    }
    let reconnected = node
        .connected_at
        .zip(row.requested_at)
        .is_some_and(|(connected, requested)| connected > requested);
    if reconnected
        && node.status == crate::models::node::NodeStatus::Online
        && current(node) == row.target_version.as_deref().unwrap_or("")
    {
        return finish(db, &row, "connected", None).await;
    }
    if row.deadline.is_some_and(|deadline| deadline <= Utc::now()) {
        return finish(
            db,
            &row,
            "failed",
            Some(if reconnected {
                "reconnected_old_version"
            } else {
                "update_reconnect_timeout"
            }),
        )
        .await;
    }
    Ok(())
}

async fn archive(db: &Database, row: &MachineUpdate) -> AppResult<()> {
    if let Some(id) = &row.attempt_id {
        db.collection::<MachineUpdateAttempt>(ATTEMPTS_COLLECTION_NAME)
            .replace_one(
                doc! {"_id": id},
                MachineUpdateAttempt {
                    id: id.clone(),
                    user_id: row.user_id.clone(),
                    state: row.clone(),
                },
            )
            .upsert(true)
            .await?;
    }
    Ok(())
}

pub async fn watched(db: &Database, attempt: &str) -> AppResult<Option<MachineUpdate>> {
    if let Some(row) = db
        .collection::<MachineUpdate>(COLLECTION_NAME)
        .find_one(doc! {"attempt_id": attempt})
        .await?
    {
        return Ok(Some(row));
    }
    Ok(db
        .collection::<MachineUpdateAttempt>(ATTEMPTS_COLLECTION_NAME)
        .find_one(doc! {"_id": attempt})
        .await?
        .map(|snapshot| snapshot.state))
}

pub async fn node_for_record(db: &Database, row: &MachineUpdate) -> AppResult<Option<Node>> {
    Ok(db
        .collection::<Node>(NODES)
        .find_one(doc! {"_id":&row.node_id,"user_id":&row.user_id,"is_active":true})
        .await?)
}

pub async fn seed_policy(db: &Database, node_id: &str) -> AppResult<()> {
    let Some(setup) = db
        .collection::<crate::models::machine_setup::MachineSetup>(
            crate::models::machine_setup::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":node_id})
        .await?
    else {
        return Ok(());
    };
    let Some(automatic) = setup.choices.automatic_updates else {
        return Ok(());
    };
    let Some(node) = super::node_service::get_node_by_id(db, node_id).await? else {
        return Ok(());
    };
    if node.user_id != setup.choices.owner_id.as_deref().unwrap_or(&setup.user_id) {
        return Ok(());
    }
    let mut row = MachineUpdate::new(&node.id, &node.user_id);
    row.automatic = automatic;
    let doc = bson::to_document(&row)
        .map_err(|_| AppError::Internal("Could not encode update policy".into()))?;
    db.collection::<MachineUpdate>(COLLECTION_NAME)
        .update_one(doc! {"_id":node_id}, doc! {"$setOnInsert":doc})
        .upsert(true)
        .await?;
    Ok(())
}
