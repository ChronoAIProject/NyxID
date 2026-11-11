//! Human presentation only: no effect identity, admission or receipt writes.
use super::*;
use crate::models::assistant_conversation::AgentEvent;
use std::collections::{BTreeMap, BTreeSet};

/// A batch of typed retained-operation wakes is one transcript notice. Keep
/// every stored event and request ID for admission/settlement; never merge by
/// prose or hide unrelated events. Old queued wake text is handled too.
pub(crate) fn delivery_notice(events: &[AgentEvent]) -> Option<String> {
    if events.is_empty() {
        return None;
    }
    let ids: Option<BTreeSet<_>> = events
        .iter()
        .map(|event| {
            (event.kind == "permission_decided" && event.channel_contexts.is_empty())
                .then(|| event.id.strip_prefix("execution-decision:"))
                .flatten()
                .and_then(|id| Uuid::parse_str(id).ok())
        })
        .collect();
    let count = ids?.len();
    Some(if count == 1 {
        "Retained operation update for 1 permission request. The result appears in the reply."
            .into()
    } else {
        format!(
            "Retained operation updates for {count} distinct permission requests. Results appear in the reply."
        )
    })
}

#[derive(Default)]
pub(super) struct ReviewNotes(BTreeMap<&'static str, Vec<String>>);

impl ReviewNotes {
    pub(super) fn push(&mut self, row: &ApprovalRequest, proposal: &Proposal) {
        let state = if proposal.state == "cancelled" {
            "cancelled"
        } else {
            match row.status.as_str() {
                "denied" => "denied",
                "expired" => "expired",
                "pending" if row.expires_at <= Utc::now() => "expired",
                "pending" => "pending",
                _ => "unavailable",
            }
        };
        self.0.entry(state).or_default().push(row.id.clone());
    }

    pub(super) fn render(self) -> Vec<String> {
        self.0
            .into_iter()
            .map(|(state, ids)| {
                let explanation = match state {
                    "pending" => "Still awaiting a decision. Nothing was sent for these requests.",
                    "unavailable" => {
                        "The permission status is unavailable; no operation was resumed."
                    }
                    _ => "Nothing was sent for these requests.",
                };
                format!(
                    "Permission requests {state}: {}. {explanation}\nRequest IDs: {}",
                    ids.len(),
                    ids.join(", ")
                )
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wake(id: &str) -> AgentEvent {
        let mut event = crate::services::assistant_team_service::event(
            "permission_decided",
            "old internal delivery instructions".into(),
            None,
        );
        event.id = format!("execution-decision:{id}");
        event
    }

    #[test]
    fn retained_wakes_count_identity_not_text_and_preserve_other_notices() {
        let first = wake(&Uuid::new_v4().to_string());
        let second = wake(&Uuid::new_v4().to_string());
        assert!(
            delivery_notice(&[first.clone(), first.clone()])
                .unwrap()
                .contains("1 permission request")
        );
        let text = crate::services::assistant_nyxagent::events_text(&[first.clone(), second]);
        assert_eq!(text.matches("\n- ").count(), 1);
        assert!(text.contains("2 distinct permission requests"));
        assert!(!text.contains("old internal delivery instructions"));
        let ordinary =
            crate::services::assistant_team_service::event("notice", "ordinary event".into(), None);
        assert!(delivery_notice(&[first.clone(), ordinary.clone()]).is_none());
        let text = crate::services::assistant_nyxagent::events_text(&[first, ordinary]);
        assert_eq!(text.matches("\n- ").count(), 2);
        assert!(text.contains("ordinary event"));
        assert!(delivery_notice(&[]).is_none());
        assert!(delivery_notice(&[wake("not-a-request-id")]).is_none());
    }
}
