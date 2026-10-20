use crate::models::remote_machine::ResourceState;
use crate::models::remote_machine_operation::{CleanupState, LifecycleAction, OperationState};
use crate::services::remote_machine_operation_service::{
    is_idempotent_replay, next_resource_state,
};

#[test]
fn same_idempotency_key_replays_only_identical_request() {
    assert!(is_idempotent_replay("digest", "digest", "accepted"));
    assert!(!is_idempotent_replay("digest", "other", "accepted"));
    assert!(!is_idempotent_replay("digest", "digest", "failed"));
}

#[test]
fn lifecycle_actions_map_to_distinct_pending_states() {
    assert_eq!(
        next_resource_state(LifecycleAction::Pause),
        ResourceState::Stopping
    );
    assert_eq!(
        next_resource_state(LifecycleAction::Resume),
        ResourceState::Resuming
    );
    assert_eq!(
        next_resource_state(LifecycleAction::Delete),
        ResourceState::Deleting
    );
    assert_eq!(OperationState::Accepted.as_str(), "accepted");
    assert_eq!(CleanupState::Unknown.as_str(), "unknown");
}
