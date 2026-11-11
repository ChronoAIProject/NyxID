# Activity history compatibility

Activity failure metadata describes an individual step, not the validity of the conversation. Web and mobile accept any string `failure.kind`. Known mappings in `assistant-history-compat.ts` (web) and `assistantHistoryCompat.ts` (mobile) are:

| Kind | Backend producer | Presentation |
| --- | --- | --- |
| `service_status` | `mcp_transport::service_tool_content` | Service HTTP error, optional status |
| `tool_error` | `mcp_transport::rpc_success`, `tool_failure`, native memory diagnostics | Tool error; existing trusted memory reasons retained on web |
| `refused` | `mcp_transport::rpc_error`, `tool_failure` | NyxID refused this call |
| `internal` | `mcp_transport::rpc_error`, `tool_failure` | NyxID could not complete this call |
| `resource_unavailable` | `mcp_transport` managed resource materialization | A required resource is unavailable |
| `retry_duplicate_action` | `assistant_nyxagent::RETRY_DUPLICATE_ACTION`, persisted directly in BSON with `status: completed` | Neutral skip: already done in the previous attempt |

Unknown kinds have generic per-activity presentation. They never imply another successful execution. Unknown activity/message statuses become `unknown`, rather than completed. The retry duplicate is not an error or a new successful execution; the server remains the authority for preventing duplicate effects.

The backend's `ToolFailure.kind` is a String, and the retry producer bypasses the struct to append BSON. There is no exhaustive Rust kind enum. The client tests pin this audited six-kind documentation and both client registries, plus arbitrary future strings. A lexical backend-source test could miss computed strings or incorrectly include unrelated `kind` fields, so it would not prove exhaustiveness. When adding a producer, update this table and both registries if it needs specialized presentation. Runtime tolerance does not depend on doing so.

Read-side sibling metadata is tolerant: activity and message statuses, message/conversation roles, embedded conversation agent kind, title source (web), recovery status, feedback vote, steering outcome (web; mobile was already open), machine receipt status (web; mobile was already open) and context mode, attachment image-input metadata, CMA operation kind, and platform-thread platform (web; mobile ignores platform_thread). Unknown values do not become success, owner, applied steering, available recovery, or a positive feedback vote.

Authorization records remain strict: acknowledgement kind/status/decider, confirmation risk/access mode, execution proposals, approval mode/status, and continuation identity. Unsupported optional cards are isolated at the history envelope, counted in the client-derived `unsupported_history_items`, and announced with a refresh/update notice. They expose no decision action. Required structure/identity/booleans/numbers and outbound control requests remain strict. SSE discriminators remain strict because unknown frames are already ignored individually rather than invalidating history. Agent management/creation schemas remain strict; only embedded read-side conversation metadata changes. Platform login/link/consent status and permission-mode schemas also remain strict because they control a connection, rather than describe a historical step. Group membership/ownership and collaboration permission DTOs are separate authority surfaces; their enums remain strict. Group activity arrays reuse the tolerant activity schema.

A malformed history response produces fixed friendly copy and a history GET retry. It retains already validated transcript state and does not make the composer read-only. Server-derived destroyed/can_use gates and active-turn, recovery, identity and send admission safeguards still apply. Invalid identity or required structure is not silently accepted.

The shared fixture `frontend/src/lib/assistant/__fixtures__/retry-duplicate-history.json` reconstructs the exact #2345 fragment at `conversation.active_turn.activities[1].failure.kind`. The issue does not include a complete production payload. The fixture has the reported 12 completed actions and one running action, with no production content.
