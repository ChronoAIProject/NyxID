# NyxAgent assistant engine

Owner execution and settlement live in `services/assistant_nyxagent_runtime.rs`;
HTTP adapters retain authentication and channel notice delivery. The shared
`services/proxy_http_response.rs` relay owns bounded admission rejection capture,
synchronous zero-usage funding release, and model stream observation. Both HTTP
owner turns and the typed personal-platform egress use this relay; neither path
may return a proven capacity refusal until its funding reservation is released.
Scheduler jobs, shared owner admission, and terminal audit transactions box
their large futures at call boundaries to keep nested polling within the
worker's normal stack, including confirmation continuations.

## Turn diagnostics and context continuity

Failed replies retain a normalized NyxID `error_code` and, only when the
upstream named one, its sanitized `upstream_error_code`. NyxID's own classes
(timeouts, Stop, preparation, settlement) never fill `upstream_error_code`.
Their `error_detail` is metadata only: stage, HTTP status, upstream request id,
response id (from `response.created` once the stream started), continuation
count, turn id, and timestamp. The original upstream response, prompts,
credentials, and tool arguments are never stored. Stages name where the turn
failed: `prepare` (before any egress), `connect` (proxy or billing refusal),
`first_byte` (no headers or no body bytes within 30 s), `upstream_response`
(non-2xx or non-SSE), `stream` (malformed stream or `response.failed`),
`idle` (no transport bytes for 120 s), `no_progress` (keepalives only, no data
event, for 610 s), `continuation`, `recovery`, `stop`, `permit`,
`turn_deadline`, `settlement`, `lease_expiry` (a stale fence reclaimed by the
next send) and `sweep` (the orphan sweep). Keepalive comments satisfy the
first-byte bound; any data event, including MCP tool-progress events the
decoder otherwise ignores, resets the no-progress bound. A reclaimed orphan
turn carries its activity and attachment checkpoint forward; running
activities become `error`.

Every settled owner turn writes a durable checkpoint (`assistant_checkpoints`,
`_id` = conversation id) in its settlement transaction: the goal (the owner
request the task is working on, 1,000 characters), the actions completed and
the actions failed or interrupted since the last successful turn (a run of
failed turns accumulates; a successful turn starts a new window), a
500-character output excerpt, the outcome and error code, and the last
successful turn and time. Guest turns never write it. `GET
…/conversations/{id}/context` returns the checkpoint, the live turn's running
actions, the binding state and the last reset reason; the browser's Working
context sheet shows them.

After a reset (failed or stopped turn, recovery, instruction, model, audience
or credential change), the next turn receives a bounded **Previous context
(after reset)** recap: up to 4 KiB for the owner's durable checkpoint, followed
by independently bounded owner requirements (newest 40 owner-provenance
messages plus the earliest, oldest first, excluding tail duplicates), the newest
20 stored messages, and this conversation's message-bound upload manifest.
The three latter sections reserve approximately 5 KiB, 5 KiB and 2 KiB, with a
framed recap below 12 KiB. Each requirement is at most 1,000 bytes. UTF-8 is
never split. Recent messages preserve bounded head/tail text, failed codes,
activity labels/statuses and attachment labels. Channel origin is preserved for
guest messages and replies so their channel-only recap remains available;
retry eligibility separately requires an explicitly owner-authored input.
Stored guest user messages keep
their `user (guest)` label even in an owner recovery recap. Guest recaps omit the checkpoint,
owner requirements, tool, upload and error metadata. Recap queries run lazily
only for reset/recovery, at most once per turn.

When owner requirements are omitted or truncated, the recap points to
`nyx__conversation_history`. Truncation is recorded by the renderer, and messages
that cannot fit in the tail remain eligible for the owner section or omission
count. The tool reads complete owner-provenance messages from
this conversation only, newest first. The exclusive `before_seq` bound and
character `offset` continue a split message before older messages; follow the
returned cursor until `complete`. Its final MCP envelope stays below 9,000
characters; guests, stale turns and credentials for another conversation cannot
read it. The attachment manifest includes retention-expired markers, and
`nyx__attachment_list` discovers older uploads omitted from its bounded display.
This recap is quoted prior context, not a new instruction. The conversation DTO
exposes `context_reset_reason` (`turn_failed`, `stopped`, `session_lost`,
`instructions_changed`, `model_changed`, `audience_changed`, `guest_turn`,
`credential_replaced`, `user_reset`) so clients can explain what was preserved.

An owner reset (`POST …/context/reset`, first-party human only) is fenced like
a model change: it fails with `turn_active` (409) while a turn runs, clears the
binding and deletes the checkpoint in one transaction, and records
`user_reset`. The next turn then starts without any recap.

Memory notes are delivered in the input of each turn's first request (and of
recovery requests into a fresh session) and are excluded from the stable
instruction fingerprint. Updating memory therefore does not reset any thread's
binding; guest turns receive no owner memory. Deploy note: the first turn of
each memory-bearing thread after rollout resets once (the fingerprint no
longer contains memory), and while old and new replicas serve the same thread
its turns can alternate resets until every replica runs the new build.

**Retry.** The owner retries a failed reply with `retry_of` (its turn id) on
`POST /turns`. History and `GET /conversations/{id}/messages/{message_id}/recovery`
return a server-owned `available` or `review` decision for a failed owner reply,
independent of which input page the browser has loaded. The focused read is
owner-only. The browser refreshes it before posting the exact original text;
attachment-only requests send empty text and no new attachment IDs. An
unavailable or stale read offers a non-executing status check and request review.

Admission rechecks the same evidence in its claim transaction and consumes the
failed reply once. It restores only original, still-bound uploads. Missing or
expired uploads, destroyed agents, active turns, consumed replies, unknown
outcomes, running/errored or digest-less activities, capped activity history,
and missing or overlong retry ancestry require review. A full 40-entry activity
list is also treated as incomplete during rollout. Legacy turns without a
complete activity count also require review; no backfill can infer omitted
effects. Upgrade every serving replica before relying on these retry claims and
evidence checks, since old replicas do not enforce them. A retry of a retry inherits every
ancestor's completed-action digest; a completed chat-key `tools/call` with the
same canonical tool and arguments is refused as `retry_duplicate_action` before
execution. This is a fence for known completed calls, not an exactly-once
guarantee for unknown outcomes. Ordinary turns are unaffected. Pending
acknowledgement cards keep their own expiry and are not re-presented by Retry.

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

### Conversation-key live-turn gate

Every per-thread `nyxid-assistant` credential carries a request-time
`AssistantTurnFence` loaded with its conversation. When the runtime flag
`assistant:live-turn-gate` is enabled, MCP tool execution, provider/proxy
execution, pinned skill reads, conversation attachment reads, exact-approval
redemption and Oracle submission require `live_turn` and
`active_turn.stop_requested=false`. Discovery/list/search, human session
routes and Stop remain available. Dedicated channel-agent keys are a separate
credential class and are not subject to this gate; a conversation credential
must not be used as a channel route or event-gateway key.

The fence uses the existing 2,100-second execution plus settlement lease and
adds no grace period. A request admitted before a concurrent Stop keeps its
request snapshot; the next request is refused with `assistant_turn_required`.
Voice delegation must enter through `begin_turn` on the same thread and must
not call tools with the bare key after the voice session or turn ends.

Trusted async service watches have a separate, narrowly bound background reader:
it may call only the declared status/result/cancel operations with the saved ID,
using the original key and live authority. It never fabricates a live turn. Results
resume the thread through event admission; see
[asynchronous service results](09-nyxbot-orchestrator.md#asynchronous-service-results).

The flag is default-off for rollout safety: deploy every auth, MCP, proxy, LLM
and worker replica with fence support, then enable `assistant:live-turn-gate`.
Rollback disables the flag before rolling back binaries. Existing fences are
always computed, and ordinary API keys, sessions, service accounts and public
requests keep their no-extra-read path.

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

## Authority: NyxBot access modes and granted specialists

NyxBot is involved in a specialist's work only when the turn's admission
snapshot says so: `ActiveTurn.report_to` (the exact assigning NyxBot thread) or
`ActiveTurn.with_nyxbot` (a personal group whose current members include the
owner's NyxBot). A direct chat with a specialist, in the app or on a channel, is
handled by that specialist alone: an ungranted call returns `missing_access`
(no card, no NyxBot event, a link to the agent's details) and the specialist
continues; the owner grants access in the agent's details. See
[NyxBot orchestrator §8](09-nyxbot-orchestrator.md#8-permission-requests-specialist--nyxbot).

Every owner has one persistent **NyxBot** agent. The workspace access mode is
selected in NyxBot settings and is visible in the composer toolbars; an agent may have its own override, shown on the agent page and in its
thread. The server resolves the effective mode from that persisted policy, so a
client cannot widen access by submitting a different mode. The three modes are:

- **Full access** (`autopilot`) never asks. It bypasses mode, webhook,
  machine/saved-login, service approval, pairing/update and skill-publication
  confirmations. Successful bypassed actions leave automatic allowance receipts.
- **Auto** (`nyxmode`) proceeds automatically for ordinary actions and asks before
  sensitive or destructive actions, and before any organization connection use (including reads).
- **Ask every time** (`ask_every_time`) asks before every action that changes something; reads do
  not prompt unless they use an organization connection.

Confirmation cards identify the action and requested scope, the risk class
(`change`, `sensitive`, or `destructive`), the effective mode, and why the card
was required. Webhook `confirmation_policy`, per-machine `machine_confirm`,
saved-login `confirm_each_sign_in`, per-service `approval_mode`, machine-update
cards and skill-publication review apply in Auto and Ask every time. Full access
bypasses all of them. Explicit deny rules, grants, guest restrictions, live
turn/key fences and publication leases remain enforced. Guests cannot confirm
or widen authority.
Specialist requests and grants are still owner/orchestrator-authorized;
specialists hold only their explicit grants on every thread:
`allow_all_services=false`, `allow_auto_connected_services=false`,
`allowed_service_ids`/`allowed_platform_service_ids` from the agent's `grants`,
and `assistant:account` only with an `account_read` grant. Lower-scoped agents
cannot mutate grants or assistant management state through conversation keys or
delegated routes.

The agent document is the durable source; thread keys mirror it at every turn
start, rotation and replacement. NyxAgent calls its model through NyxID's proxy
(`/api/v1/proxy/s/{model service}`, e.g. `chrono-llm-public`, `llm-deepseek`)
with the thread's own key. The proxy therefore lets a live assistant
conversation key (an `assistant_agent_credentials` row, which people cannot
create) reach any active catalog service with `inference` metadata even outside
its grants (`proxy::assistant_model_call`), whether the owner's own row, an org
row or the catalog row resolves it. Nothing else widens: the key's allowlist,
inventory, delegation tokens, consent screens and MCP tools stay its grants.

Legacy settings rows without `access_mode` are read with the Auto default and
are migrated on settings writes. The retired
`PATCH /conversations/{id}/access-mode` endpoint remains a compatibility route
that returns `410 Gone` with `{"error":"access_mode_retired"}`; current clients
change the workspace mode through settings or the thread switcher. Agent
overrides are changed through the human-only agent policy route and may be
cleared to inherit the workspace mode.

## Acknowledgements

`assistant_acknowledgements` is a transcript collection with UUID `_id`, owner,
conversation, credential key, kind (`service`, `account`, `action`, `operations`,
`skills`, `machine`, `saved_login`), applicable
service id/slug/name or tool/argument digest/summary, status, creation/decision/
expiry dates, and the last persisted user turn that requested it. Debug is
redacted. Index `(conversation_id,status,created_at)` has no TTL. Pending requests
expire lazily after 15 minutes; allowed actions expire after 10 minutes and can
be consumed only once. Decisions and key mutations serialize against the current
conversation and credential generation. Service and account requests come only
from specialist threads and carry `decider: "orchestrator"` (NyxBot decides) and a
bounded `request_excerpt` of the message that started the requesting work (the
user's own words when the user spoke to the specialist). Action confirmations
keep `decider: "user"`. Decided rows record `decided_by` (`user`, `orchestrator`, or `access_mode`)
and NyxBot's bounded `reason`.

Automatic allowance receipts are action rows with `status: allowed`,
`decider: user`, `decided_by: access_mode`, and
`confirmation: {risk, access_mode: autopilot, reason: access_mode|webhook|policy|approval}`. They are
written best effort only after success, with creation and decision at the same
instant and expiry equal to creation. They never satisfy card consumption,
never count as pending and are excluded from the model's user-decision note.
Their metadata-only audit is `assistant_action_auto_allowed`. History renders
“Allowed automatically · Full access” with the action summary, risk and the
bypassed automation, machine/saved-login or service-approval policy. Full-access
authored publication uses the same leased publication-and-pin core, under the
chat owner's signed identity, and links the persisted proposal for inspection or discard.
Switching to Full access retires pending owner confirmations, respecting agent
overrides. Policy writes and an idempotent startup sweep visit only pending owner
action rows and chat-key approvals in bounded batches, resolving each user/agent's
mode once. History, group and voice reads do not retire asks or re-read mode policy
for retirement. Approval expiry uses the approval service's pending status guard,
decision time and unique expiry marker. Retirement records expiry, not success;
the retried action writes its automatic receipt after it runs.

Permission-mode receipt rollout: deploy the frontend accepting `decided_by: access_mode`,
optional `continuation_of` and routed `machine`/`saved_login` requests before backend
writers, or ship both in one release.
Unknown future deciders are tolerated. After allowance the agent reports what it
did and the result in plain words; after denial it reports what it skipped. Skill
attachment reports name, version and target agent. Asking channel modes retain
the “yes NNNN” owner protocol.

Full access never requests permission to proceed, including when an authored
skill declares reduced scope. The encrypted declaration remains bound to the
proposal, and the automatic publication receipt records only the omission count
and that separate consent was bypassed. Its proposal link shows decrypted
omissions to the owner; omission text stays out of plaintext receipts, audit and
events. NyxID does not create a human scope acceptance for this path. Auto and Ask every time retain explicit, revision-bound
reduced-scope acceptance on the complete review card. Successful automatic
publication returns the outcome for narration in the same turn; reviewed
publication queues one durable completion event. Package failures retain their
safe rules and queue one repair event to the original drafting conversation,
even when Full access produced no review card.

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
assistant: when no turn is running, the browser sends a continuation turn with `acknowledgement_id` (`Approved: this
chat may use <service>. Continue.`, `Approved: account management for this chat.
Continue.`, or `Confirmed: <summary> (acknowledgement_id <id>). Retry it now.`).
The server validates an allowed/used user-decided card in the same conversation
and stores its ID as the user message's optional `continuation_of`. The message
DTO carries this field; the browser shows “Continuing after your decision” as a
muted system line. The continuation is a normal turn with no extra authority; the allowed grant is
what the retried tool call consumes. If the user allows a card while a turn is
still running (cards appear as soon as the tool call is refused, usually before
the reply finishes), the running turn cannot observe the decision: NyxAgent ends
a turn on a card and answers same-turn repeats locally. The browser therefore
queues the continuation and sends one marked continuation turn covering every card allowed
during that turn as soon as it settles; nothing is sent if the user pressed Stop.
**Deny** sends nothing.

Independently, every turn's input context includes a note listing the cards the
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
other tool returns `orchestrator_only`, whose guidance names the NyxID page the
owner uses instead and never tells a direct specialist chat that NyxBot must
take over. NyxBot also gets the agent and channel
tools, and every agent gets its memory tools (see NyxBot agents). The closed
account inventory is:

| Area | Tools (prefix every name with `nyxid__`) | Destructive |
| --- | --- | --- |
| Agent keys | `list_agent_keys`, `get_agent_key`, `update_agent_key`, `delete_agent_key`, `list_agent_key_bindings`, `bind_agent_key_credential`, `unbind_agent_key_credential` | `delete_agent_key`, `unbind_agent_key_credential` |
| Phone and speech capabilities | `list_channel_adapters` | None |
| Channel bots/routes | `list_channel_bots`, `get_channel_bot`, `update_channel_bot`, `delete_channel_bot`, `list_channel_routes`, `set_channel_route`, `delete_channel_route` | `delete_channel_bot`, `delete_channel_route` |
| Connected services | `list_my_services`, `set_service_enabled`, `delete_service` | `delete_service` |
| Nodes | `list_nodes`, `delete_node` | `delete_node` |
| Approvals | `list_approval_configs`, `set_approval_mode`, `list_pending_approvals` | None |

`list_channel_adapters` returns the compiled capability catalog, rollout status
and a link to owner setup. Binding management and secret issuance require the
first-party owner UI or API. See [NyxBot service adapters](../NYXBOT_SERVICE_ADAPTERS.md)
for phone/SMS and ElevenLabs setup, guest FAQ access and live instruction updates.

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
`forget` (which refuses obvious credential shapes); its memory notes are
supplied in each turn's input and are not part of stable instructions. A specialist calling a NyxBot-only tool gets
`orchestrator_only`. Grants resolve slugs or IDs through the owner's MCP catalog.
Creating a specialist fences the owner's NyxBot row; `max_live_subagents`
(default 8, at most 32) bounds live specialists.

NyxID starts turns itself: specialist work in the agent's home thread
(`origin: orchestrator`, with the turn's `report_to` snapshot set to the
assigning NyxBot thread), wake-ups (`origin: event`) and channel messages
(`origin: channel`), with the owner's identity and billing. Every turn snapshots
`report_to` and `with_nyxbot` once at admission, inside `begin_turn`'s
transaction: an `orchestrator` turn keeps its assigning thread only if every
event drained into it is bound to that same target; an `event` turn takes the
one target every drained event is bound to and is otherwise `None`; `user`,
`channel`, `trigger` and `group` turns are `None` (a group turn sets
`with_nyxbot` when the owner's NyxBot is a current member of that personal
group). Unbound, legacy or mixed events and `conversation.report_to` (still
written for old readers, never read) never create involvement; steering
continues the running turn and keeps its snapshot. A turn whose snapshot has
`report_to` reports there as a `subagent_settled` event when it settles; a
direct chat does not wake NyxBot and is listed in its next turn's input context
instead. A specialist's permission request exists only for an involved turn and
goes to the card's stored `routing` (its `assigned` thread, or NyxBot's home
thread for `group_with_nyxbot` after a live membership recheck); non-involved
work gets `missing_access` and continues. Events queue on
the thread (at most 20) and one event turn drains them when it is idle; other
turns carry drained events in their input context. Loop guards: at most 20 event
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
| `DELETE /conversations/{id}` | no body | 204; stops/cancels active work, then locally hard-deletes, with best-effort upstream session delete |
| `POST /conversations/{id}/stop` | no body | 204; owner-only, idempotent for an idle turn |
| `GET /conversations/{id}/capabilities` | no body | `{steer: null \| {max_chars,max_per_turn}}` |
| `POST /conversations/{id}/steer` | closed `{text,turn_id,clientRequestId}` | `{client_request_id,outcome,code}` on acceptance; safe error code on refusal/uncertainty |
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
`active_turn` is null or `{turn_id,started_at,activities,response_id,steering_allowed,stop_requested}`.
Message DTO: `id,seq,turn_id,role,text,status,error_code,created_at,activities,attachments`,
The optional `steering` field is `{client_request_id,outcome,code}`.
Here `role` is `user`, `assistant`, `orchestrator` (an instruction the orchestrator
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
scalars of the trimmed initial message. Rename rejects an active turn with HTTP 409
`turn_active`; Delete stops/cancels the active turn before removing the thread (the
numeric code is indexed in CLAUDE.md).

Models use authenticated `GET /v1/models` through the admin proxy, a 60-second
server cache of successful upstream lists and an uncached fallback
`[nyxagent/chat]`. Failures and missing credentials never populate the cache.
Discovery does not provision a key: users without one initially see the default
profile. The browser refreshes profiles after a send, so provisioning makes the
upstream list available immediately. No new NyxID environment variable is introduced.

## Steering a running web turn (0.68.0)

The owner can send text guidance to the same running response without starting
another turn. The original SSE stream and event types do not change. This uses
NyxAgent's `nyxagent-steer-v1` contract (`origin/main` at `fe97457`,
`docs/api.md`, “Steering an active turn”).

`GET /assistant/nyxagent/conversations/{id}/capabilities` returns
`{"steer":{"max_chars":32000,"max_per_turn":20}}`, or `{"steer":null}`.
The authenticated upstream capability lookup shares a bounded 60-second cache
with image capability discovery, keyed by database, catalog service and base URL.
Only metadata is cached; every operation still checks live owner/key authority.
No additional upstream lookup runs on ordinary text turns. Missing/unknown
steering capability preserves the existing blocked composer and `turn_active`
response. Existing image protocol fields are unchanged.

`POST /assistant/nyxagent/conversations/{id}/steer` accepts only
`{"text":"new direction","turn_id":"<NyxID turn UUID>","clientRequestId":"<stable client ID>"}`.
It requires a first-party human and exact acting-person conversation ownership.
Text must be nonblank and at most 32,000 Unicode characters (or the advertised
lower limit); images and unknown fields are rejected. The client ID is 1–256
printable ASCII bytes and is forwarded unchanged as `Idempotency-Key`.
The handler loads the conversation's existing encrypted thread credential;
it never provisions/replaces a key or accepts a caller destination/session.
Calls use the same catalog-controlled proxy destination as `/v1/responses`.

The worker records `active_turn.running_response` (response and session IDs) as
soon as it decodes `response.created`. The write is fenced by the turn ID and
Stop flag. Every automatic continuation clears it before dispatch and writes
its new response ID on creation; the last committed response head is never
used for steering. A missing ID returns `409 starting` with `Retry-After: 1`.
Stop-pending turns return `409 stop_pending`, and expired leases or stale
heartbeats refuse guidance. Steering does not acquire or bypass a Stop fence.

A valid dispatched steer is appended as a user transcript row on the original
turn, with an additive `steering` receipt and no new turn/admission/event budget.
The receipt is reserved transactionally before proxy dispatch. A unique index
on conversation + client request ID and the conversation write serialize
replicas; changed text or turn under the same ID returns
`409 idempotency_conflict`. Concurrent retries wait for the durable outcome,
without forwarding again. A process crash after reservation conservatively
leaves `may_not_have_applied`; it never becomes a new request after restart or
continuation. Receipts are removed with the conversation's ordinary transcript.

Only a validated upstream `200 accepted` records `applied`. Upstream 400/404,
409 `no_active_turn`/`response_mismatch`/`idempotency_conflict`, and 429
`steer_limit` record `refused` with their safe code. A 503, network failure,
unrecognized response or the bounded 15-second request deadline records
`may_not_have_applied`. No upstream error prose, credentials or request bodies
enter metadata audit/logs. Local validation, unsupported scope and `starting`
refusals do not reserve a receipt or dispatch.

The composer keeps Stop available alongside Send guidance: while a reply runs, a
hint above the field says that sending steers this reply, and the send control is
labelled **Steer** beside Stop. It shows acceptance,
retains definitely refused drafts, refreshes state on a response mismatch, and
offers an explicit **Send as new message** action for `no_active_turn` once idle.
Uncertain guidance is shown as such and never automatically resent under a new
key. Persisted outcomes remain visible on the steering message after reload.

Steering rows are not later user instructions or previous-message anchors.
Acknowledgement windows, events since the previous turn, channel thread facts,
specialist activity summaries, learning evidence and title generation exclude
them. A recap after a context reset may include the text as quoted history,
labelled `steering` and its outcome. The upstream stored thread already contains
accepted guidance, so normal later turns do not replay it.

Scope is the web composer, including event/automation work on an eligible owner
thread; steering preserves its existing turn origin, guards and delivery. Group member threads are deliberately
unsupported: they are hidden execution contexts and need an explicit member
selection plus a corresponding shared transcript action. Channel and voice steering remain unsupported. Group targeting and channel/voice
steering are follow-ups; no new NyxID environment variables are required.

## User uploads

[10 — Assistant uploads](10-uploads.md) defines owner uploads for conversations and groups. Documents are read through `nyx__attachment_read`; user images use the advertised `nyxagent-input-image-v1` protocol and the exact thread key. Older deployments explicitly report that images cannot be viewed. This does not change tool-image delivery below.

Upload classification uses sniffed bytes and case-insensitive filename extensions,
never the browser's MIME guess. The picker has no restrictive `accept` filter, so
`.text` and extensionless UTF-8 files reach the same server validation as paste
and drop. Markdown accepts both `.md` and `.markdown`; JSON and CSV are validated
before their verbatim text is retained. Empty MIME, `application/octet-stream`
and Windows' `application/vnd.ms-excel` CSV MIME do not change classification.

## Tool images

Machine screenshots and shared images use this same encrypted attachment
pipeline. Machine tool text stays within the 10,000-character result budget;
desktop streams and owner input use a separate human-only socket and never
enter tool results. Owner takeover ends the active turn; a durable machine watch
wakes the conversation on hand-back. See [machine nodes](../MACHINE_NODES.md)
and [NyxBot machines](09-nyxbot-orchestrator.md#machine-nodes-and-saved-logins).

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
model as one JSON string truncated at 10,000 characters. Pixels reach capable
agents through a separate image-input continuation below. Callers other than assistant chat keys also get an MCP `image` content
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
the list onto the assistant reply. The note describes the destination: the app,
a web group, a channel chat, or a chat-delivered automation. It never claims
“Delivered” or “shown below”. App/group notes explain where the attachment will
appear; chat notes say NyxID will attempt delivery and require no tool resend.
Unsupported chats receive an accurate capability note. A User-origin turn on a
channel-linked conversation remains app-only unless another delivery rule applies.
NyxID can show the image to the model after this response when image input is
supported; the model must not describe its contents before viewing it. A stopped
turn cannot claim another attachment slot.
Attachments are deleted with their conversation and in the admin user purge.

### Channel delivery of tool images

NyxID reads the exact settled assistant message by conversation and turn ID and
sends retained tool images before its text, one image per message, without tool
labels or captions. Direct replies, gateway streams and late replies, native
followed threads, event replies, duplicate-question chats and chat-delivered
automations all use NyxID's existing relay adapters. Gateway image rendering in
CMA is not required. Failed or cancelled turns with useful text retain their
images and the Incomplete suffix; failures without text send no images.

Machine computer-use screenshots are observations for the agent and stay in
the app; they never accompany channel replies. Their tool note explicitly names
the chat that will not receive the screenshot. Explicit machine `share_file`
images use the same destination-aware note and channel delivery as service tool
images. New tool attachment metadata always writes `chat_delivery`: `false` for
screenshots and `true` for service images and explicit shares. All channel paths
use one filter. For legacy metadata without this field, only the exact label
`Machine screenshot` is excluded. This rolling-upgrade shim can be deleted one
release after every replica runs this version. Web and group display is unchanged.

Release 0.71.0's v1 `nyxbot_image_deliveries` rows remain read-only delivery
barriers across every destination alias for the same owner/conversation/turn
and NyxBot channel. Sent items report previously delivered; sending/unknown
items and pending items in sending batches remain uncertain. Only a pending
batch or a proven pre-dispatch skip permits a v2 claim. This can suppress
another audience of that bot for seven days. The old TTL remains; v2 uses a
separate collection. Concurrent old/new sends can still race during rollout.
Remove v1 compatibility one release later and at least seven days after deploy;
see `CHANNEL_BOT_RELAY.md` for the state rules and rollout/rollback limits.

Each (source image, audience) pair has one durable `nyxbot_image_deliveries_v2`
claim (v2, `models/nyxbot_image_delivery.rs`, sent through
`services/channel_image_delivery.rs`): `_id = sha256("v2\0{owner}\0{attachment_id}\0{audience_key}")`
with plain `user_id`, `conversation_id`, `attachment_id` and `audience_key`
fields and cleanup indexes on `attachment_id` and `(user_id, conversation_id)`.
The audience (`services/channel_image_audience.rs`) is the resolved outbound
target actually sent to, after the adapter's own normalization and fallback,
serialized as
`"{channel_bot_id}\u{1f}{platform_chat_id}\u{1f}{native_thread_or_empty}"`
(the real `ChannelBot.id`, never a gateway partition or `NyxbotChannel` id;
Telegram forum topics and Slack root threads are audience boundaries, reply
chains and Lark/Feishu thread replies are not; Discord threads are channels;
WhatsApp uses `wa_id`, X the DM conversation id). Claims hold identifiers,
fixed states and codes only and have no TTL: they live as long as their source
image and are deleted in the same paths that delete `assistant_attachments`
(retention, conversation and group deletion, user purge), plus a bounded
orphan sweep on the assistant sweep cadence that removes only claims whose
source attachment no longer exists; a sender that finds its source gone after
claiming deletes the claim and reports `unavailable`. States move `pending` ->
`sending{attempt}` -> `sent | failed{code} | skipped{code} | unknown` through
compare-and-set on the attempt token: success is reported only after an
acknowledged matching write, an ambiguous write settles locally as `unknown`
and is never resent, and a claim found `sending` reports
`in_progress_or_unknown`. Only the winner of the `pending -> sending` race
dispatches; every other path (automatic replies, explicit `post_to_chat
include_images`, automation results) reports the existing state
(`previously_delivered`, the same failure or skip code, or in progress) without
resending, so one source image reaches one audience once for its whole
lifetime. Unavailable image metadata never blocks the final text: the text is
still sent and the image is counted as failed. `sending`, `unknown` and
completed claims are never retried. A transport failure or ambiguous receipt
marks an image unknown and skips the rest; a definite refusal permits the next
image, except a billing refusal stops the batch. Telegram group sends are spaced
one second apart. Image calls have a 35-second maximum and share a bounded
per-destination budget; gateway stream keepalives continue during delivery.
When fewer than ten seconds remain in the image budget, the next image is
skipped with `deadline` instead of starting a request. Unsupported destinations
and missing reply anchors record per-image failures without decrypting bytes.

The final text gets a bounded NyxID-authored line when any image failed, was
unknown or was skipped. Only the owner's private chat mentions the app; guests
and shared chats receive counts without that reference. Telegram's safe photo
fallback counts as sent and records `as_document` under the
[relay fallback rules](../CHANNEL_BOT_RELAY.md#image-outcomes-and-telegram-document-fallback).
Outbound messages and audits retain only kinds, identifiers and counts; image
bytes remain in the encrypted assistant attachment store and are zeroized after
use.

Successful member replies also copy tool attachment metadata into web-group
messages. The group endpoint requires first-party human group-read access, a
transcript attachment reference, a member conversation belonging to that group,
and matching attachment/conversation owners. Personal groups additionally require
the conversation owner to be the group owner. Organization groups allow other
participants with group-read access to read an initiating participant's retained
image; organization ownership is not substituted for conversation ownership.
Forged or cross-group references are not found. Authorized readers receive
`410 attachment_expired` for expired tool images, including after the retention
sweep, matching upload and conversation attachment reads.

After a successful upstream response, NyxID reads unseen tool images from the
live turn and checks `nyxagent-input-image-v1`. Text-only turns make no extra
capability request. A supported agent receives a user message on the same
session containing NyxID's untrusted-tool-output warning, labels and
`input_image` attachment URLs. Protocol, version, source, content types and
per-image/count/total-byte capability limits match user uploads. Omitted images
are noted. Each image round adds a model call and its normal usage charges.

At most two image rounds run per turn, using `{turn_id}:images:{n}` idempotency
keys. Newly produced images may enter the second round. Image rounds refresh
the lease, obey Stop, emit `turn.continuing` with a positive `continuation` counter
and additive `reason: tool_images` / `image_round` fields, and join
completed text with the next answer. They do not consume the automatic budget
continuation allowance. Budget/time continuations retain their existing text-only
input; unseen tool images wait until a response succeeds, so image failures
always have a completed answer to preserve. The viewing instruction forbids
repeating completed actions and permits further actions only when the images
show they are needed for the user's request.

Only attempted image rounds set reply `image_input` metadata: `sent` on success,
`unavailable` on failure or for images omitted by that round's capability limits.
Included images keep the field unset while the request is in flight. Delivery
metadata writes are best-effort and cannot discard a completed image answer.
Without image capability, or after both rounds have been spent, remaining tool
images keep this field unset and retain their existing presentation. Tool-image
unavailability reads "The agent couldn't view this image." User-upload advice
is unchanged.

The thread-key `GET /api/v1/assistant-attachments/{id}/content` endpoint serves
a tool image only to its producing conversation's current live, non-stopped
turn, with the same owner, turn ID and still-bound credential. Org-agent access
uses the existing conversation resolver. A guest may see images produced in
that same guest turn, never another turn's images. Retention and image MIME
checks apply; other contexts remain not-found-shaped. Human attachment access
is unchanged.

Deploy all NyxID replicas for consistent image fetching. Agents without the
capability keep one text-only response. If an image continuation fails (including
an old replica returning 404 on its image fetch), the turn settles with the
already completed answer and an unavailable notice. The failed upstream binding
is cleared so the next turn starts from a recap instead of a damaged session.

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
reason/time and creation/update dates. Active turns written by current workers carry a
`heartbeat_at`; old rows may omit it. Indexes include owner + descending update + ID and
active heartbeat time for the orphan sweep.
`assistant_messages` stores UUID, owner, conversation, monotonic seq, turn ID,
role, text, completed/failed status, optional stable error and date. Unique index:
conversation + seq. All dates use the repository BSON date helpers.

Before egress, a transaction writes the user message and claims `active_turn`.
A competing send gets `turn_active` while the fence is live. A fence initially expires at
`started_at + ACTIVE_TURN_TTL_SECS` (1800 seconds execution + 300 seconds settlement
grace). Automatic continuation refreshes `lease_expires_at` on that same turn. The
detached worker renews `heartbeat_at` every ten seconds through execution, continuation
gaps and settlement. Current rows use the heartbeat for orphan detection and authoritative
Stop; the existing `live_turn` lease/TTL check remains the admission and DTO fence. Legacy
rows without it keep the same lease/TTL behavior. The next send transactionally
inserts an empty failed reply for the lost turn (`error_code=turn_lost`), clears
the binding with `turn_failed`, inserts the new user message and claims its fence.
A 15-second cross-replica sweep reclaims only rows whose heartbeat is at least 75 seconds old,
through the same fenced settlement path; a fresh long-running turn is never swept.
Delete stops and cancels before removing the row, and a late worker settlement against
the deleted row is a quiet no-op. All late settlement is fenced by the exact turn ID.

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
the binding with `stopped`, then emits `turn.completed(cancelled)`. A Stop
committed before settlement wins the transaction race. If the worker is gone, or has
not settled within five seconds, NyxID runs the same fenced settlement itself; a stale
heartbeat can trigger that settlement immediately. This keeps voice acknowledgement
state, machine activity and normal completion signaling consistent. There is no NyxAgent
Stop API: stream cancellation is the supported upstream cancellation mechanism.
Stop also signs a conversation-scoped node cancel: cua, process groups and
gateway streams stop immediately, and stopped-turn machine calls are refused.
A standalone desktop Stop applies to all agent activity on that machine.

Upstream request, rebuilt entirely by NyxID:

```json
{"model":"nyxagent/chat","input":"marked NyxID context preamble, then current user text","stream":true,"store":true,"instructions":"stable server prompt, binding marker and optional recap","conversation":"conv_... when bound"}
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
user's language. Reset recaps use the independently bounded checkpoint,
requirements, recent-message and upload sections described above. Failed partial
messages are labeled; the current user message is excluded.

## Session instructions and per-turn context

The deployed NyxAgent passes `instructions` to Codex as `developerInstructions`.
Codex exposes those instructions in the initial session context, but does not
make replacements model-visible on an ordinary resume. NyxID MUST therefore
separate stable session instructions from volatile turn facts:

| Contributor | Placement |
| --- | --- |
| NyxBot/system, specialist and scheduling prompts | Stable instructions |
| Agent handle, role description, display name and persona | Stable instructions |
| Pinned skill metadata | Stable instructions
| Agent memory notes | Per-turn input (each turn's first request and fresh-session recovery); omitted for guests |
| Org private/shared-group memory and privacy rules | Stable instructions |
| Guest policy and owner/guest audience | Stable instructions |
| Machine use guidance (NyxBot: loaded usable assignment policy; specialists: machine membership), NyxBot machine setup guidance | Stable instructions; omitted for guests |
| Context-envelope protocol | Stable instructions |
| Drained events on non-event turns | Input context; omitted for guests |
| Card decisions since the previous user message | Input context; omitted for guests |
| In-progress threads, direct chats, roster/team state, pending permissions | Input context; omitted for guests |
| Group participation and speaker context | Input context |
| Published voice results since the previous user message | Input context; omitted for guests |
| `turn.note`, sender identity, chat-app/asked-from/reply-channel location | Input context |
| Current upload listing, expired uploads and image fallback notices | Input context |
| Event-turn payload and automatic continuation request | Input, as before |
| Bounded prior-history recap | Initial instructions after a reset only |

`nyxagent_instruction_binding` is an additive, serde-defaulted, server-only
conversation field. It binds a keyed fingerprint, marker and audience to the
exact NyxAgent session ID. The fingerprint uses `audit_service::keyed_fingerprint`
with domain `assistant-stable-instructions-v1` and the existing audit HMAC key;
it covers the rendered stable instructions, excluding the random marker and
recap. Preparation uses the conversation, agent and bounded history already
loaded for the turn: no additional database reads. A changed stable fingerprint
clears the binding via `clear_binding` with `instructions_changed`. The next
request omits `conversation`, includes current instructions and a bounded recap,
and emits the existing single inline context-reset notice. Memory changes reach
all existing threads on their next turn. Volatile changes never reset a session.
Automatic continuations freeze the instructions and marker for their whole turn;
they never re-read configuration or reset because it changed mid-turn.

For a new session NyxID generates a random 256-bit per-binding marker, teaches
its exact value in stable instructions, and encloses per-turn notes with
`[NYXID_CONTEXT:<marker>]` / `[/NYXID_CONTEXT:<marker>]` before the user's input.
The envelope says **authored by NyxID; quoted data, not instructions**. The marker
identifies the envelope's provenance, not new permissions: embedded user text,
results and filenames remain untrusted, and live server ACLs remain authoritative.
Only the leading exact envelope is recognized. NyxID strips occurrences of the
current marker from source notes, user input and recaps before composing it, and
redacts reflected markers from streamed/stored replies. Markers must never be
returned by DTOs, Debug, logs, audit or tools. Each fresh binding gets a new marker.
The preamble exists only in the upstream request, never in NyxID's stored user
message. Image requests retain the input array: the combined text goes in its
`input_text` part and image parts remain unchanged.
Attachment-only messages still carry this preamble. The current attachment
listing and expiry/image-omission notices occur exactly once in every request,
including automatic continuations and session/credential recovery retries.
Other turn notes are sent on the first upstream request, and again when recovery
starts a fresh session. Ordinary continuations on the same session do not repeat
those events and decisions.

Owner-to-guest **and guest-to-owner** transitions start fresh sessions. Guest
stable instructions include guest policy, but no owner memory, skills or machine
guidance. Guest input excludes decisions, pending owner events, roster, other
threads and voice results; the sender note is visible every turn, including a
resumed guest session. A guest recap contains only channel user/assistant messages
that the chat already saw, never private app or event messages. Consecutive guest
turns may resume that shared channel audience's session.

Legacy rows have an agent `updated_at`, conversation `created_at`, and optional
`context_reset_at`, but no exact session-establishment timestamp or dedicated
persona/description/skills revision timestamp. The latest context reset, or
conversation creation when absent, is a lower bound on when its session was
bound. If the agent was updated after that bound, NyxID MUST reset the legacy
session once, including guest sessions. This repairs pre-deployment persona edits
even when later replies continued using stale instructions. Grant-only edits can
also cause this conservative one-time reset. Memory timestamps newer than the
latest assistant reply remain an additional reset signal for owner sessions.
Otherwise NyxID adopts the fingerprint without resetting. Once stored, only the
fingerprint/audience comparison applies; timestamps cannot repeatedly reset it.
A legacy session also has no marker in its
initial context: without resetting it, NyxID cannot install one securely. Until
its next natural reset, its input notes are explicitly
`UNTRUSTED_LEGACY_CONTEXT`, with no authenticated-provenance claim. They remain
model-visible quoted data. This is the migration exception to marked envelopes;
there is no mass reset on deployment. Stale metadata naming a different session
is treated as legacy. Settlement persists instruction metadata with the returned
session in the existing fenced transaction; rotation, failure and destruction
clear it with the binding. Old replicas ignore the additive field safely, but
only upgraded replicas refresh prompts or deliver per-turn envelopes.

## Stream and recovery

### Streaming redaction and turn status

NyxAgent emits ordinary text as soon as it is safe to do so. The relay retains
only the longest UTF-8 character suffix that is a proper prefix of a live
credential or context marker; it does not use a fixed byte window. Retry and
credential recovery emit a `turn.notice` with code `retrying`. Clients derive
the visible phase from persisted approvals, activities, continuations, and the
live stream, and clear it when the turn settles. A failed live stream
adds an interruption notice while polling reports the turn active. Only the
status dot animates (with motion-safe); a running tool shows elapsed time.
Action summaries follow the turn status, retain recovered failures with warning
copy, and show the total count separately from the retained latest 40 entries.
Activity labels come from compiled native definitions or an authorized published
operation resolved immediately before dispatch; stale-turn writes are no-ops.
At a continuation boundary, the held tail is flushed with the same two-newline
separator used in persisted text, preventing a contiguous cross-response secret.

First-byte deadline is 30 seconds including request setup. Idle timeout is
120 seconds (upstream keepalive is 15 seconds). Turn execution is capped at
610 seconds per possible upstream turn, bounded by the continuation hard cap
(32 continuations); each stream is capped at 8 MiB and total output text at 2 MiB. Incremental decoding
handles split UTF-8 and LF/CRLF frames. Unknown event types are ignored; malformed
recognized events, invalid IDs, or nonmonotonic recognized sequence numbers fail
closed. Cached upstream replay may contain only a terminal response, so terminal
output is authoritative. Returned `conversation.id` and `response.id` are always
stored on success. Locally guessing the first session ID is incorrect.

NyxID events have a strictly increasing `cursor`: `turn.status` (also carries
`conversation_id`), `message.started`, `block.started`, `block.delta`,
`block.completed`, `message.completed`, `turn.completed`, and `turn.continuing`. They reuse the Direct
text-event grammar. Terminal status is `completed`, `failed`, or `cancelled`;
error is null or a stable `{code,message}`. The additive `turn.notice` carries
`code=context_reset` and the fixed message:

> Context reset preserves completed actions, attachments and decided cards; incomplete actions must be reviewed and rerun.

| Upstream result | NyxID action |
| --- | --- |
| Bound 404 `not_found` | Clear binding (`session_lost`), emit notice, retry once unbound with recap and same turn/idempotency key |
| 503 `capacity_exceeded` / `draining`, 409 `session_busy` | Retry proven refusals within a fixed window per request, measured from first dispatch: 120 seconds interactive, up to 600 seconds trigger. Original grace bounds the first ordinary trigger request; confirmation turns use 120 seconds. New explicit task/image requests get fresh windows within the total-turn cap. Full jitter up to 15 seconds; delta-seconds Retry-After (1–60) is a minimum. Same body/key for refusal retries; retry expiry never shortens a dispatched response/read bound |
| 401/403 or `agent_key_required` | Replace credential, clear binding, emit notice, retry once with recap |
| `stale_response`, `outcome_unknown` | Fail without retry; discard binding |
| `insufficient_credits`, `wallet_suspended`, `plan_entitlement_required` | Terminal billing refusal: no credential replacement, retry, backoff or rebind. Show top-up, suspended-wallet support/billing, or plan-entitlement guidance respectively |
| `tool_budget_exhausted`, `turn_timeout` | Continue automatically on the same session, with the same grants/TriggerRun, up to `max_auto_continuations` (default 8, hard cap 32). Preserve context if the bound is reached |
| Identical full call/result sequence and unchanged reply text | Stop with `continuation_no_progress`; preserve the session |
| Other failed turn or invalid stream | Persist failed partial reply; discard binding immediately with `turn_failed` |
| User Stop | Persist the partial reply; discard the binding with `stopped` and continue with a recap |

The transcript displays one inline system note for the latest reset, before the
first message whose `created_at` is strictly greater than `context_reset_at`, or
at the tail if no message follows. Failed settlement uses one timestamp for the
reply and reset, placing the note after the failed reply and before the next user
message. Rebind resets occur during execution and precede that turn's reply.
Live `turn.notice` displays the same note immediately and refreshes the persisted
reset timestamp; subsequent reloads retain its transcript position without a banner.

The next turn after any reset sends a recap and emits the notice. There is no replay of an uncertain action after an upstream stream has started.
Explicit budget/time terminal errors start a new continuation request on the same
session with an idempotency key suffixed `:continuation:N`; they do not reset or recap. Generic allowlisted messages replace
all raw upstream error bodies. The raw current key is redacted from reflected
assistant output, including split deltas. Wire-log capture is disabled for this
surface.

On reload, the client fetches persisted history and follows active history
through the live stream (or, without it, polls every two seconds) until its own
metadata reports settlement. The index has no periodic
poll: selected-history settlement refreshes it once, and send completion updates
the sent thread's row from its settled history. An idle thread is not polled
because its agent is busy in another thread, and its machine desktops are polled
only during a turn or while one is open.
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
the worker's stream. Explicit upstream budget/time errors retain a resumable session; other failures
invalidate the binding immediately. The numbered [6]/[7] header block
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

Machine exec requests explicitly declare `services` (connected service slugs or
IDs); omitted/empty declarations grant none. The declaration is job-bound and
rechecked against live key authority, appears on confirmation cards and audit,
and controls the catalog-derived SDK/git environment. Declare `api-github`
(or the connected GitHub PAT service) before plain git clone/fetch/pull/push.
Gateway response compression is preserved end to end. Discovery shares the
proxy/MCP ACL resolver and gateway calls use the middleware's API-key identity.

Recommend the container or `--separate-users`: a non-isolated shell can read
the node token, signing secret and locally stored credentials. Explain the
machine isolation warning without refusing the owner's choice. The generated Docker
command supplies NyxID's seccomp profile so Chromium remains sandboxed.

Owner desktops use native capture (X11 on Linux, ScreenCaptureKit on macOS),
JPEG dirty rectangles at up to 30 Hz and independent owner input. Agent actions
and observations still use cua. Takeover cancels active agent cua/command/file
work immediately; late results are discarded. Pixel/input data never enters
agent tools or transcripts during owner control. Frame-rate, latency and
bandwidth measurements are in [validation and measurements](../MACHINE_NODES.md#validation-and-measurements).

## Trigger turns

Scheduled and assistant-webhook work enters the same engine with
`TurnOrigin::Trigger`, `guest = false` and an internal fenced `TurnClaim`.
`begin_turn` admits that claim inside the conversation/credential transaction;
the run record and active turn therefore agree even across a crash. This branch
executes only for trigger turns, adding no database query to ordinary turn start.
Scheduled trigger instructions retain owner authority. Webhook data is untrusted:
In Auto and Ask every time, `confirmation_policy = changes` (default) enforces owner cards on every changing
tool call, using the same HTTP/catalog classifier as guest access and explicit
native tool contracts. The owner may explicitly select `destructive` with a
warning to confirm only destructive calls. This policy is snapshotted per run
and is bypassed with a `webhook` receipt in Full access. Agent grants and
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

### Machines in automation turns

Machine tools in scheduled turns retain the same live machine/login grants,
owner-control lockout and `machine_confirm` rules as owner-initiated turns.
Webhook turns add their per-run confirmation policy. One exact-argument,
one-use owner action card satisfies both policies in Auto and Ask every time. Full access bypasses both with an automatic receipt.
Exec, file writes/attachment saves, job cancellation and changing computer
input are destructive because they can overwrite data or interrupt arbitrary
work. Saved-login filling and requesting owner control are changing, but not
destructive: they are restricted field insertion or an owner handoff rather
than arbitrary execution. Machine listing, saved-login metadata, file listing,
file reads and job status pass the webhook read-only gate; machine-level `all`
confirmation applies in Auto and Ask every time; Full access bypasses it with a `policy` receipt. Both direct native
calls and `nyx__call_tool` use these checks. Guests and developer OAuth tokens
cannot use machine tools or the human desktop/control routes.

Assistant workspace navigation places Automations (`/assistant/automations`) and
Machines (`/assistant/machines`, Saved logins at `?tab=logins`) beside Plugins and
Approvals for both engines. Setup/pairing stay in `AssistantShell`; the desktop is
standalone under `/assistant/machines/{id}/desktop`. Studio Nodes shows only a
read-only machine summary linking to assistant settings; Developer → Triggers
retains secrets/replay. `/automations` redirects with `setup` and `agent` intact.
Server-generated browser URLs use `services::assistant_links::AssistantPage`.
Machine turns use the compact browser tool for ordinary web work. Explicit NyxAgent `tool_budget_exhausted` and `turn_timeout` results continue the existing session under the owner-configured continuation bound; they do not reset context. Stop fences the turn and cancels in-flight machine work.

Machine updates use `nyxid__machine_update` and always require an owner action
card, including requests from granted specialists. Ungranted specialists request
machine permission through NyxBot. Legacy containers receive a token-free link to
the prefilled host command; the agent ends its turn while a durable
`machine_update` watch waits for reconnect/failure/expiry. On wake, verify the
version, AX state and browser snapshot before resuming. An owner-identified,
different granted native machine may run the command after metadata-only Docker
inspection; the card binds both machines and the inspected container ID. Offer
this assistance proactively for Update available or missing old-node capabilities.
The server release is the supported update target. New setup recommends idle
automatic updates; existing machines require explicit owner opt-in.

Continuation progress is derived server-side at MCP completion from tool name,
canonical arguments and result hashes. Only a rolling digest and call count are
stored on the active turn, fenced by turn and continuation IDs. The activity UI's
bounded labels are not a progress signal. Issued/completed counters exclude incomplete windows from loop detection. Different arguments/results or reply
text continue even when every call uses the same tool.

Machine browser snapshots aggregate visible frames and support query/offset/scope
paging. Trusted native input follows extension hit-testing; explicit DOM fallback
is labelled. Secure and dev Linux desktops have separate X servers and cookies,
with a live-panel display switcher and per-display owner control. On macOS the
physical desktop is shared and takeover locks both views.


Machine browser recovery (0.41.1): secure-browser extension startup is event-driven
on persisted profiles, with automatic handshake repair/relaunch and a specific
12413 recovery message if it fails. The status CLI reads cached daemon state and
never launches another browser. A 12416 unsupported computer call includes the
bounded advertised `computer_tools`; use `nyx__machine_browser` snapshot for page
content, or dev screenshot for an owner attachment. Machine discovery shares
service-tool word matching, including full-match-first ranking.

## Thread titles

New owner threads have `title_source: provisional`. Their first owner message
supplies an immediate title: first nonempty line, collapsed whitespace, at most
60 Unicode characters, cut at a word boundary where possible. Existing rows
without `title_source` deserialize as `user` and are never automatically renamed.
Guest/channel threads, automation threads and hidden group-member threads keep
their existing naming. User renames set `title_source: user`, remain allowed
during a turn, and always win over a late generated title. The conditional write
matches only `provisional`; settlement preserves concurrent metadata changes.

**Model-route decision (U1 review):** do not use NyxAgent for titles.
Its `POST /v1/responses` `store:false` controls persistence only; it still runs
an agent with the thread key's MCP authority. `Request` rejects unknown fields
and has no `tools` switch. A prompt forbidding tools cannot isolate that run,
especially when the first reply quotes untrusted web or tool content.

`services/assistant_oneshot_inference.rs::one_shot_text` instead calls a provider
text endpoint directly. Its typed input/limits interface accepts no tool schema,
agent key, session or arbitrary provider JSON. Requests omit both `tools` and
`tool_choice`, use `stream:false`, and bound output tokens. Only OpenAI Responses
sends `store:false`; Chat Completions and Anthropic Messages omit it for provider
compatibility. Without a tools array, providers reject `tool_choice`. Compatible
Chat Completions providers can also reject `store` as an unknown field.
The helper rejects tool calls and non-text content; it never executes them.
Responses reasoning metadata is ignored, retaining only the text answer. The
three catalog `inference.wire_protocol` values are supported: Responses, Chat
Completions and Anthropic Messages. Redirects are disabled and responses are
capped at 64 KiB.

Utility inference is an admin-controlled pair on the singleton `platform_settings`
row: optional `utility_inference: {service_slug, model}` and default-false
`utility_inference_admin_modified`. The pair is atomic, rather than splitting a
platform choice from per-service model metadata. Startup seeds
`chrono-llm-public` / `gpt-6-luna` only when the pair is absent/null and the admin
marker is false; admin overrides and explicit clears are never resurrected.
This follows `inference_admin_modified` semantics without changing catalog wire
protocol defaults. Old documents/replicas remain compatible. Absent or explicitly
cleared configuration retains legacy service selection.

Platform admins can `GET` / `PUT /api/v1/admin/settings/utility-inference`. PUT
accepts the complete `{"service_slug":"chrono-llm-public","model":"gpt-6-luna"}`
pair, or JSON `null` to clear. Writes validate an existing active HTTP inference
catalog service with model-list support and platform credentials (never
`llm-nyx`), and a nonempty model ID of at most 256 bytes without whitespace or
control characters. Settings audit contains actor/change metadata only.

Titles and learning use the configured service's catalog platform route, with
live platform-key grants checked before decryption on every call. A personal
connection cannot override this route. Its model list is checked live, even
when model/capability caches are warm. The configured model must be advertised,
not shut down, and a chat model. If missing or definitely rejected before
generation, select another live small non-reasoning chat model on that same
service. Never switch to personal services because an available utility service
has an empty/broken model list, rejects models, or fails during inference.

Only an inactive/missing utility service or unavailable platform grant permits
legacy selection across the person's other services, excluding the utility
service itself. With no configuration, legacy selection also applies: at most
32 active HTTP inference services with model-list support, platform-available
first then slug; `llm-nyx` is excluded. These routes retain the normal personal
binding precedence, live org role/service scope, and provider delegation.
No credentials are provisioned and no authority is cached.

Model selection excludes expired `shutdown_date` rows and non-chat families,
including search, deep-research, codex, computer-use, instruct, pro, legacy
babbage/davinci, audio, images, embeddings and video. Prefer non-reasoning models,
then nano/mini/haiku/flash/small/fast/luna, then model ID. There is no guessed
vendor fallback. Legacy routes retain the bounded 128-entry/10-minute selected-ID
cache, partitioned by service and credential class; utility model lists bypass it.

At most three candidate attempts share the existing overall deadline. A bounded
model-list failure can move to the next legacy service without generation. Only
definite pre-generation HTTP 400/401/403/404/405/406/415/422/429 responses without
usage or generated content permit generation fallback. Each refused request
settles at zero quantity through the normal durable billing path. HTTP 408/409,
5xx, redirects, transport failures/timeouts, malformed success, incomplete/empty,
oversized or credential-reflecting output end the sequence: no second paid or
ambiguous generation. The successful output is still character-bounded.

OpenAI reasoning models (`gpt-5*`, `gpt-6*`, o-series) get 1,024 output tokens to
leave space for visible text, and a model-appropriate minimum effort (`none` for
GPT-6 Luna and GPT-5.x, `minimal` for original GPT-5, `low` for o-series and other
GPT-6 models). Responses uses `reasoning: {effort}`, Chat Completions uses
`reasoning_effort` and `max_completion_tokens`. A 400 explicitly identifying an
unsupported reasoning parameter/value suppresses that parameter on subsequent
calls for the same service/model, cached for 10 minutes (128 entries). Provider
error text is inspected only for classification, never stored or logged.
Official API contracts: [reasoning and output budgets](https://developers.openai.com/api/docs/guides/reasoning),
[Chat Completions parameters](https://developers.openai.com/api/reference/resources/chat/subresources/completions/methods/create),
and [GPT-6 Luna effort support](https://developers.openai.com/api/docs/models/gpt-6-luna).

Each failed attempt emits one metadata-only warning: fixed caller (`title`,
`learning`, or the existing shared helper's `voice` classifiers), service slug,
credential class, safe chosen model ID (empty before selection), and fixed stage.
Stages are `models_unavailable`, `no_model`, `http_status:<code>`, `empty_output`,
`incomplete:max_output_tokens|content_filter|unknown`, `too_long`,
`credential_reflection`, `timeout`, `route_unavailable`, `dispatch_unavailable`,
or `invalid_response`. No prompts, input, output, provider error prose or keys
are logged. Deadline cancellation diagnoses the active attempt once.

Both model discovery and inference use the existing billing admission and
settlement path and `BillingOwnerResolver::resolve_for_execution`. This metadata
task bills the acting person, including platform use and authorized org BYOK;
it never charges an agent or an org wallet. Org-agent titles retain the B3a
actor checks both before inference and before the conditional title update.
Provider processing policies still apply. This internal helper is also suitable
for bounded text classification; callers must independently authorize the task.

After the first successful owner exchange settles, start title generation in a
detached task. A replica permits at most four concurrent requests, with no queue
or background retries and a 20-second overall deadline (15 seconds for inference, including
discovery, and 128 output tokens for non-reasoning title models; reasoning
models receive the bounded 1,024-token allowance above). This never holds the turn permit or
delays settlement. Supply only bounded text excerpts of the first user/reply
pair (2,000 characters each), marked as untrusted data, with instructions to
produce a 3–6 word title in the user's language. Tool isolation comes from the
provider request contract, not these instructions. Send no images, attachments or session binding.
Sanitize and bound output to 60 characters and render it as text. Never log
excerpts or titles; errors are best effort with metadata-only diagnostics and the credential cannot be
reflected into the title; reflected resolved provider credentials are rejected.
Cancellation retains metering cleanup, including a usage estimate for dispatched
requests that never yield a response. The existing identifier-only conversation change
stream refreshes titles in the sidebar and active thread without a reload.
Sidebar Rename is an inline editor with optimistic metadata updates and rollback
on failure; it does not interrupt a running turn.

Machine tool discovery and execution intersect explicit per-agent capability
assignments, node-local ceilings and live actor authority. With capability editing
enabled for the acting person, new selection enables nothing. With it disabled,
NyxBot reachability and specialist Grants selection snapshot legacy capabilities
under the live ACL. Existing snapshots survive enabling the editor, and explicit
restrictions survive disabling it. `nyxid__machine_capabilities` lists the current revision and
proposes owner-reviewed widening; it cannot approve its own request. A v2 node
receives signed, 45-second authority renewed every ten seconds, and stops work on revocation
or lease expiry even after socket loss. Legacy assignments remain visibly shared;
context IDs in this phase do not isolate files or browser sessions.

Chat-key discovery rows carry dense authorized `preference_rank` and follow the
owner's saved same-catalog connection order by refilling only that group's occupied slots within equal relevance buckets; search also reports `executable`.
NyxBot enforces eligible personal-first order at the concrete execution boundary,
including raw model callbacks. Exact names stay exact; one-turn owner selection
chooses a target and does not waive org consent. See
[service execution](../NYXBOT_SERVICE_EXECUTION.md) for raw model turn grants,
pool pins, native execution and machine job grants. Guest connected search/list includes only granted
UserManaged and Platform connections and drops Internal catalog entries before
ranking. Native `nyxid` virtual tools remain subject to their separate guest
authorization; including that virtual catalog does not grant execution access.
# NyxBot model selection

`GET /assistant/nyxagent/models` returns NyxAgent's live model list for the
calling owner (cached per owner for a minute; `available: false` marks a listed
model NyxAgent cannot serve). When the list cannot be read it returns the
default model only. Model DTOs preserve an upstream `display_name`, `label`, or
`name` (in that order), with `Default model` for a nameless `nyxagent/chat`
profile. These labels never change the routing ID. The composer's combined
settings menu selects a thread's model with
`PATCH /assistant/nyxagent/conversations/{id}/model`; agent details select the
model for an agent's new threads with `PATCH /assistant/nyxagent/agents/{id}`
(`model`), and NyxBot can change an agent's model with `nyxid__update_subagent`
`model`, which the owner's access mode treats as a sensitive change.

The default composer exposes a Thinking toggle: on selects `nyxagent/research`,
off selects `nyxagent/chat`. `assistant:model-controls` (default off) replaces
Thinking with the advanced Model and Reasoning rows and enables the agent-details model selector for
staff-selected internal users. It gates presentation, not API authorization;
Thinking uses the same model mutation and validation as direct selection.

- Validation: a selection must be a well-formed `nyxagent/...` id and, when the
  live list is readable, a listed available model; otherwise the request fails
  with `model_unavailable` (HTTP 400, code 12103, "Choose another model in the
  model selector"). When the list is unreadable a well-formed id is accepted and
  a turn NyxAgent refuses settles with `model_unavailable` and the same guidance.
- A thread with any turn fence (running, or awaiting the stale-heartbeat sweep)
  refuses with `turn_active` (HTTP 409); a destroyed agent's read-only thread
  refuses with a conflict. Reselecting the current model changes nothing.
- A real change clears the upstream session and instruction binding, records a
  `model_changed` context reset and audits `assistant_model_changed` with the
  acting person (scope `conversation` or `agent`, old and new model).
- Model selection is human-only on these routes (conversation keys are refused);
  only agent maintainers may change an agent's model.

### User context reset semantics

A human owner reset records `context_reset_reason=user_reset`, clears the live
binding and deletes the durable checkpoint, so the next turn starts without a
recap; the transcript itself is unchanged. Resetting while a turn runs returns
`turn_active` (409). Guests and delegated identities cannot reset context.

### Detached stream settlement

The turn subscription returns HTTP 200 once the detached execution is accepted. Upstream failures are settled asynchronously: the stream emits the terminal normalized error event, and the failed reply persisted in history carries the sanitized error class, upstream code, and `error_detail` metadata (stage, status, request id, response id, continuations, turn id, and timestamp).

## Reply feedback

Human NyxAgent clients may submit or withdraw one thumbs-up/down vote per
assistant reply. `PUT /api/v1/assistant/nyxagent/conversations/{id}/messages/{message_id}/feedback`
and the equivalent `/groups/{id}/messages/{message_id}/feedback` accept `{vote:
"up"|"down", comment?: string}`. Only completed assistant replies (conversation)
and agent replies (group) are eligible; others return 400
`assistant_feedback_ineligible`. An unknown vote is 400 `feedback_vote_invalid`;
a comment over 1,000 characters (after trimming and removing control and bidi
characters) is 400 `feedback_comment_too_long`. A vote without a `comment`
field keeps the stored comment; an empty comment clears it. Writes and
withdrawals share a per-person limit of 30 per minute (429). Repeating the
same vote and comment is idempotent; one atomic upsert per (person, reply)
means concurrent first votes produce one row and one `submitted` event.
DELETE withdraws an existing vote and is idempotent when none exists. History
responses include the person's current vote. Conversation, group and user
deletion purge feedback rows.

Each write stores a metadata-only reply context snapshot (`context`):
surface, agent id and kind (from the thread's agent), model, effective access
mode (the agent's override, else the owner's mode), reply status and error
code, origin and channel, turn id, reply seq and time, latency from the turn's
user message, tool call and error counts, up to 20 distinct tool labels,
machine use, attachment count, steering and voice flags, reply length, and
whether the context was reset since. Group replies record agent id, org flag,
tool summary and reply length only. Reply, prompt, comment and tool-argument
text are never part of it.

Every state change emits one `tracing` line with target
`nyxid::assistant_feedback` and `event` = `submitted`, `changed`,
`comment_updated` or `withdrawn`, carrying the feedback, person, message and
parent ids, vote, previous vote, `has_comment`, comment length and the
flattened context fields. Idempotent repeats log and audit nothing. Comment,
prompt and reply text are never logged. Audit events
`assistant_feedback_submitted` / `assistant_feedback_withdrawn` carry the same
ids plus `previous_vote`, `change`, `has_comment`, `agent_id`, `agent_kind`,
`model`, `error_code` and `tool_errors`.

Admins and operators read `GET /api/v1/admin/assistant/feedback` with RFC3339
`from`/`to`, `vote`, `model`, `agent_kind`, `has_error=true` and a
`(created_at,_id)` cursor (`feedback_cursor_invalid` when malformed). Responses
hold per-day counts and up to 100 recent rows with their comment and context;
rows written before the snapshot existed have `context: null`. Message text is
never included.


### Admin NyxBot activity

Studio **Usage → NyxBot** separates assistant activity and feedback from the
billing Dashboard and List. It is available to admins and operators; the broader
usage-read permission does not grant assistant analytics or feedback access.

`GET /api/v1/admin/assistant/analytics` accepts RFC3339 `from` and `to`, defaults
to the last seven days, and rejects non-positive windows or windows longer than
31 days. The upper bound is exclusive. The response includes the effective
window, `generated_at`, `active_sessions`, `active_accounts`, `running_now`,
`outcomes`, `daily_sessions`, and `daily_turns`. MongoDB reads have a ten-second
execution limit and project only metadata, never transcript text.

- Active sessions are distinct durable conversations with persisted `role=user`
  input in the window, including channel input and steering. These are threads,
  not sessions inferred from an inactivity timeout. Active accounts are distinct
  conversation owners, which may differ from guest/channel actors.
- Running now is a live snapshot independent of the historical window: an active
  turn must have a heartbeat no older than 75 seconds, no requested Stop, and an
  unexpired effective lease (explicit lease or started-at plus the turn TTL).
  Legacy turns without heartbeats are not counted. The tab polls every 30 seconds.
- Settled outcomes use distinct turn IDs from retained assistant replies in the
  window, deduplicating voice copies. Completed, failed, and stopped are separate;
  `error_code=cancelled` is stopped. Tool indicators count turns with recorded
  activities/errors, not exact calls, since the stored activity list is bounded.
- Feedback totals and the chart use the existing feedback endpoint's daily counts
  across the whole period. The list's vote/model/agent filters and cursor do not
  change the overview. Helpful rate is helpful votes divided by all retained votes,
  with no value when there are no votes. It is not a task-success rate. Feedback
  includes group replies; turn outcomes include specialist and automated work.

Chart days are zero-filled in UTC. The UI's end date includes the whole selected
day and is translated to the respective endpoints' exclusive/inclusive bounds.
Feedback is grouped by original submission day; vote edits, withdrawals, and
transcript deletion can change historical reports. List entries lead with votes
and comments; message IDs and tool identifiers are available in reply details.
Native channel guest authority also covers positively addressed foreign
Telegram/Lark/Feishu bot actors in owner-enabled groups. The receiving bot's
existing channel/chat agent answers; native mention names do not select arbitrary
app-group members. These turns need no sender OAuth and retain guest service,
privacy and approval restrictions, including delayed image/text delivery. They
cannot link or impersonate owners, queue owner work or renew thread following.
Durable per-chat and owner-hourly handoff limits bound automated exchanges; only
admitted authenticated human messages reset the chat budget. See the
[channel relay contract and gateway rollout](../CHANNEL_BOT_RELAY.md).

### Model failure attribution and capacity admission

Only explicit pre-acceptance status/code pairs above prove a capacity refusal.
Admission is scoped to each upstream request. Refusal retries keep the same request
identity and fixed window from its first dispatch: 120 seconds for interactive,
channel and event requests, or at most ten minutes for trigger requests. The first
request of an ordinary trigger turn is also bounded by the run's grace deadline.
Automatic task and image continuations are new requests with fresh windows;
trigger confirmation continuation turns use the interactive window and ignore the
original run deadline. Every request remains bounded by the existing total-turn cap.

The admission deadline gates only another dispatch after a proven refusal and the
backoff sleep. It does not prevent a first dispatch, even after late pool admission
or slow preparation, and it never shortens the ordinary first-byte or rejection-body
read bounds. An outstanding request may finish after the admission deadline within
those bounds. `capacity_exceeded` requires a proven refusal; timeout or a broken
connection retains its uncertain-outcome classification. A request that returned
2xx is never resent if its stream later fails. Explicit continuations admit
independently, and credential replacement and session rebind retain their existing
once-per-turn recovery limits.
Waiting holds the owner's pool slot. The independent ten-second heartbeat,
250 ms Stop watcher, permit cancellation and total-turn cap remain active.
`active_turn.waiting_for_capacity` and additive `turn.status` events let clients
show the elapsed capacity wait and its bound without transcript rows.
A worker process loss follows `turn_lost` recovery; it never replays the execution.
An unwinding execution panic settles immediately as `internal_error` at stage
`panic`, through the same fenced settlement as other failures.
Only internally marked engine requests to the configured `llm-nyx` service with
503 `capacity_exceeded`/`draining` or 409 `session_busy` settle zero platform and
resale quantities. The proxy awaits the existing durable settlement and reservation
release before returning the refusal to the retry loop. A release failure stops
admission with a generic internal proxy error; it cannot trigger another funded
attempt or be attributed as an insufficient-credit/entitlement refusal. Durable
settlement recovery retains its existing retry path. Attempt telemetry/audit remains.
Other errors and accepted executions retain normal accounting and distinct
billing request IDs.

The model observer captures the live turn and a UUID attempt at HTTP request
admission for a conversation credential, a catalog inference service and a POST
model endpoint. Today's NyxAgent model bridge uses this route; optional inference
MCP tools use their separate direct forwarding path. Machine ingress is excluded.
An admission CAS registers the attempt on the captured turn. Later facts require
that exact attempt and turn; late writes cannot follow a newer attempt or turn.
This extra admission write is necessary to make late-write suppression durable
across replicas. A bounded active-request count is incremented atomically with
admission and decremented at completion (with cancellation cleanup). If requests
overlap, attribution stays uncertain for the rest of that turn: facts remain
diagnostic only. Failed cleanup also errs toward diagnostic-only facts. Facts
from older attempts are diagnostics only. A 2xx admission conditionally clears
any refusal.

Billing admission records only `insufficient_credits`, `wallet_suspended` or
`plan_entitlement_required`. On successful SSE responses the supported protocols
are Responses (`response.completed/failed/incomplete`), Chat Completions (`[DONE]`)
and Anthropic Messages (`message_stop`). Missing usage is valid. Body/transport
errors, provider error events and missing supported terminal events are abnormal;
unsupported formats are `unknown`. Consumer cancellation is diagnostic only and
cannot replace a recorded attributable provider fault. Faults are persisted
before forwarding failure/EOF where possible; HTTP status is unchanged.
Both the direct streaming path and the response wrapper use the bounded byte
decoder, recognizing LF, CRLF (including chunk splits) and CR line endings. The
observer limit applies to each frame in both transports. Forwarded bytes remain
unchanged. The direct path keeps an independent, uncapped SSE usage parser
for every service and HTTP status; model observation runs only for captured model
requests. The diagnostic frame cap never drops billing usage from large
`response.completed` events such as base64 image results. The response wrapper
only observes faults and leaves usage collection to its transport.
Oversized, invalid UTF-8, malformed data or incomplete framing makes terminal
detection `unknown` for that attempt, even if a later frame appears valid; it
cannot prove an interrupted stream. Repeated provider-error or unknown facts are
recorded once per observer rather than on every later chunk.

At settlement only a generic NyxAgent failure or broken stream can be attributed.
A matching attempt's latest uncleared billing refusal within 120 seconds wins,
then an attributable stream fault within 120 seconds (`model_interrupted`), then
the generic error. Typed failures and Stop keep their cause. Attribution retains
the original sanitized `upstream_error_code`, also exposed in the live terminal
event and history. Details and its copied diagnostic ID include that code.
Generic copy is “The assistant could not complete this turn. Try again.”;
`model_interrupted` warns that some actions may already have completed and must
be checked before retrying. Old rows and old clients remain readable; existing
failed rows are not backfilled.

### Capacity wait visibility and engine diagnostics

Each request admission wait persists `capacity_wait_started_at` and its fixed
`capacity_wait_deadline` as BSON dates on the active turn. Completed wait intervals
accumulate in `capacity_wait_ms`, excluding execution time between requests. The first upstream
request separately persists `engine_request_started_at`. If NyxAgent has not
accepted a 2xx response ten seconds after that first request, the server marks
`capacity_wait_note_origin = nyxid_system` and emits the same state to live clients.
This includes capacity refusals and slow or unanswered connections. A turn accepted
before the delay receives no acknowledgement, even if its stream is slow later.
Live acceptance carries an additive `engine_accepted` marker; an older polling
result cannot restore the waiting note after the client has seen acceptance.

The web renders a pending bubble through the agent's ordinary reply renderer:
“Got it — I'm a bit busy right now and will start on this shortly.” It shows elapsed
time from the first request and the admission bound from server timestamps;
reloading or opening another tab does not restart the timer. Acceptance changes
the bubble into the streaming reply, exhaustion into the capacity error, and Stop
clears it through the ordinary terminal state. Stop remains available throughout.

At the same ten-second threshold, channel turns and event turns with a channel
destination may send one acknowledgement with the same copy through the ordinary
reply path. All automations, including chat result delivery, are excluded. A
turn-fenced MongoDB claim precedes the external send. Concurrent workers cannot
send twice; a crash after the claim can omit the remote acknowledgement because
existing channel transports have no recipient idempotency protocol.

The acknowledgement is a NyxID system note stored only as active-turn display
metadata, without a transcript text row. It never enters model input, recap,
checkpoints, question digests, feedback or reply settlement/delivery. Guest chats
receive the same fixed copy, with no owner data. Slow admission alone does not
increase `capacity_wait_ms`; that statistic starts only at a proven capacity refusal.

All incident policy values live in `services/assistant_engine_policy.rs`, including
acknowledgement delay, admission budgets, backoff, Retry-After bounds, attribution
window, persistent-failure threshold and recurring dispatch jitter. The frontend
uses server-provided dates and does not duplicate these tunables. No environment
variables are added.

The `nyxid::assistant_engine` tracing target emits one metadata-only line per
engine attempt (origin, attempt number, HTTP status, sanitized upstream code,
accepted/backoff/failed decision, elapsed capacity wait and remaining deadline)
and one per committed settlement (final code/stage, original upstream code,
status, attempts, wait, duration and optional trigger run). Prompts, replies and
upstream prose are excluded. Attempt counts include engine dispatches and explicit
continuations; model-provider requests have their separate attribution identities.
Failed replies retain optional `attempts`/`capacity_wait_ms` in `error_detail`;
all new settled replies also carry optional `turn_metadata`. Completed replies show
Details only after a positive capacity wait or more than one attempt; failed replies
keep full Details. Duration copy uses whole words with correct singular/plural,
for example “waited 1 minute 20 seconds for capacity”. Zero or unknown waits omit
the waited phrase. Old rows omit these fields. Billing remedies open the account
panel over the current assistant view; previews outside the assistant shell omit it.

`assistant_turn_failed` commits through the audit append path in the settlement
transaction, including Stop and orphan recovery. `assistant_turn_capacity_wait`
commits once per request accepted after a positive capacity wait; repeat acceptance
callbacks do not append another event. Acceptance without an active wait is a
single conditional update and opens no transaction. A
successful turn without a wait emits neither event. Both event types are
searchable in the admin audit filters. Capacity exhaustion keeps
`capacity_exceeded`; its copy includes the measured wait (for example, “NyxBot is
at capacity right now (waited 2 minutes). Try again shortly.”). Short automation
grace windows report their actual seconds rather than claiming a two-minute wait.

### Worker identity, panics and orphan recovery

Every server process generates one engine worker UUID at startup and retains the
standard `HOSTNAME` when it is valid bounded hostname metadata. Startup emits
“NyxID engine worker started” with the UUID, hostname and version. New turns
persist the pair in optional `active_turn.worker_instance` when claimed. Engine
attempt and settlement logs identify the process writing the line; settlements
also identify the process that claimed the turn. Old turns remain readable with
no worker identity.

Web, channel, event, team, group, voice and trigger turns share the detached
runner. It polls execution under `AssertUnwindSafe(...).catch_unwind()`. A panic
becomes `internal_error` / `panic` and enters ordinary settlement immediately,
including its failed audit, terminal event, checkpoint and delivery hooks. The
copy is “NyxBot hit an internal error on this turn. Check which actions completed
before retrying.” Details shows stage `panic`. Completed actions are not replayed.
The independent heartbeat stays alive during settlement and ends with the worker.

A task-local panic hook suppresses the default hook only while polling contained
execution, because the default hook would print payload text before the catch.
The error-level `nyxid::assistant_engine` line includes only conversation/turn and
worker identifiers, source location, and the fixed payload class (`static_str`,
`string` or `opaque`). It never logs the payload. Other tasks retain their previous
panic hook. Process aborts and terminated replicas cannot be caught and still use
the stale-heartbeat sweep.

`assistant_turn_failed` includes the claimed `worker_instance` and the
`settled_by_worker_instance`, plus `heartbeat_age_ms` when a heartbeat exists.
This includes stale stopped turns, whose final class remains `cancelled` / `stop`.
The committed settlement log carries the same owner and heartbeat-age diagnostics,
so a losing sweep race emits no false reclamation line. These identities let
operators correlate a lost turn with startup logs and other activity from that
process. A missing startup/activity record alone cannot prove a replica died;
logs and deployment evidence are still needed when the process is absent.
