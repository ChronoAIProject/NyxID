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

/// `nyxid-chat/conversations/{id}:stop` -- stop active work (202-accepted
/// control command; Aevatar commits a stop fence before any successor
/// operation may start).
pub fn stop_path(user_id: &str, conversation_id: &str) -> AppResult<String> {
    Ok(format!(
        "{}:stop",
        conversation_path(user_id, conversation_id)?
    ))
}

/// `nyxid-chat/conversations/{id}:steer` -- redirect active work
/// (202-accepted; Aevatar serializes the steering fence and starts a
/// server-owned continuation turn at a safe checkpoint).
pub fn steer_path(user_id: &str, conversation_id: &str) -> AppResult<String> {
    Ok(format!(
        "{}:steer",
        conversation_path(user_id, conversation_id)?
    ))
}

/// `nyxid-chat/conversations/{id}/state` -- conditional current-state query
/// (GET; `afterStateVersion` / `turnId` cursors ride the forwarded query
/// string). This is the contract's reconnect surface: clients poll it
/// instead of replaying events.
pub fn state_path(user_id: &str, conversation_id: &str) -> AppResult<String> {
    Ok(format!(
        "{}/state",
        conversation_path(user_id, conversation_id)?
    ))
}

/// Turn/step ids follow the upstream control-identity grammar
/// (`TryValidateControlIdentity`): at most 128 chars, no whitespace or
/// control characters, and none of `/ \ ? #`. Anything else the server may
/// legitimately issue (`turn.v2`, `step:3`, …) must round-trip, so accepted
/// segments are percent-encoded before interpolation — an unencoded `:`
/// inside a step id would collide with the `:retry`/`:skip` suffix parse.
fn encode_control_segment(segment: &str) -> AppResult<String> {
    let valid = !segment.is_empty()
        && segment.len() <= 128
        && segment
            .chars()
            .all(|c| !c.is_whitespace() && !c.is_control() && !matches!(c, '/' | '\\' | '?' | '#'));
    if !valid {
        return Err(AppError::BadRequest("Invalid turn or step id.".to_string()));
    }
    Ok(urlencoding::encode(segment).into_owned())
}

/// `nyxid-chat/conversations/{id}/turns/{turn}/steps/{step}:retry` --
/// retry one failed/interrupted step (202-accepted control command).
pub fn retry_path(
    user_id: &str,
    conversation_id: &str,
    turn_id: &str,
    step_id: &str,
) -> AppResult<String> {
    let turn = encode_control_segment(turn_id)?;
    let step = encode_control_segment(step_id)?;
    Ok(format!(
        "{}/turns/{turn}/steps/{step}:retry",
        conversation_path(user_id, conversation_id)?
    ))
}

/// `nyxid-chat/conversations/{id}/turns/{turn}/steps/{step}:skip` -- skip
/// one optional step (202-accepted control command).
pub fn skip_path(
    user_id: &str,
    conversation_id: &str,
    turn_id: &str,
    step_id: &str,
) -> AppResult<String> {
    let turn = encode_control_segment(turn_id)?;
    let step = encode_control_segment(step_id)?;
    Ok(format!(
        "{}/turns/{turn}/steps/{step}:skip",
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

/// Post-create materialization poll bounds. Conversation create is
/// async-accepted (202 + `actorId`); streaming into an actor before it
/// appears in the `nyxid-chat` actor index races the materialization, so
/// create waits for the actor with the same attempt count and a comparable
/// backoff budget (~2s) to the reference client (nyxid-chat `server.mjs`
/// `waitForConversation`: 6 attempts, 250ms + 100ms/attempt).
pub const CREATE_POLL_ATTEMPTS: u32 = 6;

/// Delay before the next poll attempt (200ms, 300ms, ... capped by the
/// attempt count; ~2.0s of sleeps worst case, before network time).
pub fn create_poll_delay_ms(attempt: u32) -> u64 {
    200 + u64::from(attempt) * 100
}

/// `actorId` from a create-conversation response body. Aevatar responses
/// have been observed in both camel- and Pascal-case.
pub fn extract_actor_id(body: &serde_json::Value) -> Option<&str> {
    body.get("actorId")
        .or_else(|| body.get("ActorId"))?
        .as_str()
}

/// Whether the `nyxid-chat` actor index (`{"conversations":[{"actorId"}]}`)
/// lists the given actor.
pub fn actor_index_contains(index: &serde_json::Value, actor_id: &str) -> bool {
    index
        .get("conversations")
        .or_else(|| index.get("Conversations"))
        .and_then(serde_json::Value::as_array)
        .is_some_and(|conversations| {
            conversations.iter().any(|entry| {
                entry
                    .get("actorId")
                    .or_else(|| entry.get("ActorId"))
                    .and_then(serde_json::Value::as_str)
                    == Some(actor_id)
            })
        })
}

/// Composite-delete tolerance: deleting a conversation removes both the
/// `nyxid-chat` actor and the chat-history row, and either side may already
/// be gone (an `/api/chat`-created row has no actor; a cascaded actor
/// delete may have removed the history row first). 404 is success-shaped.
pub fn delete_status_acceptable(status: u16) -> bool {
    (200..300).contains(&status) || status == 404
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
            stop_path(USER, CONV).unwrap(),
            format!("api/scopes/{USER}/nyxid-chat/conversations/{CONV}:stop")
        );
        assert_eq!(
            steer_path(USER, CONV).unwrap(),
            format!("api/scopes/{USER}/nyxid-chat/conversations/{CONV}:steer")
        );
        assert_eq!(
            state_path(USER, CONV).unwrap(),
            format!("api/scopes/{USER}/nyxid-chat/conversations/{CONV}/state")
        );
        assert_eq!(
            retry_path(USER, CONV, "turn-1", "step_a").unwrap(),
            format!(
                "api/scopes/{USER}/nyxid-chat/conversations/{CONV}/turns/turn-1/steps/step_a:retry"
            )
        );
        assert_eq!(
            skip_path(USER, CONV, "turn-1", "step_a").unwrap(),
            format!(
                "api/scopes/{USER}/nyxid-chat/conversations/{CONV}/turns/turn-1/steps/step_a:skip"
            )
        );
        // Upstream may issue identities with `.` / `:`; they round-trip
        // percent-encoded so a raw `:` cannot collide with the `:retry`
        // suffix parse.
        assert_eq!(
            retry_path(USER, CONV, "turn.v2", "step:3").unwrap(),
            format!(
                "api/scopes/{USER}/nyxid-chat/conversations/{CONV}/turns/turn.v2/steps/step%3A3:retry"
            )
        );
        // A literal `%` also round-trips (doubly-encoded), so a caller
        // cannot smuggle traversal via pre-encoded sequences.
        assert_eq!(
            skip_path(USER, CONV, "turn-1", "%2e%2e").unwrap(),
            format!(
                "api/scopes/{USER}/nyxid-chat/conversations/{CONV}/turns/turn-1/steps/%252e%252e:skip"
            )
        );
    }

    #[test]
    fn rejects_control_segments_outside_the_upstream_grammar() {
        // The control-identity grammar forbids whitespace, control chars,
        // and `/ \ ? #` (upstream `TryValidateControlIdentity`); everything
        // else is accepted and percent-encoded.
        for bad in [
            "",
            "abc/def",
            "abc\\def",
            "abc?x=1",
            "abc#frag",
            "abc def",
            "tab\there",
            "\u{7}bell",
        ] {
            assert!(
                retry_path(USER, CONV, bad, "step_a").is_err(),
                "expected turn {bad:?} to be rejected"
            );
            assert!(retry_path(USER, CONV, "turn-1", bad).is_err());
            assert!(skip_path(USER, CONV, bad, "step_a").is_err());
            assert!(skip_path(USER, CONV, "turn-1", bad).is_err());
        }
        assert!(retry_path(USER, CONV, &"a".repeat(129), "step_a").is_err());
        assert!(retry_path(USER, CONV, "turn-1", &"a".repeat(129)).is_err());
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
            assert!(stop_path(USER, bad).is_err());
            assert!(steer_path(USER, bad).is_err());
            assert!(state_path(USER, bad).is_err());
        }
        assert!(history_path(USER, &"a".repeat(129)).is_err());
    }

    #[test]
    fn accepts_the_opaque_ids_aevatar_issues() {
        assert!(history_path(USER, CONV).is_ok());
        assert!(history_path(USER, "abc_DEF-123").is_ok());
    }

    #[test]
    fn extracts_actor_ids_in_both_observed_casings() {
        let camel = serde_json::json!({ "status": "accepted", "actorId": CONV });
        let pascal = serde_json::json!({ "ActorId": CONV });
        assert_eq!(extract_actor_id(&camel), Some(CONV));
        assert_eq!(extract_actor_id(&pascal), Some(CONV));
        assert_eq!(extract_actor_id(&serde_json::json!({})), None);
        assert_eq!(extract_actor_id(&serde_json::json!({ "actorId": 7 })), None);
    }

    #[test]
    fn finds_actors_in_the_index_and_tolerates_shape_drift() {
        let index = serde_json::json!({
            "conversations": [{ "actorId": "other" }, { "actorId": CONV }],
            "stateVersion": 3,
        });
        assert!(actor_index_contains(&index, CONV));
        assert!(!actor_index_contains(&index, "missing"));
        let pascal = serde_json::json!({ "Conversations": [{ "ActorId": CONV }] });
        assert!(actor_index_contains(&pascal, CONV));
        assert!(!actor_index_contains(&serde_json::json!({}), CONV));
        assert!(!actor_index_contains(&serde_json::json!([1, 2]), CONV));
    }

    #[test]
    fn composite_delete_accepts_success_and_absent_but_nothing_else() {
        assert!(delete_status_acceptable(200));
        assert!(delete_status_acceptable(204));
        assert!(delete_status_acceptable(404));
        assert!(!delete_status_acceptable(401));
        assert!(!delete_status_acceptable(403));
        assert!(!delete_status_acceptable(500));
        assert!(!delete_status_acceptable(502));
    }

    #[test]
    fn create_poll_backoff_is_bounded() {
        // Sleeps happen between attempts; the last attempt's delay is never
        // slept.
        let total: u64 = (0..CREATE_POLL_ATTEMPTS - 1)
            .map(create_poll_delay_ms)
            .sum();
        assert!(
            total <= 2_500,
            "poll wait must stay well under stream-start latency budgets, got {total}ms"
        );
    }
}
