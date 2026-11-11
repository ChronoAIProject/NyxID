//! Optional channel progress notices for work beyond the prompt and attached skills.
use chrono::{DateTime, Utc};
use mongodb::{
    Database,
    bson::{self, doc},
};

use super::assistant_nyxagent::{PrimaryDestination, answer_routing};
use crate::{
    errors::AppResult,
    models::assistant_conversation::{
        AssistantConversation, COLLECTION_NAME, ChannelOrigin, TurnOrigin,
    },
};

pub const DELAY: std::time::Duration = std::time::Duration::from_secs(5);
pub const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(500);
pub const TEXT: &str = "Got it, let me check and get back to you.";

/// Discovery and attached skill reads do not constitute an external check.
/// Downstream calls have the privacy-preserving activity label `tool`.
pub fn requires_check(label: &str) -> bool {
    !matches!(
        label,
        "nyxid__skill_read"
            | "nyxid__get_agent_skills"
            | "nyx__search_tools"
            | "nyx__discover_services"
            | "nyx__list_connected_services"
    )
}

pub fn destination(row: &AssistantConversation) -> Option<ChannelOrigin> {
    let turn = row.active_turn.as_ref()?;
    if turn.origin == TurnOrigin::Trigger
        || turn.trigger_run_id.is_some()
        || turn.platform_origin.is_some()
    {
        return None;
    }
    match answer_routing(row, turn).primary {
        PrimaryDestination::ChannelEvent { origin } => Some(origin),
        PrimaryDestination::ChatUpdate { origins } => origins.into_iter().next(),
        _ => None,
    }
}

/// Claim before sending. Stop, replacement and settlement fence this update;
/// neither replicas nor ambiguous remote failures may repeat the notice.
/// This never changes the final answer's delivery claim or transcript.
pub async fn claim(
    db: &Database,
    row: &AssistantConversation,
    now: DateTime<Utc>,
) -> AppResult<bool> {
    let Some(turn) = row.active_turn.as_ref() else {
        return Ok(false);
    };
    if destination(row).is_none() {
        return Ok(false);
    }
    Ok(db.collection::<bson::Document>(COLLECTION_NAME).update_one(
        doc! {"_id": &row.id, "user_id": &row.user_id, "active_turn.turn_id": &turn.turn_id,
            "active_turn.stop_requested": {"$ne": true},
            "$or": [
                {"active_turn.lease_expires_at": {"$gt": bson::DateTime::from_chrono(now)}},
                {"active_turn.lease_expires_at": bson::Bson::Null,
                    "active_turn.started_at": {"$gt": bson::DateTime::from_chrono(now - chrono::Duration::seconds(super::assistant_nyxagent::ACTIVE_TURN_TTL_SECS))}},
            ],
            "active_turn.origin": {"$ne": "trigger"}, "active_turn.trigger_run_id": bson::Bson::Null,
            "active_turn.started_at": {"$lte": bson::DateTime::from_chrono(now - chrono::Duration::from_std(DELAY).expect("bounded notice delay"))},
            "active_turn.external_check_started": true,
            "active_turn.checking_notice_claimed": {"$ne": true},
            "active_turn.capacity_notice_claimed": {"$ne": true}},
        doc! {"$set": {"active_turn.checking_notice_claimed": true}},
    ).await?.modified_count == 1)
}
