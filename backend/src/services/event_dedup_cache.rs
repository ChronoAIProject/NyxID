pub use crate::services::coordination_service::{
    EventDedupClaim, EventDedupClaimResult, EventDedupStore,
};

pub const CHANNEL_EVENT_NAMESPACE: &str = "channel-event";
pub const TRIGGER_EVENT_NAMESPACE: &str = "trigger-event";
