use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::assistant_message_feedback::{AssistantMessageFeedback, FeedbackReplyContext},
    mw::auth::AuthUser,
    services::{
        assistant_feedback_service as feedback_service, assistant_nyxagent as engine, audit_service,
    },
};
use axum::{
    Json,
    extract::{Path, Query, State},
};
use chrono::{DateTime, SecondsFormat, Utc};
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct FeedbackInput {
    pub vote: String,
    pub comment: Option<String>,
}
#[derive(Serialize, Clone)]
pub struct FeedbackResponse {
    pub vote: String,
    pub comment: Option<String>,
    pub updated_at: DateTime<Utc>,
}
impl From<AssistantMessageFeedback> for FeedbackResponse {
    fn from(v: AssistantMessageFeedback) -> Self {
        Self {
            vote: v.vote,
            comment: v.comment,
            updated_at: v.updated_at,
        }
    }
}

impl From<feedback_service::FeedbackState> for FeedbackResponse {
    fn from(v: feedback_service::FeedbackState) -> Self {
        Self {
            vote: v.vote,
            comment: v.comment,
            updated_at: v.updated_at,
        }
    }
}

/// Attach the acting person's votes to one page of transcript messages.
pub(crate) fn attach<M>(
    messages: &mut [M],
    states: Vec<feedback_service::FeedbackState>,
    slot: impl Fn(&mut M) -> (&str, &mut Option<FeedbackResponse>),
) {
    for message in messages {
        let (id, feedback) = slot(message);
        *feedback = states
            .iter()
            .find(|state| state.message_id == id)
            .cloned()
            .map(Into::into);
    }
}

fn sanitize_comment(value: &str) -> String {
    value
        .chars()
        .filter(|c| {
            !c.is_control()
                && !matches!(
                    *c,
                    '\u{200b}'
                        | '\u{200c}'
                        | '\u{200d}'
                        | '\u{200e}'
                        | '\u{200f}'
                        | '\u{202a}'
                        | '\u{202b}'
                        | '\u{202c}'
                        | '\u{202d}'
                        | '\u{202e}'
                        | '\u{2060}'
                        | '\u{2066}'
                        | '\u{2067}'
                        | '\u{2069}'
                )
        })
        .collect()
}

fn validate(input: &FeedbackInput) -> AppResult<Option<String>> {
    if input.vote != "up" && input.vote != "down" {
        return Err(AppError::BadRequest("feedback_vote_invalid".into()));
    }
    let comment = input
        .comment
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| {
            let s = sanitize_comment(s);
            if s.chars().count() > 1000 {
                return Err(AppError::BadRequest("feedback_comment_too_long".into()));
            }
            Ok(s.to_string())
        })
        .transpose()?;
    Ok(comment)
}
async fn enforce_feedback_limit(state: &AppState, user_id: &str) -> AppResult<()> {
    let limiter = crate::mw::rate_limit::PerKeyRateLimiter::with_db(
        state.db.clone(),
        "assistant_feedback",
        30,
        60,
    );
    if !limiter
        .check_shared(&format!("assistant-feedback:{user_id}"))
        .await?
    {
        return Err(AppError::RateLimited);
    }
    Ok(())
}
async fn upsert(
    state: &AppState,
    auth: &AuthUser,
    message_id: &str,
    conversation_id: Option<String>,
    group_id: Option<String>,
    input: FeedbackInput,
) -> AppResult<Json<FeedbackResponse>> {
    let comment = validate(&input)?;
    let user_id = auth.user_id.to_string();
    enforce_feedback_limit(state, &user_id).await?;
    engine::require_enabled(&state.db, &user_id).await?;
    let (turn_id, eligible, context) = if let Some(cid) = conversation_id.as_deref() {
        let (conversation, _) = engine::get_authorized(&state.db, &user_id, cid).await?;
        engine::authorize_thread_read(&state.db, &conversation).await?;
        let row = feedback_service::conversation_message(&state.db, &user_id, cid, message_id)
            .await?
            .ok_or_else(|| AppError::NotFound("message not found".into()))?;
        let eligible = row.role == "assistant" && row.status == "completed";
        let context = if eligible {
            Some(feedback_service::reply_context(&state.db, &row, &conversation).await?)
        } else {
            None
        };
        (Some(row.turn_id), eligible, context)
    } else {
        let gid = group_id
            .as_deref()
            .ok_or_else(|| AppError::NotFound("message not found".into()))?;
        crate::services::org_group_service::get(&state.db, &user_id, gid, None).await?;
        let row = feedback_service::group_message(&state.db, gid, message_id)
            .await?
            .ok_or_else(|| AppError::NotFound("message not found".into()))?;
        let eligible = row.role == "agent";
        let context = eligible.then(|| feedback_service::group_reply_context(&row));
        (None, eligible, context)
    };
    if !eligible {
        return Err(AppError::BadRequest("assistant_feedback_ineligible".into()));
    }
    let (row, previous) = feedback_service::upsert_feedback(
        &state.db,
        feedback_service::FeedbackWrite {
            user_id,
            message_id: message_id.to_owned(),
            conversation_id,
            group_id,
            turn_id,
            vote: input.vote,
            comment,
            comment_present: input.comment.is_some(),
            context,
        },
    )
    .await?;
    if let Some(event) = feedback_service::classify(previous.as_ref(), &row) {
        feedback_service::log_event(event, &row, previous.as_ref());
        audit_service::log_for_user(
            state.db.clone(),
            auth,
            "assistant_feedback_submitted",
            Some(feedback_service::audit_details(
                event,
                &row,
                previous.as_ref(),
            )),
        );
    }
    Ok(Json(row.into()))
}
pub async fn put_conversation(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((id, message_id)): Path<(String, String)>,
    Json(input): Json<FeedbackInput>,
) -> AppResult<Json<FeedbackResponse>> {
    upsert(&state, &auth, &message_id, Some(id), None, input).await
}
pub async fn put_group(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((id, message_id)): Path<(String, String)>,
    Json(input): Json<FeedbackInput>,
) -> AppResult<Json<FeedbackResponse>> {
    upsert(&state, &auth, &message_id, None, Some(id), input).await
}
async fn withdraw(
    state: &AppState,
    auth: &AuthUser,
    message_id: String,
    scope: Option<String>,
    group: bool,
) -> AppResult<StatusCode> {
    let user_id = auth.user_id.to_string();
    enforce_feedback_limit(state, &user_id).await?;
    engine::require_enabled(&state.db, &user_id).await?;
    if let Some(cid) = scope.as_deref() {
        if group {
            crate::services::org_group_service::get(&state.db, &user_id, cid, None).await?;
            if feedback_service::group_message(&state.db, cid, &message_id)
                .await?
                .is_none()
            {
                return Err(AppError::NotFound("message not found".into()));
            }
        } else {
            let (conversation, _) = engine::get_authorized(&state.db, &user_id, cid).await?;
            engine::authorize_thread_read(&state.db, &conversation).await?;
            let row = feedback_service::conversation_message(&state.db, &user_id, cid, &message_id)
                .await?
                .ok_or_else(|| AppError::NotFound("message not found".into()))?;
            if row.role != "assistant" || row.status != "completed" {
                return Err(AppError::BadRequest("assistant_feedback_ineligible".into()));
            }
        }
    }
    let deleted = feedback_service::delete_feedback(&state.db, &user_id, &message_id).await?;
    let Some(deleted) = deleted else {
        return Ok(StatusCode::NO_CONTENT);
    };
    let event = feedback_service::FeedbackEvent::Withdrawn;
    feedback_service::log_event(event, &deleted, Some(&deleted));
    audit_service::log_for_user(
        state.db.clone(),
        auth,
        "assistant_feedback_withdrawn",
        Some(feedback_service::audit_details(
            event,
            &deleted,
            Some(&deleted),
        )),
    );
    Ok(StatusCode::NO_CONTENT)
}
use axum::http::StatusCode;
pub async fn delete_conversation(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((id, message_id)): Path<(String, String)>,
) -> AppResult<StatusCode> {
    withdraw(&state, &auth, message_id, Some(id), false).await
}
pub async fn delete_group(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((id, message_id)): Path<(String, String)>,
) -> AppResult<StatusCode> {
    withdraw(&state, &auth, message_id, Some(id), true).await
}

#[derive(Default, Deserialize)]
pub struct AdminQuery {
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    pub vote: Option<String>,
    pub cursor: Option<String>,
    pub model: Option<String>,
    pub agent_kind: Option<String>,
    pub has_error: Option<bool>,
}
#[derive(Serialize)]
pub struct AdminResponse {
    pub counts: Vec<feedback_service::AdminCount>,
    pub recent: Vec<AdminFeedbackRow>,
    pub next_cursor: Option<String>,
}
/// Admin projection of the stored reply snapshot: metadata only.
#[derive(Serialize)]
pub struct FeedbackContextResponse {
    pub surface: Option<String>,
    pub agent_id: Option<String>,
    pub agent_kind: Option<String>,
    pub model: Option<String>,
    pub access_mode: Option<String>,
    pub reply_status: Option<String>,
    pub error_code: Option<String>,
    pub origin: Option<String>,
    pub via: Option<String>,
    pub turn_id: Option<String>,
    pub reply_seq: Option<i64>,
    pub reply_created_at: Option<DateTime<Utc>>,
    pub latency_ms: Option<i64>,
    pub tool_calls: u32,
    pub tool_errors: u32,
    pub tool_labels: Vec<String>,
    pub machine_used: bool,
    pub attachments: u32,
    pub steered: bool,
    pub voice: bool,
    pub reply_chars: u32,
    pub context_reset_since_reply: bool,
    pub org_group: bool,
}

impl From<FeedbackReplyContext> for FeedbackContextResponse {
    fn from(c: FeedbackReplyContext) -> Self {
        Self {
            surface: c.surface,
            agent_id: c.agent_id,
            agent_kind: c.agent_kind,
            model: c.model,
            access_mode: c.access_mode,
            reply_status: c.reply_status,
            error_code: c.error_code,
            origin: c.origin,
            via: c.via,
            turn_id: c.turn_id,
            reply_seq: c.reply_seq,
            reply_created_at: c.reply_created_at,
            latency_ms: c.latency_ms,
            tool_calls: c.tool_calls,
            tool_errors: c.tool_errors,
            tool_labels: c.tool_labels,
            machine_used: c.machine_used,
            attachments: c.attachments,
            steered: c.steered,
            voice: c.voice,
            reply_chars: c.reply_chars,
            context_reset_since_reply: c.context_reset_since_reply,
            org_group: c.org_group,
        }
    }
}

#[derive(Serialize)]
pub struct AdminFeedbackRow {
    pub context: Option<FeedbackContextResponse>,
    pub message_id: String,
    pub vote: String,
    pub comment: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}
pub async fn admin(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(q): Query<AdminQuery>,
) -> AppResult<Json<AdminResponse>> {
    super::admin_helpers::require_admin_or_operator(&state, &auth, "admin.assistant.feedback")
        .await?;
    let mut filter = doc! {};
    if let Some(v) = q.vote {
        filter.insert("vote", v);
    }
    if let Some(v) = q.model {
        filter.insert("context.model", v);
    }
    if let Some(v) = q.agent_kind {
        filter.insert("context.agent_kind", v);
    }
    if q.has_error == Some(true) {
        filter.insert("context.error_code", doc! {"$nin": [bson::Bson::Null, ""]});
    }
    if let Some(f) = q.from {
        filter.insert("created_at", doc! {"$gte":bson::DateTime::from_chrono(f)});
    }
    if let Some(t) = q.to {
        filter
            .entry("created_at".to_string())
            .or_insert(doc! {}.into());
        if let Ok(b) = filter.get_document_mut("created_at") {
            b.insert("$lte", bson::DateTime::from_chrono(t));
        }
    }
    if let Some(cursor) = q.cursor.as_deref() {
        let (dt_raw, id) = cursor.split_once('|').unwrap_or(("", ""));
        let dt = (!dt_raw.is_empty())
            .then_some(dt_raw)
            .and_then(|v| DateTime::parse_from_rfc3339(v).ok());
        let id = (!id.is_empty()).then_some(id);
        let (dt, id) = match (dt, id) {
            (Some(dt), Some(id)) => (dt.with_timezone(&Utc), id),
            _ => return Err(AppError::BadRequest("feedback_cursor_invalid".into())),
        };
        filter.insert(
            "$or",
            vec![
                doc! {"created_at": {"$lt": bson::DateTime::from_chrono(dt)}},
                doc! {"created_at": bson::DateTime::from_chrono(dt), "_id": {"$lt": id}},
            ],
        );
    }
    let mut aggregate_filter = filter.clone();
    aggregate_filter.remove("$or");
    let counts = feedback_service::admin_counts(&state.db, aggregate_filter).await?;
    let rows = feedback_service::admin_recent(&state.db, filter, 100).await?;
    let recent = rows
        .iter()
        .map(|r| AdminFeedbackRow {
            context: r.context.clone().map(Into::into),
            message_id: r.message_id.clone(),
            vote: r.vote.clone(),
            comment: r.comment.clone(),
            created_at: r.created_at.to_rfc3339(),
            updated_at: r.updated_at.to_rfc3339(),
        })
        .collect::<Vec<_>>();
    let next_cursor = (rows.len() == 100)
        .then(|| {
            rows.last().map(|r| {
                format!(
                    "{}|{}",
                    r.created_at.to_rfc3339_opts(SecondsFormat::Millis, true),
                    r.id
                )
            })
        })
        .flatten();
    Ok(Json(AdminResponse {
        counts,
        recent,
        next_cursor,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn per_user_feedback_burst_is_limited_independently() {
        let limiter = crate::mw::rate_limit::PerKeyRateLimiter::new(2, 60);
        assert!(
            limiter
                .check_shared("assistant-feedback:user-a")
                .await
                .unwrap()
        );
        assert!(
            limiter
                .check_shared("assistant-feedback:user-a")
                .await
                .unwrap()
        );
        assert!(
            !limiter
                .check_shared("assistant-feedback:user-a")
                .await
                .unwrap()
        );
        assert!(
            limiter
                .check_shared("assistant-feedback:user-b")
                .await
                .unwrap()
        );
    }

    #[test]
    fn validate_accepts_votes_and_bounds_comments() {
        let input = FeedbackInput {
            vote: "up".into(),
            comment: Some(format!("  {}  ", "x".repeat(1_000))),
        };
        let comment = validate(&input).unwrap().unwrap();
        assert_eq!(comment.len(), 1_000);
        assert!(comment.chars().all(|c| c == 'x'));
    }

    #[test]
    fn validate_rejects_comments_over_bound() {
        let error = validate(&FeedbackInput {
            vote: "up".into(),
            comment: Some("x".repeat(1_001)),
        })
        .unwrap_err();
        assert!(matches!(error, AppError::BadRequest(code) if code == "feedback_comment_too_long"));
    }

    #[test]
    fn validate_rejects_unknown_vote_with_stable_code() {
        let error = validate(&FeedbackInput {
            vote: "maybe".into(),
            comment: None,
        })
        .unwrap_err();
        assert!(matches!(error, AppError::BadRequest(code) if code == "feedback_vote_invalid"));
    }

    #[test]
    fn empty_comment_is_omitted() {
        assert!(
            validate(&FeedbackInput {
                vote: "down".into(),
                comment: Some("  \n".into())
            })
            .unwrap()
            .is_none()
        );
    }

    #[test]
    fn response_does_not_expose_owner_or_message_text() {
        let now = Utc::now();
        let response: FeedbackResponse = AssistantMessageFeedback {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: "u".into(),
            conversation_id: Some("c".into()),
            group_id: None,
            message_id: "m".into(),
            turn_id: Some("t".into()),
            vote: "up".into(),
            comment: None,
            context: None,
            created_at: now,
            updated_at: now,
        }
        .into();
        let json = serde_json::to_value(response).unwrap();
        assert_eq!(json.get("vote").and_then(|v| v.as_str()), Some("up"));
        assert!(json.get("user_id").is_none());
        assert!(json.get("message_id").is_none());
    }
}

#[cfg(test)]
mod input_tests {
    use super::sanitize_comment;
    #[test]
    fn sanitize_comment_strips_controls_and_bidi() {
        assert_eq!(sanitize_comment("ok\u{0007}\u{202e}text"), "oktext");
    }
}
