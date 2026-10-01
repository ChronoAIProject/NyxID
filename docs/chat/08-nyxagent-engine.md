# NyxAgent assistant engine

Normative NyxID 0.24.0 contract, verified against the read-only NxyAgent clone
at commit `58f647e4`. The retained design records are
[nyxagent-engine-plan.md](nyxagent-engine-plan.md) and
[nyxagent-chat-authority-plan.md](nyxagent-chat-authority-plan.md); this document records the
implemented contract and resolves its assumptions against upstream source.

## Engine and trust boundary

New-draft precedence is `assistant:nyxagent-engine` (default **on**), then
`experimental:direct-chat-engine` (default off), then Aevatar. Existing
`nyxa-{32 lowercase hex}`, `direct-*`, `nyxid-chat-*`, and `chatc-*` identifiers
select their respective enabled surface. Aevatar history and action-effect
routes remain available. An upstream error never switches engines.

All NyxAgent routes live under the human-only `/api/v1/assistant` router.
API keys, service-account, delegated, and relay tokens cannot use them. Each
handler independently enforces the effective per-person engine flag. Disabled
routes and another person's conversation return not-found-shaped responses.
No request accepts an owner, upstream URL, secret, or upstream session ID.

## Deployment prerequisites

The active admin-managed catalog row `llm-nyx` supplies the destination.
Required settings: `requires_user_credential=false`, `auth_method=none`,
`forward_access_token=true`, `inject_delegation_token=false`. Readiness reports
`master_credential_configured` the way the admin API does (decrypted content;
legacy create paths stored an encrypted empty string), but it does not fail the
contract: with `auth_method=none` the service-credential layer never injects it. A user-owned service cannot shadow this row. `execute_admin_proxy`
performs the existing billing/proxy checks. Turns, model discovery, and upstream
session deletion carry `Metered(Proxy)` billing policy. The initial hop uses
server transport; user credential-node settings cannot reroute it.

The **NyxAgent deployment** must set `NYXAGENT_SERVICE_SLUG=llm-nyx` so its
`self_prefixes` recursion guard excludes `llm-nyx__*` tools. An orchestrator key
can access all services; this guard prevents it from calling NyxAgent recursively.
NyxID cannot inspect or verify that upstream environment setting. This introduces
no NyxID environment variable.

## Per-conversation credential

The first `begin_turn` transaction creates one ordinary key for the conversation,
its encrypted credential row, its user message and its active-turn fence together.
Keys are named `NyxID Assistant chat <first 8 hex of the conversation id>`, have
platform `nyxid-assistant`, purpose General, and no expiry. Its authority follows the
conversation's role (see Authority below) and is re-applied at every turn start,
so a rotated, replaced or hand-edited key converges back. Conversation keys always
allow all nodes, including after rotation and replacement, because nodes are
reachable only through services and service consent is the node consent; a
subagent's service allowlist remains its execution gate.
The ordinary key registry does not accept `llm:proxy` or `assistant:account` as
user-assigned scopes; `proxy` already authorizes MCP and LLM proxy access.

`assistant_agent_credentials` stores UUID `_id`, `conversation_id`, `user_id`,
`api_key_id`, BSON binary `key_ciphertext`, and BSON `created_at`/`last_used_at`.
Conversation and key indexes are unique; owner + descending last-used is non-unique.
Startup retires legacy per-person credentials and their keys before removing the
old unique owner index. Concurrent first turns cannot produce duplicate keys.
`EncryptionKeys` supplies envelope encryption; raw material uses `Zeroizing`.
This per-conversation record is the deliberate exception to hash-only API-key
storage: NyxAgent HMAC-binds its session owner to the exact raw key. Debug is
redacted. No API, audit, log, or error includes the raw key or its ciphertext.

Every turn verifies owner, active state, expiry and the decrypted hash. Invalid
credentials are replaced for that conversation only. Revoke removes the encrypted
record. Rotation re-encrypts its successor in the same record/transaction; it never
strands an additional key. Replacement and rotation expire pending and allowed
acknowledgements, carry the conversation's role authority (Full for an
orchestrator, the durable grants for a subagent) to the successor, and clear the
upstream binding with `credential_replaced`. Assistant rotation
returns empty `full_key`; browser and CLI explain that its secret remains encrypted
on the server. Ordinary key rotations retain their one-time secret delivery.
Settlement writes the credential record to fence concurrent invalidation.
Provisioning and replacement audit `assistant_agent_credential_provisioned` and
`assistant_agent_credential_replaced` with metadata only, after commit.

Conversation deletion revokes its current key (including a rotated successor),
revokes child credentials through the key service, and removes credential,
acknowledgement, transcript and conversation rows in the same transaction.
Account disable removes all encrypted assistant credentials; account deletion
cascades all four assistant collections. Model discovery uses the most recently
used valid credential belonging to the caller, or the uncached fallback.

Assistant keys are hidden by default on the API keys page behind **Show assistant
chat keys**. Key metadata includes only `assistant_conversation_id`; the detail
page links **Used by assistant chat** to `/assistant?c=<id>`. Channel route creation,
updates and dispatch reject assistant keys, and the route selectors exclude them.
Native chat tools cannot delete, widen, relabel, bind credentials to, or select any
assistant key as a route agent, including keys belonging to another chat. The user
deletes the conversation instead. Rotation and revocation remain available to the human user.

## Authority: Full-only NyxBot, granted specialists

The Ask/Full choice is retired. Every owner has one persistent **NyxBot** agent;
its threads run with Full access: `allow_all_services=true`, `allow_all_nodes=true`,
auto-connected access and the internal `assistant:account` scope. Service and
account cards are never created for NyxBot and `chat_access` is `granted` for
every listed service/tool. Existing per-service proxy approval policies still
apply. Destructive account tools still require a single-use **action** card
unless the owner turned confirmations off in NyxBot settings
(`skip_destructive_confirmation`, default `false`).

A **specialist** agent (see NyxBot agents below) holds only its grants on every
one of its threads: `allow_all_services=false`, `allow_auto_connected_services=false`,
`allowed_service_ids`/`allowed_platform_service_ids` from the agent's `grants`, and
`assistant:account` only with an `account_read` grant, which reaches read-only
account tools. The agent document is the durable source; thread keys mirror it
at every turn start, rotation and replacement. NyxAgent calls its model through
NyxID's proxy (`/api/v1/proxy/s/{model service}`, e.g. `chrono-llm-public`,
`llm-deepseek`) with the thread's own key. The proxy therefore lets a live
assistant conversation key (an `assistant_agent_credentials` row, which people
cannot create) reach any active catalog service with `inference` metadata even
outside its grants (`proxy::assistant_model_call`), whether the owner's own row,
an org row or the catalog row resolves it. Nothing else widens: the key's
allowlist, inventory, delegation tokens, consent screens and MCP tools stay its
grants.

Legacy conversations deserialize with `access_mode: ask` and no `agent_id`. The
next `begin_turn` adopts them as NyxBot threads inside the transaction that
claims the turn: the row is set to `full`, the key converges to Full, and pending
service/account cards expire. There is no startup sweep, so no upgrade races a
live turn. `PATCH /conversations/{id}/access-mode` answers `410 Gone` with
`{"error":"access_mode_retired"}`; `POST /turns` still accepts `access_mode` from
older clients and ignores it. DTOs report `access_mode: "full"`.

## Acknowledgements

`assistant_acknowledgements` is a transcript collection with UUID `_id`, owner,
conversation, credential key, kind (`service`, `account`, `action`), applicable
service id/slug/name or tool/argument digest/summary, status, creation/decision/
expiry dates, and the last persisted user turn that requested it. Debug is
redacted. Index `(conversation_id,status,created_at)` has no TTL. Pending requests
expire lazily after 15 minutes; allowed actions expire after 10 minutes and can
be consumed only once. Decisions and key mutations serialize against the current
conversation and credential generation. Service and account requests come only
from specialist threads and carry `decider: "orchestrator"` (NyxBot decides) and a
bounded `request_excerpt` of the message that started the requesting work (the
user's own words when the user spoke to the specialist). Action confirmations
keep `decider: "user"`. Decided rows record `decided_by` (`user` or `orchestrator`)
and NyxBot's bounded `reason`.

Only keys identified by their actual credential row get this authority; a platform
label alone grants nothing. Their MCP listing/search includes all services visible
to the owner, with `chat_access: "granted" | "acknowledgement_required"`, plus the
virtual `nyxid` account service. Node and proxy approval enforcement remain in the
execution path. Refusals are MCP `tools/call` **results** with `isError: true`, never
JSON-RPC errors (which NyxAgent would flatten to opaque 502s):

```json
{
  "error": "acknowledgement_required",
  "kind": "service",
  "acknowledgement_id": "<uuid>",
  "service_slug": "github",
  "service_name": "GitHub",
  "summary": "Use GitHub",
  "decider": "orchestrator",
  "instructions": "NyxID asked your orchestrator for this permission. End your turn now with a one-line note about what you are waiting for; NyxID resumes you with the decision."
}
```

Chat-key `nyx__search_tools` and `nyx__list_connected_services` results also carry
a top-level `chat_access_hint` that spells out the three values, and the server
prompt tells the model that `acknowledgement_required` tools are callable and that
the card, not settings, is how access is granted. Without this the model read the
flag as "no permission" and never made the call that creates the card.

Platform-source services have a DownstreamService ID and no owner-visible
UserService row. A subagent requests them the same way: the refusal is the ordinary
`acknowledgement_required` result, the card summary reads "Allow this chat to use
<name> (NyxID platform credential)?", and Allow records the catalog ID on the key's
`allowed_platform_service_ids` (never on `allowed_service_ids`, whose REST
validation admits only UserService rows). Only assistant chat keys hold platform
grants; `ensure_service_in_scope` honours them for chat keys alone, execution
still resolves the service through the owner's visible platform grants on every
call, and orchestrators are granted everything. `full_access_required` is no
longer produced. Chat keys may mint hosted connect links for any catalog service
(`nyx__connect_service` skips the allowlist for chat keys); a subagent still needs
a grant for the resulting connection.

Account/action refusals use the same fields (`service_slug`/`service_name` are null)
and `kind: "account" | "action"`; action instructions require a retry with
`acknowledgement_id`. Denial returns `error: "acknowledgement_denied"` and instructs
the model to wait for an explicit request in a later user message. Repeated calls
in the same user turn, including after settlement, reuse that denial. Pending
requests deduplicate transactionally. Allowed service requests add the UserService
ID through the key-mutation service, incrementing `state_version`; allowed account
requests add exactly `assistant:account`. This scope never grants REST management
access, and public key creation/edit scope validation excludes it.

Action confirmation binds SHA-256 of recursively canonical JSON arguments, removing
only the top-level `acknowledgement_id`. Consumption matches the conversation,
owner, current key, tool, digest, allowed status and expiry in a transaction.
Mismatch/expired/used confirmations return an `isError` body with
`error: "acknowledgement_invalid", kind: "action"` and retry instructions.

`POST /conversations/{id}/acknowledgements/{ack_id}` accepts closed
`{"decision":"allow"|"deny"}` through the human-only router. Other owners see 404;
non-pending decisions return 409. Each successful decision audits
`assistant_acknowledgement_decided` with conversation, kind, service/tool and decision;
arguments, summaries, secrets and ciphertext are excluded from audit data.

History includes all pending and the last 20 decided records, exposing only
`id,kind,status,summary,service_slug,service_name,tool_name,created_at,decided_at,
expires_at`. The conversation index includes `pending_acknowledgements`. Pending
cards appear at the transcript tail; decided cards become compact status lines at
the request's timestamp. Allow/Deny are explicit human actions with a 750 ms
minimum throttle. The selected history refreshes when the owner's live stream
(`GET /assistant/nyxagent/live`, identifiers only) reports a change to it; while
that stream is open, the two-second poll (while a turn runs or a pending card
exists) becomes a 30-second backstop, and without it the two-second poll
applies. The index refreshes at settlement/count changes, mutations and live
events; it does not poll every two seconds.

The refusal tells the model to retry after approval, and NyxAgent cannot wait for
the decision inside its own turn, so the turn that requested the card ends before
the decision. A subagent's request is resumed by NyxID: the decision (by the
orchestrator through `nyxid__decide_permission`, or by the user on the card in the
subagent's thread) queues a `permission_decided` event and wakes the subagent with
an event turn; the browser sends nothing. For user-decided cards in an
orchestrator's thread (action confirmations), a successful **Allow** resumes the
assistant: when no turn is running, the browser sends an ordinary, visible user turn (`Approved: this
chat may use <service>. Continue.`, `Approved: account management for this chat.
Continue.`, or `Confirmed: <summary> (acknowledgement_id <id>). Retry it now.`).
The continuation is a normal turn with no extra authority; the allowed grant is
what the retried tool call consumes. If the user allows a card while a turn is
still running (cards appear as soon as the tool call is refused, usually before
the reply finishes), the running turn cannot observe the decision: NyxAgent ends
a turn on a card and answers same-turn repeats locally. The browser therefore
queues the continuation and sends one visible turn covering every card allowed
during that turn as soon as it settles; nothing is sent if the user pressed Stop.
**Deny** sends nothing.

Independently, every turn's instructions end with a note listing the cards the
user allowed or denied since the previous user message, oldest first and at most
ten (`- allowed: service <slug>`, `- denied: account management`,
`- allowed: action <tool> (acknowledgement_id <id>)`). Only identifiers are
rendered, never display names or summaries. Each decision is reported to exactly
one turn, so the model learns about decisions made in another tab, after a
reload, or by Deny, the next time the user writes.

## NyxID account tools

The virtual `nyxid` service is named **NyxID account**, category `internal`, executable,
and uses `McpToolSource::Internal`. Dispatch is in-process through the same service
layer as REST; there is no HTTP loopback. Every tool has a closed input schema,
bounded JSON output (100 list rows, 64 KiB), and an explicit destructive description.
NyxBot reaches every tool; destructive tools add the single-use action
confirmation unless the owner turned confirmations off. Specialists reach only the
read-only tools (`list_*`, `get_*`) and only with an `account_read` grant; every
other tool returns `orchestrator_only`. NyxBot also gets the agent and channel
tools, and every agent gets its memory tools (see NyxBot agents). The closed
account inventory is:

| Area | Tools (prefix every name with `nyxid__`) | Destructive |
| --- | --- | --- |
| Agent keys | `list_agent_keys`, `get_agent_key`, `update_agent_key`, `delete_agent_key`, `list_agent_key_bindings`, `bind_agent_key_credential`, `unbind_agent_key_credential` | `delete_agent_key`, `unbind_agent_key_credential` |
| Channel bots/routes | `list_channel_bots`, `get_channel_bot`, `update_channel_bot`, `delete_channel_bot`, `list_channel_routes`, `set_channel_route`, `delete_channel_route` | `delete_channel_bot`, `delete_channel_route` |
| Connected services | `list_my_services`, `set_service_enabled`, `delete_service` | `delete_service` |
| Nodes | `list_nodes`, `delete_node` | `delete_node` |
| Approvals | `list_approval_configs`, `set_approval_mode`, `list_pending_approvals` | None |

Key updates accept metadata, callback URL, rate limits, service/node flags,
`allowed_service_slugs` resolved to owner-visible IDs and node IDs. Bot updates
accept label/app ID only. Service listing uses the unified `/keys` service,
including `is_active`; lifecycle uses the existing enable/disable/delete paths.
Keys, bindings, bots, routes, services, nodes and approval settings retain owner
checks and not-found shaping. Responses explicitly project safe fields.

Creating/rotating keys, entering credentials/tokens, deciding approvals, organization
administration and billing are excluded; users complete those in the UI. Credential
entry through the existing MCP connect helper is likewise refused for conversation
keys. Full access does not bypass these boundaries or the ban on
self-widening/assistant route agents, which covers subagent keys too.

Every native call writes one `assistant_account_tool_call` audit with `api_key_id`,
`conversation_id`, `tool_name`, validated target UUID when applicable, outcome,
agent `role` and presented acknowledgement UUID when applicable. No arguments,
response bodies, credentials, user-controlled URLs or raw errors are recorded.
Execution/state-changing chat MCP requests (`nyx__call_tool`, `nyx__connect_service`,
`nyx__wait_for_connection`, `nyx__ssh_exec`, `nyx__oracle_*`, and `nyxid__*`), including
refusals, additionally record an
`assistant_mcp_tool_call` entry with the conversation, key, `agent_role` and
`requested` outcome. Only closed native/meta tool names are recorded at this boundary;
resolved service execution events supply the service/tool identity. Service
proxy call audits also include the chat's `agent_role`. Read-only discovery
(`nyx__search_tools`, `nyx__list_connected_services`, `nyx__discover_services` and
SSH service listing) does not add an `assistant_mcp_tool_call` event; existing
execution audits remain unchanged. Service-layer
errors become bounded `isError` results with existing `AppError` codes and static
safe messages; internals never enter the model context.

## NyxBot agents

Normative design: [09-nyxbot-orchestrator.md](09-nyxbot-orchestrator.md).
`assistant_agents` holds one `nyxbot` per owner (unique partial index, created on
first use) and persistent `specialist` agents with a name, role description,
`grants`, `memory` (≤ 50 notes of ≤ 500 characters), `home_conversation_id`,
`created_by`, `destroyed_at`, and an optional `display_name` (≤ 40) and
`persona` (≤ 2000, style only). Conversations are threads of an agent
(`agent_id`; legacy rows are NyxBot's) with a denormalized `role`, `report_to`,
`pending_events` and `event_streak`. NyxAgent is unchanged: every thread is an
ordinary conversation with its own key.

NyxBot-only native tools (`nyxid__` prefix): `spawn_subagent`, `message_subagent`,
`wait_for_subagents` (at most 120 s), `list_subagents`, `read_subagent`,
`grant_subagent`, `revoke_subagent`, `set_guest_access`, `update_subagent`, `decide_permission`,
`destroy_subagent`, `create_group`, `list_groups`, `post_to_group`,
`update_group`, `delete_group`, `settings_link`, `channel_bot_setup_link`,
`connect_channel_bot`, `link_channel_bot`, `list_channel_agents`,
`disconnect_channel_bot`. Every agent has `remember` and
`forget`; its memory is injected into every thread's instructions and refuses
obvious credential shapes. A specialist calling a NyxBot-only tool gets
`orchestrator_only`. Grants resolve slugs or IDs through the owner's MCP catalog.
Creating a specialist fences the owner's NyxBot row; `max_live_subagents`
(default 8, at most 32) bounds live specialists.

NyxID starts turns itself: specialist work in the agent's home thread
(`origin: orchestrator`, with `report_to` set to the assigning NyxBot thread),
wake-ups (`origin: event`) and channel messages (`origin: channel`), with the
owner's identity and billing. A NyxBot-assigned or event-resumed specialist turn
reports to `report_to` as a `subagent_settled` event; a direct chat does not wake
NyxBot and is listed in its next turn's instructions instead. A specialist's
permission request goes to `report_to` or NyxBot's home thread. Events queue on
the thread (at most 20) and one event turn drains them when it is idle; other
turns carry drained events in their instructions. Loop guards: at most 20 event
turns per owner per hour, and a NyxBot thread stops after 3 consecutive event
turns without a user message. Specialist and event turns draw from an owner pool
of `max_concurrent_subagent_turns` (default 3, at most 8) plus one for NyxBot; a
full pool returns `pool_full`. A background task runs every 15 seconds: it
resolves watches (connect links and channel bot setup links a chat handed out),
retries deferred wake-ups and starts group members that were busy. Anything the
owner finishes outside the chat (a connect link, a bot setup, owner
verification) resumes the waiting thread with
an event; the owner never replies "done". Agents are never destroyed automatically. Destroy requests Stop on live
turns, revokes every thread key and ciphertext, expires cards, disconnects the
agent's channel bots and keeps read-only threads; a destroyed specialist can be
deleted with its threads. Deleting a thread deletes only that thread.

Group chats (`assistant_groups`, `assistant_group_messages`, routes
`/assistant/nyxagent/groups[/{id}[/messages]]`) hold the owner plus 1–8 agents.
A user message goes to the members it `@mentions`, else to the lead (NyxBot when
it is a member); members hand work on with `@name`, bounded by the owner's
`max_group_handoffs` (per request, default 6) and `max_group_handoffs_per_hour`
(default 60) settings. NyxBot posting work into a group from its own thread is
woken with the members' replies (`group_settled`) once the group is quiet. Each member answers through a hidden member thread
(`group_id`, `group_seen_seq`) with its own key, grants and memory, seeing only
the transcript lines it has not been given.

Role-to-profile routing (`assistant_profile_routes`, admin
`GET/PUT /api/v1/admin/assistant/profile-routes`) validates and stores a NyxAgent
profile per role and per specialist specialty, but is inactive
(`ROUTING_ACTIVE = false`): specialists inherit NyxBot's profile.

## Channel bots

`nyxid__connect_channel_bot` (or `POST /channels`) links one of the owner's
channel bots to NyxBot or a specialist (`link_channel_bot` / `PATCH /channels/{id}`
relinks it). Telegram bots use the Agent Event Gateway (catalog slug
`cmaeg`): NyxID mints a dedicated route key and gateway agent key, creates the
gateway channel as the owner (the gateway verifies the agent key through
`GET /users/me`), points the route key's callback at the gateway, and is the
gateway's `nyxbot` provider under `/api/v1/nyxbot` (Agent Card, bindings,
conversations, Responses SSE, event context). Other platforms use NyxID's own relay:
the route key's callback is `/api/v1/nyxbot/relay/{id}`, verified with NyxID's
relay callback token. Each chat partition maps to one thread of the linked
agent, owned by the bot owner (`channel` set on the row). Only senders verified as the
owner reach it: NyxID's Telegram notification link, or a one-time link code the
owner sends from the chat app (a `t.me/<bot>?start=<code>` link for Telegram).
Other senders get a short refusal in private chats and silence in groups, and no
turn runs. Admitted gateway events keep their `event_context` encrypted for 24
hours for `readEventContext`. Asynchronous replies (event turns) go back through
the gateway's `replyToEvent` while the newest event reference is valid, or through
the relay reply API for direct channels. The gateway operator must register the
`nyxbot` provider before Telegram channels can be created.

## Browser routes

Paths below are relative to `/api/v1/assistant/nyxagent`.

| Method and path | Request | Response |
| --- | --- | --- |
| `GET /conversations` | `limit` 1–100 (default 50), optional `cursor`, optional `agent_id` | `{conversations,next_cursor}` |
| `GET /conversations/{id}` | `limit` 1–100 (default 50), optional positive `before_seq` | `{conversation,messages,acknowledgements,approvals,before_seq}` |
| `PATCH /conversations/{id}` | closed `{title}`; trimmed nonempty, max 200 Unicode scalars | conversation DTO |
| `DELETE /conversations/{id}` | no body | 204; local hard delete, best-effort upstream session delete |
| `POST /conversations/{id}/stop` | no body | 204; owner-only, no active turn is a no-op |
| `PATCH /conversations/{id}/access-mode` | ignored | `410 Gone` (`access_mode_retired`) |
| `GET` / `POST /agents` | `POST`: closed `{name,description,services?,account_read?}` | `{agents,limits}` / `201 {id,name,home_conversation_id}` |
| `GET` / `PATCH` / `DELETE /agents/{id}` | `PATCH`: `{name?,description?}`; `DELETE` only after destroy | `{agent,memory,threads}` / agent / 204 |
| `PUT /agents/{id}/grants` | closed `{services,account_read}` (specialists) | `{id,services,account_read}` |
| `POST /agents/{id}/destroy` | no body; specialists only | `{id,destroyed_at}` |
| `DELETE /agents/{id}/memory/{note_id}` | no body | 204 |
| `GET` / `PUT /settings` | `PUT`: any of `{skip_destructive_confirmation,max_live_subagents,max_concurrent_subagent_turns}` | settings with limits |
| `GET` / `POST /channels` | `POST`: closed `{bot_id,agent_id?}` | channel agents / `{channel_agent,link}` |
| `PATCH` / `DELETE /channels/{id}` | `PATCH`: closed `{agent_id}` | `{channel_agent_id,agent,changed}` / `{disconnected,platform,gateway_released}` |
| `POST /conversations/{id}/acknowledgements/{ack_id}` | closed `{decision:"allow"|"deny"}` | acknowledgement DTO |
| `GET /conversations/{id}/attachments/{attachment_id}` | no body | image bytes (owner only; see Tool images) |
| `POST /turns` | closed `{conversation_id?,agent_id?,text,model?,access_mode?}` (`agent_id` for new threads; `access_mode` ignored) | NyxID SSE events |
| `GET /models` | no body | `[{id,label}]` |

Conversation DTO: `id,title,model,access_mode,created_at,last_message_at,message_count,
pending_acknowledgements,active_turn,context_reset_at,role,agent,pending_events,channel`.
`access_mode` is always `full`; `agent` is `{id,kind,name,destroyed}` (a destroyed
agent's threads are read-only).
`active_turn` is null or `{turn_id,started_at,activities}`.
Message DTO: `id,seq,turn_id,role,text,status,error_code,created_at,activities,attachments`,
where `role` is `user`, `assistant`, `orchestrator` (an instruction the orchestrator
sent a subagent) or `event` (a NyxID-authored wake-up notice).
Acknowledgement DTO adds `decider`, `decided_by` and `reason`.
`active_turn` also carries `attachments`. An attachment is
`{id,content_type,size,label}` (see Tool images).

`approvals` lists pending proxy approval requests raised by the chat's key:
`{id,service_slug,service_name,summary,approval_mode,agent_key_prefix,created_at,expires_at}`.
Per-service approval policies (per_request or grant, decided on the approvals
page, Telegram or the mobile app) are enforced in the execution path and block
the tool call for the channel's approval timeout. The chat now surfaces those
requests as approval cards at the transcript tail, matched by the key's name in
`requester_label` and the owner as request owner or notified approver; the card
decides through the ordinary `POST /approvals/requests/{id}/decide`, and the
waiting tool call then proceeds within the same turn. Known gap: renaming the
assistant key hides cards for requests created under the old name.

`activities` lists the tool calls the chat's key made during that turn, oldest
first: `{id,label,status,started_at,ended_at}` with status `running`, `completed`
or `error`. The upstream stream carries text only, so NyxID records activity at
its own MCP boundary: every `tools/call` authenticated by a conversation key whose
turn is live appends one entry (bounded to the newest 40, label capped at 120
scalars) and settles it when the call returns; a non-2xx transport response marks
`error`, while MCP `isError` results still count as `completed`. The label is the
effective tool identifier (`nyxid__…`, `<slug>__<tool>`, the inner `tool_name` of
`nyx__call_tool`, or the meta-tool name) and never arguments, results or secrets.
Settlement copies the entries onto the assistant reply, marking any still-running
entry with the turn's outcome. The browser shows the running label beside the
streaming indicator and the full list as the reply's collapsible actions; it
learns of new entries through the two-second history poll, which also runs while
this tab's own turn streams.
Dates are RFC3339 JSON strings. Index ordering is latest update first, with ID
as the tie breaker; cursor is an opaque timestamp/ID pair. History returns an
ascending sequence page, initially the newest messages. Metadata and messages
are read in one transaction snapshot. The browser drains the index and offers
Load earlier messages for older transcript pages.

Turn text is nonblank and at most 32,768 Unicode scalars; ingress is capped at
256 KiB. Profiles use `nyxagent/` plus 1–64 lowercase letters, digits, hyphens or
underscores. Default is `nyxagent/chat`. The first send fixes the conversation's
profile; later sends always reuse the stored value. Title is the first 40 Unicode
scalars of the trimmed initial message. Rename and Delete reject an active turn
with HTTP 409 `turn_active` (the numeric code is indexed in CLAUDE.md).

Models use authenticated `GET /v1/models` through the admin proxy, a 60-second
server cache of successful upstream lists and an uncached fallback
`[nyxagent/chat]`. Failures and missing credentials never populate the cache.
Discovery does not provision a key: users without one initially see the default
profile. The browser refreshes profiles after a send, so provisioning makes the
upstream list available immediately. No new NyxID environment variable is introduced.

## Tool images

Tool execution used to decode every downstream body as lossy UTF-8, so a camera
snapshot reached NyxAgent as garbled text it could neither see nor show.
`mcp_service::execute_tool_response` now returns `ToolResponse {status, text,
media}`. `text` is byte-for-byte what callers received before, so exact-approval
receipt digests and every text consumer are unchanged. `media` is set only for a
2xx body of at most 5 MiB whose declared type is `image/png`, `image/jpeg`,
`image/gif` or `image/webp` and whose magic bytes match that type. SVG and every
other type never qualify.

For MCP `tools/call` (direct service tools and `nyx__call_tool`), a verified
image produces a text note first, because NyxAgent hands the whole result to its
model as one JSON string truncated at 10,000 characters and cannot pass pixels to
the model. Callers other than assistant chat keys also get an MCP `image` content
block (base64, `mimeType`) after the note, up to 1 MiB (NyxAgent caps a whole MCP
response at 2 MiB); larger images are described in the note only. Assistant chat
keys get the note alone: an image block would only push the note past the
truncation point, which is what made the model claim the image "arrived as a
truncated base64 string".

When the caller is an assistant chat key and its conversation has a live turn,
the image is also envelope-encrypted with `EncryptionKeys` into
`assistant_attachments` and its metadata `{id,content_type,size,label}` is pushed
onto `active_turn.attachments` (label = the tool identifier, at most 8 per turn;
the slot is claimed before anything is encrypted or stored). Settlement copies
the list onto the assistant reply. The note then tells the model the image is
already displayed under its reply, that it must not claim otherwise, and that it
cannot see the pixels and so must not describe them.
Attachments are deleted with their conversation and in the admin user purge.

`GET /conversations/{id}/attachments/{attachment_id}` is owner-only on the
human-only router (`/assistant` is already denied to delegated `account:read`),
not-found-shaped for other owners, and serves the decrypted bytes inline with
their stored type, `X-Content-Type-Options: nosniff`, a `default-src 'none';
sandbox` CSP and `Cache-Control: private, max-age=3600`. The browser fetches it
through the authenticated assistant client and renders a local object URL under
the reply, including on the streaming message while the turn runs (via the
polled `active_turn.attachments`). A failed fetch shows "Image unavailable".

## Persistence and execution

`assistant_conversations` stores owner, title, model, access mode, upstream session/last
response IDs, credential key ID, message count, optional active turn, reset
reason/time and creation/update dates. Index: owner + descending update + ID.
`assistant_messages` stores UUID, owner, conversation, monotonic seq, turn ID,
role, text, completed/failed status, optional stable error and date. Unique index:
conversation + seq. All dates use the repository BSON date helpers.

Before egress, a transaction writes the user message and claims `active_turn`.
A competing send gets `turn_active` while the fence is live. A fence expires at
`started_at + ACTIVE_TURN_TTL_SECS` (1800 seconds execution + 300 seconds settlement
grace). One shared `live_turn` check governs admission, Rename, Delete, Stop and
DTOs; an expired fence appears as `active_turn: null`. The next send transactionally
inserts an empty failed reply for the lost turn (`error_code=turn_lost`), clears
the binding with `turn_failed`, inserts the new user message and claims its fence.
Rename/Delete also work after expiry; Stop is a no-op. Late settlement matches the
old turn ID and cannot overwrite a reclaimed turn. No uncertain operation is replayed.

A detached task owns the shared
`DirectChatPermit`, upstream stream and settlement. Browser SSE is a broadcast
subscription; closing it does not abort upstream. A lagged receiver continues
reading instead of ending the stream; the later `block.completed` contains the
full text, and `turn.completed` settles the browser. Only a closed channel or a
terminal event ends the subscription. The task transactionally saves
the assistant reply and clears the fence before emitting terminal events.
Ambiguous commit retries recognize the same saved message. Temporary persistence
failures retry with capped backoff within one five-minute deadline, including
each database attempt. At the deadline the worker logs conversation/turn IDs only,
emits `turn.completed(failed, assistant_unavailable)`, and releases its permit.
The remaining fence expires normally, including after a NyxID process crash.
Body validation and ownership checks precede rate-limit admission, so rejected
malformed or wrong-owner requests do not consume the user's turn allowance.

Stop sets a durable flag on the exact active turn. Its worker polls every 250 ms,
drops the upstream stream, saves partial text as failed/error `cancelled`, clears
the binding with `turn_failed`, then emits `turn.completed(cancelled)`. A Stop
committed before settlement wins the transaction race. There is no NyxAgent Stop
API: stream cancellation is the supported upstream cancellation mechanism.

Upstream request, rebuilt entirely by NyxID:

```json
{"model":"nyxagent/chat","input":"current user text","stream":true,"store":true,"instructions":"server prompt and optional recap","conversation":"conv_... when bound"}
```

Only `conversation` is optional; it is omitted when unbound. NyxID never sends
`previous_response_id`, input history, caller query strings, cookies, or browser
Authorization. Server extras supply `Authorization: Bearer <assistant key>` and
`Idempotency-Key: <turn UUID>`; headers are JSON content type and SSE Accept.
A shared guard prevents caller bearer forwarding from replacing server-owned
Authorization. The seam test proves direct HTTP and node header ordering for
both cookie and bearer-JWT callers, including mixed-case collisions.

The bounded constant prompt identifies the signed-in NyxID web assistant,
directs it to inspect/use connected services, discover/connect via hosted links,
help with bots/keys/nodes/approvals, never solicit raw credentials, and use the
user's language. Recaps contain the most recent 20 stored messages with at most
8 KiB including recap delimiters, UTF-8 safe, explicitly labeled as prior history. Failed partial
messages are labeled. The current user message is excluded from the recap.

## Stream and recovery

First-byte deadline is 30 seconds including request setup. Idle timeout is
120 seconds (upstream keepalive is 15 seconds). Turn execution is capped at
30 minutes; stream bytes at 8 MiB and output text at 2 MiB. Incremental decoding
handles split UTF-8 and LF/CRLF frames. Unknown event types are ignored; malformed
recognized events, invalid IDs, or nonmonotonic recognized sequence numbers fail
closed. Cached upstream replay may contain only a terminal response, so terminal
output is authoritative. Returned `conversation.id` and `response.id` are always
stored on success. Locally guessing the first session ID is incorrect.

NyxID events have a strictly increasing `cursor`: `turn.status` (also carries
`conversation_id`), `message.started`, `block.started`, `block.delta`,
`block.completed`, `message.completed`, `turn.completed`. They reuse the Direct
text-event grammar. Terminal status is `completed`, `failed`, or `cancelled`;
error is null or a stable `{code,message}`. The additive `turn.notice` carries
`code=context_reset` and the fixed message:

> Conversation context was reset; the assistant was given a recap of this chat.

| Upstream result | NyxID action |
| --- | --- |
| Bound 404 `not_found` | Clear binding (`session_lost`), emit notice, retry once unbound with recap and same turn/idempotency key |
| `session_busy`, `capacity_exceeded` | Four jittered delays of approximately 2/4/8/16 seconds, same body and key; fail after five requests |
| 401/403 or `agent_key_required` | Replace credential, clear binding, emit notice, retry once with recap |
| `stale_response`, `outcome_unknown` | Fail without retry; discard binding |
| `insufficient_credits` (typed `AppError::InsufficientCredits`, nested `error.code`, NyxID's flat 402 `{"error":"insufficient_credits"}`, or `response.failed`) | Fail without credential replacement, retry, backoff or rebind, with the fixed message "There aren't enough credits to run this turn." During a rolling deploy old replicas keep reporting `assistant_unavailable`; existing failed rows are not backfilled |
| Any failed turn, timeout, invalid stream, cancellation or other error | Persist failed partial reply; discard binding immediately with `turn_failed` |

The transcript displays one inline system note for the latest reset, before the
first message whose `created_at` is strictly greater than `context_reset_at`, or
at the tail if no message follows. Failed settlement uses one timestamp for the
reply and reset, placing the note after the failed reply and before the next user
message. Rebind resets occur during execution and precede that turn's reply.
Live `turn.notice` displays the same note immediately and refreshes the persisted
reset timestamp; subsequent reloads retain its transcript position without a banner.

The next turn after any reset sends a recap and emits the notice. There is no
retry after an upstream stream has started. Generic allowlisted messages replace
all raw upstream error bodies. The raw current key is redacted from reflected
assistant output, including split deltas. Wire-log capture is disabled for this
surface.

On reload, the client fetches persisted history and follows active history
through the live stream (or, without it, polls every two seconds) until its own
metadata reports settlement. The index has no periodic
poll: selected-history settlement refreshes it once, as does send completion.
Unselected active conversations do not trigger expensive index-page polling. Browser subscriptions have a 45-second opening deadline
and 135-second idle deadline; losing that subscription triggers history refresh without
cancelling the server turn. Identity changes clear client presentation caches and
fence late responses. No browser reconnect resends the message.

Markdown same-origin hosted `/connect/nyx_clk_*` links render explicit Connect
buttons in a new tab with `noopener noreferrer`; other safe links remain links.
Footnote fragments stay local. Links never auto-open. Active rows disable Rename
and Delete; Stop remains available in the existing composer.

## Readiness and verified resolutions

`GET /assistant/readiness` adds `nyxagent.enabled`, `nyxagent.row` (present, active,
no user credential, auth none, token forwarding, no delegation, and the
informational `master_credential_configured` flag),
and `nyxagent.credential_exists`. Startup warns about an invalid/missing row.
It reveals no raw key, ciphertext, or upstream session binding. Readiness verifies
the catalog contract in [Deployment prerequisites](#deployment-prerequisites),
but the operator must configure NyxAgent's recursion guard there separately.

The brief's statement that upstream cannot cancel and will always commit is
superseded by its D5 addendum and `api.rs:142–144`: dropping SSE cancels runtime.
Browser disconnect therefore drops only the subscription; explicit Stop drops
the worker's stream. `responses.rs:503–524` poisons every failed session, so every
failure invalidates the binding immediately. The numbered [6]/[7] header block
mentioned in D4 is WebSocket assembly in this checkout; the actual HTTP overwrite
was `proxy_service`'s bearer forwarding after shared assembly. Both HTTP paths now
use the shared guard and the ordering test. The exact scope correction and models
bootstrap behavior are described above.

The Docker test replica advertises `localhost:27017` behind host port 27019.
The harness defaults single loopback seeds to direct connection, preserving
explicit URI options and multi-host discovery. It still verifies replica-set
transactions; it starts no database and skips no tests.

## Upstream contract trace

Source paths and lines below refer to NxyAgent `58f647e4` in `/tmp/nxyagent`.

| NyxID request/response element | NxyAgent source and accepting/producing excerpt |
| --- | --- |
| POST `/v1/responses` | `src/api.rs:62`: `.route("/v1/responses", post(create_response))` |
| Bearer original key | `src/nyxid.rs:60`: `.and_then(\|s\| s.strip_prefix("Bearer "))`; `63`: `s.starts_with("nyxid_") \|\| s.starts_with("nyx_")` |
| Stable owner and MCP key forwarding | `src/nyxid.rs:78`: `mac.update(token.as_bytes());`; `90`: `request.header("x-api-key", token)` |
| Closed request body | `src/responses.rs:182`: `#[serde(deny_unknown_fields)]` |
| `input` string | `src/responses.rs:184`: `pub input: Value,`; `201`: `Value::String(s) if !s.is_empty() => messages.push(s.clone()),` |
| `model`, `instructions` | `src/responses.rs:185–186`: `pub model: Option<String>,` / `pub instructions: Option<String>,` |
| `stream: true`, `store: true` | `src/responses.rs:188,190`: `pub stream: bool,` / `pub store: bool,` |
| Optional `conversation` string | `src/responses.rs:193`: `pub conversation: Option<Value>,`; `259`: `Some(Value::String(s)) => Some(s.clone()),` |
| No `previous_response_id` alongside conversation | `src/responses.rs:268`: `if conversation.is_some() && self.previous_response_id.is_some()` |
| `Idempotency-Key` header | `src/api.rs:122`: `.get("idempotency-key")`; `src/responses.rs:334`: `k.is_empty() \|\| k.len() > 256` |
| Returned first-turn session ID | `src/responses.rs:342`: `.map(\|k\| format!("conv_{}", &hash((&p.owner, k))[..32]))` |
| JSON Content-Type / SSE Accept | `src/api.rs:105`: `body: std::result::Result<Json<Request>, JsonRejection>,`; `108`: `let stream = req.stream;` (SSE selection is the body field, not Accept) |
| Idempotency conflict / stale head / uncertain outcome | `src/responses.rs:395,427,536`: `"idempotency_conflict"`, `"stale_response"`, `"outcome_unknown"` |
| SSE type and monotonic sequence | `src/responses.rs:558–560`: `data["type"] = json!(kind);` / `data["sequence_number"] = json!(self.sequence);` / `self.sequence += 1;` |
| SSE text delta / done | `src/responses.rs:610,637`: `self.emit("response.output_text.delta",...)` / `self.emit("response.output_text.done",...)` |
| Terminal success / failure | `src/responses.rs:522,528`: `.emit("response.failed", json!({"response":output.response}))` / `.emit("response.completed", json!({"response":output.response}))` |
| Terminal-only cached replay and response shape | `src/responses.rs:469`: `if p.cached.is_some()`; `431` constructs `"output":[]` and `"conversation":if r.store{json!({"id":id})}else{Value::Null}` |
| Stop by dropping SSE | `src/api.rs:142`: `// Dropping the HTTP stream cancels the turn and kills its ephemeral runtime.` |
| Every failed execution poisons context | `src/responses.rs:513`: `p.session.recovery_required = true;`; `417–422` returns `unknown_outcome()` when this is set |
| Missing session / busy session / capacity | `src/error.rs:27`: `Self::new(404, "not_found", "Resource not found or expired.")`; `src/coordination.rs:78`: `"session_busy"`; `src/api.rs:115`: `"capacity_exceeded"` |
| Credential / NyxID / profile errors | `src/nyxid.rs:70,113`: `"agent_key_required"`, `"nyxid_error"`; `src/runtime.rs:217`: `"model_not_configured"` |
| Recursion guard deployment setting | `src/config.rs:55`: `self_service_slugs(std::env::var("NYXAGENT_SERVICE_SLUG").ok().as_deref())`; `src/nyxid.rs:43`: `self_prefixes: self_prefixes(&configured_self_slugs())`; `146` filters these prefixes |
| GET `/v1/models` | `src/api.rs:60`: `.route("/v1/models", get(models))`; `92`: `a.auth(&h).await?;`; `95`: `"id":format!("nyxagent/{n}")` |
| DELETE `/v1/sessions/{id}` | `src/api.rs:68,73`: `for base in ["/v1/sessions", "/v1/conversations"]` / `.delete(delete_session)`; `233`: `a.responses.sessions.update(&p.owner, &id, None).await?` |

Aevatar deletion/action-route retirement, the mobile app's pre-existing obsolete
`POST /chat` use, and changes to the NxyAgent repository are outside this change.

## Testing and verification

Round-4 regressions cover platform-source Ask refusals and Full execution through
both MCP call paths, defensive service decisions, node execution after service
consent, node scope across key lifecycle/mode changes, assistant-key deletion
refusals in both modes, and discovery audit suppression. The stale channel e2e
specs were aligned to the descriptor-driven UI (`frontend/src/lib/channel-platforms.ts`)
and shared fixture (`frontend/src/test/fixtures/channel-platforms.ts`), so their
label changes preserve the UI contract rather than weaken assertions.


## Trigger turns

Scheduled and assistant-webhook work enters the same engine with
`TurnOrigin::Trigger`, `guest = false` and an internal fenced `TurnClaim`.
`begin_turn` admits that claim inside the conversation/credential transaction;
the run record and active turn therefore agree even across a crash. This branch
executes only for trigger turns, adding no database query to ordinary turn start.
Scheduled trigger instructions retain owner authority. Webhook data is untrusted:
`confirmation_policy = changes` (default) enforces owner cards on every changing
tool call, using the same HTTP/catalog classifier as guest access and explicit
native tool contracts. The owner may explicitly select `destructive` with a
warning to confirm only destructive calls. This policy is snapshotted per run
and is not bypassed by the global skip-destructive setting. Agent grants and
billing continue through the existing engine. Webhooks default to a dedicated
thread for every agent, keeping event text out of the home context used by later
owner turns and private channel chats. An explicit `home` choice carries UI/tool
and setup-prefill warnings that later full-authority turns are outside the webhook
confirmation policy. Scheduled work retains the NyxBot-home/specialist-dedicated
defaults. Dedicated/per-run automation threads are excluded from home adoption
at creation, turn admission and recovery after deletion.

`ActiveTurn.trigger_run_id` connects settlement and confirmations to metadata-only
run history. Confirmation requests copy this identity and the active turn ID;
automation settlement keeps the run waiting until cards are decided, scheduling
its fallback at the earliest card expiry. The decision transaction wakes the
work item; settlement reads card state transactionally to avoid lost wakeups. The
scheduler resumes it as another trigger turn without a second occurrence-budget
charge. Ordinary browser and chat continuations skip these cards. A failed/lost
turn is terminal and never replayed. Final replies remain in the transcript;
optional chat/push delivery claims one outbound attempt. An interrupted, ambiguous
send is reported as failed rather than retried, since a remote provider cannot
participate in the MongoDB transaction. Full details are in
[NyxBot schedules](../NYXBOT_SCHEDULES.md).

All NyxAgent REST routes require first-party human credentials; developer OAuth
apps cannot turn their consent into the owner's full assistant authority. CLI
device login, mobile and web first-party sessions carry no OAuth client ID.
Ordinary denial stickiness remains keyed to the latest user/orchestrator message;
only trigger turns key denials to their active run turn.

Automations require all replicas upgraded before creation. Old trigger readers
fail listings and ingress on new enum variants; existing retry/replay and TTL
retention do not scan or delete automation rows. Setup links contain only an
owner-bound watch ID, with prefill fetched after authentication and consumed in
the trigger-insert transaction. Cron gaps collapse at the first valid instant;
folds use the earlier instant. Recovery writes one missed-count/bounds summary
and one audit. Busy/pool/overlap work backs off to 30 seconds; budget exhaustion
waits for its next UTC budget window, all capped at the initial grace deadline.
