# Automatic CMA session cards (v1)

Running CMA sessions automatically appear as cards on the right of the assistant.
There is no pin, session picker, or toolbar toggle. The cards show the session
title, current task status, stream connection, and up to 30 public action
summaries, newest first. Concurrent sessions have separate cards.
Each card is labelled **Your CMA session · Across chats** to distinguish the
account overview from activity belonging to the currently open conversation.

The browser discovers CMA connections on entry and every minute, then checks
their session lists every five seconds. Sessions in `starting`, `working`,
`waiting_approval`, or `paused` appear automatically; idle and terminal sessions
are excluded. When a successful list refresh confirms a session is no longer
active, its card disappears and the stream is aborted. A streamed completion
can appear before that refresh. Failed refreshes retain the last known list;
they do not imply that work has finished. Each scan follows up to 100 pages of
50 sessions, deduplicates agent IDs, and rejects repeated cursors.

On desktop the bounded, rounded cards use the machine desktop card's border,
surface, spacing, and minimise control. Below the `lg` breakpoint, they sit above
chat in a scrolling area whose height shrinks when the keyboard opens; the
conversation and composer remain available. **Minimise** retains the title,
task status, stream connection, and latest action. **Maximise** restores the
action list. Streaming continues in both states, and connection errors and
reconnect remain available.
The **Session details** button expands labelled connection and session-ID
fields. Its chevron follows the expanded state, and opening the disclosure
scrolls those fields into view within the card without scrolling the chat.

Discovery follows the authenticated person across chats and assistant screens.
Reload discovers the current sessions again. Only account-bound minimise
preferences are persisted; logout and account changes clear them. The previous
manual pin is discarded during the version-2 storage migration. Session titles,
activity events, messages, reasoning, and credentials are not stored by this
feature.

## Scope and task attribution

This is an account overview within the assistant, available across its chats
and screens. It is independent of which specialist is currently selected or
whether NyxBot is taking a turn. CMA's default `GET /api/v1/agents` inventory
is personal to the credential used by that connection. This v1 does not request
organization inventories or incoming shared-resource inventories, and does not
provide a platform-wide view. A specialist using another CMA credential can
have sessions outside this overview.

A CMA Agent/session is also separate from a NyxID specialist. Session title
and status do not identify the NyxID task that caused the work. CMA's detail,
list, and creation-provenance contracts do not supply a NyxID conversation and
turn link; its creation actor alone cannot establish the current task.
NyxID's browser activity DTO currently carries tool labels and optional machine
receipts, without a CMA session ID. Consequently the cards do not display a
specialist name or claim ownership by the open task.

To provide task attribution, the server needs to record a verified CMA session
reference alongside the invocation's conversation, turn, and specialist. The
MCP execution path already has this trusted NyxID context, before its downstream
result is reduced to an activity label and digest. It must capture a structured
CMA receipt there and project the minimal linkage to the owner. Reusing a CMA
session for another task requires an invocation/turn link, not a permanent
assignment of that session to its original specialist. This linkage is not
implemented in this v1; browser inventory discovery cannot reconstruct it.

## Connection contract

The implementation was verified against CMA commit
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

A read-only browser integration check also mounted the actual card component
against the deployed NyxID REST proxy using the existing CLI credential on the
local test server. It discovered three real active sessions, received successful
detail/head/follow responses, displayed live stream connections, and exercised
minimise/maximise and session details. The harness rejected non-GET requests.
This verifies the component and deployed proxy together; it does not test web
login/cookie authentication or start a new task to exercise live tool updates.

The local preview's website-review title and action descriptions are authored
sample fixtures. In the actual observer, task labels are deterministic mappings
of returned CMA statuses/events and action descriptions come from CMA's
`tool_started.title` / `tool_updated.title`, with a generic fallback when an
update has no known title. No NyxID model generates progress descriptions.
Connecting/Live/Reconnecting/Disconnected describe the local stream connection.

Connections come from NyxID's paged `GET /api/v1/proxy/services` discovery.
Only services whose name or slug includes a distinct CMA token are considered
(for example `cma`, `cma-staging`, or `cma_staging`). They must be connected or
require no connection. This includes custom CMA services and avoids probing
unrelated services for agent endpoints. Each connection's proxied OpenAPI
document must advertise the following GET routes:

| Operation | Canonical CMA route |
| --- | --- |
| List sessions | `/api/v1/agents?limit=50&cursor=…` |
| Read session | `/api/v1/agents/{id}` |
| Capture event head | `/api/v2/agents/{agent_id}/events/head` |
| Follow recorded events | `/api/v2/agents/{agent_id}/events?after_cursor=…&follow=true` |

When no OpenAPI document is configured (404/501), the observer uses the canonical
routes. A configured document that omits the required routes is unsupported.
Each observer verifies session detail and event-head identity. Both session
discovery and the SSE follow use the existing authenticated NyxID REST proxy,
with its current credential resolution, access checks, and metering.

Configure the CMA service's base URL to the CMA API origin when its OpenAPI
paths include `/api/v1` and `/api/v2`. NyxID appends the advertised path to the
configured base URL. A base URL ending in `/api/v1` would duplicate that prefix
and cannot reach the sibling v2 API. An API origin with a shared deployment
prefix works when the advertised paths are relative to that prefix. All requests
use the discovered NyxID connection.

## Observation and recovery

The observer is independent of NyxBot's active turn and MCP tool-call lifecycle.
Discovery, observation, and reconnect issue GET requests only. They do not start
a CMA task, resend the NyxBot prompt, or change how tool results enter NyxBot's
conversation. The current NyxBot activity contract carries tool labels without
CMA session IDs, so discovery includes active sessions from the default personal
inventories returned through the person's CMA connections. It does not claim
that a session was created by the currently open NyxBot conversation.

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
sessions, leaving the assistant, and account changes abort the subscription.
Discovery requests also have a 30-second deadline. Route discovery is cached for
five minutes per account and connection.

This v1 provides status and action summaries. Desktop viewing/control, approval
actions, and automatic binding to a CMA invocation are subsequent capabilities.

## Implementation and checks

- `frontend/src/components/assistant/cma-session-card.tsx`: automatic cards,
  minimise/maximise controls, responsive presentation.
- `frontend/src/hooks/use-cma-session.ts`: scoped queries and observer lifetime.
- `frontend/src/lib/assistant/cma-session-api.ts` and `cma-session-events.ts`:
  proxy requests, durable cursor recovery, bounded SSE decoding and projection.
- `frontend/src/stores/cma-session-store.ts`: account-bound minimise preferences
  and migration away from manual pins.
- CMA unit tests cover fragmented UTF-8/SSE, replay and cursor recovery,
  duplicates, withheld/additive events, foreign-session rejection, 401 handling,
  account transitions, history bounds, incomplete tool outcomes, connection
  filtering, active-session discovery, pagination, and preference migration.
- `frontend/e2e/cma-session.spec.ts` runs the real browser UI against HTTP
  fixtures, covering automatic appearance, sessions starting after chat opens,
  later streamed updates while minimised, reload, completion cleanup, concurrent
  cards, long activity lists, details disclosure visibility and keyboard
  operation, and chat use on mobile with the keyboard open.
  These fixtures do not establish connectivity to a deployed CMA instance.
