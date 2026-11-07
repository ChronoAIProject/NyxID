//! Evidence projection for the voice surface. Raw tool labels never become speech.
use crate::{
    errors::{AppError, AppResult},
    models::{
        assistant_conversation::{ActiveTurn, AssistantConversation, COLLECTION_NAME as THREADS},
        assistant_voice::{REQUESTS, RequestState, VoiceRequest, VoiceTaskProgress},
        assistant_voice_session::VoiceSession,
    },
};
use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use mongodb::{
    Database,
    bson::{self, doc},
};
use std::collections::HashMap;

pub fn pending(r: &VoiceRequest) -> bool {
    matches!(
        r.state,
        RequestState::Queued | RequestState::Claimed | RequestState::AwaitingConfirmation
    )
}

/// A closed coordinator read: only this authorized call's own task metadata,
/// never another call, hidden transcript, tool output or new execution.
pub(super) async fn task_status(
    db: &Database,
    call: &VoiceSession,
) -> AppResult<serde_json::Value> {
    use crate::models::assistant_voice::REQUESTS;
    crate::services::assistant_voice::thread(db, &call.user_id, &call.conversation_id).await?;
    let scope = doc! {"session_id":&call.id,"user_id":&call.user_id,"conversation_id":&call.conversation_id};
    let mut active = scope.clone();
    active.insert(
        "state",
        doc! {"$in":["queued","claimed","awaiting_confirmation"]},
    );
    let mut requests: Vec<VoiceRequest> = db
        .collection(REQUESTS)
        .find(active)
        .sort(doc! {"message_seq":-1})
        .limit(9)
        .await?
        .try_collect()
        .await?;
    if requests.len() < 9 {
        let mut settled = scope;
        settled.insert("state", doc! {"$in":["completed","cancelled"]});
        settled.insert(
            "_id",
            doc! {"$nin":requests.iter().map(|r|r.id.clone()).collect::<Vec<_>>()},
        );
        let recent: Vec<VoiceRequest> = db
            .collection(REQUESTS)
            .find(settled)
            .sort(doc! {"message_seq":-1})
            .limit((9 - requests.len()) as i64)
            .await?
            .try_collect()
            .await?;
        requests.extend(recent);
    }
    let has_more = requests.len() > 8;
    requests.truncate(8);
    refresh(db, call, &mut requests).await?;
    let sources: Vec<crate::models::assistant_message::AssistantMessage> = db
        .collection(crate::models::assistant_message::COLLECTION_NAME)
        .find(
            doc! {"_id":{"$in":requests.iter().map(|r|r.message_id.clone()).collect::<Vec<_>>()},
            "user_id":&call.user_id,"conversation_id":&call.conversation_id},
        )
        .await?
        .try_collect()
        .await?;
    let titles: HashMap<_, _> = sources
        .into_iter()
        .map(|m| (m.id, m.text.chars().take(120).collect::<String>()))
        .collect();
    Ok(serde_json::json!({
        "type":"task_status","observed_at":Utc::now(),"has_more":has_more,
        "tasks":requests.iter().map(|r| serde_json::json!({
            "task_id":r.id,"state":r.state,
            "title":titles.get(&r.message_id),
            "outcome":crate::services::assistant_voice::request_outcome(r),
            "progress":r.progress.as_ref().map(|p|serde_json::json!({
                "stage":p.stage,"detail":p.detail,"last_completed_step":p.last_completed_step,"updated_at":p.updated_at,
            })),
            "result_available":r.result_message_id.is_some(),
        })).collect::<Vec<_>>()
    }))
}

// Exact protocol identifiers, not intent/permission classification. Unknown
// activities stay in telemetry without becoming generic spoken progress.
pub fn activity_label(label: &str) -> Option<&'static str> {
    match label.rsplit("__").next().unwrap_or(label) {
        "search_issues" | "list_issues" | "list_user_issues" | "issue_read" => {
            Some("Checking issue progress")
        }
        _ => None,
    }
}

pub fn project(
    r: &VoiceRequest,
    turn: Option<&ActiveTurn>,
    now: DateTime<Utc>,
) -> VoiceTaskProgress {
    let activities = turn.map(|t| t.activities.as_slice()).unwrap_or_default();
    let completed: Vec<_> = activities
        .iter()
        .filter(|a| a.status == "completed" && a.failure.is_none())
        .collect();
    let last = completed.iter().max_by_key(|a| a.ended_at);
    let latest = activities.iter().max_by_key(|a| a.started_at);
    let (stage, detail) = match r.state {
        RequestState::Queued => ("queued", String::new()),
        RequestState::AwaitingConfirmation => (
            "awaiting_approval",
            r.progress
                .as_ref()
                .filter(|p| p.stage == "awaiting_approval")
                .map(|p| p.detail.clone())
                .unwrap_or_else(|| "Waiting for your approval of the pending action".into()),
        ),
        RequestState::Completed => ("result_ready", String::new()),
        RequestState::Cancelled => (
            "ended",
            terminal_detail(r.terminal_error_code.as_deref()).into(),
        ),
        RequestState::Claimed if r.async_operation_pending => ("waiting_service", String::new()),
        RequestState::Claimed
            if turn.is_some_and(|t| {
                now - t.heartbeat_at.unwrap_or(t.started_at) > chrono::Duration::seconds(45)
            }) =>
        {
            ("checking_status", String::new())
        }
        RequestState::Claimed if turn.is_some_and(|t| t.waiting_for_capacity) => {
            ("waiting_capacity", String::new())
        }
        RequestState::Claimed
            if latest.is_some_and(|a| a.failure.is_some() || a.status == "error") =>
        {
            ("tool_error", String::new())
        }
        RequestState::Claimed if latest.is_some_and(|a| a.status == "running") => (
            "working",
            activity_label(&latest.unwrap().label)
                .unwrap_or_default()
                .into(),
        ),
        RequestState::Claimed if !completed.is_empty() => ("working", String::new()),
        RequestState::Claimed if turn.is_some() => ("planning", String::new()),
        RequestState::Claimed => ("checking_status", String::new()),
    };
    VoiceTaskProgress {
        stage: stage.into(),
        detail,
        last_completed_step: last
            .and_then(|a| activity_label(&a.label).map(str::to_owned))
            .or_else(|| {
                turn.is_none()
                    .then(|| r.progress.as_ref()?.last_completed_step.clone())
                    .flatten()
            }),
        completed_steps: if turn.is_some() {
            completed.len()
        } else {
            r.progress.as_ref().map_or(0, |p| p.completed_steps)
        },
        updated_at: now,
    }
}

pub fn terminal_detail(code: Option<&str>) -> &'static str {
    match code {
        Some("cancelled") | None => {
            "This task was stopped. Any earlier actions may already have completed."
        }
        Some("confirmation_expired") => {
            "Approval expired before this task could continue. Ask again when you are ready."
        }
        Some("acknowledgement_denied" | "permission_denied" | "forbidden" | "access_denied") => {
            "Permission was refused, so the task could not finish. Review access before trying again."
        }
        Some("voice_request_expired" | "timeout" | "turn_timeout") => {
            "The task timed out before returning an answer. Check completed actions before trying again."
        }
        Some("turn_lost") => {
            "The task was interrupted by a server restart. Check completed actions before trying again."
        }
        Some("model_authority_changed") => {
            "Access settings changed during the task. Review completed actions before trying again."
        }
        _ => {
            "The task failed before returning an answer. Some actions may already have finished; check them before retrying."
        }
    }
}

pub async fn refresh(
    db: &Database,
    call: &VoiceSession,
    requests: &mut [VoiceRequest],
) -> AppResult<()> {
    if requests.is_empty() {
        return Ok(());
    }
    let ids: Vec<_> = requests
        .iter()
        .filter(|r| pending(r))
        .map(|r| {
            r.task_conversation_id
                .as_deref()
                .unwrap_or(&r.conversation_id)
        })
        .collect();
    let rows: Vec<AssistantConversation> = db
        .collection(THREADS)
        .find(doc! {"_id":{"$in":ids},"user_id":&call.user_id})
        .await?
        .try_collect()
        .await?;
    let turns: HashMap<_, _> = rows
        .into_iter()
        .filter_map(|r| r.active_turn.map(|t| (t.turn_id.clone(), t)))
        .collect();
    for r in requests {
        let turn = turns.get(&r.turn_id);
        let mut next = project(r, turn, Utc::now());
        if let Some(old) = &r.progress {
            next.updated_at = old.updated_at;
        }
        if r.progress.as_ref() != Some(&next) {
            next.updated_at = Utc::now();
            db.collection::<bson::Document>(REQUESTS)
                .update_one(
                    doc! {"_id":&r.id,"state":bson::to_bson(&r.state).map_err(|_|AppError::Internal("Voice progress serialization failed".into()))?},
                    doc! {"$set":{"progress":bson::to_bson(&next).map_err(|_|AppError::Internal("Voice progress serialization failed".into()))?}},
                )
                .await?;
            r.progress = Some(next);
        }
        if let Some(first) = turn.and_then(|t| t.activities.iter().min_by_key(|a| a.started_at)) {
            let ms = (first.started_at - call.created_at)
                .num_milliseconds()
                .max(0);
            mark(db, r, "first_activity_ms", ms).await?;
            mark(db, r, "tool_started_ms", ms).await?;
        }
        if let Some(at) = turn.and_then(|t| t.activities.iter().filter_map(|a| a.ended_at).min()) {
            mark(
                db,
                r,
                "first_tool_finished_ms",
                (at - call.created_at).num_milliseconds().max(0),
            )
            .await?;
        }
        if r.state == RequestState::AwaitingConfirmation {
            mark(
                db,
                r,
                "approval_wait_ms",
                (Utc::now() - call.created_at).num_milliseconds(),
            )
            .await?;
        }
    }
    Ok(())
}

pub async fn mark(
    db: &Database,
    r: &mut VoiceRequest,
    name: &'static str,
    ms: i64,
) -> AppResult<()> {
    if r.timing.contains_key(name) {
        return Ok(());
    }
    // All callers use closed static names. $min keeps the earliest observation
    // under duplicate provider events, retries and a newer lease generation.
    db.collection::<bson::Document>(REQUESTS)
        .update_one(
            doc! {"_id":&r.id,"user_id":&r.user_id},
            doc! {"$min":{format!("timing.{name}"):ms.max(0)}},
        )
        .await?;
    r.timing.insert(name.into(), ms.max(0));
    Ok(())
}

#[derive(Default)]
pub struct Coalescer {
    sent: HashMap<String, String>,
    last_ms: Option<i64>,
}
impl Coalescer {
    pub fn due(&mut self, r: &VoiceRequest, elapsed_ms: i64, age_ms: i64) -> bool {
        let Some(p) = &r.progress else { return false };
        let key = format!("{}:{}", p.stage, p.detail);
        if p.detail.trim().is_empty()
            || age_ms < 2000
            || self.last_ms.is_some_and(|t| elapsed_ms - t < 8000)
            || self.sent.get(&r.id) == Some(&key)
        {
            return false;
        }
        self.sent.insert(r.id.clone(), key);
        self.last_ms = Some(elapsed_ms);
        true
    }
}

pub fn idle_seconds(call: &VoiceSession, now: DateTime<Utc>) -> i64 {
    (now - call
        .last_work_at
        .unwrap_or(call.last_user_at)
        .max(call.last_user_at))
    .num_seconds()
}

pub async fn keep_work_alive(
    db: &Database,
    call: &mut VoiceSession,
    pending: bool,
) -> AppResult<()> {
    if pending {
        *call =
            super::session::write(db, call, doc! {"last_work_at":bson::DateTime::now()}).await?;
    }
    Ok(())
}

pub fn close_notice(reason: Option<&str>, continuing: bool) -> String {
    let reason = match reason {
        Some("time_limit") => "The call reached its time limit.",
        Some("idle") => "The quiet call ended.",
        Some("client_lost" | "sideband_lost" | "voice_connection_lost" | "provider_closed") => {
            "The voice connection ended."
        }
        Some("credential_changed" | "access_refused") => {
            "The call ended because its access changed."
        }
        Some("server_update") => "The call ended during a server update.",
        _ => "The audio call ended.",
    };
    if continuing {
        format!(
            "{reason} Work already started will continue, subject to its permissions and limits."
        )
    } else {
        reason.into()
    }
}
