# NyxBot Notifications

Notifications is the acting person's durable feed for work that happens outside
the conversation they are watching. It appears behind the default-off
`assistant:notifications` flag in the NyxAgent assistant on web and mobile.
The mobile approvals **Activity** screen keeps its existing purpose.

## Audience and kinds

Each row belongs to a person, never a guest or an organization. An organization
specialist's private work belongs to the acting member. An organization channel
route belongs to its linking administrator. Source access is checked again when
reading previews or sending push; leaving a group or losing organization access
must not expose the retained source through the feed.

| Kind | Authoritative source | Identity |
| --- | --- | --- |
| `automation_result` | A trigger run completes, including result delivery | One row per run |
| `automation_problem` | A run fails, an actionable admission wait expires, result delivery fails or becomes unknown, or a trigger auto-pauses | One row per run or existing pause notice |
| `needs_decision` | An assistant acknowledgement is pending | One row per card |
| `agent_update` | Background agent/event replies, specialist reports and public group replies settle | One unread coalesced row per visible source |
| `chat_update` | A channel bot's reply settles in a private chat, group or topic | One unread row per channel conversation |
| `group_mention` | A collaboration message mentions the person | One unread row per group |

An ordinary user-origin web turn produces no notification. A failed background
turn still produces an update, using the saved failure notice for its preview.
Hidden member execution threads route through their visible group transcript;
they do not produce a second private-thread update. Automation turns use the run
notification rather than a second generic agent update. Technical overlap skips,
missed historical windows and the owner's explicit cancellation do not produce
an actionable problem. Reason codes come from the existing run and delivery
states; notifications introduce no replacement failure taxonomy.

For `agent_update`, `chat_update` and `group_mention`, a new event increments
`count`, replaces only the latest source identifiers, and advances `updated_at`
on the existing unread row. Reading that row closes its coalescing window. The
next event creates a new row. When different coalesced kinds share a source,
they increment the same row and its kind follows the latest activity. The
source key contains the visible source type and id, without the kind. A unique
partial index on `(user_id, source_key)`
for `coalesced: true, unread: true` enforces the window across replicas; updates
and increments happen in the source transaction. Run and card rows use UUID v5
identities derived from the source identity, so recovery cannot duplicate them.
Pause rows reuse the scheduler's pause notice identity.

## Stored data and retention

`assistant_notifications` stores UUID string ids, the audience, kind, source
identifiers, count, stable outcome/reason codes, timestamps, read/resolution
state and push state. It stores no message body, title, summary, preview, device
token or credential. Date fields use BSON dates, including optional dates.

`source.type` is closed to `conversation`, `group`, `channel_chat`,
`collaboration_group` and `trigger`. `source.id` identifies the visible source.
A personal channel chat uses its assistant conversation id, with a separate
`channel_id` for the NyxBot channel connection and `thread_id` for its provider
chat. Organization channel sources use that public provider chat's thread id.
Member activity retains the acting member's private `conversation_id`; guest
activity carries no private conversation id and links the route's administrator
to the agent's Channels page. Its preview comes from the sanitized public
transcript. `conversation_id` is also carried on automation sources that have a
saved run thread, so opening the thread can mark all related activity seen.
Optional message, turn, run, card, group, agent and API-key references support
batched preview reads, card resolution and exact-source cleanup.

Personal NyxBot chats through a shared platform entry are also `chat_update`
sources. Their server-authored platform origin distinguishes external chat input
from a web continuation of the saved conversation. They use the conversation id
and a personal binding reference rather than an ordinary NyxBot channel id.
Revoking that personal binding removes its notifications; the saved conversation
remains available for web continuation. An exact old turn may still save its reply
after unlink, but it cannot recreate a notification for the revoked binding.

List previews are computed after checking live source visibility. Each referenced
collection is loaded in one batch. Present sources that are temporarily unavailable
or no longer authorized are hidden without deleting their rows; a repaired channel
can make its retained rows visible again. Previews strip markdown, collapse whitespace
into a single line and contain at most 200 Unicode characters. Titles and links
come from the current authorized source. Missing or deleted sources are purged;
clients never render a broken source entry. Logs and audits never include these
previews.

The TTL index on `expires_at` removes rows 30 days after their last update.
A pending decision's retention extends through its card expiry. Deciding,
expiring or superseding a card resolves its notification, including key
replacement, agent destruction, automation retirement and collaboration
invalidation paths. Reading a decision marks it seen without resolving it.

Source deletion also removes rows in the existing cleanup paths: conversations,
personal/organization/collaboration groups, hidden member threads, agent
destruction and purge, channel retirement and thread deletion, trigger deletion,
and personal erasure. Parent write fences serialize late producers against
retirement and deletion. Notification cleanup uses the source reference rather
than assuming the source owner and recipient are the same person.
Detaching a provider binding retains the logical channel and its readable
histories, so it retains their notification rows. Disconnecting the channel or
deleting its provider chat removes the corresponding rows.

## HTTP contract

All routes are below `/api/v1/assistant/nyxagent/notifications`. They require a
first-party human session or first-party human JWT, the NyxAgent engine flag and
`assistant:notifications`. API keys, conversation keys, delegated tokens,
OAuth-client tokens, relay tokens and service-account tokens are rejected. With
the notification flag off the routes return the same 404-shaped refusal as other
gated assistant routes. Requests always act for the authenticated person.

| Method and suffix | Behavior |
| --- | --- |
| `GET /` | Cursor-paged newest-first list |
| `GET /summary` | Bell counts and source unread markers |
| `POST /read` | Idempotently mark ids, a source or a fenced snapshot seen |

List query parameters are `cursor`, `limit` (1–50), `unread_only` (default false),
`pending_only` (default false) and `kind` (comma-separated subset of the six kinds).
The **Needs you** filter sends `kind=needs_decision&pending_only=true`, which
includes seen pending cards and excludes resolved cards. **All** retains resolved
decisions as history. Ordering is descending
`updated_at`, then `_id`. The response is:

```json
{
  "items": [{
    "id": "uuid",
    "kind": "agent_update",
    "count": 3,
    "source": {
      "type": "conversation",
      "id": "nyxa-thread",
      "label": "Research",
      "conversation_id": "nyxa-thread",
      "group_id": null,
      "channel_id": null,
      "trigger_id": null,
      "agent_id": "uuid"
    },
    "title": "Research",
    "preview": "The report is ready.",
    "url": "https://nyx.example/assistant?c=nyxa-thread",
    "read": false,
    "resolved": false,
    "created_at": "2026-11-10T08:00:00Z",
    "updated_at": "2026-11-10T08:05:00Z"
  }],
  "next_cursor": null,
  "as_of": "2026-11-10T08:06:00Z"
}
```

`as_of` is the server's list-start fence, not a client timestamp. All response
datetimes are RFC3339 UTC strings ending in `Z`. Links are canonical
`AssistantPage` URLs: saved conversations, visible groups, exact review cards or
Automations when a run has no remaining thread.

Summary uses one aggregate with a bounded source breakdown and returns:

```json
{
  "unread_count": 4,
  "needs_decision_count": 1,
  "by_source": [{
    "source_type": "conversation",
    "id": "nyxa-thread",
    "conversation_id": "nyxa-thread",
    "unread": 4
  }],
  "by_source_truncated": false
}
```

Counts include unseen, unresolved activity. A seen pending card remains visible
under **Needs you**, but stops contributing to the badge. Source entries are
bounded to 200; `by_source_truncated` lets clients conservatively refresh or mark
the open source when its entry was omitted. Channel and automation entries carry
`conversation_id` so clients can place their sidebar markers.

Read accepts exactly one selector:

```json
{"ids":["uuid"]}
```

```json
{"source":{"type":"conversation","id":"nyxa-thread"}}
```

```json
{"all":true,"before":"2026-11-10T08:06:00Z"}
```

At most 100 ids may be supplied. A conversation selector also matches channel
and automation rows referencing that conversation. A group selector matches
both personal and collaboration notifications referencing that group. Mark-all affects only rows
whose `updated_at` is at or before the supplied server fence. Activity arriving
after the viewed snapshot remains unread. These operations never decide a card.
Explicit mark-all appends the metadata-only `assistant_notifications_read` audit
with count and scope type. Automatic source reads and individual reads create no
per-row audit noise.

## Web and mobile behavior

A bell in the assistant shell opens Notifications. The badge caps at **9+** and
shows pending decisions distinctly. Filters are **All**, **Needs you**,
**Automations** and **Chats**. Rows show their kind, current source title, bounded
preview, age, unread state and a count chip when activity coalesced. The panel
supports cursor pagination, empty/error states and mark-all with the viewed
`as_of` fence. Opening a row navigates to its canonical source and marks it seen.
An empty page can still have a cursor when all its sources are hidden. Both clients
keep pagination available until that cursor is exhausted, bound automatic traversal
to five empty pages and prevent repeated cursors from causing automatic loops.

On web the panel is URL-driven at `/assistant?panel=notifications`, preserving
Back behavior. Opening a visible thread marks its matching notification rows
seen after a debounce; hidden tabs do not auto-read. Sidebar dots derive from
the summary. Collaboration groups keep their existing transcript unread/mention
badge and skip a duplicate notification dot.
When the bounded summary omits the open source, clients issue one conservative
read per source opening. Exact source observations can re-arm the debounce for
later activity without an unbounded read-and-refetch loop.

Mobile provides a separate Notifications screen with a `FlatList`, pull to
refresh, cursor pagination and the same filters. It refreshes on focus, app
foreground, push receipt and notification live frames. Push taps use the existing
canonical assistant-link router and mark the supplied `notification_id` seen.
Approvals Activity remains separate.

The existing owner live stream emits identifiers only:

```json
{"type":"notification","id":"uuid"}
```

Creation, coalescing and read/resolution changes invalidate list and summary.
`resync` also invalidates them. The MongoDB stream projection excludes content
before events enter application memory. Old clients safely ignore unknown frames.

## Optional push

NyxBot settings adds `notifications.push` with six boolean fields matching the
kind names. All default false; the in-app feed is on whenever the feature flag
is on. Existing users receive no new push until they opt in. Preferences are
additive to `GET`/`PUT /api/v1/assistant/nyxagent/settings` and old stored settings
deserialize with the defaults.

Push is queued only when a notification row is first created, never when a
coalesced count increases. `push.kind` snapshots that first event's kind, so a
later mention or agent reply cannot change which preference authorizes its push.
The push uses that original kind and current source references for its preview.
The worker rechecks preferences, visibility and the
existing FCM/APNs configuration. It permits at most one push per person/source
in ten minutes and at most thirty pushes per person/hour, across replicas.

The row's durable send barrier is:

```text
pending → sending → sent | failed | unknown | skipped
```

A sweep checks flags and preferences, then prepares previews in one batch per
person before claiming any sends. Each row takes its sending claim immediately
before its own provider call, which has a ten-second timeout; unsent rows do not
wait in `sending` behind other deliveries. A row that changes after preparation
returns atomically to its prior pending or retry state, with its reservations
released and no attempt consumed, for preparation in the next sweep. A lookup
failure leaves unclaimed rows due, and one person's failure does not strand
another person's rows.

A claim left in `sending` for ten minutes becomes `unknown` and is never resent.
A conclusive receipt may be settled without sending again. Only an explicit retryable refusal permits a bounded
retry under the same rules as [automation notification delivery](../NYXBOT_SCHEDULES.md#notification-result-delivery).
Retries wait at least sixty seconds, permit at most three attempts and stop
thirty minutes after the first attempt. The claim transaction checks the current
retry deadline and attempt budget before it can authorize another send.
No provider prose or preview enters delivery diagnostics.

The body uses the current source's title and at most 200 preview characters.
Data includes the kind, `notification_id` and canonical `deeplink`. An automation
whose own destination is `deliver_to: notification` suppresses the feed's push,
including its problem result; the run still appears in the feed. Existing pause
notice push must likewise retain only one external delivery.

Telegram notification bots and channel bots are not destinations for this feed.
Owners who want a report in a channel chat select automation `deliver_to: chat`.

## NyxBot digests

`nyxid__list_notifications` is a read-only native NyxBot tool. When the owner asks
what they missed, or an automation asks for a digest, NyxBot calls it. It is
available only with the notification flag on, in the owner's NyxBot threads,
including trigger turns for scheduled digests. Specialists, guests, collaboration
execution threads and external plugin clients cannot use it.

Arguments are optional RFC3339 `since`, a `kinds` subset, `unread_only` (default
true) and `limit` (1–50). Results use the same authorized, bounded metadata,
previews and canonical links as the human API. It never marks notifications seen.
`since` filters the latest update time. A coalesced row's count covers its whole
unread window, whose beginning is returned as `created_at`; a digest must not
describe that count as the number of activities inside the requested time range.
The stable NyxBot instructions include this digest guidance. Their fingerprint
changes once on upgrade, causing the normal upstream session refresh.
MCP annotations declare `readOnlyHint: true`, `destructiveHint: false` and
`openWorldHint: false`; the native action classifier treats it as `read`.

## Rollout and operations

Upgrade every API replica and scheduler worker before enabling
`assistant:notifications`; settings requests reject unknown fields on old
servers. The flag defaults off and resolves for the acting person, including
organization overrides. Existing notifications are not backfilled.

Human handlers reuse the feature snapshot returned by the engine gate. Turn
admission snapshots the notification decision on `ActiveTurn`; settlement,
turn-loss recovery and turn-bound cards reuse it without reading flags again.
Other producers use a bounded process cache with a 30-second lifetime. Flag
changes therefore converge on those producers within that window; admitted turns
retain their snapshot until settlement. The cache contains rollout decisions,
never source content or organization authorization grants. Live visibility is
checked independently of that cache.

The summary uses one aggregate over the person's unread rows, with source
visibility checked before counting. Its input is bounded by retention, while
the returned source breakdown is capped at 200. It scans all retained unread
rows to preserve exact counts after access revocation; it does not truncate the
input and count inaccessible sources or present a partial total as an exact one.

Notification writes participate in the source transaction when one exists.
Database-only transaction retries preserve source settlement and notification
creation together. Post-commit delivery consumes durable row state, so a process
crash cannot require replaying a task merely to send its notification.
