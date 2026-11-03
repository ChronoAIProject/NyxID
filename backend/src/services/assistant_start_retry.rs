//! Retries only proven pre-execution refusals, never ambiguous gateway timeouts.
use crate::{
    errors::AppResult,
    models::assistant_conversation::{AssistantConversation, COLLECTION_NAME},
};
use mongodb::{
    Database,
    bson::{self, doc},
};
use std::time::Duration;

pub const BUDGET: Duration = Duration::from_secs(15);
pub const MAX_RETRIES: u32 = 2;

pub fn refused(status: u16, code: &str) -> bool {
    // HTTP status alone (including 502/504) says nothing about dispatch.
    matches!(status, 502..=504) && code == "dependency_unavailable"
}

pub fn delay(retries: u32) -> Option<Duration> {
    (retries < MAX_RETRIES).then(|| Duration::from_secs(2 * u64::from(retries + 1)))
}

/// Fresh evidence from the same live turn, in addition to the upstream's
/// pre-admission refusal or a transport-proven failed connection. An accepted
/// engine response or any model/tool activity irrevocably closes this window.
pub async fn untouched(db: &Database, row: &AssistantConversation) -> AppResult<bool> {
    let Some(turn) = &row.active_turn else {
        return Ok(false);
    };
    if turn.platform_origin.is_some() {
        return Ok(false);
    }
    Ok(db
        .collection::<bson::Document>(COLLECTION_NAME)
        .find_one(doc! {
            "_id": &row.id, "user_id": &row.user_id,
            "active_turn.turn_id": &turn.turn_id, "active_turn.stop_requested": {"$ne": true},
            "active_turn.capacity_accepted_at": bson::Bson::Null,
            "active_turn.running_response": bson::Bson::Null,
            "active_turn.model_attempt_id": bson::Bson::Null,
            "active_turn.activity_total": {"$in": [0, bson::Bson::Null]},
            "active_turn.reply.text": {"$in": ["", bson::Bson::Null]},
            "nyxagent_session_id": &row.nyxagent_session_id,
        "credential_api_key_id": &row.credential_api_key_id,
        })
        .projection(doc! {"_id":1})
        .await?
        .is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn startup_retry_requires_proof_and_is_bounded() {
        for status in [502, 503, 504] {
            assert!(refused(status, "dependency_unavailable"));
            assert!(!refused(status, ""));
            assert!(!refused(status, "outcome_unknown"));
        }
        assert!(!refused(500, "dependency_unavailable"));
        assert_eq!(delay(0), Some(Duration::from_secs(2)));
        assert_eq!(delay(1), Some(Duration::from_secs(4)));
        assert_eq!(delay(2), None);
    }
}
