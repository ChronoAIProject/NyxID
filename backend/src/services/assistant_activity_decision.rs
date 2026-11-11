//! Metadata-only links between a gated step, its decision and the exact effect.
//! These records are presentation evidence, never authority or retry inputs.
use super::assistant_acknowledgement_service::ChatAuthority;
use crate::{
    errors::AppResult,
    models::{
        approval_request::COLLECTION_NAME as APPROVALS,
        assistant_acknowledgement::{AuthoredSkillReview, COLLECTION_NAME as ACKS},
        assistant_conversation::{
            ActivityDecision, ActivityExecution, COLLECTION_NAME as CONVERSATIONS, TurnActivity,
        },
    },
};
use futures::TryStreamExt;
use mongodb::{
    ClientSession, Database,
    bson::{self, Document, doc},
};
use std::collections::HashMap;

// Raw HTTP has no MCP activity wrapper. Allocate an ID in request scope, but
// persist a step only if a trusted decision gate actually runs. Ordinary model
// callbacks do not acquire a new activity or issue additional database queries.
pub struct ProxyActivity {
    id: String,
    pub used: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
impl ProxyActivity {
    pub fn new(id: String) -> Self {
        Self {
            id,
            used: Default::default(),
        }
    }
}
tokio::task_local! { pub static PROXY_ACTIVITY: ProxyActivity; }
tokio::task_local! { pub static DECISION_PENDING: std::sync::Arc<std::sync::atomic::AtomicBool>; }

pub fn execution(chat: &ChatAuthority) -> Option<ActivityExecution> {
    Some(ActivityExecution {
        machine: None,
        conversation_id: chat.conversation_id.clone(),
        turn_id: chat.turn_id.clone()?,
        activity_id: super::machine_activity_service::activity_id()
            .or_else(|| PROXY_ACTIVITY.try_with(|a| a.id.clone()).ok())?,
        status: "running".into(),
    })
}

/// Called only at trusted gates, after their owner/key/turn checks. The update
/// shares their transaction when present, including reused pending cards.
pub async fn bind(
    db: &Database,
    chat: &ChatAuthority,
    kind: &str,
    id: &str,
    executing: bool,
    mut session: Option<&mut ClientSession>,
) -> AppResult<()> {
    let Some(execution) = execution(chat) else {
        return Ok(());
    };
    let conversations = db.collection::<Document>(CONVERSATIONS);
    if super::machine_activity_service::activity_id().is_none() {
        let _ =
            PROXY_ACTIVITY.try_with(|a| a.used.store(true, std::sync::atomic::Ordering::Relaxed));
        let insert = conversations.update_one(
            doc! {"_id": &chat.conversation_id, "user_id": &chat.user_id,
            "credential_api_key_id": &chat.api_key_id, "active_turn.turn_id": &execution.turn_id,
            "active_turn.stop_requested": {"$ne": true},
            "active_turn.activities.id": {"$ne": &execution.activity_id}},
            super::assistant_nyxagent::append_activity(doc! {
                "id": &execution.activity_id, "label": "service_request", "status": "running",
                "started_at": bson::DateTime::now(), "ended_at": bson::Bson::Null,
            }),
        );
        match session.as_deref_mut() {
            Some(session) => {
                insert.session(session).await?;
            }
            None => {
                insert.await?;
            }
        }
    }
    if executing && kind == "approval" {
        let approvals = db.collection::<Document>(APPROVALS);
        let record=approvals.update_one(
            doc!{"_id":id,"status":"approved","activity_execution":bson::Bson::Null},
            doc!{"$set":{"activity_execution":bson::to_bson(&execution).expect("execution metadata")}},
        );
        match session.as_deref_mut() {
            Some(session) => {
                record.session(session).await?;
            }
            None => {
                record.await?;
            }
        }
    }
    let _ = DECISION_PENDING
        .try_with(|pending| pending.store(!executing, std::sync::atomic::Ordering::Relaxed));
    let decision = ActivityDecision {
        kind: kind.into(),
        id: id.into(),
        state: if executing { "allowed" } else { "waiting" }.into(),
        executing,
    };
    let update = conversations.update_one(
        doc! {"_id": &chat.conversation_id, "user_id": &chat.user_id,
            "credential_api_key_id": &chat.api_key_id, "active_turn.turn_id": &execution.turn_id,
            "active_turn.activities.id": &execution.activity_id},
        doc! {"$set": {"active_turn.activities.$.decision": bson::to_bson(&decision).expect("decision metadata")}},
    );
    match session {
        Some(session) => {
            update.session(session).await?;
        }
        None => {
            update.await?;
        }
    }
    Ok(())
}

pub enum Completion {
    Succeeded,
    Failed,
    Cancelled,
}
impl Completion {
    fn state(&self) -> &'static str {
        match self {
            Self::Succeeded => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

/// Complete only the exact consumed occurrence. A late result cannot change a
/// newer execution, another card, or a different turn's activity.
pub async fn finished(
    db: &Database,
    conversation: &str,
    turn: &str,
    activity: &str,
    decision: Option<&ActivityDecision>,
    completion: Completion,
    machine: Option<&crate::models::machine_receipt::MachineReceipt>,
) -> AppResult<()> {
    let Some(decision) = decision.filter(|d| d.executing) else {
        return Ok(());
    };
    let collection = match decision.kind.as_str() {
        "acknowledgement" => ACKS,
        "approval" => APPROVALS,
        _ => return Ok(()),
    };
    let mut outcome = doc! {"activity_execution.status":completion.state(),
    "activity_execution.machine":bson::to_bson(&machine).expect("receipt metadata")};
    if decision.kind == "approval" {
        outcome.insert("activity_outcome", completion.state());
    }
    db.collection::<Document>(collection)
        .update_one(
            doc! {
                "_id": &decision.id, "activity_execution.conversation_id": conversation,
                "activity_execution.turn_id": turn, "activity_execution.activity_id": activity,
                "activity_execution.status": "running",
            },
            doc! {"$set": outcome},
        )
        .await?;
    Ok(())
}

fn execution_state(row: &Document) -> &str {
    if let Ok(machine) = row
        .get_document("activity_execution")
        .and_then(|e| e.get_document("machine"))
    {
        return match machine.get_str("status").unwrap_or_default() {
            "running" => "allowed",
            "finished" | "completed" => "completed",
            "error" => "failed",
            "cancelled" | "stopped" => "cancelled",
            _ => "unavailable",
        };
    }
    match row
        .get_document("activity_execution")
        .ok()
        .and_then(|e| e.get_str("status").ok())
    {
        Some("completed") => "completed",
        Some("failed") => "failed",
        Some("cancelled") => "cancelled",
        Some("unknown") => "unavailable",
        // Permission and dispatch alone never prove successful execution.
        _ => "allowed",
    }
}
fn expired(row: &Document) -> bool {
    row.get_datetime("expires_at")
        .is_ok_and(|t| *t <= bson::DateTime::now())
}
fn ack_state(row: &Document) -> &str {
    let status = row.get_str("status").unwrap_or_default();
    if row.get_str("decided_by").ok() != Some("access_mode")
        && (status == "pending"
            || (status == "allowed" && row.get_str("kind").ok() == Some("action")))
        && expired(row)
    {
        return "expired";
    }
    match status {
        "denied" => "denied",
        "expired" => "expired",
        "pending" => "waiting",
        "allowed" | "used" => execution_state(row),
        _ => "unavailable",
    }
}
fn effect_state(state: &str) -> &str {
    match state {
        "completed" | "pinned" => "completed",
        "denied" | "rejected" => "denied",
        "expired" => "expired",
        "pending" | "waiting" => "waiting",
        "queued" | "preparing" | "dispatched" | "publishing" => "allowed",
        "failed" | "publication_failed" | "published_unpinned" => "failed",
        "cancelled" | "not_sent" | "invalidated" => "cancelled",
        _ => "unavailable",
    }
}
fn approval_state(row: &Document) -> &str {
    if let Ok(p) = row.get_document("execution_proposal") {
        let version = p
            .get("version")
            .and_then(|v| v.as_i64().or_else(|| v.as_i32().map(i64::from)));
        let Some(version @ 1..=4) = version else {
            return "unavailable";
        };
        let state = p
            .get_document(format!("v{version}"))
            .ok()
            .and_then(|p| p.get_str("state").ok());
        if state == Some("pending") && expired(row) {
            return "expired";
        }
        return if state == Some("completed") {
            match row.get_str("activity_outcome").unwrap_or_default() {
                "completed" => "completed",
                "failed" => "failed",
                _ => "unavailable",
            }
        } else {
            state.map(effect_state).unwrap_or("unavailable")
        };
    }
    match row.get_str("status").unwrap_or_default() {
        "pending" if expired(row) => "expired",
        "pending" => "waiting",
        "rejected" => "denied",
        "expired" => "expired",
        "approved" => execution_state(row),
        _ => "unavailable",
    }
}
fn projection() -> Document {
    let mut fields = doc! {"_id":1,"status":1,"kind":1,"decided_by":1,"expires_at":1,
    "activity_execution":1,"activity_outcome":1,"authored_skill":1};
    fields.insert("execution_proposal.version", 1);
    for version in 1..=4 {
        fields.insert(format!("execution_proposal.v{version}.state"), 1);
    }
    fields
}
fn authored_skill(row: &Document) -> Option<AuthoredSkillReview> {
    bson::from_document(row.get_document("authored_skill").ok()?.clone()).ok()
}

/// ID-batched presentation reads only for new linked activities. Legacy rows
/// cause no extra queries. Never load/decrypt provider results or draft bodies.
pub async fn refresh(
    db: &Database,
    user: &str,
    conversation: &str,
    activities: Vec<&mut TurnActivity>,
) -> AppResult<()> {
    refresh_scoped(db, user, Some(conversation), activities).await
}

/// Called only after authorizing the group transcript. These IDs are already
/// published activity metadata; return states only, never private card content.
pub async fn refresh_group(
    db: &Database,
    messages: &mut [crate::models::assistant_group::GroupMessage],
) -> AppResult<()> {
    let mut owners: HashMap<String, Vec<&mut TurnActivity>> = HashMap::new();
    for message in messages {
        let owner = message
            .author_user_id
            .as_ref()
            .unwrap_or(&message.user_id)
            .clone();
        owners
            .entry(owner)
            .or_default()
            .extend(message.activities.iter_mut());
    }
    for (owner, activities) in owners {
        refresh_scoped(db, &owner, None, activities).await?;
    }
    Ok(())
}

async fn refresh_scoped(
    db: &Database,
    user: &str,
    conversation: Option<&str>,
    activities: Vec<&mut TurnActivity>,
) -> AppResult<()> {
    let mut ack_ids = Vec::new();
    let mut approval_ids = Vec::new();
    let mut consent_ids = Vec::new();
    for activity in &activities {
        if let Some(d) = &activity.decision {
            match d.kind.as_str() {
                "acknowledgement" => ack_ids.push(d.id.clone()),
                "approval" => approval_ids.push(d.id.clone()),
                "shared_consent" => consent_ids.push(d.id.clone()),
                _ => {}
            }
        }
    }
    let mut states = HashMap::new();
    if !ack_ids.is_empty() {
        ack_ids.sort();
        ack_ids.dedup();
        let mut filter = doc! {"_id":{"$in":ack_ids},"user_id":user};
        if let Some(conversation) = conversation {
            // Skill drafting can reuse the same owner's exact review card
            // across conversations. Only its already-linked state is exposed.
            filter.insert(
                "$or",
                bson::bson!([
                    {"conversation_id": conversation}, {"authored_skill": {"$ne": null}},
                ]),
            );
        }
        let mut rows: Vec<Document> = db
            .collection(ACKS)
            .find(filter)
            .projection(projection())
            .await?
            .try_collect()
            .await?;
        refresh_machines(db, user, &mut rows).await?;
        refresh_incomplete(db, user, &mut rows).await?;
        let mut proposals = Vec::new();
        for row in &rows {
            states.insert(
                (
                    "acknowledgement".to_string(),
                    row.get_str("_id").unwrap_or_default().to_owned(),
                ),
                ack_state(row).to_owned(),
            );
            if let Some(skill) = authored_skill(row) {
                proposals.push(skill.proposal_id.clone());
            }
        }
        if !proposals.is_empty() {
            let proposals: Vec<Document> = db.collection(crate::models::assistant_agent_learning::PROPOSALS_COLLECTION_NAME)
                .find(doc! {"_id":{"$in":proposals}})
                .projection(doc! {"_id":1,"agent_id":1,"revision":1,"status":1,"publication.acknowledgement_id":1})
                .await?.try_collect().await?;
            for row in &rows {
                if let Some(skill) = authored_skill(row)
                    && let Some(p) = proposals.iter().find(|p| {
                        p.get_str("_id").ok() == Some(&skill.proposal_id)
                            && p.get_str("agent_id").ok() == Some(&skill.agent_id)
                            && p.get("revision")
                                .and_then(|v| v.as_i64().or_else(|| v.as_i32().map(i64::from)))
                                .unwrap_or(0)
                                == skill.revision
                            && p.get_document("publication")
                                .ok()
                                .and_then(|p| p.get_str("acknowledgement_id").ok())
                                == row.get_str("_id").ok()
                    })
                    && matches!(
                        row.get_str("status").unwrap_or_default(),
                        "used" | "allowed"
                    )
                {
                    states.insert(
                        (
                            "acknowledgement".into(),
                            row.get_str("_id").unwrap_or_default().to_owned(),
                        ),
                        effect_state(p.get_str("status").unwrap_or_default()).into(),
                    );
                }
            }
        }
    }
    if !approval_ids.is_empty() {
        approval_ids.sort();
        approval_ids.dedup();
        let mut rows: Vec<Document> = db
            .collection(APPROVALS)
            .find(doc! {
                "_id":{"$in":approval_ids}, "$or":[{"user_id":user},{"notify_user_ids":user}],
            })
            .projection(projection())
            .await?
            .try_collect()
            .await?;
        refresh_machines(db, user, &mut rows).await?;
        refresh_incomplete(db, user, &mut rows).await?;
        for row in rows {
            states.insert(
                (
                    "approval".into(),
                    row.get_str("_id").unwrap_or_default().to_owned(),
                ),
                approval_state(&row).to_owned(),
            );
        }
    }
    if !consent_ids.is_empty() {
        consent_ids.sort();
        consent_ids.dedup();
        let mut filter = doc! {"_id":{"$in":consent_ids},"actor_user_id":user};
        if let Some(conversation) = conversation {
            filter.insert("conversation_id", conversation);
        }
        let rows: Vec<Document> = db
            .collection(crate::models::assistant_sharing::CONSENTS_COLLECTION_NAME)
            .find(filter)
            .projection(doc! {"decision":1,"expires_at":1})
            .await?
            .try_collect()
            .await?;
        for row in rows {
            let state = match row.get_str("decision").unwrap_or_default() {
                "skip" => "denied",
                "pending" | "once" if expired(&row) => "expired",
                "pending" => "waiting",
                // Consent is permission, not evidence of a completed effect.
                "once" | "always" | "consumed" => "allowed",
                _ => "unavailable",
            };
            states.insert(
                (
                    "shared_consent".into(),
                    row.get_str("_id").unwrap_or_default().to_owned(),
                ),
                state.to_owned(),
            );
        }
    }
    for activity in activities {
        if let Some(d) = &mut activity.decision {
            d.state = states
                .get(&(d.kind.clone(), d.id.clone()))
                .cloned()
                .unwrap_or_else(|| "unavailable".into());
        }
    }
    Ok(())
}

/// Connected proxy approvals use the same activity, and only a successful
/// connected decision may record an execution identity.
pub async fn approval(
    db: &Database,
    chat: &ChatAuthority,
    id: &str,
    executing: bool,
) -> AppResult<()> {
    bind(db, chat, "approval", id, executing, None).await
}

async fn refresh_machines(db: &Database, user: &str, rows: &mut [Document]) -> AppResult<()> {
    let mut receipts: Vec<(usize, crate::models::machine_receipt::MachineReceipt)> = rows
        .iter()
        .enumerate()
        .filter_map(|(i, row)| {
            let machine = row
                .get_document("activity_execution")
                .ok()?
                .get_document("machine")
                .ok()?;
            Some((i, bson::from_document(machine.clone()).ok()?))
        })
        .collect();
    super::machine_activity_service::refresh_jobs(
        db,
        user,
        receipts.iter_mut().map(|(_, r)| r).collect(),
    )
    .await?;
    for (index, receipt) in receipts {
        if let Ok(execution) = rows[index].get_document_mut("activity_execution") {
            execution.insert(
                "machine",
                bson::to_bson(&receipt).expect("receipt metadata"),
            );
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "assistant_activity_decision_tests.rs"]
mod assistant_acknowledgement_service_tests;

// A stopped/lost worker is not still waiting to execute a consumed card. Only
// unfinished occurrences need this batched live-fence read; completed evidence
// survives Stop and later turns unchanged.
async fn refresh_incomplete(db: &Database, user: &str, rows: &mut [Document]) -> AppResult<()> {
    let conversations: Vec<_> = rows
        .iter()
        .filter_map(|r| {
            let e = r.get_document("activity_execution").ok()?;
            (e.get_str("status").ok() == Some("running"))
                .then(|| e.get_str("conversation_id").ok())
                .flatten()
                .map(str::to_owned)
        })
        .collect();
    if conversations.is_empty() {
        return Ok(());
    }
    let live: Vec<Document> = db
        .collection(CONVERSATIONS)
        .find(doc! {"_id":{"$in":conversations},"user_id":user})
        .projection(doc! {"active_turn.turn_id":1,"active_turn.stop_requested":1})
        .await?
        .try_collect()
        .await?;
    for row in rows {
        let Ok(e) = row.get_document_mut("activity_execution") else {
            continue;
        };
        if e.get_str("status").ok() != Some("running") {
            continue;
        }
        let turn = live
            .iter()
            .find(|r| r.get_str("_id").ok() == e.get_str("conversation_id").ok())
            .and_then(|r| r.get_document("active_turn").ok())
            .filter(|t| t.get_str("turn_id").ok() == e.get_str("turn_id").ok());
        if let Some(turn) = turn {
            if turn.get_bool("stop_requested").unwrap_or(false) {
                e.insert("status", "cancelled");
            }
        } else {
            e.insert("status", "unknown");
        }
    }
    Ok(())
}

/// Preserve the provider stream byte-for-byte. A dropped/incomplete response
/// never becomes a confirmed successful occurrence merely from its headers.
pub async fn proxy_response(
    db: &Database,
    chat: &ChatAuthority,
    turn: &str,
    activity: &str,
    connect: bool,
    response: AppResult<axum::response::Response>,
) -> AppResult<axum::response::Response> {
    let response = match response {
        Ok(response) => response,
        Err(error) => {
            super::assistant_nyxagent::activity_finished_for_turn(
                db,
                &chat.user_id,
                &chat.conversation_id,
                turn,
                activity,
                false,
                None,
            )
            .await?;
            return Err(error);
        }
    };
    // A tunnel's HTTP body is only the handshake. Its detached stream can
    // still be running; neither 101 nor CONNECT headers prove completion.
    if response.status() == axum::http::StatusCode::SWITCHING_PROTOCOLS
        || (connect && response.status().is_success())
    {
        return Ok(response);
    }
    let (parts, body) = response.into_parts();
    let ok = parts.status.is_success();
    let db = db.clone();
    let user = chat.user_id.clone();
    let conversation = chat.conversation_id.clone();
    let turn = turn.to_owned();
    let activity = activity.to_owned();
    let stream = async_stream::stream! {
        use futures::StreamExt;
        let mut body=http_body_util::BodyStream::new(body);
        while let Some(chunk)=body.next().await {
            if chunk.is_err() {
                let _=super::assistant_nyxagent::activity_finished_for_turn(&db,&user,&conversation,&turn,&activity,false,None).await;
                yield chunk; return;
            }
            yield chunk;
        }
        let _=super::assistant_nyxagent::activity_finished_for_turn(&db,&user,&conversation,&turn,&activity,ok,None).await;
    };
    Ok(axum::response::Response::from_parts(
        parts,
        axum::body::Body::new(http_body_util::StreamBody::new(stream)),
    ))
}
