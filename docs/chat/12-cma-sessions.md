# CMA session status cards

Active CMA sessions appear automatically **only in the NyxID conversation that
originated the verified CMA receipt**. A CMA glyph and live activity dot in that
conversation's header identify current CMA work. Unrelated chats and account
inventory never establish attribution.

Each compact card leads with the CMA glyph and actual task title, followed by
one line labelled **Agent:** for the overall agent status and its stream connection.
One **Submission details** disclosure contains the **Request status** section
and metadata for all submitted requests. It starts collapsed. Each request is
tracked separately, so a
newer queued request can appear alongside an older request still running. A
tight shared list shows the three newest
public actions; **Show all N actions** expands the bounded 30-action history.
The **View execution** clipboard-list icon, source chat, supplied CMA link, pin, and minimise
controls share the header. Execution access remains available while minimised.
Each icon has a state-aware tooltip available on hover and keyboard focus.
The compact action list and Session details use spacing without internal
horizontal dividers.
Motion identifies the latest reported running action while the agent is working
or starting. Other action markers and the request rows stay static, preserving
their reported statuses. Queued requests use a clock. The overall agent status
provides the single busy indicator when the card is minimised or no running
action has been reported. Reduced-motion preferences disable the spin.
The solid **Pinned** control, filled pin icon, and unpin tooltip identify global
monitoring. **Pin CMA session** makes that card
available in other chats and on every other page. Assistant pages reuse the
shared preview column; other pages use a bounded floating stack. **Unpin CMA
session** immediately restores source-only visibility. Pins and minimise
preferences are account-bound, survive reload and navigation, and are cleared
on logout or account changes. Preferences contain only connection/session IDs
and booleans, never titles, task data, events, or credentials. Version-2 minimise
preferences migrate; obsolete version-1 inventory pins are discarded.

**Open source chat** links to the server-recorded conversation and specialist,
including NyxBot's agent identity. **View in CMA** is available only when CMA
supplies a valid `callback_url`. Both the operation receipt and live session card
use the same browser-link validation: absolute HTTP(S), a hostname, no userinfo,
control characters, whitespace, backslashes, query, fragment, or cloud metadata
endpoint. Valid supplied HTTP(S) scheme, host, port, and path are preserved
verbatim, including their original casing. External
links use `target="_blank" rel="noopener noreferrer"`; NyxID never derives a CMA
UI URL or fetches the callback.
For preview cards, a callback supplied by the current agent-detail response takes
precedence over the receipt's stored link. Omission retains the receipt link;
explicit null, invalid, or non-string detail values hide it.

One account-bound provider above the router owns discovery and streams. Rendering
a header indicator, docking a card, pinning, or changing routes cannot create a
second stream. Removing a visible session, unpinning away from its source, or
changing accounts aborts its observer and pending requests. Refreshing receipt
objects does not restart a live stream.
Pinning an already visible, authorized session immediately carries that exact
account/connection/agent observation into the pin query while its receipt refresh
is pending. Same-document navigation keeps the card and stream running through
a slow refresh. This does not promote an unseen pin or another account's data;
a confirmed withdrawn receipt removes it. Each changed pin selection refreshes
immediately, including when that selection has an older empty cache entry or prior error.
The promotion survives retryable outages and is consumed by the first
authoritative refresh result. Successful withdrawal or
an authorization denial clears it permanently until a fresh visible source is
explicitly pinned again; an old successful pin cache cannot revive work after
unpin/repin.

On desktop CMA joins the existing preview-card stack. Machine and CMA cards
share one scrolling column, its saved width, and its resize divider. Each CMA
card aligns left within its container, fills the available width up to 500px,
and uses the machine card's surface, spacing, and shadow with rounded corners;
it does not create another column. Below the `lg` breakpoint, cards use
the machine's inline border and surface and sit above chat in a scrolling area
whose height shrinks when the keyboard opens; the
conversation and composer remain available. **Minimise** retains the title,
task status, stream connection, and latest action. **Maximise** restores the
action list. Streaming continues in both states, and connection errors and
reconnect remain available.
The header stays outside the card's internal scrolling body. Status, recovery
messages, Reconnect, and activity share that body, so touch scrolling and keyboard
focus can reach recovery even at a short keyboard viewport or while minimised.
The **Session details** button expands labelled connection and session-ID
fields and a detailed last-activity timestamp. Its chevron follows the expanded state, and opening the disclosure
scrolls those fields into view within the card without scrolling the chat.

## Verified origin and ownership

The compiled CMA async-operation adapter supplies authenticated receipts through
the existing MCP admission/dispatch path. Admission captures the exact default
personal connection identity. After successful submission or an authenticated
status observation establishes the CMA agent ID, NyxID records an
`assistant_cma_session_origins` row with the owner, service/connection, remote
agent, original conversation, turn, specialist, and validated callback. Creation
and response operations require an observed agent ID; their request arguments
cannot supply one. Agent-input receipts bind the successfully called resource.
The earliest verified invocation retains its origin when later turns or chats
reuse the same remote session, including when observations arrive out of order.
Attribution outlives the async watch's retention and is removed with the source
conversation or personal-account erasure. Persistence uses the existing DB-only
transaction runner, the personal erasure admission fence, and a write to the
same source conversation row that deletion removes. Held observations cannot
recreate a deleted source or write after erasure acceptance. Current-turn guest
state does not revoke a previously owner-admitted invocation.
Origin writes and the post-submission metadata read are optional, bounded, and
isolated from provider execution/delivery. A later authenticated observation can
repair a failed write; no metadata failure redispatches the provider request.
Diagnostics are static and throttled, without provider bodies or credential data.

Only the exact personal default connection is supported by this observer.
Shared-agent, group, guest, scheduled-invocation, organization-owned,
node-routed (including catalog node bindings), platform-key, target-specific,
and agent-credential-override paths
remain outside automatic session discovery. Thread operation receipts use the
assets preview area and follow the dedicated card layout, minimise control and
metadata disclosure independently of automatic session discovery. Unconfirmed
submissions show recorded status without a live stream, actions or pin.
Matching live cards retain per-request status, notices and metadata together
inside one **Submission details** disclosure. Request rows have no nested
disclosures. **Session details** continues to hold shared agent information.
Their single header execution icon opens a panel focused on the newest pending
verified request, or
the newest verified request if none is pending. It uses the verified local Agent
reference, never the remote session ID. Request rows and unmatched submission
history initially show three items; older items are revealed three at a time.
Live sessions precede submission history in the shared assets area.
Legacy receipts without the new server identity remain
unattributed; no migration guesses their origin. Unrelated sessions created
outside NyxID do not appear automatically.

The read-only `GET /api/v1/assistant/nyxagent/cma-sessions` projection accepts a
`conversation`, up to 50 comma-separated `pins` (`<connection UUID>:<agt_id>`),
and an optional `after` cursor. These are selectors, never authority. Each
50-row page is owner-filtered and rechecks private-history access, active personal
connection ownership, destination, credential identity/epoch, and routing
configuration (credential binding, stored headers, identity propagation, auth,
token exchange, and node configuration); binding lookups are reused within the
request. Labels and lazy OAuth refresh do not change identity. It returns only
minimal link metadata, with no credential or operation-result material. A
changed credential/destination cannot inherit an earlier attribution. A missing
or inaccessible source is excluded even for a persisted pin.

`discover=true` returns at most 50 eligible personal connections per page,
with their real endpoint labels and slugs. Connection discovery uses indexed
owner/service/source existence checks, never walks historical agents, and has
bounded database deadlines. `agents` intersects up to 50 exact connection/agent
candidates with the source/pin scope. The browser requests CMA's supported
single-status filters (`starting`, `working`, `waiting_approval`, `paused`)
separately, follows advancing active-inventory cursors, and joins each page to
these receipts **before** applying the 50-card limit. Terminal history and
unattributed active inventory cannot consume card slots. Pins use a separate
exact receipt query, ahead of the final limit, independent of inventory order
or inventory outages. Inventory is only a candidate source, never origin evidence.

Discovery polls every 15 seconds with a 30-second outer request deadline and a
25-second collection budget. It follows at most 100 connection pages and 100
pages per active status/connection, rejecting repeated cursors. Candidate pages
are joined before counting toward the 50-card limit. At most three connections
are probed concurrently, with a ten-second deadline per source connection.
A collection deadline returns completed peer observations and preserves known
denials, rather than discarding the entire poll. Ephemeral account/source-scoped
scheduling resumes at the next unvisited eligible connection/page on the next
poll; slow early connections cannot permanently starve later connections.
Previous source observations have their exact receipts refreshed independently
of the current inventory position. No historical terminal inventory is scanned.
A denied or unsupported connection clears only its own observations; healthy
connections continue updating. Retryable route/inventory failures retain prior
observations for that same
source/account/connection and newly completed candidates already joined to valid
receipts. A later denial clears all results from its connection, including
earlier pages; a successful complete inventory withdraws absent work. Pin route failures
are isolated using independently refreshed exact pin receipts. Their route budget
is shared across all selected connections within the collection budget, including
time spent reading receipts, so later pins also get a probe. Pins do not depend
on active inventory succeeding. Caller cancellation and account changes abort
the collection; backend session 401 handling remains authoritative.
There is no source discovery without a selected source conversation, and no pin
query without pins. Up to 100 authorized source/pin candidates receive detail checks: active
candidates every five seconds and confirmed inactive pins every 15 seconds.
The final 50-card limit follows active-detail filtering, so inactive pins cannot
consume source-card slots. Only rendered cards own streams. Confirmed idle/terminal detail, withdrawn receipt authorization,
and 401/403/404 detail responses remove cards and cancel their observers.
Retryable network, timeout, 429, and 5xx refresh failures retain the last valid
observation for the same account and scope. No placeholder data crosses account
or source query keys. Pins alone never authorize observation.

## Connection contract

The underlying event protocol was previously verified against CMA commit
[`80ab60e1e66bbd47fe4612ac6be5a7bf802e19a6`](https://github.com/ChronoAIProject/cma/tree/80ab60e1e66bbd47fe4612ac6be5a7bf802e19a6),
particularly `docs/API_Agent_Events_V2.md`, `docs/API_REST.md`,
`api/crates/layer1-public/src/agent_v2/{head,shapes,stream}.rs`, and
`api/crates/layer2-core/src/sandbox/agent_io/event.rs`.

Rechecked against CMA commit
[`9b2994c2d08cbcd618435a16891c54c76bec66bc`](https://github.com/ChronoAIProject/cma/tree/9b2994c2d08cbcd618435a16891c54c76bec66bc)
on 2026-11-04. The current public event vocabulary still supplies the status,
turn, tool-title/status, approval, authentication, and title-change fields used
by the card. Read-only calls through the connected CMA tool gateway also
successfully returned a real Agent list, a v2 event head, and a finite v2 SSE
replay (`follow=false`). That replay contained 50 durable events, including
`status_changed`, `turn_started`, and `turn_completed`. The sampled session had
no tool events, so tool-summary fields were verified against current source,
not observed in that live sample.

An earlier read-only browser integration check mounted the card component
against the deployed NyxID REST proxy using the existing CLI credential on the
local test server. It discovered three real active sessions, received successful
detail/head/follow responses, displayed live stream connections, and exercised
minimise/maximise and session details. The harness rejected non-GET requests.
That check verified the earlier component and deployed proxy together; it did
not test web login/cookie authentication or start a new task to exercise live
tool updates. It preceded server-origin receipts and pins. The current changes
are covered by local server and browser fixtures, without a new live CMA task.

The local preview's website-review title and action descriptions are authored
sample fixtures. In the actual observer, task labels are deterministic mappings
of returned CMA statuses/events and action descriptions come from CMA's
`tool_started.title` / `tool_updated.title`, with a generic fallback when an
update has no known title. No NyxID model generates progress descriptions.
Connecting/Live/Reconnecting/Disconnected describe the local stream connection.

Connections come only from verified receipts for the compiled catalog `cma`
adapter. The exact connection's proxied OpenAPI document must advertise these
GET routes:

| Operation | Canonical CMA route |
| --- | --- |
| Read session | `/api/v1/agents/{id}` |
| Capture event head | `/api/v2/agents/{agent_id}/events/head` |
| Follow recorded events | `/api/v2/agents/{agent_id}/events?after_cursor=…&follow=true` |

The route resolver also recognizes the CMA agents-list path in the configured
spec; source discovery calls it with one supported active status per request. When no OpenAPI document is
configured (404/501), it uses canonical routes. A configured document omitting
required routes is unsupported. Detail and event-head reads verify remote
identity. Requests use the existing authenticated REST proxy, current credential
resolution, access checks, and metering.

Configure the CMA service's base URL to the CMA API origin when its OpenAPI
paths include `/api/v1` and `/api/v2`. NyxID appends the advertised path to the
configured base URL. A base URL ending in `/api/v1` would duplicate that prefix
and cannot reach the sibling v2 API. An API origin with a shared deployment
prefix works when the advertised paths are relative to that prefix. All requests
use the discovered NyxID connection.

## Observation and recovery

The observer is independent of NyxBot's active turn and MCP tool-call lifecycle.
Discovery, observation, and reconnect issue GET requests only. They do not start
a CMA task, resend a prompt, or change how results enter the conversation.

On subscription, the browser captures the v2 event head and reads session detail.
If a turn is active, it follows from that turn's start cursor to reconstruct
current activity. Otherwise it starts at the captured head, avoiding old work.
For each durable `cma.agent.event`, it validates the agent ID, incarnation,
sequence, and agreement between the SSE ID and durable cursor. It applies the
event before advancing the reconnect cursor. Duplicate events are ignored;
unknown and withheld (`kind: null`) events still advance the cursor. A partial
frame never advances it.

The card projects public tool titles, tool status, turn outcomes, approval or
sign-in waits, pause, stop, deletion, and title changes. Messages, model text,
reasoning deltas, tool arguments, results, and approval details are not rendered.
Finishing a turn does not mark an unfinished tool as successful; its outcome
becomes unknown. An approval decision alone does not imply resumed work; the
card waits for a subsequent status or turn event. History and live gaps produce
a notice.

Transport state is displayed independently as Connecting, Live, Reconnecting,
or Disconnected. A lost connection retains the last observed task state.
Transient network failures, 429s, and 5xx responses retry the GET follow from
the last applied cursor with a 1–30 second backoff. Initial reads/headers have a
30-second deadline; an idle stream has a 90-second deadline. Authorization
failures, a missing session, incarnation conflicts, and malformed or foreign
events stop automatic observation and expose an explicit reconnect action.
A downstream 401 does not sign the person out of NyxID. Account identity is
checked before requests and before applying responses; removing inactive
sessions, leaving an unpinned source, and account changes abort the subscription.
Pinned subscriptions continue across routes.
Discovery requests also have a 30-second deadline. Route discovery is cached for
five minutes per account and connection.

Cards provide status and action summaries; desktop control and CMA approval
actions remain separate capabilities.

## Implementation and checks

- `backend/src/services/cma_session_origin.rs` and its model: immutable source
  receipts, personal connection identity, bounded owner-authorized projection.
- `backend/src/handlers/cma_sessions.rs`: additive minimal projection DTOs.
- `frontend/src/components/assistant/cma-session-provider.tsx`: one observer
  lifetime across routes, shared with cards and the source-session indicator.
  Route children stay mounted across sign-in, account replacement and logout;
  only the account-scoped observer resets. Observations publish before paint.
- `frontend/src/components/assistant/cma-session-card.tsx`: responsive cards,
  pin/minimise controls, source and external navigation, global floating stack.
- `frontend/src/hooks/use-cma-session.ts`, `frontend/src/lib/assistant/cma-session-api.ts`,
  and `frontend/src/lib/assistant/cma-session-events.ts`: scoped polling, proxy
  requests, recovery, SSE projection.
- `frontend/src/stores/cma-session-store.ts`: bounded account preferences/reset.
- Regression tests cover server receipt ownership and original source retention,
  absent attribution, guest/group exclusion, connection changes, scoped/pinned
  projection, source deletion, source/other-page visibility, reload and unpin,
  specialist deep links, supplied callback validation, account reset, stream
  cancellation, app-login return URLs (including password-MFA to app login),
  and existing replay/cursor behavior.
- `frontend/e2e/cma-session.spec.ts` exercises the real UI against HTTP fixtures,
  including the shared machine/CMA column, mobile keyboard/layout, disclosure
  controls, cross-page pins, slow pin refresh without stream restart, and
  clickable/keyboard-reachable recovery at short viewport heights. Multi-connection
  API regressions distinguish denial from transient route/list failures and
  receipt withdrawal. These fixtures establish no live CMA connection.
- `frontend/e2e/login-redirect.spec.ts` checks app approval and password-MFA to
  app login against HTTP fixtures, preserving the absolute `return_to` and its
  assistant conversation query through profile restoration.

## Catalog domain changes and auto-connected services

NyxID-owned, auto-provisioned HTTP connections follow the current catalog target
at proxy resolution. Reconciliation also updates the stored endpoint and inherited
OpenAPI URL, keeping connection and endpoint IDs unchanged. This applies to all
catalog services, including CMA and cmaeg; it does not rewrite explicit/custom,
user-credential, gateway-provider or node destinations. Reconciliation runs during
normal auto-provisioning (key listing, login and device approval), catalog URL/spec
updates, and startup. Matching proxy snapshots add no database reads. Reconciliation
batches connection, endpoint and catalog reads, and starts transactions only for
mismatches; consistent pages perform no writes. Per-owner repair failures are
logged without failing listing/login/approval. Admin updates return without waiting
for repair: catalog updates and startup schedule bounded background passes with a
five-minute limit. Live resolution already follows the catalog while repair runs.
Repairs emit only identifier/changed-field audit metadata.

NyxBot's cmaeg transport already resolves its target directly from the catalog;
ordinary cmaeg proxy connections now follow catalog moves too. No user needs to
delete or reconnect an auto-connected service.

CMA still checks that the resolved connection targets the current catalog
deployment at admission and immediately before sending. A mismatched explicit
connection returns the existing `cma_configuration_required` conflict with the
connection ID to inspect. Repair does not change credentials, key epochs, resource
IDs, idempotency keys or retained creation fingerprints. A retained creation bound
to the catalog **before** a domain move remains fenced against replay to the new
deployment. Preserve that request and reconcile its outcome; do not create another
request identity to bypass the fence. Endpoint-only repair under an unchanged
catalog leaves valid retained creations recoverable.

During a rolling upgrade, repaired rows work on old replicas too: old provisioning
skips existing connections and does not revert their URLs. Until reconciliation
visits a row, an old replica can still return the original conflict. The existing
CMA safety check remains enforced throughout rollout.

### Creation request encryption and writer cutover

CMA sandbox/agent creation does not require `CONTEXTUAL_RECORD_WRITES=true`.
Retained requests use the shared encrypted-field writer: legacy envelope encryption
with the default setting off, contextual encryption with it on. The existing
optional `request_context_version` on both creation and sandbox rows selects the
reader: absent/null means legacy and `3` means contextual. Unknown or mismatched
formats fail closed; readers never guess from ciphertext or retry another format.
Existing contextual rows remain readable with writes off. No plaintext request is
stored, and IDs, generations, idempotency keys and deployment fingerprints retain
their existing meaning. Same-key pending retries preserve the saved ciphertext;
only a proven-not-created retry replaces it and its marker together.

During a mixed deployment, an old replica refuses a new legacy-format sandbox
replay before any send: its marker check returns `cma_creation_recovery_unavailable`,
and its contextual decoder cannot decrypt legacy ciphertext. This leaves the
retained row and occupied slot intact for an upgraded replica. Receipt-only
reconciliation remains valid; it clears recovery material only on the existing
authoritative terminal evidence. Rolling back restores the creation/replay outage,
not a plaintext fallback or automatic deletion. Account purge still deletes both
owner-scoped record collections, while chat/watch cleanup does not discard an
unresolved creation.

This release does not enable writer cutover or disable legacy reads. The CMA
creation collections (`assistant_cma_sandboxes`, `assistant_cma_creations`) are in
the operator rewrap inventory with a dedicated CMA request adapter (see
[the encrypted field inventory](../security/ENCRYPTED_FIELD_INVENTORY.md)); retain
legacy reads until legacy pending requests have completed/been purged or the rewrap
has converted them. An inventory pass alone does not establish that these CMA rows
have been converted.

### Admin inspection of held account slots

Use a first-party human platform-admin session; this procedure does not require
the owner's optional CMA-management feature. Do not make a new creation attempt
or edit MongoDB to investigate a held reservation.

1. Find `cma_sandbox_slot_occupied` in the audit log. Its metadata identifies the
   owner, reservation, catalog service and holding class. New refusals keep the
   original HTTP 409/code 1004/message bytes; a separate transactional TTL claim
   bounds these audits to one per owner/reservation/hour across upgraded replicas.
   A process-local cache suppresses looping clients before database work; the
   diagnostic never writes a reservation or creation document.
2. List `GET /api/v1/admin/cma-sandbox-reservations`, optionally with `owner_id`,
   admission `states`, and the returned `next_cursor` as `after`. Pages are at
   most 100. Rows include legacy slot holders and their matching creation facts,
   three revision/generation fences and known remote identities. No request
   bytes, ciphertext, idempotency keys or request hashes are exposed.
3. `POST .../{id}/inspect` uses only exact owner-authenticated CMA GETs for saved
   identities. It cannot replay, create, pause, delete or retire anything. Local
   writes are an admin-attributed audit only. No owner state, observation time,
   recovery cooldown or revision changes, including on failed reads. `exists` reports
   the ID and state; `proven_absent` cites durable `confirmed_not_created` evidence;
   `unknown` gives a closed reason. The exact historical refusal subtype is not
   retained in legacy rows and is therefore not claimed. `owner_retry_will_recover`
   means the original dispatched Sandbox creation is still inside the 23-hour
   window: the owner's next allocating create or original-request retry attempts
   recovery automatically. No admin replay is needed, and an unavailable CMA
   still leaves the slot held.
4. If the row already has its live NyxID sandbox, the refusal is correct: create
   Agents in that sandbox's workspace, or the owner retires it using the existing
   owner route. The admin resolver will not free a live associated slot.
5. For an unassociated row, `POST .../{id}/resolve` with `action: "associate"`,
   the exact `remote_sandbox_id`, and the inspection's `generation`, `revision`
   and `creation_revision` freshly checks CMA ownership/existence and binds it.
   A saved Sandbox receipt or exact operation/Agent lineage must tie that resource
   to this creation. Ownership alone is insufficient; `cma_creation_provenance_required`
   refuses an arbitrary same-owner sandbox. For `action: "release"`, those
   same fences and already durable non-creation proof are required; no evidence
   text from the caller is trusted. The existing no-resource transition clears
   recovery material atomically. Stale state requires another inspection; the
   personal-owner erasure fence applies to every resolution state change.

Inspection writes only `admin_cma_sandbox_reservation_inspected` under the admin
actor; resolution couples `admin_cma_sandbox_reservation_resolved` to its state
change transaction. The direct GET boundary uses owner authorization but bypasses
proxy metering, billing, service-request activity and owner proxy rate limiting.
Refused resolution attempts also produce a best-effort metadata-only audit.
These operations do not change owner admission/replay/recovery behavior.

An unknown creation with no remote receipt may remain unresolvable through GETs.
The generic CMA command cache lasts 24 hours; NyxID bounds replay to 23 hours and
admin inspection never replays. A 404, list omission, timeout, failed lifecycle or
expired cache proves no absence. CMA would need an owner-authenticated lookup of
the exact creation identity (owner, deployment, operation, original idempotency
identity and request fingerprint) returning either the durable operation/resource
receipt or authoritative never-created/no-pending-work evidence. Such negative
evidence must fence late or concurrent admission under the same identity; an
expired cache lookup alone is insufficient. Until that contract exists, record
an explicit CMA protocol blocker and keep the slot occupied. See [API](../API.md#admin-cma-sandbox-reservations).

The read contract was checked against CMA commit
`259c26cf5be0837f0f8ee4d400380e7608fb0301`: `api/crates/layer1-public/src/rest/idempotency.rs`
(retention and durable-ledger distinction), `agents.rs` (lifecycle reads by ID),
and `sandboxes.rs` (Sandbox detail/list and mutation routes).

### Sandbox trust confirmation and stranded requests

CMA's exact `409 trust_confirmation_required` problem for `POST /api/v1/sandboxes`
is proof that no sandbox was admitted. NyxID releases that creation's account slot
and clears its encrypted request in the same transaction, with the existing
receipt and generation fences. The response and `commands_digest` reach the
caller unchanged. A problem with a resource/operation identifier, wrong type,
status or code is not that proof. Workspace trust refusals never free the existing
parent sandbox, and lifecycle Agent worker failures remain uncertain.

CMA records this refusal in its idempotency cache. Repeating the original bytes
and key returns the same refusal; **adding confirmation requires a new
Idempotency-Key**. Review the indicated commands, then submit `trust_confirmed:true`
and the exact `commands_digest` using a fresh key. A changed-body retry under the
old key returns `cma_creation_authority_changed` with this instruction. NyxID does
not silently replace upstream keys. This contract was checked against CMA commit
[`68f26f17fd18a701958a27ea5b2b933227fa2fbf`](https://github.com/ChronoAIProject/cma/tree/68f26f17fd18a701958a27ea5b2b933227fa2fbf),
`api/crates/layer1-public/src/rest/{idempotency,instances,sandboxes}.rs` and
`api/crates/layer2-core/src/sandbox/service_creation.rs`.

An affected account needs no deletion or reconnection. A new slot-allocating CMA
creation first re-observes its one occupied unknown sandbox request, using the
retained original bytes and key through the existing guarded replay. An explicit
recovery does the same. Automatic proxy-admission recovery does not require the
optional NyxBot CMA sandbox-management feature. Explicit management recovery still
requires that feature at entry and immediately before replay, plus the existing
first-party-human handler check. A proven refusal settles recovery as complete and permits
the new creation; a real receipt preserves the original sandbox and its slot.
Ambiguous replies, changed authority, unavailable dependencies or identifier-bearing
problems never release capacity. No pending, never-dispatched request or unknown
lifecycle Agent request is automatically resent.

All original owner, catalog deployment, request, parent generation and erasure
fences apply. Generic sandbox replay is limited to 23 hours from first dispatch,
inside CMA's 24-hour idempotency retention. Explicit recovery outside that window
returns `cma_creation_replay_expired` without sending. During automatic recovery,
lookup or replay failures after finding the occupied slot instead return
`cma_sandbox_account_slot_occupied`, explaining that the earlier creation is not
confirmed absent and can be rechecked later. The new request is not dispatched.
The initial slot lookup's database errors and the incoming request's own validation
and authorization errors retain their usual responses. Such older uncertain requests require
authoritative CMA investigation, not a new identity or a local force-unlock.
A rolling old replica may still leave a refusal unknown; an upgraded replica can
recover it within that window. No schema or writer-cutover change is required.


## Automatic reservation reconciliation

A dedicated NyxID worker observes held NyxID-created reservations without owner
activity or the optional management feature. It walks at most 16 rows per page
using the `source, occupies_account_slot, _id` index, a renewable cluster lease
and durable cursor. Pages run on a 30-second tick; a completed pass rests five
minutes. Each row has a 15-second budget, and failures do not stop later rows.
No ordinary proxy, MCP or turn request gains a read. The worker makes exact
owner-authorized GETs only, without proxy metering, billing or request-rate use.

Saved lifecycle/Agent/sandbox receipts use the existing deployment and identity
checks to complete association and creation journaling. Confirmed journals and
ordinary associated sandboxes cause no remote read. A retiring bound sandbox
settles only when CMA returns `retired` for that exact identity. Generation,
parent version, creation revision and erasure fences guard the local transaction;
retirement releases the slot and clears recovery material without deleting any
remote object. Admins may perform the same exact observation with
`confirm_retirement`; inspect itself stays audit-only. A 404, list omission,
failed operation, timeout or elapsed time never frees a slot.

System-attributed `cma_sandbox_reservation_recovered` records settlement (atomic
with retirement; best effort after the existing multistep receipt recovery).
`cma_sandbox_reservation_recovery_deferred` reports fixed reason codes at most
once per reservation/generation/reason per hour via an existing coordination
claim. Neither event retains provider bodies, request material or credentials.
Old replicas can continue reading and writing all rows: no new persisted enum
value or sandbox/creation field is introduced. Repeated observation and races
with owner recovery are safe under the existing fences.

`operation_without_resource` is not a terminal non-creation result. CMA may
have a sandbox internally while exposing no Agent in the public operation.
Queued/running operations keep being observed; `waiting_for_connection` needs
the owner to complete CMA's connection and retry the original request; a failed
operation with no public resource stays held pending authoritative evidence.
The inspector reports these distinctions and guidance. Checked against CMA
`71dfed36f8806dcc68ba4ca1d73d6688e5935c9e`,
`api/crates/cma/src/store/agent_lifecycle_writes.rs` (`record_sandbox`, `fail`,
`resume_connection`) and `agent_lifecycle_operations.rs` (`view`). The missing
creation identity/absence contract is tracked in ChronoAIProject/cma#2354;
NyxID cannot safely force-release these rows.

## Refusal reference

NyxID's own CMA refusals use the standard error envelope
`{"error":"...","error_code":...,"message":"..."}`. Most reasons have no
dedicated numeric code: over HTTP the reason is carried only in `message`, after
a prefix fixed by the error kind. Responses that CMA itself returns and NyxID
forwards (for example `409 trust_confirmation_required`) reach the caller
unchanged and are not listed here.

| Kind | HTTP | `error_code` | `error` | HTTP `message` |
| --- | --- | --- | --- | --- |
| Bad request | 400 | 1000 | `bad_request` | `Bad request: <reason>` |
| Validation | 400 | 1008 | `validation_error` | `Validation error: <reason>` |
| Forbidden | 403 | 1002 | `forbidden` | `Forbidden: <reason>` |
| Not found | 404 | 1003 | `not_found` | `Not found: <reason>` |
| Conflict | 409 | 1004 | `conflict` | `Conflict: <reason>` |

Native `nyxid__` CMA tools return error tool results with the same `error` and
`error_code`. Their `message` is the reason without a prefix (at most 300
characters) for Validation and Forbidden, `Resource not found.` for Not found,
`The resource changed. Inspect it and request a new acknowledgement.` for
Conflict, and `The account operation could not be completed. Review it in the
NyxID UI.` for Bad request.

The HTTP message `Conflict: cma_sandbox_account_slot_occupied` is byte-pinned:
slot diagnostics never change its status, code or message. The reasons below are
exact current text; `{login_url}` is the only variable part.

**CMA gateway outcomes** (any CMA read or command):

| Kind | Reason | When |
| --- | --- | --- |
| Conflict | `cma_observation_unavailable` | CMA timed out, was unavailable, answered 401/403, or returned an invalid or oversized (over 2 MiB) response. |
| Conflict | `cma_configuration_required` | The owner's resolved CMA catalog service, base URL or execution fingerprint no longer matches, or the identity assertion cannot be attached. |
| Conflict | `cma_inventory_incomplete` | CMA's resource inventory is incomplete. |
| Conflict | `CMA resource state changed; refresh before retrying` | CMA answered 409 with any other code. |
| Conflict | `sandbox_credential_setup_required: Sign in once at {login_url}; keep the original creation request for recovery` | CMA answered `409 sandbox_credential_setup_required`; `{login_url}` is the CMA deployment's `/auth/login` URL. |
| Conflict | `desktop_unavailable`, `desktop_busy`, `desktop_runtime_unavailable` | CMA refused the desktop outside the descriptor read; the descriptor read instead returns `available: false` with the same `reason`. |
| Not found | `CMA resource not found` | CMA answered 404. |
| Forbidden | `CMA resource access denied` | CMA reported a different owner, or the native route is not permitted. |
| Validation | `Invalid CMA request` | Invalid identifier, cursor or `Idempotency-Key` (1–200 visible ASCII characters with no spaces, excluding `"` and `\`). |
| Validation | `Direct proxy destination is not permitted` | The CMA destination fails the proxy destination policy. |

**Proxy admission and creation** (catalog CMA requests from MCP or chat keys):

| Kind | Reason | When |
| --- | --- | --- |
| Bad request | `Invalid CMA execution request` | Malformed managed request, for example both a Sandbox and a Workspace parent, a query on a change, or a creation reply over 2 MiB. |
| Bad request | `Omit machine_included when selecting an existing parent` | Agent creation names a parent and `machine_included`. |
| Bad request | `CMA tasks require stream=true` | Task submission without `stream: true`. |
| Bad request | `CMA changes require a stable Idempotency-Key` | A change other than a file write has no `Idempotency-Key`. |
| Validation | `Invalid CMA creation request` | The creation body is not a bounded JSON object or contains credential-shaped keys. |
| Forbidden | `cma_operation_not_supported: use the published personal Agent and Sandbox operations` | The operation is not a published personal Agent or Sandbox operation. |
| Conflict | `cma_sandbox_account_slot_occupied` | The account's one NyxID-created Sandbox allocation is held (byte-pinned). |
| Conflict | `cma_sandbox_account_slot_occupied: the earlier creation is not confirmed absent; retry later to recheck its original request; do not delete or reconnect` | Automatic recovery of the held allocation could not prove the earlier creation absent; the new request is not dispatched. |
| Conflict | `cma_nyxid_sandbox_required` | The selected parent is not this account's NyxID-created Sandbox. |
| Conflict | `cma_sandbox_retiring: new Agents and tasks are closed` | The parent Sandbox is retiring or retired. |
| Conflict | `cma_creation_authority_changed` | A retried creation's owner, catalog deployment, request or parent changed. |
| Conflict | `cma_creation_authority_changed: the original request was confirmed not created; submit the corrected request with a new Idempotency-Key (CMA binds the original key to its original body)` | A changed body reuses the key of a Sandbox creation confirmed not created. |
| Conflict | `cma_creation_already_completed: query the original resource` | The original creation is confirmed. |
| Conflict | `cma_creation_observation_pending: query the original resource` | The original creation already has a resource identity. |
| Conflict | `cma_creation_replay_expired: retain the original request and confirm its result without resubmitting` | Replay is outside the 23-hour window from first dispatch. |
| Conflict | `cma_creation_result_unknown` | A dispatched creation's result cannot be confirmed (30-second read limit, undecodable success body, or no saved remote ID). |
| Conflict | `cma_creation_idempotency_conflict` | The same key was used with a different service or request. |
| Conflict | `cma_creation_recovery_unavailable` | No retained creation record or request material exists for recovery. |
| Conflict | `cma_creation_absence_proof_required: CMA has not provided authoritative proof that this creation was never made` | A release lacks durable `confirmed_not_created` evidence. |
| Conflict | `cma_retirement_idempotency_conflict` | A retirement sent through the CMA proxy uses a different `Idempotency-Key` from the one recorded for the Sandbox's pending retirement. The management route does not refuse this case: it retries a pending retirement with the recorded key. |

**Management routes and Agent reads**:

| Kind | Reason | When |
| --- | --- | --- |
| Not found | `CMA Sandbox access is not enabled` | `assistant:cma-sandboxes` is off for the owner. |
| Forbidden | `A personal account is required` | The owner is not an active person. |
| Not found | `Sandbox not found`, `Sandbox was not created through NyxID` | No NyxID-created Sandbox matches. |
| Not found | `CMA Agent not found`, `CMA Agent parent not found` | No owned Agent or parent reference matches. |
| Validation | `One Idempotency-Key is required` | A `pause` or `revive` command, or a retirement, has no, or more than one, `Idempotency-Key` (`recover_creation` takes none). |
| Validation | `Sandbox label must contain 1–120 bytes without control characters` | After trimming, a rename's label is empty, longer than 120 bytes, or contains a control character. |
| Conflict | `cma_retirement_recovery_unavailable` | A retirement is pending without a recorded `Idempotency-Key`, so neither the management route nor the CMA proxy can retry it. |
| Conflict | `cma_sandbox_retired` | A `pause` or `revive`, or a retirement of a Sandbox not yet marked retired, targets a Sandbox that no longer occupies the account slot. Retiring an already-retired Sandbox instead returns HTTP 200 with `pending: false`; `recover_creation` uses a separate path. |
| Conflict | `cma_sandbox_retiring` | `pause` or `revive` is sent while the Sandbox's retirement is pending. |
| Validation | `Invalid CMA event cursor` | Stream cursor and `Last-Event-ID` differ, repeat, or are invalid. |
| Conflict | `cma_sandbox_generation_changed` | The request's generation is stale. |
| Conflict | `cma_deployment_changed` | The CMA catalog service changed since the reference was created. |
| Conflict | `cma_agent_parent_unavailable`, `cma_agent_registration_changed` | The Agent's parent or registration cannot be bound. |
| Conflict | `cma_agent_observation_stale` | CMA reports an older Agent revision than NyxID last observed. |
| Forbidden | `CMA Agent identity changed`, `CMA Agent owner mismatch`, `CMA Agent authority changed`, `CMA Response identity changed` | Live CMA identity no longer matches the stored reference. |
| Forbidden | `CMA desktop is unavailable` | A desktop stream was requested for an Agent without an available desktop. |
| Forbidden | `A current human session is required`, `Human session ended`, `Human session verification changed`, `CMA observation authority unavailable` | Stream revalidation of the human session or Agent authority failed. |
| Forbidden | `Desktop authentication or origin refused` | The desktop WebSocket lacks a verified session or allowed origin. |

**Native tools**:

| Kind | Reason | When |
| --- | --- | --- |
| Forbidden | `CMA requires this owner's enabled feature and current CMA grant` | The chat is not eligible for CMA tools. |
| Validation | `Invalid CMA tool arguments` | Arguments are missing or malformed. |
| Validation | `Use an attachment of at most 32 MiB from this conversation` | `nyxid__cma_save_attachment` names an upload over 32 MiB or from another conversation. An upload outside the current chat's scope is refused earlier as not found. |
| Conflict | `CMA action access mode changed; confirm again` | The access mode changed after a non-read action was confirmed. |
| Conflict | `cma_resource_page_too_large` | A `nyxid__list_cma_resources` page exceeds 64 KiB. |

**Admin reservation routes and operator migration**:

| Kind | Reason | When |
| --- | --- | --- |
| Not found | `CMA reservation not found` | No reservation matches. |
| Validation | `Invalid reservation cursor or owner`, `Invalid reservation states`, `remote_sandbox_id is required` | Invalid list filters, or `associate` without `remote_sandbox_id`. |
| Conflict | `CMA reservation changed; inspect it again before resolving` | Generation, revision or creation revision fences changed. |
| Conflict | `Account erasure is in progress for this owner; inspection is unavailable` | The owner's account erasure has been accepted. |
| Conflict | `cma_sandbox_live: this account already has its NyxID sandbox; create agents in its workspace or ask the owner to retire it` | The row is bound to a live Sandbox. |
| Conflict | `cma_sandbox_retirement_proof_required: CMA has not confirmed retirement of this exact sandbox` | `confirm_retirement` without CMA `retired` for that identity. |
| Conflict | `cma_creation_record_missing: no exact creation evidence is available; the reservation cannot be resolved` | No creation record exists. |
| Conflict | `cma_creation_provenance_required: no saved receipt or CMA lineage ties a sandbox to this creation; ownership alone cannot establish NyxID creation provenance` | `associate` without receipt or lineage. |
| Conflict | `cma_sandbox_retired: a retired resource cannot resolve an uncertain creation` | `associate` names a retired Sandbox. |
| Conflict | `Contextual writer cutover is required before migration` | The operator rewrap walk reached CMA request rows before contextual writes were enabled. |
