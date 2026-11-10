# CMA session status cards

Active CMA sessions appear automatically **only in the NyxID conversation that
originated the verified CMA receipt**. A CMA glyph and live activity dot in that
conversation's header identify current CMA work. Unrelated chats and account
inventory never establish attribution.

Each compact card leads with the CMA glyph and actual task title, followed by
one line for task and stream status. A tight shared list shows the three newest
public actions; **Show all N actions** expands the bounded 30-action history.
Source chat, supplied CMA link, pin, and minimise controls share the header.
Each icon has a state-aware tooltip available on hover and keyboard focus.
The compact action list and Session details use spacing without internal
horizontal dividers.
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
Matching live cards retain request notices and the feature-gated execution
panel from their consolidated receipts. The execution panel uses the verified
local Agent reference, never the remote session ID. Legacy receipts without the new server identity remain
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
- `frontend/src/hooks/use-cma-session.ts`, `cma-session-api.ts`, and
  `cma-session-events.ts`: scoped polling, proxy requests, recovery, SSE projection.
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
