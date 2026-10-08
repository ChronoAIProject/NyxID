//! Durable, owner/turn-bound receipts for guidance to a running NyxAgent response.
use super::{api_key_mutation_service as transactions, assistant_nyxagent as engine};
use crate::{
    errors::{AppError, AppResult},
    models::{
        assistant_conversation::{
            AssistantConversation, COLLECTION_NAME as CONVERSATIONS, RunningResponse, TurnOrigin,
        },
        assistant_message::{AssistantMessage, COLLECTION_NAME as MESSAGES, Steering},
    },
};
use chrono::Utc;
use mongodb::{
    Database,
    bson::{self, doc},
};
use uuid::Uuid;

pub const MAX_CHARS: usize = 32_000;

pub fn unavailable(row: &AssistantConversation, turn_id: &str) -> Option<&'static str> {
    if row.guest_turn
        || row.group_id.is_some()
        || row.voice_parent_conversation_id.is_some()
        || row.channel.is_some()
    {
        return Some("steer_unsupported");
    }
    let Some(turn) = engine::live_turn(row, Utc::now()).filter(|t| t.turn_id == turn_id) else {
        return Some("no_active_turn");
    };
    if turn.stop_requested {
        return Some("stop_pending");
    }
    if turn.heartbeat_at.is_some_and(|at| {
        at <= Utc::now() - chrono::Duration::seconds(engine::ACTIVE_TURN_HEARTBEAT_STALE_SECS)
    }) {
        return Some("no_active_turn");
    }
    if turn.origin == TurnOrigin::Channel || turn.voice_request_id.is_some() {
        return Some("steer_unsupported");
    }
    match &turn.running_response {
        None => return Some("starting"),
        Some(response) if response.credential_api_key_id != row.credential_api_key_id => {
            return Some("steer_unavailable");
        }
        Some(_) => {}
    }
    None
}

/// Called before each request and immediately at response.created. The turn ID
/// and Stop flag fence late stream frames and continuation gaps on all replicas.
pub async fn set_running_response(
    db: &Database,
    row: &AssistantConversation,
    response: Option<&RunningResponse>,
) -> AppResult<bool> {
    let turn = row.active_turn.as_ref().expect("claimed turn");
    let mut filter = doc! {"_id": &row.id, "user_id": &row.user_id, "active_turn.turn_id": &turn.turn_id, "active_turn.stop_requested": false};
    if let Some(response) = response {
        filter.insert("credential_api_key_id", &response.credential_api_key_id);
    }
    Ok(db
        .collection::<AssistantConversation>(CONVERSATIONS)
        .update_one(
            filter,
            doc! {"$set": {"active_turn.running_response": bson::to_bson(&response).map_err(|_| AppError::Internal("Failed to encode running response".into()))?}},
        )
        .await?
        .matched_count
        == 1)
}

pub enum Admission {
    Dispatch(AssistantMessage),
    Replay(AssistantMessage),
    Refused(&'static str),
}

/// The write before dispatch is also an at-most-once barrier. If this process
/// dies after reserving, retries report uncertainty instead of sending again.
/// This is stricter than upstream's turn-local idempotency and survives restarts.
pub async fn reserve(
    db: &Database,
    owner: &str,
    id: &str,
    turn_id: &str,
    client_id: &str,
    text: &str,
) -> AppResult<Admission> {
    let db = db.clone();
    let (owner, id, turn_id, client_id, text) = (
        owner.to_owned(),
        id.to_owned(),
        turn_id.to_owned(),
        client_id.to_owned(),
        text.to_owned(),
    );
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let operation: AppResult<_> = Box::pin(async {
            let messages = db.collection::<AssistantMessage>(MESSAGES);
            if let Some(saved) = messages.find_one(doc! {"user_id": &owner, "conversation_id": &id, "steering.client_request_id": &client_id}).session(&mut *session).await? {
                return Ok(if saved.text == text && saved.turn_id == turn_id {
                    Admission::Replay(saved)
                } else { Admission::Refused("idempotency_conflict") });
            }
            let conversations = db.collection::<AssistantConversation>(CONVERSATIONS);
            let Some(mut row) = conversations.find_one(doc! {"_id": &id, "user_id": &owner}).session(&mut *session).await? else {
                return Ok(Admission::Refused("not_found"));
            };
            if let Some(reason) = unavailable(&row, &turn_id) { return Ok(Admission::Refused(reason)); }
            let response = row.active_turn.as_ref().unwrap().running_response.as_ref().unwrap();
            let now = Utc::now();
            let message = AssistantMessage {
                steering: Some(Box::new(Steering {
                    finalized: false,
                    client_request_id: client_id.clone(), response_id: response.response_id.clone(), session_id: response.session_id.clone(),
                    outcome: "may_not_have_applied".into(), code: Some("steer_unavailable".into()), http_status: 503,
                })),
                voice: None, execution_pending: false, id: Uuid::new_v4().to_string(),
                conversation_id: id.clone(), user_id: owner.clone(), seq: row.message_count + 1,
                turn_id: turn_id.clone(), role: "user".into(), text: text.clone(), status: "completed".into(),
                error_code: None, created_at: now, activities: vec![], attachments: vec![], origin: row.active_turn.as_ref().map(|turn| turn.origin), via: None,
            };
            row.message_count += 1;
            row.updated_at = now;
            conversations.replace_one(doc! {"_id": &id, "user_id": &owner}, &row).session(&mut *session).await?;
            messages.insert_one(&message).session(&mut *session).await?;
            Ok(Admission::Dispatch(message))
        }).await;
        transactions::transaction_result(operation)
    }).await.map_err(transactions::map_transaction_error)
}

/// A concurrent retry waits for the first request's durable outcome. It never
/// performs a second upstream call, even after the response/turn has changed.
pub async fn replay(db: &Database, mut message: AssistantMessage) -> AppResult<AssistantMessage> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(16);
    while !message
        .steering
        .as_ref()
        .is_some_and(|receipt| receipt.finalized)
        && tokio::time::Instant::now() < deadline
    {
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        let Some(current) = db
            .collection::<AssistantMessage>(MESSAGES)
            .find_one(doc! {"_id": &message.id, "user_id": &message.user_id})
            .await?
        else {
            break;
        };
        message = current;
    }
    Ok(message)
}

pub async fn settle(
    db: &Database,
    message: &AssistantMessage,
    outcome: &str,
    code: Option<&str>,
    status: u16,
) -> AppResult<()> {
    db.collection::<AssistantMessage>(MESSAGES).update_one(
        doc! {"_id": &message.id, "user_id": &message.user_id, "conversation_id": &message.conversation_id},
        doc! {"$set": {"steering.finalized": true, "steering.outcome": outcome, "steering.code": code, "steering.http_status": i32::from(status)}},
    ).await?;
    // Assistant live notifications watch conversation metadata. Wake other
    // tabs even when the response ended before this acknowledgement arrived.
    db.collection::<AssistantConversation>(CONVERSATIONS)
        .update_one(
            doc! {"_id": &message.conversation_id, "user_id": &message.user_id},
            doc! {"$set": {"updated_at": bson::DateTime::from_chrono(Utc::now())}},
        )
        .await?;
    Ok(())
}
