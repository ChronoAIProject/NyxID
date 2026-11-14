//! Task identity is persisted on existing roots; it is never bearer authority.
use super::*;
use crate::models::{assistant_conversation::AssistantConversation, assistant_group::GroupRequest};
use crate::services::assistant_nyxagent::{OwnerTurnIds, TurnStart};
use crate::services::execution_permission_channel as channel;

pub(crate) fn collection(task: &TaskRef) -> &'static str {
    match task.root_kind {
        TaskRootKind::Message => "assistant_messages",
        TaskRootKind::Invocation => "assistant_invocations",
        TaskRootKind::GroupRequest => "assistant_group_requests",
    }
}

/// Server-created direct delegation edge. Neither field is accepted from a tool.
#[derive(Clone)]
pub(crate) struct Parent {
    conversation_id: String,
    task: TaskAdmission,
}

pub(crate) fn parent(chat: &acks::ChatAuthority) -> Option<Parent> {
    child(chat).map(|task| Parent {
        conversation_id: chat.conversation_id.clone(),
        task,
    })
}

pub(crate) fn admit<'a>(
    db: &'a Database,
    session: &'a mut ClientSession,
    row: &'a mut AssistantConversation,
    start: &'a TurnStart,
    ids: &'a OwnerTurnIds,
) -> futures::future::BoxFuture<'a, AppResult<()>> {
    Box::pin(async move {
        let turn = row.active_turn.as_ref().ok_or_else(refused)?;
        if row.guest_turn
            || (row.automation_thread && row.voice_parent_conversation_id.is_none())
            || row.platform_thread.is_some()
        {
            if turn
                .execution_task
                .as_ref()
                .into_iter()
                .chain(start.execution_parent.as_ref().map(|p| &p.task))
                .chain(turn.events.iter().filter_map(|e| e.execution_task.as_ref()))
                .any(|task| task.channel_source.is_some())
            {
                return Err(refused());
            }
            return Ok(());
        }
        let mut inherited = turn
            .execution_task
            .clone()
            .or_else(|| start.execution_parent.as_ref().map(|p| p.task.clone()))
            .or_else(|| {
                (start.origin == crate::models::assistant_conversation::TurnOrigin::Event)
                    .then(|| turn.events.iter().find_map(|e| e.execution_task.clone()))
                    .flatten()
            });
        if let Some(task) = &inherited
            && start.origin == crate::models::assistant_conversation::TurnOrigin::Event
            && turn
                .events
                .iter()
                .filter_map(|e| e.execution_task.as_ref())
                .any(|other| {
                    other.task != task.task
                        || other.epoch != task.epoch
                        || other.actor_user_id != task.actor_user_id
                        || other.audience != task.audience
                        || other.channel_source != task.channel_source
                })
        {
            return Err(refused());
        }
        if inherited.is_none()
            && row.channel.is_some()
            && start.origin != crate::models::assistant_conversation::TurnOrigin::Channel
        {
            return Ok(());
        }
        if !start.execution_enrollment && inherited.is_none() {
            return Ok(());
        }
        // Unsupported fresh channel lanes are known from the loaded row; they
        // must not add enrollment/readiness commands to their legacy path.
        if inherited.is_none()
            && start.origin == crate::models::assistant_conversation::TurnOrigin::Channel
            && (row.agent_owner_id.is_some()
                || row.shared_agent_thread
                || row.group_id.is_some()
                || row.collaboration.is_some())
        {
            return Ok(());
        }
        // Queue roots and voice requests already carry durable enrollment.
        // Only a genuinely fresh human turn may choose legacy on a stale hint.
        let marked = inherited.is_some()
            || turn.voice_request_id.is_some()
            || row.group_request_id.is_some()
            || row.collaboration.is_some();
        if marked {
            require_enrolled(db, &row.user_id, Some(session)).await?;
        } else if !super::super::execution_permission_enrollment::live_enabled(
            db,
            &row.user_id,
            Some(session),
        )
        .await?
        {
            return Ok(());
        }
        if let Some(request) = turn.voice_request_id.as_deref() {
            let voice = voice_task(db, session, row, request).await?;
            if inherited.as_ref().is_some_and(|t| !same_task(t, &voice)) {
                return Err(refused());
            }
            inherited = Some(voice);
        }
        if let Some(parent) = &start.execution_parent {
            let parent_turn = parent.task.parent_turn_id.as_deref().ok_or_else(refused)?;
            let conversations = db.collection::<AssistantConversation>("assistant_conversations");
            let filter = doc! {"_id":&parent.conversation_id,"user_id":&row.user_id,
            "active_turn.turn_id":parent_turn,"active_turn.stop_requested":false,
            "active_turn.execution_task.task":bson::to_bson(&parent.task.task).map_err(encoding)?,
            "active_turn.execution_task.epoch":parent.task.epoch};
            let source = conversations
                .find_one_and_update(filter, doc! {"$inc":{"execution_dispatch_fence":1_i64}})
                .session(&mut *session)
                .await?
                .ok_or_else(refused)?;
            if super::super::assistant_nyxagent::live_turn(&source, Utc::now()).is_none() {
                return Err(refused());
            }
        }

        let now = bson::DateTime::now().to_chrono();
        let channel_source = if let Some(source) =
            inherited.as_ref().and_then(|t| t.channel_source.clone())
        {
            if row.agent_owner_id.is_some()
                || row.shared_agent_thread
                || row.group_id.is_some()
                || row.collaboration.is_some()
            {
                return Err(refused());
            }
            Some(source)
        } else if start.origin == crate::models::assistant_conversation::TurnOrigin::Channel {
            if !crate::services::security_readiness_service::serving_report(db, channel::READINESS)
                .await?
                .serving_ready
            {
                return Ok(());
            }
            let event = start.channel_event_id.as_deref().ok_or_else(refused)?;
            Some(Box::new(channel::admit(db, session, row, event).await?))
        } else {
            None
        };
        if let Some(source) = &channel_source
            && (start
                .channel_event_id
                .as_ref()
                .is_some_and(|id| id != &source.event_id)
                || turn
                    .events
                    .iter()
                    .flat_map(|e| &e.source_event_ids)
                    .any(|id| id != &source.event_id))
        {
            return Err(refused());
        }
        let current_audience = if let Some(source) = &channel_source {
            channel::audience(source)
        } else if let Some(group) = row
            .collaboration
            .as_ref()
            .map(|b| &b.group_id)
            .or(row.group_id.as_ref())
        {
            let group = db.collection::<crate::models::assistant_group::AssistantGroup>("assistant_groups")
            .find_one(doc! {"_id":group,"$or":[{"user_id":&row.user_id},{"participant_user_ids":&row.user_id}]})
            .session(&mut *session).await?.ok_or_else(refused)?;
            super::group_audience(&group)
        } else {
            Audience::Private {
                user_id: row.user_id.clone(),
            }
        };
        let task = if let Some(parent) = inherited {
            parent
        } else {
            let (root, audience) = if let Some(request) = row
                .collaboration
                .as_ref()
                .map(|b| b.request_id.as_str())
                .or(row.group_request_id.as_deref())
            {
                let request = db
                    .collection::<GroupRequest>("assistant_group_requests")
                    .find_one(doc! {"_id":request,"actor_user_id":&row.user_id})
                    .session(&mut *session)
                    .await?
                    .ok_or_else(refused)?;
                if let Some(existing) = request.execution_task {
                    if existing.actor_user_id != row.user_id
                        || existing.version != VERSION
                        || existing.cancelled
                        || existing.task.root_kind != TaskRootKind::GroupRequest
                        || existing.task.root_id != request.id
                        || existing.audience != current_audience
                    {
                        return Err(refused());
                    }
                    row.active_turn.as_mut().unwrap().execution_task = Some(existing.clone());
                    fence(db, session, &existing).await?;
                    return Ok(());
                }
                (
                    TaskRef {
                        root_kind: TaskRootKind::GroupRequest,
                        root_id: request.id,
                    },
                    current_audience.clone(),
                )
            } else if let Some(id) = turn.invocation_id.as_ref() {
                (
                    TaskRef {
                        root_kind: TaskRootKind::Invocation,
                        root_id: id.clone(),
                    },
                    current_audience.clone(),
                )
            } else {
                (
                    TaskRef {
                        root_kind: TaskRootKind::Message,
                        root_id: ids.message_id.clone(),
                    },
                    current_audience.clone(),
                )
            };
            let proposed = TaskAdmission {
                channel_source: channel_source.clone(),
                version: VERSION,
                task: root,
                actor_user_id: row.user_id.clone(),
                audience,
                epoch: 1,
                hard_deadline: channel_source
                    .as_ref()
                    .map_or(now + chrono::Duration::hours(24), |s| s.expires_at),
                cancelled: false,
                parent_turn_id: None,
            };
            if proposed.task.root_kind != TaskRootKind::Message {
                let roots = db.collection::<Document>(collection(&proposed.task));
                let actor_field = if proposed.task.root_kind == TaskRootKind::GroupRequest {
                    "actor_user_id"
                } else {
                    "user_id"
                };
                let mut root_filter = doc! {"_id":&proposed.task.root_id};
                root_filter.insert(actor_field, &row.user_id);
                let root = roots
                    .find_one(root_filter)
                    .session(&mut *session)
                    .await?
                    .ok_or_else(refused)?;
                if let Ok(existing) = root.get_document("execution_task") {
                    bson::from_document::<TaskAdmission>(existing.clone()).map_err(encoding)?
                } else {
                    if roots.update_one(doc! {"_id":&proposed.task.root_id,"execution_task":null,"cancel_requested":{"$ne":true}},
                    doc! {"$set":{"execution_task":bson::to_bson(&proposed).map_err(encoding)?}})
                    .session(&mut *session).await?.matched_count != 1 { return Err(refused()); }
                    proposed
                }
            } else {
                proposed
            }
        };
        if task.actor_user_id != row.user_id
            || task.audience != current_audience
            || task.cancelled
            || task.version != VERSION
            || task.hard_deadline <= now
        {
            return Err(refused());
        }
        if task.task.root_kind != TaskRootKind::Message || task.task.root_id != ids.message_id {
            fence(db, session, &task).await?;
        }
        row.active_turn.as_mut().unwrap().execution_task = Some(task);
        Ok(())
    })
}

/// Voice continuations/recovery reuse the original request's visible message
/// root. The root request and message are fenced together with turn admission.
async fn voice_task(
    db: &Database,
    session: &mut ClientSession,
    row: &AssistantConversation,
    request_id: &str,
) -> AppResult<TaskAdmission> {
    use crate::models::assistant_voice::{REQUESTS, VoiceRequest};
    let requests = db.collection::<VoiceRequest>(REQUESTS);
    let source = requests
        .find_one(doc! {"_id":request_id,"user_id":&row.user_id,
        "task_conversation_id":&row.id,"state":{"$ne":"cancelled"}})
        .session(&mut *session)
        .await?
        .ok_or_else(refused)?;
    let root = requests.find_one_and_update(doc! {
        "_id":source.root_request_id.as_deref().unwrap_or(&source.id),"user_id":&row.user_id,
        "task_conversation_id":&row.id,"execution_enrolled":true,"state":{"$ne":"cancelled"}
    },doc! {"$inc":{"execution_dispatch_fence":1_i64}}).session(&mut *session).await?.ok_or_else(refused)?;
    if row.voice_parent_conversation_id.as_deref() != Some(root.conversation_id.as_str()) {
        return Err(refused());
    }
    let messages = db.collection::<Document>("assistant_messages");
    let filter = doc! {"_id":&root.message_id,"conversation_id":&root.conversation_id,"user_id":&row.user_id};
    let message = messages
        .find_one(filter.clone())
        .session(&mut *session)
        .await?
        .ok_or_else(refused)?;
    if let Ok(task) = message.get_document("execution_task") {
        return bson::from_document(task.clone()).map_err(encoding);
    }
    let task = TaskAdmission {
        channel_source: None,
        version: VERSION,
        task: TaskRef {
            root_kind: TaskRootKind::Message,
            root_id: root.message_id,
        },
        actor_user_id: row.user_id.clone(),
        audience: Audience::Private {
            user_id: row.user_id.clone(),
        },
        epoch: 1,
        hard_deadline: root.created_at + chrono::Duration::hours(24),
        cancelled: false,
        parent_turn_id: None,
    };
    if messages
        .update_one(
            filter,
            doc! {"$set":{"execution_task":bson::to_bson(&task).map_err(encoding)?}},
        )
        .session(session)
        .await?
        .matched_count
        != 1
    {
        return Err(refused());
    }
    Ok(task)
}

pub(super) fn live_filter(task: &TaskAdmission) -> AppResult<Document> {
    if task.version != VERSION
        || task.cancelled
        || task.epoch < 1
        || task.hard_deadline <= Utc::now()
    {
        return Err(refused());
    }
    let mut filter = doc! {"_id":&task.task.root_id,"execution_task.version":VERSION as i32,
    "execution_task_completed":{"$ne":true},
    "execution_task.task":bson::to_bson(&task.task).map_err(encoding)?,"execution_task.actor_user_id":&task.actor_user_id,"execution_task.epoch":task.epoch,
    "execution_task.audience":bson::to_bson(&task.audience).map_err(encoding)?,
    "execution_task.cancelled":false,"execution_task.hard_deadline":{"$gt":bson::DateTime::now()}};
    if let Some(source) = &task.channel_source {
        filter.insert(
            "execution_task.channel_source",
            bson::to_bson(source).map_err(encoding)?,
        );
    }
    if task.task.root_kind == TaskRootKind::GroupRequest {
        filter.insert("dropped", doc! {"$ne":true});
    }
    if task.task.root_kind == TaskRootKind::Invocation {
        filter.insert("cancel_requested", false);
        filter.insert("conversation_deleted", false);
        filter.insert("continuation_blocked", doc! {"$ne":true});
        filter.insert("terminal_status", bson::Bson::Null);
    }
    Ok(filter)
}

pub(super) async fn check(db: &Database, task: &TaskAdmission) -> AppResult<()> {
    channel::check(db, task).await?;
    if db
        .collection::<Document>(collection(&task.task))
        .find_one(live_filter(task)?)
        .projection(doc! {"_id":1})
        .await?
        .is_none()
    {
        return Err(refused());
    }
    Ok(())
}

pub(crate) async fn fence(
    db: &Database,
    session: &mut ClientSession,
    task: &TaskAdmission,
) -> AppResult<()> {
    require_enrolled(db, &task.actor_user_id, Some(session)).await?;
    if let Some(source) = &task.channel_source {
        channel::validate(db, session, source, true).await?;
    }
    let filter = live_filter(task)?;
    if db
        .collection::<Document>(collection(&task.task))
        .update_one(filter, doc! {"$inc":{"execution_task_fence":1_i64}})
        .session(session)
        .await?
        .matched_count
        != 1
    {
        return Err(refused());
    }
    Ok(())
}

/// A successful message task is terminal once no human decision or admitted
/// descendant can resume it. Failure remains recoverable through owner Retry;
/// invocation/group terminal state is owned by their existing root services.
pub(crate) fn settle<'a>(
    db: &'a Database,
    session: &'a mut ClientSession,
    row: &'a AssistantConversation,
    turn: &'a crate::models::assistant_conversation::ActiveTurn,
    message: &'a crate::models::assistant_message::AssistantMessage,
) -> futures::future::BoxFuture<'a, AppResult<()>> {
    Box::pin(async move {
        let Some(task) = turn.execution_task.as_ref() else {
            return Ok(());
        };
        channel::deliveries::enqueue(db, session, row, turn, message).await?;
        let receipts: Vec<_> = turn
            .events
            .iter()
            .filter_map(|e| e.id.strip_prefix("execution-decision:"))
            .collect();
        if !receipts.is_empty() && message.status == "completed" {
            for version in proposals::v3::AssistantVersion::ALL {
                db.collection::<Document>("approval_requests").update_many(version.selector(doc!{"_id":{"$in":&receipts},"user_id":&row.user_id,
                    "execution_proposal.v1.state":{"$in":["completed","not_sent","unknown","denied","expired","cancelled"]}}),
                    version.update(doc!{"$set":{"execution_proposal.v1.delivered_turn_id":&turn.turn_id}})).session(&mut *session).await?;
            }
        }
        let v2_receipts: Vec<_> = turn
            .events
            .iter()
            .filter_map(|e| e.id.strip_prefix("execution-decision-v2:"))
            .collect();
        if !v2_receipts.is_empty() && message.status == "completed" {
            db.collection::<Document>("approval_requests").update_many(doc!{"_id":{"$in":v2_receipts},"user_id":&row.user_id,"execution_proposal.v2.state":{"$in":["completed","not_sent","unknown","denied","expired","cancelled"]}},doc!{"$set":{"execution_proposal.v2.delivered_turn_id":&turn.turn_id}}).session(&mut *session).await?;
        }
        if task.task.root_kind != TaskRootKind::Message
            || message.status != "completed"
            || turn.report_to.is_some()
        {
            return Ok(());
        }
        let reference = bson::to_bson(&task.task).map_err(encoding)?;
        let pending_card = db.collection::<Document>("assistant_acknowledgements").find_one(doc! {
        "user_id":&row.user_id,"execution_task.task":&reference,"status":{"$in":["pending","allowed"]},"expires_at":{"$gt":bson::DateTime::now()}
    }).session(&mut *session).await?.is_some();
        let descendant = db
            .collection::<Document>("assistant_conversations")
            .find_one(doc! {
                "user_id":&row.user_id,"$or":[
                    {"_id":{"$ne":&row.id},"active_turn.execution_task.task":&reference},
                    {"pending_events.execution_task.task":&reference}
                ]
            })
            .session(&mut *session)
            .await?
            .is_some();
        let pending_effect = db.collection::<Document>("approval_requests").find_one(proposals::v3::source_selector(doc! {
            "user_id":&row.user_id,"execution_proposal.v1.task.task":&reference,
            "execution_proposal.v1.state":{"$in":["pending","queued","preparing","dispatched","waiting"]},
            "execution_proposal.v1.task.hard_deadline":{"$gt":bson::DateTime::now()}
        })).session(&mut *session).await?.is_some();
        let mut background = false;
        for (collection, states) in [
            ("machine_jobs", vec!["running"]),
            (
                "assistant_async_operations",
                vec!["submitting", "waiting", "cancelling", "ready", "queued"],
            ),
        ] {
            background |= db
                .collection::<Document>(collection)
                .find_one(doc! {
                    "user_id":&row.user_id,"execution_task.task":&reference,"state":{"$in":states},
                    "expires_at":{"$gt":bson::DateTime::now()}
                })
                .session(&mut *session)
                .await?
                .is_some();
        }
        if !pending_card && !descendant && !pending_effect && !background {
            db.collection::<Document>(collection(&task.task))
                .update_one(
                    doc! {"_id":&task.task.root_id,"execution_task.epoch":task.epoch},
                    doc! {"$set":{"execution_task_completed":true}},
                )
                .session(session)
                .await?;
        }
        Ok(())
    })
}

pub(crate) fn stop<'a>(
    db: &'a Database,
    row: &'a AssistantConversation,
) -> futures::future::BoxFuture<'a, AppResult<()>> {
    Box::pin(async move {
        let Some(turn) = row.active_turn.as_ref() else {
            return Ok(());
        };
        let Some(task) = turn.execution_task.as_ref() else {
            return Ok(());
        };
        let mut session = db.client().start_session().await?;
        session.start_transaction().and_run((db,row,turn,task), |session, context| {
        let (db,row,turn,task) = *context;
        Box::pin(async move {
        super::super::api_key_mutation_service::transaction_result(async {
            let changed = db.collection::<Document>("assistant_conversations").update_one(
                doc! {"_id":&row.id,"user_id":&row.user_id,"active_turn.turn_id":&turn.turn_id},
                doc! {"$set":{"active_turn.stop_requested":true}}).session(&mut *session).await?;
            if changed.matched_count == 1 {
                db.collection::<Document>(collection(&task.task)).update_one(
                    doc! {"_id":&task.task.root_id,"execution_task.epoch":task.epoch},
                    doc! {"$set":{"execution_task.cancelled":true},"$inc":{"execution_task.epoch":1_i64}})
                    .session(&mut *session).await?;
                let reference = bson::to_bson(&task.task).map_err(encoding)?;
                db.collection::<Document>("assistant_conversations").update_many(
                    doc! {"user_id":&row.user_id,"active_turn.execution_task.task":&reference,"active_turn.execution_task.epoch":task.epoch},
                    doc! {"$set":{"active_turn.stop_requested":true},"$inc":{"authority_fence":1_i64}}).session(&mut *session).await?;
                super::proposals::cancel_in_session(db,session,
                    doc!{"user_id":&row.user_id,"execution_proposal.v1.task.task":&reference},"task_stopped").await?;
                db.collection::<Document>("assistant_acknowledgements").update_many(
                    doc! {"user_id":&row.user_id,"execution_task.task":&reference,"execution_task.epoch":task.epoch,"status":{"$in":["pending","allowed"]}},
                    doc! {"$set":{"status":"expired","expires_at":bson::DateTime::now(),"expiry_reason":"task_stopped"}}).session(&mut *session).await?;
                db.collection::<Document>("assistant_conversations").update_many(
                    doc! {"user_id":&row.user_id,"pending_events.execution_task.task":&reference},
                    doc! {"$pull":{"pending_events":{"execution_task.task":&reference,"execution_task.epoch":task.epoch}}}).session(&mut *session).await?;
            }
            Ok(())
        }.await)
    })}).await.map_err(super::super::api_key_mutation_service::map_transaction_error)
    })
}

/// Only producers with a verified live turn can attach this server lineage.
pub(crate) fn child(chat: &acks::ChatAuthority) -> Option<TaskAdmission> {
    chat.execution_task.clone().map(|mut task| {
        task.parent_turn_id = chat.turn_id.clone();
        task
    })
}

fn same_task(a: &TaskAdmission, b: &TaskAdmission) -> bool {
    a.version == b.version
        && a.task == b.task
        && a.actor_user_id == b.actor_user_id
        && a.audience == b.audience
        && a.channel_source == b.channel_source
        && a.epoch == b.epoch
        && a.hard_deadline == b.hard_deadline
        && !a.cancelled
        && !b.cancelled
}

/// Batch marked roots only. Unmarked queues add no commands. A cancelled root
/// cannot be turned back into an unmarked event after a worker restarts.
pub(crate) fn filter_events<'a>(
    db: &'a Database,
    session: &'a mut ClientSession,
    actor: &'a str,
    mut events: Vec<crate::models::assistant_conversation::AgentEvent>,
) -> futures::future::BoxFuture<'a, AppResult<Vec<crate::models::assistant_conversation::AgentEvent>>>
{
    Box::pin(async move {
        let mut live = Vec::new();
        for kind in [
            TaskRootKind::Message,
            TaskRootKind::Invocation,
            TaskRootKind::GroupRequest,
        ] {
            let filters: Vec<_> = events
                .iter()
                .filter_map(|e| e.execution_task.as_ref())
                .filter(|t| t.task.root_kind == kind && t.actor_user_id == actor)
                .filter_map(|t| live_filter(t).ok())
                .collect();
            if filters.is_empty() {
                continue;
            }
            let name = collection(&TaskRef {
                root_kind: kind,
                root_id: String::new(),
            });
            let rows: Vec<Document> = db
                .collection::<Document>(name)
                .find(doc! {"$or":filters})
                .projection(doc! {"execution_task":1})
                .session(&mut *session)
                .await?
                .stream(&mut *session)
                .try_collect()
                .await?;
            for row in rows {
                live.push(
                    bson::from_document::<TaskAdmission>(
                        row.get_document("execution_task")
                            .map_err(encoding)?
                            .clone(),
                    )
                    .map_err(encoding)?,
                );
            }
        }
        events.retain(|e| {
            e.execution_task
                .as_ref()
                .is_none_or(|task| live.iter().any(|root| same_task(task, root)))
        });
        Ok(events)
    })
}

/// One event turn has one logical root. Leave unrelated notices for another
/// turn instead of splicing tasks or starving the whole queue.
pub(crate) fn partition_events(
    pending: &mut Vec<crate::models::assistant_conversation::AgentEvent>,
    events: Vec<crate::models::assistant_conversation::AgentEvent>,
    origin: crate::models::assistant_conversation::TurnOrigin,
) -> Vec<crate::models::assistant_conversation::AgentEvent> {
    if origin != crate::models::assistant_conversation::TurnOrigin::Event
        || !events.iter().any(|e| e.execution_task.is_some())
    {
        return events;
    }
    let selected = events.first().and_then(|e| e.execution_task.clone());
    let (selected_events, deferred): (Vec<_>, Vec<_>) =
        events
            .into_iter()
            .partition(|e| match (&selected, &e.execution_task) {
                (Some(a), Some(b)) => same_task(a, b),
                (None, None) => true,
                _ => false,
            });
    pending.extend(deferred);
    selected_events
}
