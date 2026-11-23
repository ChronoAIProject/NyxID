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

fn unavailable() -> (Value, bool) {
    (
        json!({
            "error": "memory_recovery_unavailable",
            "message": "This recovery part could not be admitted. Check the original failed write and any saved parts before retrying. Do not resend a part with an uncertain outcome.",
        }),
        true,
    )
}

pub(crate) async fn run<F, Fut>(
    state: &crate::AppState,
    chat: &ChatAuthority,
    args: &Value,
    execute: F,
) -> (Value, bool)
where
    F: FnOnce(Value) -> Fut,
    Fut: Future<Output = (Value, bool)>,
{
    let db = &state.db;
    let requested = args.get("recovery");
    let Some((turn_id, activity_id)) = chat
        .turn_id
        .as_ref()
        .zip(ACTIVITY_ID.try_with(Clone::clone).ok().flatten())
    else {
        return if requested.is_some() {
            unavailable()
        } else {
            execute(args.clone()).await
        };
    };
    if chat.guest || chat.collaboration_thread {
        return if requested.is_some() {
            unavailable()
        } else {
            execute(args.clone()).await
        };
    }

    // Resolve organization aliases before dispatch so the reference is bound
    // to the ID used for the write even if an alias is reassigned.
    let mut args = if args.get("org").is_some() {
        match super::org_agent_service::tool_arguments(
            db,
            &chat.user_id,
            chat.is_orchestrator(),
            args,
        )
        .await
        {
            Ok(normalized) => normalized,
            Err(_) if requested.is_some() => return unavailable(),
            Err(_) => return execute(args.clone()).await,
        }
    } else {
        args.clone()
    };
    let target = match args.get("agent").and_then(Value::as_str) {
        None if args.get("agent").is_some() => None,
        None => Some(chat.agent_id.clone()),
        Some(agent) if agent == chat.agent_name || agent == chat.agent_id => {
            Some(chat.agent_id.clone())
        }
        Some(agent) if chat.is_orchestrator() && Uuid::parse_str(agent).is_ok() => {
            Some(agent.to_owned())
        }
        Some(agent) if chat.is_orchestrator() && args.get("org").is_none() && agent.len() <= 64 => {
            super::assistant_team_service::live_specialist(db, &chat.user_id, agent)
                .await
                .ok()
                .map(|specialist| specialist.id)
        }
        Some(_) => None,
    };
    let Some(target) = target else {
        return if requested.is_some() {
            unavailable()
        } else {
            execute(args).await
        };
    };
    if args.get("agent").is_some() {
        args["agent"] = json!(target);
    }
    let replacement = match args.get("replace_id") {
        None => Bson::Null,
        Some(id) => {
            let Some(id) = id
                .as_str()
                .filter(|id| (1..=64).contains(&id.chars().count()))
            else {
                return if requested.is_some() {
                    unavailable()
                } else {
                    execute(args).await
                };
            };
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
    let write_target = doc! {
        "agent_id": target,
        "private": chat.org_agent_access.as_ref().is_some_and(|access| access.shared().is_some()),
        "replacement": replacement,
    };
    let collection = db.collection::<Document>(COLLECTION_NAME);
    let scope = doc! {
        "_id": &chat.conversation_id,
        "user_id": &chat.user_id,
        "active_turn.turn_id": turn_id,
        "active_turn.stop_requested": {"$ne": true},
        "active_turn.activities": {"$elemMatch": {"id": &activity_id, "status": "running"}},
    };
    let mut recovery = None;
    if let Some(requested) = args.get("recovery") {
        let Ok(part) = serde_json::from_value::<MemoryRecovery>(requested.clone()) else {
            return unavailable();
        };
        if Uuid::parse_str(&part.failed_activity_id).is_err()
            || requested.get("committed").is_some()
            || part.part == 0
            || part.part > part.total
            || part.total > MAX_PARTS
            || (part.total > 1 && args.get("replace_id").is_some())
        {
            return unavailable();
        }
        let filter = doc! {"$and": [
            scope.clone(),
            {"active_turn.activities": {"$elemMatch": {
                "id": &part.failed_activity_id, "label": "nyxid__remember",
                "status": "error", "memory_recovery_target": &write_target,
            }}},
            {"active_turn.activities": {"$not": {"$elemMatch": {
                "memory_recovery.failed_activity_id": &part.failed_activity_id,
                "$or": [
                    {"memory_recovery.total": {"$ne": i32::from(part.total)}},
                    {"memory_recovery.part": i32::from(part.part), "$nor": [
                        {"status": "error", "failure.reason": {"$in": REJECTED},
                            "memory_recovery.committed": false},
                    ]},
                ],
            }}}},
        ]};
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
        if !claimed.is_ok_and(|result| result.modified_count == 1) {
            return unavailable();
        }
        // Approval, validation and note storage still handle the ordinary write.
        args.as_object_mut()
            .expect("recovery arguments")
            .remove("recovery");
        recovery = Some(part);
    }

    let (mut value, is_error) = execute(args).await;
    if let Some(mut recovery) = recovery {
        if !is_error && value.get("remembered").is_some_and(Value::is_string) {
            recovery.committed = true;
            let _ = collection.update_one(
                scope,
                doc! {"$set": {"active_turn.activities.$[activity].memory_recovery.committed": true}},
            )
            .array_filters(vec![doc! {"activity.id": &activity_id, "activity.status": "running"}])
            .await;
        }
        value["recovery"] = json!(recovery);
    } else if is_error
        && value["details"]["reason"].as_str().is_some_and(|reason| reason != "memory_unauthorized" && REJECTED.contains(&reason))
        && collection.update_one(
            scope,
            doc! {"$set": {"active_turn.activities.$[activity].memory_recovery_target": write_target}},
        )
        .array_filters(vec![doc! {"activity.id": &activity_id, "activity.status": "running"}])
        .await.is_ok_and(|result| result.modified_count == 1)
    {
        value["details"]["failed_activity_id"] = json!(activity_id);
    }
    (value, is_error)
}
