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

/// Actor-id prefix of a `nyxid-chat` conversation (upstream
/// `NyxIdChatServiceDefaults.ActorIdPrefix`; ids are `nyxid-chat-{guid:N}`).
const NYXID_CHAT_ACTOR_PREFIX: &str = "nyxid-chat-";

/// Conversation-id prefix of a workflow-chat conversation (upstream
/// `ChatHistoryActorIds.CreateConversationId`; ids are `chatc-{hash[..32]}`).
const WORKFLOW_CHAT_CONVERSATION_PREFIX: &str = "chatc-";

/// Drop chat-history rows this surface cannot address.
///
/// `chat-history` is a **shared** read model. Two families in it are ours:
/// `nyxid-chat-…` conversation actors (turns via `:stream`) and `chatc-…`
/// workflow-chat conversations (turns via `POST /assistant/workflow-chat`,
/// continued with `conversation.conversationId`). Anything else in the index
/// belongs to another product surface and is dropped, keeping the client
/// honest about what it can address: every id this route returns is a valid
/// target for its family's turn route, the transcript route, and delete.
///
/// Shape-tolerant by design — an index that is not `{"conversations": [...]}`
/// (or a row without a string `id`) is returned untouched, matching the
/// deploy-independence posture of the transcript route.
pub fn filter_chat_history_index(index: &mut serde_json::Value) -> bool {
    let Some(rows) = index
        .get_mut("conversations")
        .and_then(serde_json::Value::as_array_mut)
    else {
        return false;
    };
    let before = rows.len();
    rows.retain(|row| {
        row.get("id")
            .and_then(serde_json::Value::as_str)
            // Keep unknown-shaped rows: a row we cannot classify is not
            // evidence that it belongs to another surface.
            .is_none_or(|id| {
                id.starts_with(NYXID_CHAT_ACTOR_PREFIX)
                    || id.starts_with(WORKFLOW_CHAT_CONVERSATION_PREFIX)
            })
    });
    rows.len() != before
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
/// (`TryValidateControlIdentity`): at most 128 UTF-16 code units (the C#
/// `string.Length` the upstream counts — NOT bytes, so 65 `é`s pass), no
/// whitespace or control characters, and none of `/ \ ? #`. Accepted
/// segments are percent-encoded before interpolation — an unencoded `:`
/// inside a step id would collide with the `:retry`/`:skip` suffix parse.
///
/// Two documented NyxID narrowings of the upstream grammar, both
/// fail-closed here rather than deeper with a less precise error:
/// - `%` is rejected: the proxy's path hardening repeat-decodes nested
///   escapes, so a literal `%2F`-shaped id would be 400'd downstream even
///   double-encoded.
/// - dot-only segments (`.`, `..`) are rejected: they form
///   traversal-shaped path segments if any later layer normalizes.
fn encode_control_segment(segment: &str) -> AppResult<String> {
    let valid = !segment.is_empty()
        && segment.encode_utf16().count() <= 128
        && segment.chars().all(|c| {
            !c.is_whitespace() && !c.is_control() && !matches!(c, '/' | '\\' | '?' | '#' | '%')
        })
        && !segment.chars().all(|c| c == '.');
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

/// The one workflow the assistant surface may start. Pinned server-side:
/// Aevatar's `/api/chat` runs whatever catalog workflow the body names
/// (`direct`, `auto`, `auto_review`, file-loaded definitions, …), and which
/// engine backs the platform chat is a platform decision, not a caller input.
pub const WORKFLOW_CHAT_WORKFLOW: &str = "studio";

/// Matches the client cap in `aevatar-transport.ts` (`MAX_MESSAGE_CHARS`).
const WORKFLOW_CHAT_PROMPT_MAX_CHARS: usize = 32_768;

/// The caller half of the workflow-chat turn contract. Everything else in
/// Aevatar's `HttpChatInput` (workflow selection, inline YAML, llmControl,
/// toolContext, metadata, headers) is deliberately not expressible here.
///
/// `deny_unknown_fields` keeps client drift loud: Aevatar rejects unknown
/// body members (`JsonUnmappedMemberHandling.Disallow`), and a field that
/// silently vanished here would surface as a confusing upstream 400 instead.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkflowChatTurnRequest {
    pub prompt: String,
    /// `chatc-…` conversation to continue; absent starts a new conversation.
    #[serde(default)]
    pub conversation_id: Option<String>,
    /// Chat-history read fence for continuations: the `stateVersion` the
    /// client last observed. Aevatar requires `> 0` alongside a conversation
    /// id (`CHAT_HISTORY_RESERVATION_UNAVAILABLE` otherwise).
    #[serde(default)]
    pub minimum_state_version: Option<i64>,
    /// Client idempotency identity for the create/turn (Aevatar replays the
    /// same conversation/turn for a repeated id, 409s on payload mismatch).
    #[serde(default)]
    pub command_id: Option<String>,
}

/// Opaque client token (command ids): UUID-shaped material only, so nothing
/// structural or attacker-shaped rides through to the upstream body.
fn validate_client_token(value: &str, label: &str) -> AppResult<()> {
    let valid = !value.is_empty()
        && value.len() <= 64
        && value.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    if !valid {
        return Err(AppError::BadRequest(format!("Invalid {label}.")));
    }
    Ok(())
}

/// Build the upstream `/api/chat` body for a caller turn request.
///
/// The `conversation` object is always present: without it Aevatar runs the
/// turn ephemerally and **persists nothing** to chat history, which would
/// silently drop the conversation from the sidebar contract. `workflow` is
/// pinned to [`WORKFLOW_CHAT_WORKFLOW`]. A body `scopeId` is ignored by
/// Aevatar (trusted scope wins), so none is sent.
pub fn workflow_chat_body(request: &WorkflowChatTurnRequest) -> AppResult<serde_json::Value> {
    let prompt = request.prompt.trim();
    if prompt.is_empty() || request.prompt.chars().count() > WORKFLOW_CHAT_PROMPT_MAX_CHARS {
        return Err(AppError::BadRequest(format!(
            "Prompt must contain between 1 and {WORKFLOW_CHAT_PROMPT_MAX_CHARS} characters."
        )));
    }

    let conversation = match (&request.conversation_id, request.minimum_state_version) {
        (None, None) => serde_json::json!({ "conversationId": null }),
        (None, Some(_)) => {
            return Err(AppError::BadRequest(
                "minimumStateVersion requires a conversationId.".to_string(),
            ));
        }
        (Some(id), version) => {
            validate_conversation_id(id)?;
            if !id.starts_with(WORKFLOW_CHAT_CONVERSATION_PREFIX) {
                // A `nyxid-chat-…` actor id is a different surface; failing
                // fast beats an upstream CONVERSATION_NOT_FOUND after a run
                // was already admitted.
                return Err(AppError::BadRequest(
                    "Only workflow conversations can be continued here.".to_string(),
                ));
            }
            let Some(version) = version.filter(|v| *v > 0) else {
                return Err(AppError::BadRequest(
                    "Continuing a conversation requires the last observed minimumStateVersion."
                        .to_string(),
                ));
            };
            serde_json::json!({ "conversationId": id, "minimumStateVersion": version })
        }
    };

    let command_id = match &request.command_id {
        Some(id) => {
            validate_client_token(id, "commandId")?;
            id.clone()
        }
        None => uuid::Uuid::new_v4().to_string(),
    };

    Ok(serde_json::json!({
        "commandId": command_id,
        "conversation": conversation,
        "prompt": request.prompt,
        "workflow": WORKFLOW_CHAT_WORKFLOW,
    }))
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

    /// The chat-history index is shared across product surfaces. Both of
    /// ours stay — `nyxid-chat-…` actors (turns via `:stream`) and `chatc-…`
    /// workflow conversations (turns via the workflow-chat route) — while
    /// rows from any other surface are dropped.
    #[test]
    fn history_index_keeps_both_addressable_families() {
        let mut index = serde_json::json!({
            "conversations": [
                { "id": "chatc-650906f30cc985fa341477281303b6de", "title": "workflow chat" },
                { "id": CONV, "title": "assistant chat", "messageCount": 7 },
                { "id": "voicec-9c1d3f24a88b41f7", "title": "another product" },
                { "id": "nyxid-chat-bb78fd3b1d834e6b958f8dbc626cef17", "messageCount": 0 },
            ]
        });
        assert!(filter_chat_history_index(&mut index));
        let ids: Vec<&str> = index["conversations"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["id"].as_str().unwrap())
            .collect();
        assert_eq!(
            ids,
            vec![
                "chatc-650906f30cc985fa341477281303b6de",
                CONV,
                "nyxid-chat-bb78fd3b1d834e6b958f8dbc626cef17"
            ]
        );
    }

    #[test]
    fn history_index_filter_reports_no_change_when_every_row_is_addressable() {
        let mut index = serde_json::json!({
            "conversations": [{ "id": CONV }, { "id": "chatc-650906f30cc985fa3414" }]
        });
        assert!(!filter_chat_history_index(&mut index));
        assert_eq!(index["conversations"].as_array().unwrap().len(), 2);
    }

    /// Shape drift must not empty the sidebar: an index we do not recognise,
    /// or a row without a string id, is left exactly as upstream sent it.
    #[test]
    fn history_index_filter_leaves_unknown_shapes_untouched() {
        let mut array_shaped = serde_json::json!([{ "id": "chatc-1" }]);
        assert!(!filter_chat_history_index(&mut array_shaped));
        assert_eq!(array_shaped.as_array().unwrap().len(), 1);

        let mut missing_key = serde_json::json!({ "rows": [{ "id": "chatc-1" }] });
        assert!(!filter_chat_history_index(&mut missing_key));
        assert!(missing_key.get("conversations").is_none());

        let mut idless = serde_json::json!({ "conversations": [{ "title": "no id" }] });
        assert!(!filter_chat_history_index(&mut idless));
        assert_eq!(idless["conversations"].as_array().unwrap().len(), 1);
    }

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
        // Non-ASCII identities count UTF-16 code units like the upstream
        // C# validator: 65 `é`s are 65 units (130 UTF-8 bytes) and pass.
        let accented = "é".repeat(65);
        assert!(retry_path(USER, CONV, &accented, "step_a").is_ok());
    }

    #[test]
    fn rejects_control_segments_outside_the_upstream_grammar() {
        // The control-identity grammar forbids whitespace, control chars,
        // and `/ \ ? #` (upstream `TryValidateControlIdentity`); everything
        // else is accepted and percent-encoded.
        // `%` and dot-only segments are documented NyxID narrowings of the
        // upstream grammar: nested escapes would be 400'd by the proxy's
        // repeat-decode hardening, and `.`/`..` are traversal-shaped.
        for bad in [
            "",
            "abc/def",
            "abc\\def",
            "abc?x=1",
            "abc#frag",
            "abc def",
            "tab\there",
            "\u{7}bell",
            "%2e%2e",
            "a%2Fb",
            ".",
            "..",
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

    const WORKFLOW_CONV: &str = "chatc-650906f30cc985fa341477281303b6de";

    fn turn_request(json: serde_json::Value) -> WorkflowChatTurnRequest {
        serde_json::from_value(json).unwrap()
    }

    /// The upstream body is server-built end to end: pinned workflow, always
    /// a `conversation` object (persistence contract), never a `scopeId`.
    #[test]
    fn workflow_body_creates_a_conversation_with_the_pinned_workflow() {
        let body = workflow_chat_body(&turn_request(serde_json::json!({
            "prompt": "hi",
            "commandId": "0d4b0a52-3d5f-4d2e-9f10-8f6f9b1c2d3e",
        })))
        .unwrap();
        assert_eq!(
            body,
            serde_json::json!({
                "commandId": "0d4b0a52-3d5f-4d2e-9f10-8f6f9b1c2d3e",
                "conversation": { "conversationId": null },
                "prompt": "hi",
                "workflow": "studio",
            })
        );
    }

    #[test]
    fn workflow_body_continues_with_the_observed_state_version() {
        let body = workflow_chat_body(&turn_request(serde_json::json!({
            "prompt": "and then?",
            "conversationId": WORKFLOW_CONV,
            "minimumStateVersion": 18,
        })))
        .unwrap();
        assert_eq!(body["conversation"]["conversationId"], WORKFLOW_CONV);
        assert_eq!(body["conversation"]["minimumStateVersion"], 18);
        assert_eq!(body["workflow"], "studio");
        // No client commandId: the server mints one (idempotency identity
        // must always exist for create-replay recovery).
        assert!(
            body["commandId"].as_str().is_some_and(|id| !id.is_empty()),
            "server must mint a commandId"
        );
    }

    #[test]
    fn workflow_body_rejects_out_of_contract_turns() {
        // Empty / oversized prompt.
        assert!(workflow_chat_body(&turn_request(serde_json::json!({ "prompt": "  " }))).is_err());
        assert!(
            workflow_chat_body(&turn_request(
                serde_json::json!({ "prompt": "a".repeat(32_769) })
            ))
            .is_err()
        );
        // Continuation without the read fence (upstream would 503).
        assert!(
            workflow_chat_body(&turn_request(serde_json::json!({
                "prompt": "hi", "conversationId": WORKFLOW_CONV,
            })))
            .is_err()
        );
        assert!(
            workflow_chat_body(&turn_request(serde_json::json!({
                "prompt": "hi", "conversationId": WORKFLOW_CONV, "minimumStateVersion": 0,
            })))
            .is_err()
        );
        // Fence without a conversation.
        assert!(
            workflow_chat_body(&turn_request(serde_json::json!({
                "prompt": "hi", "minimumStateVersion": 3,
            })))
            .is_err()
        );
        // A nyxid-chat actor id is the other surface.
        assert!(
            workflow_chat_body(&turn_request(serde_json::json!({
                "prompt": "hi", "conversationId": CONV, "minimumStateVersion": 3,
            })))
            .is_err()
        );
        // Path-escaping conversation ids and non-token command ids.
        assert!(
            workflow_chat_body(&turn_request(serde_json::json!({
                "prompt": "hi", "conversationId": "chatc-a/b", "minimumStateVersion": 3,
            })))
            .is_err()
        );
        assert!(
            workflow_chat_body(&turn_request(serde_json::json!({
                "prompt": "hi", "commandId": "not a token!",
            })))
            .is_err()
        );
        // Unknown fields fail loudly instead of silently dropping.
        assert!(
            serde_json::from_value::<WorkflowChatTurnRequest>(serde_json::json!({
                "prompt": "hi", "workflow": "direct",
            }))
            .is_err(),
            "a caller-supplied workflow selection must be rejected"
        );
        assert!(
            serde_json::from_value::<WorkflowChatTurnRequest>(serde_json::json!({
                "prompt": "hi", "scopeId": "someone-else",
            }))
            .is_err()
        );
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
