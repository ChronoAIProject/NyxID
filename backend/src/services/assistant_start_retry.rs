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

/// Preserve the same upstream request when a tool-first turn has already
/// started work. Only the first-byte expiry does a read; ordinary streaming
/// has no extra polling. Earlier continuations cannot extend this request.
pub struct FirstByteDeadline {
    deadline: tokio::time::Instant,
    tool_deadline: tokio::time::Instant,
    started_at: bson::DateTime,
    checked: bool,
}

impl FirstByteDeadline {
    pub fn new(first_byte: Duration, tool_budget: Duration) -> Self {
        let now = tokio::time::Instant::now();
        Self {
            deadline: now + first_byte,
            tool_deadline: now + tool_budget,
            started_at: bson::DateTime::now(),
            checked: false,
        }
    }

    pub async fn wait<F: std::future::Future>(
        &mut self,
        db: &Database,
        row: &AssistantConversation,
        future: F,
    ) -> Result<F::Output, super::assistant_nyxagent::TurnError> {
        let mut future = std::pin::pin!(future);
        if let Ok(output) = tokio::time::timeout_at(self.deadline, &mut future).await {
            return Ok(output);
        }
        let error =
            || super::assistant_nyxagent::TurnError::internal("first_byte_timeout", "first_byte");
        if self.checked {
            return Err(error());
        }
        self.checked = true;
        let turn = row.active_turn.as_ref().ok_or_else(error)?;
        let evidence_deadline = self
            .tool_deadline
            .min(tokio::time::Instant::now() + Duration::from_secs(2));
        let active = tokio::time::timeout_at(evidence_deadline, async {
            db.collection::<bson::Document>(COLLECTION_NAME)
                .find_one(doc! {
                    "_id": &row.id, "user_id": &row.user_id,
                    "credential_api_key_id": &row.credential_api_key_id,
                    "active_turn.turn_id": &turn.turn_id,
                    "active_turn.stop_requested": false,
                    "active_turn.activities": {"$elemMatch": {"started_at": {"$gt": self.started_at}}},
                })
                .projection(doc! {"_id": 1})
                .await
        }).await.ok().and_then(Result::ok).flatten().is_some();
        if !active {
            return Err(error());
        }
        self.deadline = self.tool_deadline;
        tokio::time::timeout_at(self.deadline, &mut future)
            .await
            .map_err(|_| error())
    }
}

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
