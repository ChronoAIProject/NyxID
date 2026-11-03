//! Classification is structural and independent of flags or caller identity.
use crate::models::{assistant_group::AssistantGroup, assistant_group_collaboration::VERSION};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupKind {
    Personal,
    Organization,
    Collaboration,
    Unsupported,
}

pub fn classify(group: &AssistantGroup) -> GroupKind {
    match &group.collaboration {
        Some(marker) if marker.version == VERSION => GroupKind::Collaboration,
        Some(_) => GroupKind::Unsupported,
        None if group.created_by_user_id.is_some() => GroupKind::Organization,
        None => GroupKind::Personal,
    }
}

/// Reader-first guard: legacy execution/history entry points must never fall
/// back to creator equality for a collaboration group (including unknown versions).
pub fn require_legacy(group: &AssistantGroup) -> crate::errors::AppResult<()> {
    match classify(group) {
        GroupKind::Personal | GroupKind::Organization => Ok(()),
        _ => Err(crate::errors::AppError::NotFound("Group not found".into())),
    }
}
