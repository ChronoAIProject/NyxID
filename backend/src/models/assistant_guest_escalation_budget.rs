use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "assistant_guest_escalation_budgets";

/// One specialist's rolling admissions and pending conversation leases.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AssistantGuestEscalationBudget {
    /// The specialist's UUID string; one budget per agent.
    #[serde(rename = "_id")]
    pub agent_id: String,
    pub attempts: Vec<AdmissionTime>,
    pub pending: Vec<PendingApproval>,
}

/// Transparent to preserve the original BSON datetime array representation.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AdmissionTime(
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")] pub DateTime<Utc>,
);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PendingApproval {
    pub thread: String,
    pub token: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn guest_budget_preserves_bson_datetime_storage() {
        let now = bson::DateTime::now().to_chrono();
        let budget = AssistantGuestEscalationBudget {
            agent_id: uuid::Uuid::new_v4().to_string(),
            attempts: vec![AdmissionTime(now)],
            pending: vec![PendingApproval {
                thread: "thread".into(),
                token: "token".into(),
                expires_at: now,
            }],
        };
        let doc = bson::to_document(&budget).unwrap();
        assert!(
            doc.get_array("attempts").unwrap()[0]
                .as_datetime()
                .is_some()
        );
        assert!(
            doc.get_array("pending").unwrap()[0]
                .as_document()
                .unwrap()
                .get_datetime("expires_at")
                .is_ok()
        );
        let restored: AssistantGuestEscalationBudget = bson::from_document(doc).unwrap();
        assert_eq!(restored.attempts[0].0, now);
        assert_eq!(restored.pending[0].expires_at, now);
    }
}
