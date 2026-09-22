# NyxAgent chat authority: retained design record (superseded by 08)

Status: retained D10–D15 design record (2026-09-17). The implemented normative
contract is [08-nyxagent-engine.md](08-nyxagent-engine.md), which supersedes this brief.

## Goal

1. The assistant must be able to **manage the user's NyxID account from chat**:
   agent-key permissions and bindings, channel bots and their agent routes,
   connected-service lifecycle, nodes, approval settings.
2. **Each chat conversation gets its own agent key**, and that key may use a
   connected service, manage the account, or perform a destructive action
   **only after the user has acknowledged it in that same chat**.

## Facts that fix the design (verified in this checkout)

- Every management route (`/api-keys`, `/keys`, `/user-services`,
  `/channel-bots`, `/channel-conversations`, `/approvals`, `/orgs`, `/catalog`)
  is mounted in `api_v1_human_only` in `routes.rs`, which layers
  `reject_api_key_tokens`. A catalog row pointing NyxID at its own REST API and
  forwarding the agent key therefore cannot work. Management must be exposed as
  **native MCP tools dispatched in-process** through the service layer, never
  through an HTTP loopback.
- NyxAgent (`/tmp/nxyagent/src/nyxid.rs`) gives the model only the six
  `nyx__*` tools; everything else is reached through `nyx__search_tools` and
  `nyx__call_tool`. Its `rpc()` treats a JSON-RPC-level `error` as an opaque
  502, but a `tools/call` **result** with `isError: true` reaches the model as
  text (`handlers/mcp_transport.rs::tool_result`). Every refusal defined below
  must be returned as an `isError` tool result with a JSON body, never as a
  JSON-RPC error.
- NyxAgent's model bridge calls `/api/v1/proxy/s/chrono-llm-public/responses`
  with the same key, so a conversation key must be able to reach auto-connected
  platform services from its first turn (`allow_auto_connected_services`).
- The existing approvals system (`service_approval_configs`, `approval_requests`,
  `approval_grants`) is per user-service opt-in and blocks the proxied call
  while waiting. Do not repurpose it for chat acknowledgements; the
  acknowledgement flow below is separate, non-blocking, and conversation-bound.
  Existing approval configs continue to apply to proxied calls as today.

## Decisions (implement all)

### D10. One agent key per conversation

- `assistant_agent_credentials` becomes per conversation: add
  `conversation_id` (unique index), keep `user_id` (non-unique index) and
  `api_key_id`. Provision inside `begin_turn` for a new conversation (same
  transaction), not lazily per user. Keys: name `NyxID Assistant chat
  <first 8 hex of the conversation id>`, platform `nyxid-assistant`, scopes
  `proxy`, `allow_all_services = false`, `allowed_service_ids = []`,
  `allow_auto_connected_services = true`, `allow_all_nodes = false`,
  `allowed_node_ids = []`, no expiry, purpose General.
- Deleting a conversation deletes its key through `key_service::delete_api_key`
  (which already revokes children and clears bindings) and its credential row.
  Account deletion/disable cascades as today plus the new collection.
- Revoke/rotate/expiry handling stays as shipped, now keyed by the credential
  row's `api_key_id`; replacement re-provisions for that conversation with an
  empty allowlist (all acknowledgements must be given again) and emits the
  existing `context_reset` notice.
- `GET /assistant/nyxagent/models` uses the caller's most recently used valid
  credential row, else the fallback list.
- A `nyxid-assistant` key can never be selected as a channel route agent
  (`channel-conversations` create/update and the channel route CLI/UI paths
  reject it with a clear validation error) and can never be widened by the
  chat tools (`nyxid__update_agent_key` refuses platform `nyxid-assistant`).
- API keys page: assistant chat keys are hidden by default behind a
  "Show assistant chat keys" toggle; the key detail page shows "Used by
  assistant chat" with a link to `/assistant?c=<id>`.

### D11. Acknowledgements

New collection `assistant_acknowledgements` (model + `COLLECTION_NAME`, UUID
`_id`): `conversation_id`, `user_id`, `api_key_id`, `kind`
(`service` | `account` | `action`), `service_id`/`service_slug`/`service_name`
(service kind), `tool_name` + `arguments_digest` + `summary` (action kind),
`status` (`pending` | `allowed` | `denied` | `expired` | `used`),
`created_at`, `decided_at`, `expires_at` (pending: 15 minutes; allowed
`action`: 10 minutes, single use), redacted Debug. Indexes:
`(conversation_id, status, created_at)`, TTL is not used (rows are part of the
transcript record; expire lazily like the turn fence).

Semantics, enforced in the MCP dispatch path for calls authenticated with a
conversation key (identify the key by its credential row, not by platform
string alone):

- `nyx__list_connected_services` lists **all** the user's connected services
  (same visibility a session user has) and adds `chat_access`:
  `"granted"` (in the key's allowlist or auto-connected) or
  `"acknowledgement_required"`. `nyx__search_tools` includes tools of
  non-granted services and the `nyxid` account tools with the same field, so
  the model can plan and ask.
- `nyx__call_tool` on a service tool whose service is not granted:
  find-or-create a pending `service` acknowledgement for
  `(conversation, service)` and return an `isError` result
  `{"error":"acknowledgement_required","kind":"service","acknowledgement_id":...,
  "service_slug":...,"service_name":...,"instructions":"Ask the user to approve
  access to <name> for this chat (a card is shown in the chat), then retry."}`.
  Allowed → the service's `UserService` id is added to the key's
  `allowed_service_ids` (through the key-mutation service, bumping
  `state_version`), the row becomes `allowed`, and the retry succeeds with no
  further prompt for that service in that conversation. Denied → result
  `acknowledgement_denied` with the same shape; a later call may open a new
  request only if the user asks again (the model is told so in `instructions`).
- `nyxid__*` tools require an `account` acknowledgement for the conversation
  (one per conversation, persisted by adding the key scope
  `assistant:account`; add that constant to `mw/auth.rs`, exclude it from
  `VALID_API_KEY_SCOPES` so users cannot self-assign it, and never let it
  grant anything on REST routes).
- Destructive `nyxid__*` tools additionally require a one-time `action`
  acknowledgement: the first call without a valid `acknowledgement_id`
  argument creates a pending `action` row whose `summary` is a human sentence
  ("Delete agent key 'ci-bot' (nyxid_ag_12345678)") and whose
  `arguments_digest` is SHA-256 over the canonical JSON of the tool arguments
  without `acknowledgement_id`; the result tells the model to ask the user to
  confirm the card and retry with `acknowledgement_id`. The retry succeeds only
  if the row is `allowed`, unexpired, unused, bound to the same conversation
  key, and its digest matches; it is then marked `used`. Any mismatch is a
  refusal, never a silent proceed.
- Decisions are human-only:
  `POST /assistant/nyxagent/conversations/{id}/acknowledgements/{ack_id}`
  body `{ "decision": "allow" | "deny" }`, owner-scoped, flag-gated, 409 on a
  non-pending row, and every decision is audited (metadata only:
  conversation, kind, service or tool, decision).
- The history DTO gains `acknowledgements` (pending and the last 20 decided,
  with `id, kind, status, summary, service_slug, service_name, tool_name,
  created_at, decided_at, expires_at`; never arguments). The index DTO gains
  `pending_acknowledgements: u32`.

### D12. Native `nyxid` account service

Expose a virtual MCP service (slug `nyxid`, name "NyxID account", category
`internal`, a new `McpToolSource::Internal` variant, `executable: true`) whose
tools dispatch in-process with the calling key's `AuthUser` context to the
same service-layer functions the REST handlers use. Never call NyxID over
HTTP. Tool inventory (closed set; each has a JSON schema, a description that
states when it is destructive, and returns bounded JSON):

- Agent keys: `nyxid__list_agent_keys`, `nyxid__get_agent_key`,
  `nyxid__update_agent_key` (name, description, platform, callback_url,
  rate limits, `allow_all_services`, `allow_auto_connected_services`,
  `allow_all_nodes`, `allowed_service_slugs` resolved to ids, `allowed_node_ids`;
  refuses `nyxid-assistant` keys), `nyxid__delete_agent_key` (destructive),
  `nyxid__list_agent_key_bindings`, `nyxid__bind_agent_key_credential`,
  `nyxid__unbind_agent_key_credential` (destructive).
- Channel bots: `nyxid__list_channel_bots`, `nyxid__get_channel_bot`,
  `nyxid__update_channel_bot` (label and non-secret fields only; the schema
  has no token/secret fields), `nyxid__delete_channel_bot` (destructive),
  `nyxid__list_channel_routes`, `nyxid__set_channel_route` (bot conversation
  → agent key, default agent, `allow_agent_initiated`), `nyxid__delete_channel_route`
  (destructive).
- Connected services: `nyxid__list_my_services` (the `/keys` listing including
  `is_active`), `nyxid__set_service_enabled`, `nyxid__delete_service`
  (destructive).
- Nodes: `nyxid__list_nodes`, `nyxid__delete_node` (destructive).
- Approvals: `nyxid__list_approval_configs`, `nyxid__set_approval_mode`,
  `nyxid__list_pending_approvals` (read-only; deciding approvals from chat is
  excluded on purpose).

Explicitly excluded (the model directs the user to the UI): creating or
rotating agent keys (secrets would enter the model context), entering any
credential or token, deciding approvals, org administration, billing.

Every `nyxid__*` call writes one audit event with `api_key_id`,
`conversation_id`, tool name, target id, outcome, and the acknowledgement id
when one was used. Errors from the service layer map to bounded `isError`
results using the existing `AppError` codes; internal details never leak.

### D13. Frontend

- Inline acknowledgement cards in the NyxAgent chat: pending cards render at
  the transcript tail with Allow/Deny buttons and per-kind copy ("Allow this
  chat to use GitHub?", "Allow this chat to manage your NyxID account (keys,
  channel bots, services, nodes, approval settings)?", "Confirm: <summary>");
  decided cards render in place as a compact status line. Cards are keyed by
  acknowledgement id, never auto-decided, and use a ≥ 750 ms throttle like the
  other human decision surfaces. The history poll runs at 2 s while a turn is
  active **or** a pending acknowledgement exists.
- After a decision the composer gets focus so the user can tell the assistant
  to continue; no message is sent automatically.
- API keys page toggle and key detail link per D10.
- Update the HTTP fixtures so the e2e suite covers: service acknowledgement
  request → Allow → retry succeeds; Deny → refusal; action acknowledgement
  for a destructive tool; hidden assistant keys toggle.

### D14. Docs, config, tests

- `08-nyxagent-engine.md`: replace the per-user credential section with the
  per-conversation model, add "Acknowledgements" and "NyxID account tools"
  sections with the exact tool inventory, result shapes, and audit contract.
- CLAUDE.md: Rule 5 credential exception wording (per conversation), Rule 9
  note that `nyxid-assistant` keys are chat-scoped and cannot be route
  agents, the new scope constant, the new route, and the new collection in
  the NyxAgent paragraph. `docs/ENV.md`: still no environment variable.
- Version stays 0.24.0 (unreleased branch).
- Tests at the same density as the shipped work: acknowledgement lifecycle
  (pending/allow/deny/expire/used, digest binding, single use, wrong
  conversation), MCP gating for conversation keys (listing fields, refusal
  shapes as `isError` results, allow → retry passes), every `nyxid__*` tool's
  authorization and audit, the self-widening and route-agent refusals,
  per-conversation provisioning and deletion, keys-page toggle, cards and
  polling, e2e flows. Re-run the full verification set from the first brief
  and report verbatim.

### D15. Access mode: "Ask before acting" (default) or "Full access"

Mirror Codex's permission modes. Each conversation has `access_mode`:
`ask` (default; D11 acknowledgements apply) or `full` (the chat may use every
connected service, manage the account, and run destructive actions without
per-action cards).

- Storage: `AssistantConversation.access_mode` (serde default `ask`), exposed
  on the conversation DTOs. Switching is a human-only, owner-scoped, flag-gated
  route `PATCH /assistant/nyxagent/conversations/{id}/access-mode`
  `{ "access_mode": "ask" | "full" }`, refused with `turn_active` while a turn
  is live. A new draft carries the mode chosen in the composer selector into
  its first `POST /turns` (`access_mode` optional field, default `ask`).
- Effect on the conversation key, applied in the same transaction as the mode
  change: `full` sets `allow_all_services = true`, `allow_all_nodes = true`,
  keeps `allow_auto_connected_services = true`, and adds the `assistant:account`
  scope; `ask` reverts `allow_all_services`/`allow_all_nodes` to false and
  removes the scope, but keeps `allowed_service_ids` accumulated from earlier
  acknowledgements (they were individually approved).
- MCP behaviour in `full` mode: `chat_access` is `"granted"` everywhere, no
  `service`/`account`/`action` acknowledgement is created, and destructive
  `nyxid__*` tools execute directly. Every tool call is still audited with
  `access_mode: "full"`. The chat's own key still cannot be widened by the
  tools, and the D12 exclusions (key creation/rotation, credential entry,
  deciding approvals, org admin, billing) still hold; they are not
  permissions but secret-handling boundaries.
- Frontend: a "Mode" selector next to the profile selector on the NyxAgent
  composer with the two options and one-line descriptions; choosing "Full
  access" opens a confirmation dialog that states exactly what the chat may
  then do (use all connected services and nodes, change agent keys, channel
  bots, services, nodes and approval settings, delete resources) and requires
  an explicit confirm; the selector is disabled while a turn is active. The
  active mode is shown as a small badge in the chat header ("Full access").
  The last chosen mode is remembered for new drafts in the assistant draft
  store (browser-local, per user); it is never applied to an existing
  conversation without the explicit switch.
- Audit: `assistant_access_mode_changed` with conversation id, old/new mode.
- Docs: add the mode to `08-nyxagent-engine.md` and to the CLAUDE.md NyxAgent
  paragraph. Tests: mode switch transaction and key effects both ways,
  `turn_active` refusal, MCP behaviour in `full` mode (no cards, audited),
  selector + confirmation dialog, e2e for switching a chat to full access and
  running a destructive tool without a card.
