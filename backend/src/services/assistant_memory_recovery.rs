//! Correlate native remember calls using the live turn's own activity rows.
use hmac::{Hmac, Mac};
use mongodb::bson::{self, Bson, Document, doc};
use serde_json::{Value, json};
use sha2::Sha256;
use uuid::Uuid;

use crate::models::assistant_conversation::{COLLECTION_NAME, MemoryRecovery};

use super::{
    assistant_acknowledgement_service::ChatAuthority, machine_activity_service::ACTIVITY_ID,
};

pub const MAX_PARTS: u8 = 10;

// Storage and internal errors can mean the write committed. An unconfirmed
// attempt must never make its slot available for another write.
const REJECTED: &[&str] = &[
    "note_too_long",
    "note_empty",
    "secret_detected",
    "invalid_note",
    "memory_full",
    "replace_target_not_found",
    "memory_conflict",
    "memory_unauthorized",
];

pub(crate) fn unavailable() -> (Value, bool) {
    (
        json!({
            "error": "memory_recovery_unavailable",
            "message": "This recovery part could not be admitted. Check the original failed write and any saved parts before retrying. Do not resend a part with an uncertain outcome.",
        }),
        true,
    )
}

fn write_target(state: &crate::AppState, chat: &ChatAuthority, args: &Value) -> Option<Document> {
    let target = match args.get("agent") {
        None => chat.agent_id.as_str(),
        Some(agent) => agent.as_str()?,
    };
    if target != chat.agent_id && (!chat.is_orchestrator() || Uuid::parse_str(target).is_err()) {
        return None;
    }
    let replacement = match args.get("replace_id") {
        None => Bson::Null,
        Some(id) => {
            let id = id
                .as_str()
                .filter(|id| (1..=64).contains(&id.chars().count()))?;
            // A malformed replacement selector could contain sensitive text.
            // Store only its domain-separated keyed digest, never the input.
            let mut mac =
                <Hmac<Sha256> as Mac>::new_from_slice(state.audit_chain_hmac_key.as_slice())
                    .expect("HMAC accepts arbitrary key lengths");
            mac.update(b"assistant-memory-replacement\0");
            mac.update(id.as_bytes());
            Bson::String(hex::encode(mac.finalize().into_bytes()))
        }
    };
    Some(doc! {
        "agent_id": target,
        "private": chat.org_agent_access.as_ref().is_some_and(|access| access.shared().is_some()),
        "replacement": replacement,
    })
}

fn scope(chat: &ChatAuthority) -> Option<(String, Document)> {
    let turn_id = chat.turn_id.as_ref()?;
    let activity_id = ACTIVITY_ID.try_with(Clone::clone).ok().flatten()?;
    let scope = doc! {
        "_id": &chat.conversation_id,
        "user_id": &chat.user_id,
        "active_turn.turn_id": turn_id,
        "active_turn.stop_requested": {"$ne": true},
        "active_turn.activities": {"$elemMatch": {"id": &activity_id, "status": "running"}},
    };
    Some((activity_id, scope))
}

pub(crate) async fn record_rejection(
    state: &crate::AppState,
    chat: &ChatAuthority,
    args: &Value,
    normalized: Option<&Value>,
    value: &mut Value,
) {
    if args.get("recovery").is_some()
        || !value["details"]["reason"].as_str().is_some_and(|reason| {
            REJECTED.contains(&reason)
                && !matches!(reason, "memory_unauthorized" | "replace_target_not_found")
        })
    {
        return;
    }
    let Some((activity_id, scope)) = scope(chat) else {
        return;
    };
    let resolved;
    let args = if let Some(normalized) = normalized {
        normalized
    } else {
        let Ok(mut args) = super::org_agent_service::tool_arguments(
            &state.db,
            &chat.user_id,
            chat.is_orchestrator(),
            args,
        )
        .await
        else {
            return;
        };
        if super::assistant_team_service::normalize_memory_agent(&state.db, chat, &mut args)
            .await
            .is_err()
        {
            return;
        }
        resolved = args;
        &resolved
    };
    let Some(write_target) = write_target(state, chat, args) else {
        return;
    };
    let stored = state.db.collection::<Document>(COLLECTION_NAME).update_one(
        scope,
        doc! {"$set": {"active_turn.activities.$[activity].memory_recovery_target": write_target}},
    )
    .array_filters(vec![doc! {"activity.id": &activity_id, "activity.status": "running"}])
    .await;
    if stored.is_ok_and(|result| result.modified_count == 1) {
        value["details"]["failed_activity_id"] = json!(activity_id);
    }
}

pub(crate) fn part(args: &Value) -> Option<MemoryRecovery> {
    let requested = args.get("recovery")?;
    let mut part: MemoryRecovery = serde_json::from_value(requested.clone()).ok()?;
    if requested.get("committed").is_some()
        || Uuid::parse_str(&part.failed_activity_id).is_err()
        || part.part == 0
        || part.part > part.total
        || part.total > MAX_PARTS
        || (part.total > 1 && args.get("replace_id").is_some())
    {
        return None;
    }
    part.committed = false;
    Some(part)
}

fn origin(part: &MemoryRecovery, target: &Document) -> Document {
    doc! {"active_turn.activities": {"$elemMatch": {
        "id": &part.failed_activity_id, "label": "nyxid__remember",
        "status": "error", "memory_recovery_target": target,
    }}}
}

fn no_duplicate(part: &MemoryRecovery) -> Document {
    doc! {"active_turn.activities": {"$not": {"$elemMatch": {
            "memory_recovery.failed_activity_id": &part.failed_activity_id,
            "$or": [
                {"memory_recovery.total": {"$ne": i32::from(part.total)}},
                {"memory_recovery.part": i32::from(part.part), "$nor": [
                    {"status": "error", "failure.reason": {"$in": REJECTED},
                        "memory_recovery.committed": false},
                ]},
            ],
    }}}}
}

pub(crate) async fn eligible(
    state: &crate::AppState,
    chat: &ChatAuthority,
    args: &Value,
    part: &MemoryRecovery,
) -> bool {
    let (Some((_, scope)), Some(target)) = (scope(chat), write_target(state, chat, args)) else {
        return false;
    };
    state
        .db
        .collection::<Document>(COLLECTION_NAME)
        .find_one(doc! {"$and": [scope, origin(part, &target), no_duplicate(part)]})
        .projection(doc! {"_id": 1})
        .await
        .is_ok_and(|row| row.is_some())
}

pub(crate) async fn run<F, Fut>(
    state: &crate::AppState,
    chat: &ChatAuthority,
    args: &Value,
    mut part: MemoryRecovery,
    approved: bool,
    execute: F,
) -> (Value, bool)
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = (Value, bool)>,
{
    let (Some((activity_id, scope)), Some(target)) = (scope(chat), write_target(state, chat, args))
    else {
        return unavailable();
    };
    let collection = state.db.collection::<Document>(COLLECTION_NAME);
    let filter = doc! {"$and": [scope.clone(), origin(&part, &target), no_duplicate(&part)]};
    let claimed = collection
        .update_one(
            filter,
            doc! {"$set": {"active_turn.activities.$[activity].memory_recovery":
            bson::to_bson(&part).expect("memory recovery metadata")}},
        )
        .array_filters(vec![
            doc! {"activity.id": &activity_id, "activity.status": "running"},
        ])
        .await;
    match claimed {
        Ok(result) if result.modified_count == 1 => {}
        Ok(_) if approved => {
            // The exact one-use card was consumed. Honor the authorized save
            // independently if the original row is no longer available.
            let continued = collection
                .find_one(doc! {"$and": [
                    scope,
                    {"active_turn.activities.id": {"$ne": &part.failed_activity_id}},
                    no_duplicate(&part),
                ]})
                .projection(doc! {"_id": 1})
                .await;
            if !continued.is_ok_and(|row| row.is_some()) {
                return unavailable();
            }
            let (mut value, error) = execute().await;
            if !error {
                value["recovery_status"] = json!("unlinked_after_approval");
            }
            return (value, error);
        }
        _ => return unavailable(),
    }
    let (mut value, is_error) = execute().await;
    if !is_error && value.get("remembered").is_some_and(Value::is_string) {
        part.committed = true;
        let _ = collection.update_one(
            scope,
            doc! {"$set": {"active_turn.activities.$[activity].memory_recovery.committed": true}},
        )
        .array_filters(vec![doc! {"activity.id": &activity_id, "activity.status": "running"}])
        .await;
    }
    value["recovery"] = json!(part);
    (value, is_error)
}
