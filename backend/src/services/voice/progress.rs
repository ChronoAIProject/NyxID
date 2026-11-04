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

// Exact protocol identifiers, not intent/permission classification. Unknown
// providers still get truthful stage/count evidence without leaking identifiers.
pub fn activity_label(label: &str) -> &'static str {
    match label.rsplit("__").next().unwrap_or(label) {
        "list_connected_services" => "Checking connected services",
        "get_authenticated_user" => "Reading account identity",
        "search_issues" | "list_issues" | "list_user_issues" | "issue_read" => {
            "Checking issue progress"
        }
        "search_tools" | "tool_search" => "Finding the available service tools",
        _ => "Running a task step",
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
        RequestState::Queued => (
            "queued",
            "I am waiting for resources to start your request".to_string(),
        ),
        RequestState::AwaitingConfirmation => (
            "awaiting_approval",
            r.progress
                .as_ref()
                .filter(|p| p.stage == "awaiting_approval")
                .map(|p| p.detail.clone())
                .unwrap_or_else(|| "Waiting for your approval of the pending action".into()),
        ),
        RequestState::Completed => ("result_ready", "The answer is ready in this chat".into()),
        RequestState::Cancelled => (
            "ended",
            terminal_detail(r.terminal_error_code.as_deref()).into(),
        ),
        RequestState::Claimed if r.async_operation_pending => (
            "waiting_service",
            "The service is still processing the request".into(),
        ),
        RequestState::Claimed
            if turn.is_some_and(|t| {
                now - t.heartbeat_at.unwrap_or(t.started_at) > chrono::Duration::seconds(45)
            }) =>
        {
            (
                "checking_status",
                "Waiting for a fresh execution update".into(),
            )
        }
        RequestState::Claimed if turn.is_some_and(|t| t.waiting_for_capacity) => {
            ("waiting_capacity", "Waiting for model capacity".into())
        }
        RequestState::Claimed
            if latest.is_some_and(|a| a.failure.is_some() || a.status == "error") =>
        {
            (
                "tool_error",
                "A service step failed; I am checking whether I can continue".into(),
            )
        }
        RequestState::Claimed if latest.is_some_and(|a| a.status == "running") => {
            ("working", activity_label(&latest.unwrap().label).into())
        }
        RequestState::Claimed if !completed.is_empty() => (
            "working",
            format!(
                "{} task steps finished; the task is still running",
                completed.len()
            ),
        ),
        RequestState::Claimed if turn.is_some() => {
            ("planning", "I am working on your request".into())
        }
        RequestState::Claimed => (
            "checking_status",
            "Waiting for a fresh execution update".into(),
        ),
    };
    VoiceTaskProgress {
        stage: stage.into(),
        detail,
        last_completed_step: last.map(|a| activity_label(&a.label).into()).or_else(|| {
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
            "The task failed before returning an answer. The error details are in this chat; check completed actions before retrying."
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
    sent: HashMap<String, (String, usize)>,
    last_ms: Option<i64>,
}
impl Coalescer {
    pub fn due(&mut self, r: &VoiceRequest, elapsed_ms: i64, age_ms: i64) -> bool {
        let Some(p) = &r.progress else { return false };
        let key = (format!("{}:{}", p.stage, p.detail), p.completed_steps);
        if age_ms < 2000
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
            "{reason} Accepted tasks continue in the background, subject to their permissions and limits. Their answers or failure details will appear in this chat. Pending approvals remain available here."
        )
    } else {
        format!("{reason} Task results and any failure details are separate messages in this chat.")
    }
}
