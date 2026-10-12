//! Shared feedback integration fixtures. Feedback route cases live here so new
//! assistant message/group fields are initialized in one place.
#[cfg(test)]
mod fixtures {
    use crate::models::{
        assistant_group::{AssistantGroup, GroupMessage},
        assistant_message::AssistantMessage,
    };
    use chrono::Utc;
    pub fn message(id: &str, conversation_id: &str, user_id: &str) -> AssistantMessage {
        AssistantMessage {
            steering: None,
            voice: None,
            execution_pending: false,
            id: id.into(),
            conversation_id: conversation_id.into(),
            user_id: user_id.into(),
            seq: 1,
            turn_id: "feedback-turn".into(),
            role: "assistant".into(),
            text: "reply".into(),
            status: "completed".into(),
            error_code: None,
            created_at: Utc::now(),
            activities: vec![],
            attachments: vec![],
            origin: None,
            via: None,
        }
    }
    pub fn group_message(id: &str, group_id: &str, user_id: &str) -> GroupMessage {
        GroupMessage {
            activities: vec![],
            org_group: false,
            author_user_id: None,
            author_display_name: None,
            request_id: None,
            attachments: vec![],
            id: id.into(),
            group_id: group_id.into(),
            user_id: user_id.into(),
            seq: 1,
            role: "agent".into(),
            agent_id: Some("feedback-agent".into()),
            agent_name: None,
            text: "reply".into(),
            created_at: Utc::now(),
        }
    }
    pub fn group(id: &str, owner: &str) -> AssistantGroup {
        AssistantGroup {
            id: id.into(),
            user_id: owner.into(),
            participant_user_ids: vec![owner.into()],
            created_by_user_id: None,
            name: "Feedback".into(),
            member_agent_ids: vec!["feedback-agent".into()],
            lead_agent_id: "feedback-agent".into(),
            created_by: "user".into(),
            message_count: 1,
            pending_agent_ids: vec![],
            hops_remaining: 0,
            followers: vec![],
            pending_checked_at: None,
            last_message_at: Some(Utc::now()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }
}
