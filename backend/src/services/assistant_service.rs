//! Assistant chat pass-through: resolution and upstream path construction.
//!
//! The browser talks to NyxID's own `/api/v1/assistant/*` routes (PRD
//! decision 4) rather than reaching Aevatar through the generic proxy. Two
//! invariants live here:
//!
//! 1. **Admin-managed resolution.** Aevatar is resolved as the admin-seeded
//!    `DownstreamService` (`service_category = "internal"`,
//!    `requires_user_credential = false`), never through the caller's
//!    personal `UserService`. This matters because `lookup_user_service`
//!    matches on slug *and* `catalog_service_id`, so a user who happens to
//!    have connected Aevatar personally would otherwise silently route
//!    through their own row while everyone else routed through the platform
//!    one -- the assistant would work for exactly the people who tested it
//!    and no one else. Behavior must not depend on caller-owned rows.
//! 2. **Server-owned scope.** The Aevatar scope segment is derived from the
//!    verified `AuthUser.user_id`, never from a browser-supplied path or
//!    body param, so a caller cannot address another user's scope.

use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::{
    COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
};
use mongodb::bson::doc;

/// Catalog slug of the admin-seeded Aevatar service.
pub const AEVATAR_SLUG: &str = "aevatar";

/// Resolve the admin-managed Aevatar service.
///
/// Deliberately reads `downstream_services` (the admin catalog) directly:
/// this is the "admin service" path, and it must resolve identically for
/// every caller. See the module invariants.
pub async fn resolve_admin_service(db: &mongodb::Database) -> AppResult<DownstreamService> {
    let service = db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find_one(doc! { "slug": AEVATAR_SLUG, "is_active": true })
        .await?
        // Absent/inactive is a platform provisioning fault, not a caller
        // error: `Internal` keeps the detail server-side.
        .ok_or_else(|| {
            AppError::Internal(format!(
                "assistant: no active downstream service with slug '{AEVATAR_SLUG}'"
            ))
        })?;

    // Guard the provisioning contract: a service that demands a per-user
    // credential cannot back a platform surface, and misconfiguring it that
    // way would degrade into per-user connections without any visible error.
    if service.requires_user_credential {
        return Err(AppError::Internal(format!(
            "assistant: service '{AEVATAR_SLUG}' requires a user credential and cannot back the platform assistant surface"
        )));
    }

    Ok(service)
}

/// Reject ids that could escape the upstream path segment they are
/// interpolated into. Aevatar ids are opaque `nyxid-chat-<hex>` handles;
/// anything carrying a separator is a caller trying to reshape the URL.
fn validate_conversation_id(conversation_id: &str) -> AppResult<()> {
    let valid = !conversation_id.is_empty()
        && conversation_id.len() <= 128
        && conversation_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !valid {
        return Err(AppError::BadRequest("Invalid conversation id.".to_string()));
    }
    Ok(())
}

/// `nyxid-chat/conversations` -- create a conversation actor (POST).
pub fn conversations_path(user_id: &str) -> String {
    format!("api/scopes/{user_id}/nyxid-chat/conversations")
}

/// `chat-history` -- the conversation history index (GET).
///
/// This is the authoritative user-facing list per the Chat History contract:
/// each row carries a server-owned title, timestamps, and message count, and
/// only conversations that have materialized at least one terminal turn
/// appear. The `nyxid-chat` actor index (`conversations_path`) is lifecycle
/// plumbing -- it returns bare actor ids and includes not-yet-started actors
/// -- so the list surface reads this instead.
pub fn history_index_path(user_id: &str) -> String {
    format!("api/scopes/{user_id}/chat-history")
}

/// `chat-history/conversations/{id}` -- transcript read.
pub fn history_path(user_id: &str, conversation_id: &str) -> AppResult<String> {
    validate_conversation_id(conversation_id)?;
    Ok(format!(
        "api/scopes/{user_id}/chat-history/conversations/{conversation_id}"
    ))
}

/// `nyxid-chat/conversations/{id}` -- delete.
pub fn conversation_path(user_id: &str, conversation_id: &str) -> AppResult<String> {
    validate_conversation_id(conversation_id)?;
    Ok(format!(
        "api/scopes/{user_id}/nyxid-chat/conversations/{conversation_id}"
    ))
}

/// `nyxid-chat/conversations/{id}:stream` -- AG-UI SSE turn.
pub fn stream_path(user_id: &str, conversation_id: &str) -> AppResult<String> {
    Ok(format!(
        "{}:stream",
        conversation_path(user_id, conversation_id)?
    ))
}

/// `nyxid-chat/conversations/{id}:approve` -- approval decision.
pub fn approve_path(user_id: &str, conversation_id: &str) -> AppResult<String> {
    Ok(format!(
        "{}:approve",
        conversation_path(user_id, conversation_id)?
    ))
}

/// `v1/chat/completions` -- OpenAI-compatible surface. Scope-free: the
/// endpoint is stateless and carries its history in the request body.
pub fn completions_path() -> String {
    "v1/chat/completions".to_string()
}

/// `api/chat` -- ad-hoc workflow chat (`StartWorkflowChat`). Scope-free on
/// the wire: Aevatar derives the scope from the propagated identity JWT, so
/// the server-owned-scope invariant holds with nothing in the path.
pub fn workflow_chat_path() -> String {
    "api/chat".to_string()
}

/// `api/ws/chat` -- WebSocket twin of the workflow chat
/// (`StartWorkflowChatWebSocket`).
pub fn workflow_chat_ws_path() -> String {
    "api/ws/chat".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const USER: &str = "add69059-bece-4f0e-9559-99cfd10b47eb";
    const CONV: &str = "nyxid-chat-f8369965a444433f92ec50e67ad8ee52";

    #[test]
    fn builds_the_aevatar_paths_from_the_server_side_scope() {
        assert_eq!(
            conversations_path(USER),
            format!("api/scopes/{USER}/nyxid-chat/conversations")
        );
        assert_eq!(
            history_path(USER, CONV).unwrap(),
            format!("api/scopes/{USER}/chat-history/conversations/{CONV}")
        );
        assert_eq!(
            stream_path(USER, CONV).unwrap(),
            format!("api/scopes/{USER}/nyxid-chat/conversations/{CONV}:stream")
        );
        assert_eq!(
            approve_path(USER, CONV).unwrap(),
            format!("api/scopes/{USER}/nyxid-chat/conversations/{CONV}:approve")
        );
        assert_eq!(
            history_index_path(USER),
            format!("api/scopes/{USER}/chat-history")
        );
        assert_eq!(completions_path(), "v1/chat/completions");
        assert_eq!(workflow_chat_path(), "api/chat");
        assert_eq!(workflow_chat_ws_path(), "api/ws/chat");
    }

    #[test]
    fn rejects_conversation_ids_that_would_escape_the_path_segment() {
        for bad in [
            "",
            "../../admin",
            "abc/def",
            "abc:stream",
            "abc?x=1",
            "abc#frag",
            "abc def",
            "%2e%2e",
        ] {
            assert!(
                history_path(USER, bad).is_err(),
                "expected {bad:?} to be rejected"
            );
            assert!(stream_path(USER, bad).is_err());
            assert!(approve_path(USER, bad).is_err());
        }
        assert!(history_path(USER, &"a".repeat(129)).is_err());
    }

    #[test]
    fn accepts_the_opaque_ids_aevatar_issues() {
        assert!(history_path(USER, CONV).is_ok());
        assert!(history_path(USER, "abc_DEF-123").is_ok());
    }
}
