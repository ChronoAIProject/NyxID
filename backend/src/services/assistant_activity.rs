//! Metadata-only activity for the acting person's turns and pending cards.

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::models::assistant_conversation::{ActiveTurn, AssistantConversation};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityState {
    Idle,
    Stopping,
    Approval,
    Waiting,
    Working,
    Drafting,
    Retry,
    Thinking,
}

#[derive(Clone, Debug, Serialize)]
pub struct ThreadActivity {
    pub conversation_id: String,
    pub state: ActivityState,
}

struct TurnFacts<I> {
    stop_requested: bool,
    queued: bool,
    waiting_for_capacity: bool,
    activities: I,
    reply_present: bool,
    continuations: u32,
}

// Keep precedence aligned with shared/agent-avatars/activity.ts.
fn turn_activity<'a>(
    turn: Option<TurnFacts<impl Iterator<Item = &'a str>>>,
    pending_owner_action: bool,
    streamed_text_present: bool,
) -> ActivityState {
    use ActivityState::*;
    if turn.as_ref().is_some_and(|turn| turn.stop_requested) {
        return Stopping;
    }
    if pending_owner_action {
        return Approval;
    }
    let Some(turn) = turn else {
        return Idle;
    };
    if turn.queued || turn.waiting_for_capacity {
        return Waiting;
    }
    let mut errors = 0;
    for status in turn.activities {
        match status {
            "running" => return Working,
            "error" => errors += 1,
            _ => {}
        }
    }
    if turn.reply_present || streamed_text_present {
        return Drafting;
    }
    if turn.continuations > 0 || errors >= 2 {
        return Retry;
    }
    Thinking
}

fn live_state(
    row: &AssistantConversation,
    turn: &ActiveTurn,
    pending_owner_action: bool,
) -> ActivityState {
    turn_activity(
        Some(TurnFacts {
            stop_requested: turn.stop_requested,
            queued: turn.queued_at.is_some(),
            waiting_for_capacity: turn.waiting_for_capacity,
            activities: turn.activities.iter().map(|row| row.status.as_str()),
            reply_present: super::assistant_stream_checkpoint::reply_for_snapshot(row, turn)
                .is_some_and(|reply| !reply.text.is_empty()),
            continuations: turn.continuations,
        }),
        pending_owner_action,
        false,
    )
}

pub fn thread_state(row: &AssistantConversation, now: DateTime<Utc>) -> Option<ActivityState> {
    super::assistant_nyxagent::live_turn(row, now).map(|turn| live_state(row, turn, false))
}

impl ThreadActivity {
    pub(crate) fn observe(
        row: &AssistantConversation,
        now: DateTime<Utc>,
        pending_owner_action: bool,
    ) -> Option<(Self, bool)> {
        let live = super::assistant_nyxagent::live_turn(row, now);
        let state = match live {
            Some(turn) => live_state(row, turn, pending_owner_action),
            None if pending_owner_action => ActivityState::Approval,
            None => return None,
        };
        Some((
            Self {
                state,
                conversation_id: row.id.clone(),
            },
            live.is_some(),
        ))
    }

    pub fn from_conversation(row: &AssistantConversation, now: DateTime<Utc>) -> Option<Self> {
        Some(Self {
            state: thread_state(row, now)?,
            conversation_id: row.id.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct Fixture {
        turns: Vec<Case>,
    }

    #[derive(Deserialize)]
    struct Case {
        name: String,
        turn: Option<FixtureTurn>,
        #[serde(default)]
        pending_owner_action: bool,
        #[serde(default)]
        streamed_text_present: bool,
        expected: String,
    }

    #[derive(Deserialize)]
    struct Activity {
        status: String,
    }

    #[derive(Deserialize)]
    struct Reply {
        text: Option<String>,
    }

    #[derive(Deserialize)]
    struct FixtureTurn {
        #[serde(default)]
        stop_requested: bool,
        #[serde(default)]
        queued: bool,
        #[serde(default)]
        waiting_for_capacity: bool,
        #[serde(default)]
        activities: Vec<Activity>,
        reply: Option<Reply>,
        #[serde(default)]
        continuations: u32,
    }

    #[test]
    fn assistant_activity_matches_the_shared_turn_fixture() {
        let fixture: Fixture = serde_json::from_str(include_str!(
            "../../../shared/agent-avatars/activity-fixture.json"
        ))
        .unwrap();
        assert!(!fixture.turns.is_empty());
        for case in fixture.turns {
            let turn = case.turn.as_ref().map(|turn| TurnFacts {
                stop_requested: turn.stop_requested,
                queued: turn.queued,
                waiting_for_capacity: turn.waiting_for_capacity,
                activities: turn.activities.iter().map(|row| row.status.as_str()),
                reply_present: turn
                    .reply
                    .as_ref()
                    .and_then(|reply| reply.text.as_ref())
                    .is_some_and(|text| !text.is_empty()),
                continuations: turn.continuations,
            });
            let actual = turn_activity(turn, case.pending_owner_action, case.streamed_text_present);
            assert_eq!(
                serde_json::to_value(actual).unwrap(),
                case.expected,
                "{}",
                case.name
            );
        }
    }
}
