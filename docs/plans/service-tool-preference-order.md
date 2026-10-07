# Per-service agent discovery order (inside the service group)

Branch: `service-tool-preference-order`. Planner: Fable 5.1. Status: Part A
implemented; all required local acceptance gates passed and recorded in §16.
ROOT personally reviewed the final source and closed every substantiated finding.
Fresh published-head CI and Opus 5.5 sign-off are tracked on PR #1796.
Part B is a conditional proposal outside this delivery (§6).

This revision supersedes the global "Reorder" editor approved on `e72036b7`
(PR #1796); that version is at
`git show e72036b7:docs/plans/service-tool-preference-order.md`. Its
validations remain historical evidence for boundaries this revision does not
change (model serialization, CAS mechanics, route rejection layers, test-only
wrappers, the `/keys` detail read bound); every criterion marked *revised*
below needs fresh evidence (§15). Base: `origin/main` `cac8ce77` (#1797 cloud
OAuth providers) merged into branch HEAD `bac66b3f`; the UI baseline is the
#1685 grouped service cards, unchanged by #1797. Nothing from this feature is
deployed: production has no `/api/v1/service-preferences` route, and the
production-backed frontend on port 4630 cannot persist an order until the
backend ships. It has no preference emulation. At the user's explicit request,
ROOT also runs a separate seeded sample API/Vite preview on port 4631, with
temporary JSON persistence under `/tmp/nyxid-service-preference-mock-preview`.
That preview changes neither repository source nor production data. Final
real-route browser artifacts provide fresh layout evidence (§16).

## 1. The user's question, answered from the runtime

A person's Anthropic card holds 30 connections (26 disabled) and 34 agent keys.
They asked how an AI agent learns which tools exist, how it picks among several
connections of one service, and how to set a preferred alternative.

**How agents learn tools (MCP).** `nyx__search_tools` (keyword query, 25
results) and `nyx__list_connected_services` (one row per connection, with
`executable`), then `nyx__call_tool` by tool name. Each connection is its own
service for discovery: tools are named `<connection slug>__<operation>`
(`llm-anthropic__messages_create`, `llm-anthropic-2__messages_create`), so four
enabled Anthropic connections produce four copies of every Anthropic tool with
identical descriptions. Search ranks by `(query words matched desc, words in
the tool name desc, loader order asc)`; identical copies tie and fall to the
**default discovery order**: the caller's personal connections first, then
organization connections in membership order, each newest `created_at` first
(`load_callable_user_services`), with an organization connection dropped when
it shares a slug with a personal one, and with the caller's agent-key service
and node scope applied. Disabled connections are never loaded. **Enabled
connections whose credential is revoked, expired or missing are loaded for
search** (`include_non_executable: true`) and carry `executable: false` in the
listing; search matches today carry only `name`, `description`, `inputSchema`
(and `chat_access` for chat keys). `tools/list` and `/api/v1/mcp/config`
publish only executable operations and are id-sorted in the digest.

**How an agent picks.** By naming a tool or slug; NyxID runs exactly that
connection. There is no fallback between connections of one service unless the
caller addresses a **service pool** slug (`/proxy/s/<pool>` or model
`pool:<slug>`), whose strategy (round-robin, weighted, priority with failover)
is the only deterministic execution routing that exists.

**Where NyxID picks on its own.** Implicit LLM gateway routes
(`/api/v1/llm/{provider}/…` and the OpenAI-compatible gateway resolving a
`model` without `pool:`) call `resolve_proxy_target_from_user_service(slug =
None, catalog_service_id = Some)` → `lookup_user_service` →
`user_service_service::find_by_catalog_service_id`, an **unsorted `find_one`
over active rows** (database natural order), personal owner first, then
organizations in `primary_org_id` order. The person has no control over which
active Anthropic connection that returns. The group UI states this (§5.2); it
is the subject of the Part B question (§6).

**What Part A adds.** A per-service **agent discovery order**, set inside the
expanded service card. It is advisory metadata for discovery: NyxID lists the
preferred connection's tools first when same-service copies tie on relevance,
shows the rank and executability to agents, and explains the rules inline. It
never changes which connection runs a call the agent named, never retries or
reroutes, never changes pools, and (Part A) never changes the implicit gateway
choice.

## 2. Scope decision

| Need | Mechanism | Decided by |
|---|---|---|
| Show agents my preferred Anthropic connection first | Part A: group-relative discovery order (advisory) | the person, per service group |
| Always run Anthropic through X, fall back to Y | existing service pool (`priority`, failover), addressed by the pool slug | the person, via Service Pools (linked from the card) |
| Which connection a named tool or slug uses | exact addressing; unchanged | the agent, by name |
| Which connection `/llm/anthropic` uses | today: natural order; Part B (if confirmed): first connection in the saved order within the same owner tier, selected once, no retry | user answer pending |

Out of scope in all cases: ordering unrelated services relative to each other,
org-level orders, automatic fallback or substitution after a failed call,
changes to `tools/list`, `/mcp/config`, `catalog_digest`, approvals, execution
authority, billing, pools, or named-target resolution. Independent clients with
their own tools cannot be made to honor NyxID's order; the guarantee is the
listing order plus explicit `preference_rank` / `executable` metadata.

## 3. Semantics (normative)

### 3.1 Groups, listability, ranks

- **Group key** `G(c)`: `catalog:<catalog_service_id>` for catalog-backed
  connections, else `connection:<user_service_id>`; identical to the frontend
  `groupServiceConnections` id, derivable server-side from
  `McpToolSource::UserManaged.catalog_service_id` and `KeyView.catalog_service_id`.
  A `connection:` group is an immutable singleton and is never orderable.
- **Stored order**: the existing flat per-identity list
  (`service_preferences.ordered`, `version`, CAS, `MAX_ORDERED_SERVICES = 200`
  account-wide). Position has meaning only relative to other ids of the same
  group.
- **Listed** connection for a caller: loaded by that caller's discovery chain
  (enabled, HTTP, authorized, inside agent-key and node scope). Listed does not
  mean callable: `executable` is the separate flag computed from credential and
  route state, and `/keys` metadata (`classifyConnection`) reports only
  unavailable or unverified. UI text never says callable, working or verified.
- **`preference_rank(c)`**: dense 1-based position of `c` among the listed
  connections of `G(c)` that appear in the stored list, by stored order.
  A catalog group whose caller-visible listed members number one still yields
  rank 1 for that member when it is stored (a scoped agent key sees "#1").
  `null` when `c` is not stored, not listed, or in a `connection:` singleton.
  A disabled stored connection shows `null` and resumes its stored position
  when re-enabled.
- Unranked listed connections follow the ranked ones in default discovery
  order.

### 3.2 Discovery order: slot replacement, no comparator

Both discovery surfaces start from a list whose order is already fixed by
today's code and **only refill the slots a group already occupies**. No sort
key compares connections of different groups.

**Listing** (`nyx__list_connected_services`). Let `L` be the loaded service
list after every existing filter (today's output order). For each group `G`
with two or more members in `L`: `S_G` = the ascending slot indices of `G`'s
members in `L`; `M_G` = `G`'s members ordered by `(preference_rank asc with
null last, position in L asc)`. Write `M_G[i]` into `L[S_G[i]]`. Every other
slot is untouched. With an empty rank map `M_G` equals the original member
sequence, so the output is byte-identical to today.

**Search** (`nyx__search_tools`). The service vector handed to search is the
**original loader order, never the listing-permuted vector**: the rank-load
helper (`order_discovery`) returns `(original services, ranks)` and the two
surfaces permute independently. Pre-sorting services would change
`candidate_index` and therefore which group's tools occupy which slot of a
mixed-relevance bucket (A-low, Slack-high, A-high must not become A-high,
Slack-high). Let `C` be today's relevance-sorted candidate list
`(matched desc, in_name desc, candidate_index asc)` built from the original
order. Partition `C` into contiguous buckets of identical `(matched,
in_name)`. Within one bucket, for each group `G`: `S` = ascending slots of
tools whose connection is in `G`; `T` = those tools re-sequenced by
`(preference_rank of their connection asc, null last, candidate_index asc)`,
so every tool of the rank-1 connection comes first in its original relative
order, then rank 2, then unranked. Write `T[i]` into slot `S[i]`. Slots of
other groups, other buckets and singleton groups never move; a tool never
changes bucket. Truncate to `MAX_SEARCH_RESULTS = 25` **after** refilling, so
a preferred connection's copies survive truncation first. With an empty rank
map the output is byte-identical to today, which is what the `#[cfg(test)]`
wrappers assert.

**Metadata** (additive, from the loaded `McpToolService`, no extra reads):
every listing row and search match carries `preference_rank` (group-relative
or `null`); search matches additionally carry `executable` (listing rows
already do). Tool descriptions, `tools/list`, `/mcp/config` unchanged; the
search `hint` gains one sentence naming the two fields and stating that
relevance comes first and that the field is advisory.

### 3.3 Projection semantics: `/keys` versus MCP

The two surfaces derive ranks from different, equally authoritative
inventories and the plan promises no equivalence between them:

- **REST (`/keys`, `/keys/{id}`)** ranks over that endpoint's authorized
  inventory: the caller's active HTTP user services after service-scope and
  organization-visibility filtering, as `list_keys_read_only_with_grants`
  already computes it. It performs no node, operation-scope, guest or
  provider-readiness evaluation and adds no discovery reads. A catalog group
  whose caller-visible stored member is single still yields rank 1 there.
- **MCP (`nyx__search_tools`, `nyx__list_connected_services`)** ranks over the
  inventory the discovery chain actually loaded for that caller: after node
  scope, operation scope, service allowlist and guest filtering. A restricted
  agent key whose allowlist covers two of four stored Anthropic connections
  therefore sees dense ranks 1 and 2 on them, while the owner's `/keys` pills
  on the same rows may read `#2` and `#4`.
- The UI pills are the owner's REST projection and say so: the explanation
  panel (§5.2 item 4) states that a restricted agent's ranks are dense over
  what that agent is allowed to see.

### 3.4 Stale ids, scope, identity (unchanged from the accepted design)

Ids are `UserService` UUIDs, never slugs. Stale ids (deleted, disabled,
un-shared, outside an agent key's allowlist) are inert, filtered at read time,
never cascaded and never silently deleted by a group save (§4.2). Keyed by
`AuthUser.user_id` / `McpAuthContext.user_id`: person-owned agent keys and chat
keys use the owner's order after their own scope filter; org-owned agent keys
and service accounts have no document and keep default order. The stored list
is never returned raw. Preference is applied after every visibility, scope and
node filter and is not read by `execute_tool`, proxy resolution, approvals,
execution authority, pools, billing or (Part A) the LLM gateway.

## 4. Storage and API

**Assessment.** The existing flat document with CAS stores group-relative
orders unchanged; ranks derive per group at read time. The unreleased global
full-list `PUT` is replaced (ROOT decision) by a group-scoped,
server-authoritative merge, because a full-list client write can move or drop
ids of other groups and the server cannot tell. No new collection, index,
migration or subsystem.

### 4.1 `GET /api/v1/service-preferences` (grouped)

Shared authenticated read class (SA and relay rejected; API keys and delegated
`account:read` allowed). Returns stored ids filtered to the caller's
authorized inventory, grouped, and nothing else:

```json
{ "version": 3, "updated_at": "2026-10-07T09:00:00Z",
  "groups": [ { "group": "catalog:<uuid>", "ordered": ["<user_service_id>", "..."] } ] }
```

Only groups with at least one authorized stored id appear; `connection:`
singleton groups are intentionally omitted from the rendering (they carry no
order) even when their id is stored and visible. A missing document gives
`version: 0`, `updated_at: null`, `groups: []`. No stored counts, hidden
counts or capacity fields are exposed on this read, which API keys and
delegated readers share.

### 4.2 `PUT /api/v1/service-preferences/groups/{group}`

Human-only (delegated, API-key, SA, relay rejected at the route;
`login_client_context::require_first_party_human` in the handler), 16 KiB body,
unknown fields rejected.

Body: `{ "ordered": ["<id>", ...], "expected_version": 3 }`. There is no
`clear` flag: `ordered: []` is the explicit **reset to default** for this
group and removes the group's currently authorized stored ids; a non-empty
`ordered` sets the order.

Validation (400 `ValidationError`): `group` not `catalog:<uuid>` /
`connection:<uuid>`; a `connection:` group; more than 200 ids in the request;
catalog UUID not canonical lowercase RFC variant/version 1–8 (nil/max and unknown versions rejected); non-canonical or non-v4 connection UUID; duplicates; any id not in the caller's
authorized inventory for `group` (one message, "unknown service id", for
foreign, missing and wrong-group ids); `expected_version` outside
`0..=2^53-2`.

Merge (`replace_group`, service layer), with `O` the stored list:

1. `version != expected_version` → 409 `Conflict` (1004).
2. `A` = ids of `O` that resolve in the caller's authorized inventory with
   `G == group`, in stored order (disabled rows are in that inventory; deleted,
   un-shared and out-of-scope rows are not). `H` = every other id of `O`
   (other groups, hidden or stale same-group ids, ids the caller cannot see).
   `H` keeps its elements byte for byte and its relative order; nothing in `H`
   is ever removed by this route.
3. `N`: for a **non-empty** `ordered`, `N` = submitted ids followed by every
   member of `A` the request omitted, in their previous relative order, so a
   draft built from a stale inventory cannot drop a connection that was
   granted or stored meanwhile. For `ordered: []`, `N = []` (intentional
   reset of this group's currently authorized stored ids).
4. Slot replacement: the first `min(|A|, |N|)` elements of `N` are written
   into the slots `A` occupied in `O`, in ascending slot order. Remaining
   elements of `N` (newly stored ids) are inserted immediately after the last
   slot `A` occupied, or appended to the end when `A` is empty. When `N` is
   empty, `A`'s slots are removed. `H` keeps its elements and relative order
   in every case, and an already stored interleaved list resubmitted in its
   current order reproduces `O` exactly.
5. Capacity: if the merged length exceeds `MAX_ORDERED_SERVICES = 200` →
   400 `ValidationError` "Agent order storage is full (200 connections across
   all services). Reset the agent order of another service, or release
   unavailable preferences for services you can no longer access, then try
   again." No ids, counts or probes. A reset (`ordered: []`) never fails on
   capacity because it only removes. **No automatic prune of any kind**: a
   scoped save preserves all unrelated, stale and hidden ids exactly.
6. Merged list identical to `O` → no-op: unchanged metadata, no version bump,
   no audit. Otherwise persist with the existing insert-or-CAS (`version + 1`),
   legacy-row upgrade and BSON-millisecond timestamps.
7. Audit `service_preference_updated` with `{ group, count, version }` only.
8. Respond with the §4.1 shape.

### 4.2a `DELETE /api/v1/service-preferences/hidden` (capacity recovery, accepted)

Human-only with exactly the scoped PUT's fences: delegated, API-key, SA and
relay rejected at the route, `require_first_party_human` in the handler,
16 KiB body with unknown fields rejected, body `{ "expected_version": n }`,
CAS (409 code 1004 on mismatch), identity bound to `AuthUser.user_id`.

Effect: removes from the stored list every id that is **not in the actor's
current authorized inventory** (the same inventory the scoped PUT validates
against, i.e. the caller's visible `/keys` rows including disabled rows and
custom `connection:` singletons). It therefore releases lost-access
organization ids and deleted ids, and retains every accessible id even when
the grouped GET does not render it (custom singleton ids, disabled rows). The
complement is computed against the inventory, never against the rendered
`groups` array. It never reorders or removes a visible id, never alters any
visible group, and has no execution effect. Identical list → no-op (no bump,
no audit). Audit `service_preference_hidden_released` with `{ released_hidden: n,
version }`, no ids. Response: the §4.1 grouped shape. No counts are returned
or needed: the client offers the action from the capacity banner alone.
CLI: `nyxid service preference release-hidden`. This is the only mechanism
that removes ids the caller did not submit, and it is explicit, confirmed and
not an editor.

### 4.3 `GET /api/v1/keys`, `GET /api/v1/keys/{id}`

`KeyResponse.preference_rank` is the §3.1 group-relative dense rank (`null`
for unranked, disabled, `connection:` singleton). `KeyResponse.preference_position`
(additive, `null` unless the row's id is stored) is the row's 1-based position
among the group's stored ids that the caller can see, including disabled rows;
it backs the saved-position pill (§5.1). List: one document read over the
request's own authorized inventory. Detail: the bounded `detail_rank` path
restricted to the row's group (unchanged bound).
Both preference GET and detail rank resolve only stored IDs through the shared
live visibility helper, capped at 200; they do not render/decrypt the inventory
or reload credentials/providers. Absent or empty orders stop after the preference
document read. Saved orders add a selected-ID service read and live owner/source,
platform-access and endpoint/catalog existence projections as applicable.
A fresh Services page reads both `/keys` and `/service-preferences`, so it adds
two preference-document lookups in aggregate. Empty/absent order adds no second
inventory walk; saved GET adds only these bounded selected-ID reads.

### 4.4 CLI (contract updated with the API)

`nyxid service list` adds `Pref` (rank, `saved:p` for disabled stored rows, or
`-`). `nyxid service preference show [--group <catalog-slug|group-key>]`
prints groups with rank/position, slug, label, id and enabled state.
`nyxid service preference set --group <…> <ID_OR_SLUG>...` resolves slugs via
`/keys` preferring enabled rows, refuses mixed groups, reads `version` from
GET, PUTs the scoped route, and exits non-zero on 409 with "preference order
changed elsewhere; re-run" and on the capacity 400 with the server message.
`nyxid service preference reset --group <…>` sends `ordered: []`.
`nyxid service preference release-hidden` calls the DELETE after a `--yes`
or interactive confirmation that explains restored access will require
setting those preferences again. The superseded flat `set` is removed.

## 5. UI: inside the service group only

Unchanged: the #1685 grid and collapsed card layout, `ServiceViewToolbar`
(filters, search, Personal/All, Auto-connected, saved views), table view mode,
insight/billing/pool panels, `/keys/services/$groupId`, page-level controls.
No flat editor, no page-level Reorder button, no card replacement.

### 5.1 Per-row pills (`ServiceConnectionTable`)

- Geometry keeps the `e72036b7` fix: the Connection / Slug cell's first line
  (details chevron, icon, label link, readiness badge) is unchanged; pills
  render on their own line directly below it, before the slug line, never
  squeezed into the label line, with `flex-wrap` and `max-w-full` so long or
  duplicate labels are never truncated by a pill.
- **Discovery pill**: `Badge variant="accent"` reading `Discovery #n`,
  `aria-label="Discovery preference n for <service>"`, from `preference_rank`.
  The service name belongs to the row's catalog group, including in the mixed
  table view. Omit a denominator: filtered rows cannot establish the complete
  group's count. Enabled HTTP rows only.
- **Saved-position pill**: for a disabled row whose id is stored,
  `Badge variant="secondary"` reading `Saved #p`,
  `aria-label="Saved order position p; disabled connections are not listed to
  agents"`, from `preference_position`. An enabled non-HTTP row instead shows
  `Saved #p · SSH` (or its actual protocol), with an accessible explanation that
  it is outside connected MCP discovery. Moving it retains its saved position
  without making it discoverable; it has no discovery rank or MCP tool prefix.
- Both appear wherever the renderer is used (expanded card, table view mode,
  overview page, DEV routing preview) and coexist with the pool `Priority n`
  row and the readiness badge; the terms are distinct by design.

### 5.2 Compact summary and collapsed selection disclosure

In the expanded card body, beside the billing / pool / agent-use / last-edit
lines, for every group with two or more connections in its full inventory:
`Preferred in discovery: <label> · k enabled · d disabled` when a rank exists, or
`No agent order · default discovery order · k enabled · d disabled`. Counts
use `is_active`; the summary does not claim that enabled rows are listed,
callable, working or verified. While the preference GET is loading, failed or
returned 404, preserve any preferred connection known from `/keys` metadata
and add a muted availability suffix. Without known rank metadata, use
`Loading agent order`, `Agent order could not be loaded`, or `Saved agent
order unknown` (404) with the counts, never
`No agent order` from a failed read. The summary and the single **Agent order** action stay visible in the expanded
card and the overview Connections tab. Loading/error/404 availability is a
short truthful line; 404 says **Saving agent order requires the backend
update** and the disabled action retains the same reason (it does not describe
Service Pools as unavailable), while read errors provide Retry beside the summary.
**How selection works** is a native disclosure, collapsed by default on both
surfaces. Its native marker is hidden and it uses the same lucide ChevronRight
(size 3.5, gap 1.5, transition-transform, motion-reduce, 90-degree open rotation)
as Hide connections, with no added toggle state; native keyboard semantics
remain. Opening or closing it, including on production 404 or during a dirty
draft, must not change the draft, filters, URL, saved-view state or send a write.
Long explanation text and protocol/gateway/pool distinctions live only inside
this disclosure. No full tool-prefix list is repeated above the rows. Content,
from data already on the page:

1. Enabled HTTP alternatives have distinct tool prefixes based on their row
   slug (`<slug>__…`); use row identity rather than an exhaustive prefix list.
   Non-HTTP rows are identified by actual protocol without invented prefixes,
   and their saved-position/discovery distinction is explained;
   each connection is a separate copy of the same tools.
2. Default order: "Without an agent order NyxID lists your own connections
   before organization connections, newest first within each; an agent key's
   allowed connections and nodes narrow what it sees."
3. Preference rule: "Your agent order is advisory for discovery: when copies
   tie on relevance, NyxID lists them in your order. A better keyword match
   from another connection is listed above a preferred one."
4. Health and scope rule: "Enabled HTTP connections can be listed even when their
   credential is revoked, expired or missing; agents see `executable: false`
   on those. Disabled connections are not listed. NyxID does not verify
   providers here; row badges report unavailable or unverified only. The
   numbers on these pills are your view; an agent key allowed to use only some
   of these connections sees ranks 1, 2, … over the ones it may use, in the
   same relative order."
5. Explicit behavior: "Agents run exactly the tool or slug they name. Routing
   and failover across these connections happen only through a service pool
   you call by its slug" with the existing pool link or create link; for LLM
   provider groups: "`/api/v1/llm/<provider>` and gateway models without
   `pool:` use one active connection chosen by NyxID in database order, not by
   this list" (Part B replaces this sentence if confirmed).
6. Server state is visible outside the disclosure: after a preference GET
   404, preserve known preferred metadata or say `Saved agent order unknown`,
   and show `Saving agent order requires the backend update.` The disabled
   action has the same matching reason. No fake save is
   possible.

### 5.3 Collapsed card chip

Next to the `n disabled` badge: `Preferred: <label>` when a rank-1 connection
exists; activating it expands the card. Nothing otherwise.

### 5.4 Ordering mode (inline, one group at a time)

- Control: `Agent order` button (`ListOrdered`, `variant="ghost" size="sm"`) in
  the expanded card footer between "Hide connections" and "Service details",
  and beside the compact summary above the Connections table on
  `/keys/services/$groupId`, with no duplicate action. Rendered for
  every group whose **full inventory (enabled and disabled) has two or more
  connections**, including the 30/26 case. States: enabled; disabled with
  `title` "Loading agent order" while keys/preference load or refetch;
  disabled with "Agent order could not be loaded · Retry" on read error
  (Retry in the panel); disabled with "Saving agent order requires the backend update" after
  GET 404; disabled while another group is being ordered. Hidden only in table
  view mode (its rows mix groups).
- `orderingGroupId` plus the draft live in `GroupedServiceCards` (and the
  overview page). While set, the group's card stays mounted and expanded
  regardless of filter changes, Personal/All switches or saved-view restores
  (the card shows "Kept visible while ordering"), and its
  `ServiceConnectionTable` receives `ordering` and renders **all** of the
  group's connections with the note "Showing all N connections while
  ordering; filters still apply to other services." Other cards, the toolbar,
  saved views and the `useServiceCardView` store are untouched.
- Rows become `useSortable` `<tr>`s in a `SortableContext`
  (`verticalListSortingStrategy`); `PointerSensor` (`distance: 6`),
  `KeyboardSensor` with `sortableKeyboardCoordinates`; dnd-kit
  `accessibility.announcements` only. Each row gains, before the details
  chevron: a `GripVertical` handle (`aria-label="Drag <label>"`,
  `touch-none cursor-grab`) and `Move up` / `Move down` buttons
  (`aria-label="Move <label> up"`). Live pills on the pill line: `Discovery #n`
  for enabled rows, `Saved #p · disabled` for disabled rows. Detail/insight
  toggles stay usable; row links are inert during ordering.
- Disabled connections are part of the order, draggable (muted) and keep
  their saved position; `Move disabled to end` collapses the 26-row tail;
  `Reset to default` sends `ordered: []` after a confirm.
- Save / Cancel row under the table, right-aligned; Save `variant="primary"`,
  dirty-gated via `useAppForm` field `ordered: string[]` with
  `zodResolver(servicePreferenceGroupRequestSchema)`; edits call `setValue`
  (default `shouldDirty: true`), resets use `{ shouldDirty: false, shouldTouch: false }`.
  Save sends the group's connection ids (enabled and disabled) in table order
  with the current `version`; success invalidates `["service-preference"]` and
  `["keys"]`, exits ordering, toasts "Agent order saved for <service>" and
  returns focus to `Agent order`.
- Recovery: 409 → in-card banner "This service's agent order changed in
  another tab" with **Reload order** (refetch, reset this group's rows, stay
  in ordering) and **Overwrite** (refetch version, re-submit). 400 unknown id
  → refresh `/keys`, drop rows no longer present, stay dirty. 400 capacity →
  in-card banner with the server message, a `Reset to default` shortcut for
  this group, and **Release unavailable preferences** (§4.2a) whose confirm
  states that preferences for services the person can no longer access will be
  removed and must be set again if access is restored; on success the banner
  offers **Retry save** with the refetched version; the draft is kept
  throughout. The release action exists only in this banner inside the edited
  card or overview table, never elsewhere. Network/5xx → `ErrorBanner` with
  Retry, rows kept.
  A `/keys` refetch during ordering (invalidation from another feature) keeps
  the draft: new connections are appended with a "New" marker, removed ones
  are dropped with a notice. A preference refetch during ordering never
  resets the draft; only Reload does.
- Guards: collapsing the card, starting ordering on another group, switching
  view mode or tab, in-app navigation (TanStack `useBlocker`) and leaving the
  overview page ask "Discard unsaved agent order?"; filter changes do not
  prompt because the card stays mounted; identity change discards the draft
  silently and exits ordering.

### 5.5 Reuse

One renderer (`ServiceConnectionTable`) carries pills and ordering for the
card and the overview page; `GroupCard` adds chip, summary line and footer
control; one `service-agent-order-panel.tsx` serves both places;
`useServicePreference()` (GET; 404 → `unavailable` sentinel) and
`useSaveServiceGroupOrder()`; schemas in `schemas/service-preference.ts`;
`KeyInfo.preference_rank` and `preference_position`.

## 6. Part B (conditional): implicit gateway routes follow the saved order

Pending the user's asynchronous answer. If **no**: §5.2 item 5 keeps the
"database order" wording and nothing below is built. If **yes**: Part B is
delivered in the same change set as Part A (not deferred to a later PR), with
these fences:

- Applies only to implicit provider resolution: `/api/v1/llm/{provider}/…` and
  gateway `model` resolution without a `pool:` alias. Named slugs,
  `/proxy/{id}`, `nyx__call_tool`, `_nyxid_via`, pools, durable/exact-target
  and delegated executions are unchanged.
- Dedicated entry point, explicit context: `ProxyExecutionContext` gains
  `implicit_provider_selection: Option<ImplicitProviderSelection>`; only the
  two gateway call sites set it. `lookup_user_service` is unchanged for every
  other caller; when the context is present and `slug = None`, it calls
  `service_preference_service::select_implicit_provider_connection(db, owner_id, catalog_service_id, &saved_group_order)`,
  which picks the first **enabled** row of that owner for the catalog id that
  appears in the caller's saved order and otherwise returns exactly what
  `find_by_catalog_service_id` returns today. The generic finder used by
  credential probes, listing, auth and exact approval observe/redeem is not
  modified.
- Owner tiers preserved: personal rows are tried before organizations in
  `primary_org_id` order; the order chooses only within one owner tier, so
  `find_effective_service_owner` stays consistent.
- Scope enforced at selection: actor identity, agent-key service allowlist,
  node scope and owner access are applied to the candidate set before
  selection, so a scoped key gets its first allowed row, never a 403 caused by
  the order.
- Selection, not retry: the row is selected once before admission; approval
  target, execution-authority digest, audit `user_service_id` and billing
  owner bind to that row; if its credential cannot be materialized or the
  provider fails, the request fails as today with no second attempt.
- UI: §5.2 item 5 becomes "`/api/v1/llm/<provider>` and gateway models
  without `pool:` use the first enabled connection in this order among your
  own connections, then your organizations'; without an order, NyxID's
  default choice. No retry on another connection." The summary line adds
  "· also first for implicit gateway calls".
- Acceptance (Part B only): **AC-B1** named/exact targets unchanged with and
  without an order; **AC-B2** implicit route selects the rank-1 enabled own
  connection when two or more are enabled and today's row when none is
  ranked; **AC-B3** an org-only rank never outranks a personal row;
  **AC-B4** the selected id is bound in approval target, exact digest, audit
  and billing owner; **AC-B5** `find_by_catalog_service_id` and its callers
  are untouched (grep plus existing tests), and callers without the context
  resolve as before; **AC-B6** a scoped agent key whose rank-1 row is outside
  its allowlist gets its first allowed row; **AC-B7** a provider failure on
  the selected row produces no second attempt (upstream hit count 1).

## 7. Files

Backend: `services/service_preference_service.rs` (group key,
`rank_map_by_group`, `position_map_by_group`, `grouped_visible`,
`replace_group` with slot merge, reset and capacity, `release_hidden`; global
`replace` removed), `services/mcp_service.rs` (slot refill for listing and
search buckets; `executable` + `preference_rank` on matches),
`handlers/mcp_transport.rs` (group map into `order_discovery`; hint),
`handlers/service_preference.rs` (grouped GET, `put_group`, `delete_hidden`),
`routes.rs` (scoped PUT and hidden DELETE replace the global PUT under the
same rejection layers and body limit), `handlers/keys.rs` (`preference_rank`,
`preference_position`), `api_docs.rs`, `cli/src/commands/service.rs`,
`cli/src/cli.rs`.

Frontend: `components/dashboard/service-connection-table.tsx` (pill line,
ordering mode, Save/Cancel), `components/dashboard/grouped-service-cards.tsx`
(chip, summary line, footer control, `orderingGroupId`, kept-mounted rule,
guards), new `components/dashboard/service-agent-order-panel.tsx`,
`pages/service-overview.tsx`, `hooks/use-service-preference.ts`,
`schemas/service-preference.ts`, `types/keys.ts`. Removed:
`components/dashboard/service-preference-editor.tsx` and its test, the
`pages/keys.tsx` Reorder button and editor swap and their tests;
`e2e/service-preference.spec.ts` rewritten for the in-card flow. Wizard:
regenerate `cli/src/wizard/assets` and `bundle-meta/index.hash` if their
producer inputs change (the frontend bundle is a producer) and keep the
freshness test green.

## 8. Documentation

`docs/API.md` (minimal grouped GET, scoped PUT with `ordered: []` reset,
slot-merge and append-omitted guarantees, capacity error, hidden DELETE
semantics against the authorized inventory, `preference_rank` and
`preference_position`, 400/409); `docs/API_DISCOVERY.md` "Tool search
semantics" (slot refill within relevance buckets, `executable` +
`preference_rank` on matches, non-executable listed connections,
independent-client non-guarantee); `docs/AI_SERVICES_ARCHITECTURE.md`
("Agent discovery order" subsection under "Service cards and saved filter
defaults" with the §2 table and the implicit-gateway explanation);
`docs/chat/08-nyxagent-engine.md` (one sentence); `CLAUDE.md` Rule 8 (one
bullet). `docs/plans/service-tool-preference-review.md` is ROOT-owned.

## 9. Rollout and honesty constraints

Additive fields and routes only. Against a server without the route, the UI
shows known preferred metadata or `Saved agent order unknown`, enabled/disabled
counts and `Saving agent order requires the backend update.`, the collapsed
selection disclosure, and the Agent order control disabled with the same
  reason; the unavailable endpoint is `/api/v1/service-preferences`, not Pools.
  The production-backed preview has no preference mock, local persistence or
  fabricated success toast. The separately authorized seeded sample preview
  on 4631 is explicitly local sample data and uses temporary JSON persistence;
  it never writes to production.
No data migration (stored flat lists remain valid under the slot merge), no
version bump solely for this feature, no flag.

## 10. Tests and commands

```bash
export NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27029/?replicaSet=nyxidPreferenceReview&directConnection=true'
export CARGO_TARGET_DIR=/tmp/nyxid-service-preference-target CARGO_INCREMENTAL=0
export CARGO_PROFILE_TEST_DEBUG=0 CARGO_PROFILE_DEV_DEBUG=0
cargo test -p nyxid service_preference -j 1
cargo test -p nyxid search_all_tools -j 1
cargo test -p nyxid asking_for_an_agent_finds_agent_creation_first -j 1
cargo test -p nyxid curation_router_scoped_discovery_history_and_route_confinement -j 1
cargo test -p nyxid-cli --bin nyxid service_preference -j 1
cargo test -p nyxid-cli --test service_preference -j 1
cargo test -p nyxid-cli --test wizard_bundle_freshness -j 1
cargo fmt --all -- --check
cargo clippy -p nyxid -p nyxid-cli --all-targets -j 1 -- -D warnings
# From frontend/; the temporary config only isolates happy-dom from live servers.
NODE_ENV=test npx vitest run --config /tmp/nyxid-service-preference-vitest.config.mts --maxWorkers=2
npm run lint -- --no-warn-ignored
npm run build
npm run build:wizard
npx playwright test e2e/service-preference.spec.ts e2e/wizard-scope.spec.ts --workers=1 --output=/tmp/nyxid-service-preference-inline-browser-complete-results
```

Docker is unavailable; the review instance above is the only database target
and tests never skip silently. One Cargo build at a time, task-owned target
`/tmp/nyxid-service-preference-target`, `CARGO_INCREMENTAL=0`, jobs 1. The
Playwright fixture serves `/users/me` with `profile_config.services_view`,
`/keys` with an Anthropic group of 30 connections (4 enabled, 26 disabled, two
sharing the label "Anthropic", one org-owned, several stored ids among the
disabled) plus one unrelated Slack group interleaved in `/keys` order,
`/service-preferences` in grouped, 404 and 503 modes,
`/service-preferences/groups/*` with CAS/409/400-unknown/400-capacity/503
modes, `DELETE /service-preferences/hidden` with CAS/409/503 modes,
`/service-insights` → `{connections: []}`, an explicit Priority-7 pool on the
org connection (organization pool reads return empty to avoid duplicates),
`/catalog?include_all=true`, and records any `PUT /users/me/preferences/services`.

## 11. Implementation tasks (ordered, Part A complete)

All 14 Part A tasks below are implemented. §16 records their fresh acceptance
execution evidence. Task 15 remains conditional on the user's Part B answer.

Backend

1. `service_preference_service.rs`: `group_key`, `rank_map_by_group`
   (REST projection, catalog singleton-visible → rank 1),
   `position_map_by_group`, `grouped_visible` (minimal shape),
   `replace_group` with the §4.2 partition, append-omitted rule, slot
   replacement, `[]` reset, post-merge capacity check (no prune) and no-op on
   the existing CAS and legacy-row logic; `release_hidden` computing the
   complement of the authorized inventory (§4.2a); delete the global
   `replace`; property-style merge tests (AC-30, AC-31).
2. `mcp_service.rs`: `permute_listing_slots(services, ranks)` (returns a new
   vector) and bucket-wise slot refill inside `search_all_tools_ranked`, both
   consuming the **original** loader vector; `executable` + `preference_rank`
   on matches; `#[cfg(test)]` wrappers kept; tests for interleaved groups,
   mixed-relevance interleaving (A-low, Slack-high, A-high), multiple tools
   per connection, 25-truncation, empty-map byte identity, non-executable
   enabled rows.
3. `mcp_transport.rs`: `order_discovery` returns `(original services, ranks)`
   with ranks from the MCP projection (`McpToolSource::UserManaged.catalog_service_id`
   over the loaded, filtered services); `handle_meta_search` passes the
   original vector to search; `handle_meta_list_connected` applies the listing
   permutation; hint sentence.
4. `handlers/service_preference.rs`: grouped GET; `put_group` (path group
   validation, group inventory, `replace_group`, conditional audit);
   `delete_hidden` (first-party guard, CAS, conditional audit).
5. `routes.rs`: replace the global PUT with the scoped PUT and the hidden
   DELETE under the same rejection layers and body limit; `api_docs.rs`.
6. `handlers/keys.rs`: `preference_rank` and `preference_position`;
   `detail_rank` restricted to the row's group.
7. CLI: `Pref` column, `preference show`, `preference set --group`,
   `preference reset --group`, `preference release-hidden` (confirmation);
   remove the flat `set`; tests incl. mocked 409, capacity 400 and release.

Frontend

8. `schemas/service-preference.ts` (+ test) and `hooks/use-service-preference.ts`
   (`unavailable` sentinel, `useSaveServiceGroupOrder` incl. `[]` reset and
   its `release` mutation, invalidations from
   `useDeleteKey`/`useUpdateKey`).
9. `service-connection-table.tsx`: pill line below the label line
   (`Discovery #n`, `Saved #p`); `ordering` prop with sortable rows, handle,
   Move up/down, `Move disabled to end`, `Reset to default`, live pills,
   Save/Cancel, in-card banners incl. the capacity banner with
   **Release unavailable preferences** confirm and **Retry save**;
   all-connections rendering; draft preservation across `/keys` refetch.
10. `grouped-service-cards.tsx`: chip, summary line (enabled/disabled counts,
    explicit order-saving backend status), footer control with the stated states and the
    full-inventory ≥ 2 rule, `orderingGroupId`, kept-mounted-while-ordering,
    guards (collapse, other group, view/tab, `useBlocker`, identity).
11. `service-agent-order-panel.tsx` with §5.2 items 1–6 and pool link; mount
    in the card and `service-overview.tsx`.
12. **Source migration of the superseded UI**: delete
    `service-preference-editor.tsx` (+ test), remove the `keys.tsx` Reorder
    button, editor swap and their `keys.test.tsx` cases, drop the flat
    `ordered` response schema; rewrite `e2e/service-preference.spec.ts` for
    the in-card flow with the §10 fixture (30/26/duplicate labels, interleaved
    unrelated group, prod-404 mode, 409/400/capacity/503 modes, release and
    retry-save, refetch during ordering, navigation blocker).
13. Regenerate the CLI wizard bundle if its producers changed; keep the
    freshness test green.
14. Docs per §8; record fresh evidence per §15.

Part B (only if the user answers yes; same change set): 15. `ImplicitProviderSelection`
context and `select_implicit_provider_connection`; gateway call sites;
tests AC-B1..B7; §5.2 item 5 wording and summary suffix; docs.

## 12. Acceptance criteria (Part A)

- **AC-01** (local): legacy document with `_id`, `created_at`, `updated_at`
  deserializes to `ordered = []`, `version = 0`; BSON round-trip with dates.
- **AC-02** (local, *revised*): `put_group` validation rejects 201 request
  connection ids, non-canonical or non-v4 connection UUIDs, duplicates, bad
  `expected_version`; catalog group UUIDs require canonical lowercase RFC
  variant/version 1–8 and reject nil/max/unknown versions;
  unknown body fields (including a `clear` field), malformed group key,
  `connection:` group, foreign id, missing id and wrong-group id; the message
  for the last three is byte-identical; `ordered: []` is accepted as the
  reset.
- **AC-03** (local, *revised*): `rank_map_by_group` yields dense per-group
  ranks over listed ids only; disabled, deleted or out-of-scope stored ids
  leave no gap; a catalog group with exactly one visible listed stored member
  yields rank 1; `connection:` singletons yield `null`; `position_map_by_group`
  counts disabled stored rows; listing slot refill changes only slots occupied
  by groups with a stored rank and leaves every other slot byte-identical for
  any stored list, including interleaved groups.
- **AC-04** (local, *revised*): search slot refill keeps
  `search_all_tools_matches_words_in_any_order_and_ranks_full_matches_first`
  green; with Anthropic copies A1 A2 A3 interleaved with equally relevant
  Slack tools (A1 S1 A2 S2 A3) and A3 ranked first, the output is A3 S1 A1 S2
  A2 (Slack slots untouched); a connection contributing three tools keeps
  their relative order while all three precede the rank-2 connection's tools;
  a better match from an unranked connection stays in its higher bucket above
  a preferred partial match; **adversarial mixed relevance**: loader order
  A1 (low-relevance endpoint), Slack (high), A2 (high) with A2 ranked first
  yields Slack, A2 in the high bucket and A1 in the low bucket exactly as
  without ranks (no tool changes bucket, Slack keeps its slot), and the same
  input through a pre-sorted service vector is shown to differ, which is why
  the test feeds the original vector; with 30 tying copies the 25 survivors
  begin with the preferred connection's copies; every match carries
  `executable` and `preference_rank`; an enabled connection with a revoked
  credential appears with `executable: false`.
- **AC-05** (local, *revised*): with an empty rank map the serialized output
  of existing search/list fields and their order is byte-identical to the legacy
  fixture contract; new additive rank/executable metadata has its null/default
  values. The `#[cfg(test)]` wrappers and existing expected-order fixtures assert this; the
  assistant-account agent-creation search test stays green.
- **AC-06** (local, *revised*): `list_connected_services_ranked` rows carry
  group-relative `preference_rank` or `null` and `executable`, refilled per
  §3.2; `count` unchanged; singleton and platform rows keep their slots.
- **AC-07** (DB, *revised*): scoped PUT `expected_version: 0` → `version 1`;
  second PUT `expected_version: 1` → `version 2`; PUT with `expected_version: 1`
  afterwards → 409 code 1004, document unchanged; concurrent first saves and
  concurrent later saves on two different groups each produce exactly one 200
  and one 409, and the loser succeeds after Reload with both orders intact;
  resubmitting a group's current order (interleaved with another group's ids)
  is a no-op with no version bump and no audit.
- **AC-08** (DB, *revised*): a restricted agent key allowlisted to one of
  three stored Anthropic connections reads `groups` with exactly that id and
  the raw `version`, with no count or capacity field present, and `/keys` shows
  that row as `preference_rank: 1`; the human reads ranks 1..3; the same key
  allowlisted to the second and fourth of four stored connections gets MCP
  listing ranks 1 and 2 while the owner's `/keys` pills read 2 and 4 (§3.3).
- **AC-09** (DB): deleting a stored connection removes it from GET without
  any write to `service_preferences` and the rest renumber densely; disabling a
  stored connection yields `preference_rank: null` with its `preference_position`
  and dense ranks for the rest; re-enabling restores its relative position
  with no preference write.
- **AC-10** (DB, *revised*): scoped PUT and hidden DELETE are rejected before
  the handler for API-key, SA, delegated and relay tokens and for OAuth
  application tokens by the first-party guard; GET succeeds for API key and
  delegated `account:read`, rejected for SA and relay.
- **AC-11** (DB): `/keys` and `/keys/{id}` report group-relative rank and
  position; a command-monitoring `GET /keys` shows exactly one `find` on
  `service_preferences`; detail reads at most the stored ids restricted to the
  row's group and loads no credentials or providers.
- **AC-12** (DB, MCP via `assistant_authority_tests::fixture`, *revised*): two
  Anthropic connections interleaved with one Slack connection in loader order;
  saving the second Anthropic first makes `nyx__search_tools` (non-empty tying
  query) emit its tools in the first Anthropic slot while the Slack tool keeps
  its slot; listing shows ranks 1, 2, `null` with unchanged Slack slot; a
  guest-turn chat key sees the same order restricted to its grants; an enabled
  Anthropic connection with a revoked credential is listed with
  `executable: false` and its rank.
- **AC-13** (DB): `nyx__call_tool` on a named tool, `/proxy/s/<slug>`
  resolution and `/api/v1/llm/<provider>` connection choice are identical with
  and without a saved order (two enabled same-catalog connections; same
  response and execution audit data/actor/target; per-call ids differ);
  `/mcp/config` `catalog_digest` and service order byte-identical. (If Part B
  is confirmed, the gateway clause is replaced by AC-B2 and AC-B7.)
- **AC-14** (local, grep): no `service_preference` / `preference_rank` /
  `preference_position` in `proxy_service.rs`, `execution_authority.rs`,
  `mcp_approval.rs`, `services/billing`, `service_pool_service.rs`,
  `llm_gateway_service.rs`, `handlers/llm_gateway.rs`,
  `handlers/service_insights.rs`, `services/service_insights_activity.rs`;
  `execute_tool*` never reference the collection or rank map. (Part B adds
  exactly the dedicated selector and the two gateway call sites to the allowed
  list.)
- **AC-15** (DB, *revised*): one `service_preference_updated` audit per
  changed scoped save with `event_data` exactly `{group, count, version}`,
  one `service_preference_hidden_released` per changed release with exactly
  `{released_hidden, version}`, none on no-op or rejected capacity, no
  connection IDs anywhere in either audit; the scoped audit retains its
  catalog group. Chain verification passes.
- **AC-16** (local, CLI, *revised*): `Pref` column shows rank, `saved:p`, or
  `-`; `preference show` lists groups; `preference set --group` resolves slugs
  preferring enabled rows, refuses mixed groups, sends `expected_version` from
  GET; `preference reset --group` sends `ordered: []`; `release-hidden`
  requires confirmation and sends the DELETE with `expected_version`; mocked
  409 and capacity 400 exit non-zero with the stated messages; the flat `set`
  no longer exists.
- **AC-17** (local, *revised*): pills render below the connection identity header with its existing
  label/readiness metadata; the header wraps readiness when needed and
  reserves a usable label span (at least 64px at the default text size) for
  long/duplicate labels in both normal and editing rows; enabled HTTP stored rows show `Discovery #n` with the §5.1 aria
  label; disabled stored rows show `Saved #p`; active non-HTTP stored rows show
  their actual protocol's saved-position pill and no MCP prefix/discovery rank;
  unstored and `connection:`
  singleton rows show neither; a pool-member row shows `Priority n` plus its
  pill; two rows labelled "Anthropic" are distinguished by slug and pill;
  filters hide rows but never change visible pill text or any row/group
  position; the collapsed chip reads `Preferred: <label>` only with a rank-1
  connection and expands the card. Verified in the expanded card, table view
  mode and `/keys/services/$groupId`.
- **AC-18** (local, *revised*): the summary line reads `k enabled · d
  disabled` from `is_active`, uses "default discovery order" (never "newest
  first" alone, never listed/callable/working/verified), and appears for every
  group with a full inventory ≥ 2; **How selection works** is collapsed by
  default on card and overview and contains the §5.2 selection, scope, protocol,
  pool and conditional gateway details. No exhaustive prefix list or repeated
  open explanation appears above rows. Summary/status/Retry and one Agent order
  action remain discoverable while loading, on read error and after a GET 404
  (disabled with the stated reasons), including the 30/26 group. Disclosure
  toggles on 404 and while dirty preserve draft/filters/URL and write nothing;
  narrow-width layout contains horizontal scrolling to the table.
- **AC-19** (real-route Playwright with real sensors, plus unit tests for
  buttons, *revised*): in the expanded 30-connection card, mouse-dragging a
  disabled stored row above an enabled one updates `Saved #p` and `Discovery
  #n` pills consistently; keyboard dragging with Escape cancellation; touch
  dragging at a 390px viewport with contained table scrolling and no document
  horizontal overflow; Move up/down,
  `Move disabled to end` and `Reset to default` (with confirm) behave as
  §5.4; the single dnd-kit live region announces; the Slack card, toolbar,
  store and saved view are unchanged throughout.
- **AC-20** (local, *revised*): Save disabled until a change, enabled after a
  move; Cancel exits ordering without writes and removes the editor/Save action.
  Save sends exactly the group's connection ids
  (enabled and disabled) in table order plus `version` to the scoped route,
  nothing for another group; `Reset to default` sends exactly `ordered: []`
  and `expected_version`.
- **AC-21** (local, *revised*): 409 renders the in-card banner with **Reload
  order** (resets this group only, clears dirty) and **Overwrite** (refetches
  version, re-submits); 400 unknown id refreshes `/keys` and drops missing rows
  while dirty; 400 capacity shows the server message with `Reset to default`
  and **Release unavailable preferences** (confirm text names the consequence
  for restored access; the DELETE is sent only after confirmation; success is
  followed by **Retry save** with the refetched version) and keeps the draft;
  the release action is absent everywhere outside this banner; network error
  shows Retry and keeps rows; a
  read failure prevents entering ordering; after a GET 404 the summary line,
  panel server-state line and disabled control are visible, no PUT is ever
  sent, and pills from `/keys` still render.
- **AC-22** (local, *revised*): grouped response schema accepts exactly
  `version: 0`, `updated_at: null`, `groups: []` and rejects unknown fields;
  group request schema rejects 201 ids, non-UUID, duplicates, negative
  `expected_version` and any extra field, and accepts `ordered: []`.
- **AC-23** (local): `npm run lint` (no `console.log`, no raw `useForm`),
  `npm run build`, `cargo clippy -p nyxid -p nyxid-cli --all-targets -- -D warnings`,
  wizard freshness test.
- **AC-24** (docs review, *revised*): §8 docs present; discovery doc states
  the slot-refill rule, non-executable listing, advisory nature and the
  independent-client non-guarantee; API doc states the merge guarantees and
  capacity error; architecture doc carries the §2 table and the
  implicit-gateway explanation.
- **AC-25**: slug-based key reads, catalog-curation writes, key-update and
  saved-view routes retain prior auth and behavior; OAuth application tokens
  cannot write preferences.
- **AC-26** (*revised*): scoped REST and MCP responses expose no hidden ids
  and no rank gaps; stale read filtering performs no write; canonical id,
  version, body-size, unknown-field, wrong-group, reset and no-op behavior pass
  explicit tests; a scoped save for group A leaves every stored id outside
  A's authorized set (other groups, hidden or stale same-group ids, ids
  outside the caller's inventory) present, byte-identical and in the same
  relative order, and never deletes any of them; normal saves and resets never
  remove an ID outside the currently authorized target group. Only the
  explicit hidden DELETE can release IDs outside the authorized inventory.
- **AC-27** (*revised*): browser tests on the grouped UI prove the §5.4 flow
  in the expanded card and on `/keys/services/$groupId`, pills in all three
  surfaces, the chip, the prod-404 read-only state, conflict/unknown-id/
  capacity/network recovery including the release confirmation and retry
  save, draft survival across a `/keys` refetch, the
  kept-mounted card under filter changes, guards on collapse/other
  group/view/tab/navigation, focus return, and that saved view, filters, store
  and other cards are unchanged (no `PUT /users/me/preferences/services`).
- **AC-28** (*revised*): read failure cannot enable saving; a two-connection
  group (one enabled, one disabled) can be ordered; the 30/26/duplicate-label
  group renders, drags and saves; identity change discards the draft and exits
  ordering; caches are identity-separated; stale inventory recovery has tests.
- **AC-29** (DB, *revised*): grouped GET returns only groups with ≥ 1
  authorized stored id and never a `connection:` group; stored ids the caller
  cannot see never appear; the response carries no count or capacity field for
  any caller class.
- **AC-30** (local, property-style, *revised*): for random inventories,
  stored lists (including stale, hidden and foreign ids interleaved) and
  sequences of scoped saves and `[]` resets on distinct groups: `H` elements
  and their relative order are preserved; no id changes group; resubmitting
  the current order is the identity; for non-empty saves, omitted authorized
  ids (including ones granted after the draft was built) are appended after
  the submitted ones in their previous relative order, never dropped; newly
  stored ids land right after the group's last slot; `[]` removes exactly the
  group's currently authorized stored ids; a legacy flat list from `e72036b7`
  yields the same same-group relative order before and after the first scoped
  save.
- **AC-31** (DB, new, capacity recovery): (a) with a stored list of 198 ids
  including live ids the caller cannot see, a scoped save adding three new
  ids returns the capacity 400 with the fixed-200 message and no ids, leaves
  the document unchanged and writes no audit; a `[]` reset on any group
  succeeds at capacity; a reorder without new ids succeeds at exactly 200;
  (b) **all-hidden saturation**: a 200-id document consisting only of
  lost-access organization ids and deleted ids; the human's first-party
  `DELETE /service-preferences/hidden` with the current version empties it,
  audits `released_hidden: 200`, and the previously failing scoped save then
  succeeds; (c) **retention**: with a document mixing an accessible custom
  `connection:` singleton id, an accessible disabled row id, visible catalog
  group ids, a lost-access org id and a deleted id, the release removes
  exactly the last two and keeps the first three in place and order, even
  though the grouped GET renders neither the singleton nor any count;
  (d) CAS: a stale `expected_version` → 409 and no change; identical list →
  no-op, no bump, no audit; (e) first-party matrix: API-key, SA, delegated,
  relay and OAuth application callers are rejected before any change;
  (f) identity: the complement is computed for `AuthUser.user_id` only, and a
  second person's document is untouched; (g) UI: the release action is
  offered only in the capacity banner inside the edited card or overview
  table, requires confirmation with the restored-access consequence, and is
  followed by a successful **Retry save** of the kept draft.

## 13. Decisions

Decided by ROOT (no further approval needed): disabled connections remain
draggable and keep their saved position with a distinct `Saved #p` pill; the
unreleased global `PUT` is replaced by the scoped contract and the CLI
contracts updated accordingly; wording uses "enabled/disabled" counts and
"default discovery order", never listed/callable/working/verified.

Part A is authorized by ROOT for implementation against this architecture and
the normative corrections above; it proceeds independently of Part B.

Decided by ROOT on capacity recovery (part of Part A, no user approval
needed): the explicit human-only `DELETE /service-preferences/hidden` is
accepted with the scoped PUT's fences; the automatic dead-id prune is
rejected, so scoped saves preserve all unrelated, stale and hidden ids
exactly; the grouped GET stays minimal (`groups`, `version`, `updated_at`)
with no counts or capacity fields; there is no `clear` flag, `ordered: []` is
the reset, and non-empty saves append omitted now-authorized members before
the slot merge.

Pending for the user: **Part B** (§6). If yes, it ships in the same change
set.

## 14. Handoff

Plan file: `docs/plans/service-tool-preference-order.md`. Part A: group-relative,
advisory agent discovery order stored in the existing per-identity document
with CAS and an account-wide 200 cap, written only through a group-scoped
server-authoritative slot merge that preserves every other id byte for byte and
in relative order. Non-empty saves retain omitted authorized group members;
a `[]` reset clears the group's currently authorized stored ids. The explicit,
confirmed, first-party release removes only ids outside the actor's current
authorized inventory. Resubmission is a no-op, and resets remain available
at capacity;
ranks derived per catalog group after authorization with explicit REST versus
MCP projections; discovery applies slot refill inside today's loader list and
inside identical-relevance search buckets built from the original vector, so
unrelated services never move and empty preferences are byte-identical; `executable` and `preference_rank` added
to MCP discovery rows; UI entirely inside the expanded service card and the
per-service overview (pill line below the readable wrapping label/readiness header, summary line,
compact honest server-state line and collapsed-by-default selection disclosure, inline
drag/keyboard ordering with Save/Cancel, reset and in-banner capacity
recovery), leaving the #1685 grid,
toolbar, filters, saved views, insights and pools unchanged. Part B is
specified conditionally with a dedicated gateway entry point and fences.
14 completed Part A tasks plus one conditional Part B task, 31 Part A acceptance
criteria plus 7 conditional Part B criteria.

## 15. Evidence policy

Historical evidence from `e72036b7` stands for unchanged boundaries: AC-01
model tests, the CAS/no-op/legacy-row mechanics reused by `replace_group`
(re-run anyway as part of AC-07), route rejection layers (AC-10, AC-25), the
`/keys` detail read bound (AC-11), and the `#[cfg(test)]` wrapper arrangement.
Every criterion marked *revised*, every new criterion (AC-29 to AC-31), and
every UI criterion requires fresh execution against the §10 commands and the
MongoDB 8.0.17 review instance, recorded with command and outcome before
sign-off. The §16 matrix checks criteria only from actual passing execution
and the stated source/document review; historical evidence alone does not
complete a revised criterion.


## 16. Part A implementation and fresh review evidence

The global editor has been removed. Part A uses the existing grouped/table/
overview architecture, scoped CAS writes and same-group discovery-slot refill.
The user's latest screenshot review replaces the open repeated explanation
with the §5.2 compact summary and collapsed **How selection works** disclosure.
The card keeps its single Agent order header action; overview places its single
action alongside the compact summary. Production source is frozen for ROOT's
independent UI/source review. ROOT owns publication and the separate review
record; Part B remains unimplemented pending the user's answer.

The final authority correction is narrowly opt-in in `api-client.ts`: guards
run after DEV module loading, before fetch, after fetch before 401/session side
effects, and after JSON parsing. Preference GET/save/release and the keys list
bind captured dashboard or installed Mode A authority. The Mode A identity is
a generation token with no secret in query keys; its real local shim works with
an empty dashboard AuthStore. Catalog group validation now agrees across REST,
CLI and Zod: canonical lowercase RFC variant, recognized UUID versions 1–8,
including seeded v5 and created v4 groups; nil/max and version 9 are rejected.
Ordered connection UUIDs remain v4.

Local Rust validation uses Rust 1.94.1. All Rust runs use MongoDB 8.0.17 at
`mongodb://127.0.0.1:27029/?replicaSet=nyxidPreferenceReview&directConnection=true`,
`NYXID_TEST_DATABASE_URL`, target `/tmp/nyxid-service-preference-target`,
`CARGO_INCREMENTAL=0`, `CARGO_PROFILE_TEST_DEBUG=0`,
`CARGO_PROFILE_DEV_DEBUG=0`, and `-j 1`. No DB test skips, Docker replacement,
external daemon changes or other-task cache deletions were used. Frontend full
runs use the isolated happy-dom URL 4629; real-route browser tests use 4611.
The production-backed review server 4630 is preserved and has no preference
emulation or feature writes. ROOT's separately user-authorized seeded preview
on 4631 uses only temporary sample API/Vite configuration and JSON persistence
in `/tmp/nyxid-service-preference-mock-preview`; both servers are preserved.

Passing executions on the Part A source:

| Gate | Exact command / outcome | Log |
|---|---|---|
| Final backend feature/DB including UUID boundaries | `cargo test -p nyxid service_preference -j 1`: 14 passed, zero failed/ignored, 3.18s after 8m35s compile | `/tmp/nyxid-service-preference-inline-backend-uuid.log` |
| Neighboring discovery search | `cargo test -p nyxid search_all_tools -j 1`: 5 passed, zero failed/ignored, 0.01s after 10m10s compile | `/tmp/nyxid-service-preference-inline-final-neighbor-search.log` |
| Named assistant-account search | `/tmp/nyxid-service-preference-target/debug/deps/nyxid_server-a548fe5994bb0fe3 asking_for_an_agent_finds_agent_creation_first`: 1 passed, zero failed/ignored, 0.01s | `/tmp/nyxid-service-preference-inline-final-neighbor-agent.log` |
| Neighboring curation confinement | `/tmp/nyxid-service-preference-target/debug/deps/nyxid_server-a548fe5994bb0fe3 curation_router_scoped_discovery_history_and_route_confinement`: 1 passed, zero failed/ignored, 1.81s against the explicit MongoDB URI | `/tmp/nyxid-service-preference-inline-final-curation.log` |
| CLI targets compiled together | `cargo test -p nyxid-cli --bin nyxid --test service_preference --test wizard_bundle_freshness --no-run -j 1 --message-format=json`: exit0, 2m53s | `/tmp/nyxid-service-preference-inline-final-cli-compile.log`, compiler artifacts in `/tmp/nyxid-service-preference-inline-final-cli-compile.jsonl` |
| CLI preference unit tests | `/tmp/nyxid-service-preference-target/debug/deps/nyxid-002d4b25df51439a service_preference`: 3 passed, zero failed/ignored, 0.01s | `/tmp/nyxid-service-preference-inline-final-cli-unit.log` |
| CLI real subprocess integration | `/tmp/nyxid-service-preference-target/debug/deps/service_preference-bdaeb69b7981a5b3`: 4 passed, zero failed/ignored, 1.90s | `/tmp/nyxid-service-preference-inline-final-cli-integration.log` |
| Rebuilt wizard source closure freshness | `/tmp/nyxid-service-preference-target/debug/deps/wizard_bundle_freshness-41d9c8f58e4486a5`: 1 passed, zero failed/ignored, 0.06s | `/tmp/nyxid-service-preference-inline-final-wizard-freshness.log` |
| Rust formatting | `cargo fmt --all -- --check`: exit0, no formatting differences | `/tmp/nyxid-service-preference-inline-final-fmt.log` |
| Final backend/CLI all-target Clippy | `cargo clippy -p nyxid -p nyxid-cli --all-targets -j 1 -- -D warnings`: exit0, no errors/warnings, 5m15s after the test-only helper correction | `/tmp/nyxid-service-preference-inline-final-clippy-recheck.log` |
| Focused final UI/schema/authority | `NODE_ENV=test npx vitest run --config /tmp/nyxid-service-preference-vitest.config.mts src/hooks/service-order-transport.test.tsx src/hooks/use-service-group-order.test.tsx src/hooks/use-service-preference.test.tsx src/hooks/use-keys-identity.test.tsx src/hooks/use-keys.test.tsx src/components/cli-wizard/access-scope-mode-a.test.tsx src/lib/api-client.test.ts src/schemas/service-preference.test.ts src/pages/keys.test.tsx src/pages/service-overview.test.tsx`: 10 files, 99 tests passed, 6.10s | `/tmp/nyxid-service-preference-transport-disclosure-unit-recheck.log` |
| ROOT independent real transport | `service-order-transport.test.tsx` + `api-client.test.ts`: 27 tests/2 files passed, 1.76s, exit0 | `/tmp/nyxid-service-preference-group-pm-transport.log` |
| Final real browser and built standalone wizard | `npx playwright test e2e/service-preference.spec.ts e2e/wizard-scope.spec.ts --workers=1 --output=/tmp/nyxid-service-preference-inline-final-browser-results`: 21/21 passed, 1.7m; normal/edit labels ≥64px at390/1024/1440, same chevron/native disclosure and specific404 message | `/tmp/nyxid-service-preference-inline-final-browser.log` |
| ROOT independent final browser | Same 21 card/overview/recovery/geometry/wizard scenarios: 21/21 passed, 55.6s | `/tmp/nyxid-service-preference-group-pm-final-browser.log` |
| Final wizard regeneration | `npm run build:wizard`: exit0; 171-file producer manifest, hash prefix `6a459342b100` | `/tmp/nyxid-service-preference-inline-final-wizard-build.log` |
| Final lint | `npm run lint -- --no-warn-ignored`: exit0, zero errors, same 29 unrelated baseline warnings, none in feature files | `/tmp/nyxid-service-preference-inline-final-lint.log` |
| Final full frontend after source freeze | `NODE_ENV=test npx vitest run --config /tmp/nyxid-service-preference-vitest.config.mts --maxWorkers=2`: 466 files, 4773 tests passed, 190.34s, exit0 | `/tmp/nyxid-service-preference-inline-final-full.log` |
| Final production build | `npm run build`: exit0; TypeScript/app, legal prerender, credential-accept and mock-footprint checks passed | `/tmp/nyxid-service-preference-inline-final-build.log` |
| Final chevron/readability focused unit pass | `NODE_ENV=test npx vitest run --config /tmp/nyxid-service-preference-vitest.config.mts src/hooks/use-service-group-order.test.tsx src/pages/keys.test.tsx src/pages/service-overview.test.tsx src/pages/nyxbot-onboarding.test.tsx src/schemas/service-preference.test.ts`: 5 files, 113 tests passed, 6.53s | `/tmp/nyxid-service-preference-chevron-readable-unit.log` |

The initial focused disclosure run failed only because the provider explanation
asserted visibility before opening the newly collapsed disclosure; the regression
now opens it and continues to verify null inference/nonstandard catalog metadata.
The real transport test's Mode A assertion was corrected to the shim's actual
URL/options fetch signature, preserving the local-CSRF and zero-old-fetch proof.
The earlier full Part A run passed 465 files/4765 tests before the transport
correction. The next full run found one stale mocked `/keys` call signature in
onboarding; both mount/Back assertions now include the opt-in authority guard.
The normal-row geometry correction reserves label width and wraps readiness;
focused card/overview browser proof uses 64px actual label spans, duplicate/long
labels and no bounding-box overlap at 390/1024/1440. The final full/build/browser
gates above passed after the last chevron and specific404-copy changes. The
neighboring discovery/assistant/curation, CLI, wizard freshness and final
package Clippy gates also passed. CLI fixtures were
reconciled to the scoped route and grouped response, retaining exact
body/rank/conflict assertions.

The worktree's existing backend/CLI build scripts watch package-local Git
paths that are absent here, causing repeated Cargo invocations to rebuild
the main test crate. After the final search compile, the assistant and
curation regressions ran against that exact freshly compiled backend binary.
The CLI unit, subprocess and freshness targets were compiled together once;
the compiler-artifact JSON identified the actual test executables recorded
above. These are direct test executions, not additional Cargo invocations.

The first final Clippy execution exited 101 after 7m9s with one `dead_code`
error: `order_services_by_preference` was called only by regression tests but
still compiled into the production binary. It is now a private `#[cfg(test)]`
helper, matching the other test wrappers, with no suppression or runtime change.
The test code and all production discovery/execution behavior are unchanged;
the final all-target Clippy recheck passed after compiling both production and
test variants. Formatting passed again after this correction. The frontend
source, bundle and successful frontend/browser checks are unaffected.
Failure log: `/tmp/nyxid-service-preference-inline-final-clippy.log`;
recheck log: `/tmp/nyxid-service-preference-inline-final-clippy-recheck.log`.

Acceptance implementation/test evidence (all 31 Part A criteria have local
evidence; this does not close ROOT's separate review findings or grant Opus
sign-off):

- [x] **AC-01** — `service_preference_bson_dates_and_legacy_defaults` passes BSON date round-trip, missing ordered/version defaults.
- [x] **AC-02** — mounted HTTP scopes/validation, backend canonical/dense unit and strict schemas pass all auth/body/ID/version/group fences and reset. Frontend/backend/CLI nil/max/version9 and versions1–8 boundaries pass.
- [x] **AC-03** — `service_preference_validation_and_dense_visibility` and MCP stable-order/slot tests pass dense group ranks, disabled-inclusive saved positions, custom singleton exclusion and interleaved slot preservation.
- [x] **AC-04** — feature slot/mixed-relevance/multiple-operation/cap/empty-map regressions and all 5 neighboring `search_all_tools` tests passed, including full-match relevance precedence.
- [x] **AC-05** — `service_preference_stable_order_relevance_cap_and_legacy` and paired MCP fixture assert legacy fields/order unchanged with empty maps; only documented additive metadata differs. The named `asking_for_an_agent_finds_agent_creation_first` regression also executed and passed.
- [x] **AC-06** — `service_preference_stable_order_relevance_cap_and_legacy`, `service_preference_slots_interleaving_multiple_operations_and_mixed_relevance`, and the mounted MCP fixture pass `list_connected_services_ranked` group-relative rank/null and executable metadata, same-group slot refill, unchanged count, and unchanged singleton/platform slots.
- [x] **AC-07** — first-insert/current-version races plus `service_preference_distinct_group_cas_retry_preserves_both_orders` pass loser409 → reload/retry and both final group orders, logical interleaved no-op and legacy missing-version upgrade.
- [x] **AC-08** — scoped HTTP/MCP fixtures pass singleton rank1 and fully scoped guest/relay dense ranks without hidden IDs or count fields.
- [x] **AC-09** — live-org/stale fixture passes deletion, disabling/re-enabling, stored-position retention and no preference writes on reads.
- [x] **AC-10** — real mounted HTTP middleware/auth fixture rejects agent/API-key, SA, relay, delegated and OAuth preference writes; supported metadata GET policies preserved.
- [x] **AC-11** — command-monitoring fixture passes exactly one preference read on `/keys`, no absent/empty GET inventory walk, bounded saved GET/detail selected-ID/source/endpoint reads and no provider/credential rendering.
- [x] **AC-12** — paired MCP fixture passes original loader slots, unrelated Slack slots, guest pre-rank visibility, dense ranks and listed non-executable revoked credentials.
- [x] **AC-13** — actual named MCP call fixture compares response, downstream hits and execution audit actor/target/count before/after preferences; mounted slug-proxy and both implicit LLM routes fixture proves identical target/body/audit and exactly six upstream effects. Config/catalog digest/tools-list stay equal; Part B behavior remains unchanged.
- [x] **AC-14** — static `rg` boundary check returns no matches in proxy/execution-authority/approval/billing/pool/LLM/insights files; inspected `execute_tool*` code has no preference reads. Preference loaders are confined to REST metadata and MCP discovery.
- [x] **AC-15** — real DB fixture verifies exact scoped audit `{group,count,version}`, hidden-release `{released_hidden,version}`, no connection IDs, no extra audits on rejected/no-op writes and valid chained append path.
- [x] **AC-16** — 3 CLI unit and 4 real subprocess tests pass grouped show/table/JSON, slug/group resolution, strict scoped body/version,409/capacity errors, reset, confirmed release and rejection of flat set. Reversed saved-order output derives current HTTP ranks; active SSH does not inflate ranks and disabled rows retain saved positions.
- [x] **AC-17** — real browser covers below-header Discovery/Saved pills, pool Priority, duplicate/long labels, usable ≥64px normal/editing label width at390/1024/1440 with readiness/pill non-overlap, contained tablet/mobile table overflow, filtered stable ranks, collapsed chip and actual overview/table renderer. Hook/table test covers HTTP versus SSH saved-position semantics.
- [x] **AC-18** — real browser card/overview disclosure scenarios pass collapsed defaults, production404 reason/status, known pills, dirty draft/URL/filter preservation and narrow layout. Hook tests cover loading/error honesty and null-inference provider explanation without invented URLs.
- [x] **AC-19** — actual mouse/touch/keyboard sensors pass pickup/move/drop, Escape rollback, single live region, disabled-to-end, confirmed default reset and unrelated Slack/toolbar preservation.
- [x] **AC-20** — browser fixtures assert dirty gating, exact scoped request body/order/version, disabled rows included, reset `ordered:[]`, Cancel and no saved-view writes.
- [x] **AC-21** — browser passes successful409 Overwrite/Reload, actual400 stale-ID refresh + failed inventory read/Retry, network retry, capacity confirmation/cancel/release/refetched-version Retry save and production404 no-write behavior.
- [x] **AC-22** — strict grouped/request/release schema tests pass absent response, unknown fields, duplicates, versions,201 IDs and reset shape; >200 local/retry/overwrite errors remain visible without uncaught Zod errors.
- [x] **AC-23** — final full frontend/production build/lint/wizard regeneration, rebuilt-source wizard freshness and Rust formatting pass. Final backend/CLI all-target Clippy passes with `-D warnings`, exit0, no suppression. Remote CI/final review remains ROOT-owned.
- [x] **AC-24** — API/discovery/architecture/chat/CLAUDE docs match scoped merge, relevance-slot/advisory semantics, executable metadata, restricted ranks, protocol distinctions, explicit pools and unchanged implicit gateway. §5.2 incorporates the user's compact disclosure correction.
- [x] **AC-25** — mounted HTTP scopes/slug/config fixture preserves route confinement and OAuth write rejection; dedicated `curation_router_scoped_discovery_history_and_route_confinement` execution also passed against the explicit isolated database.
- [x] **AC-26** — live visibility/property/HTTP fixtures pass hidden/stale/unrelated relative-order preservation, scope-dense responses, reset/no-op and strict body/auth errors. Only explicit hidden release removes unauthorized IDs.
- [x] **AC-27** — browser proves kept-mounted filtered group/full metadata and pool Priority, collapse/other-card/view/tab/navigation guard, overview row History guard, delayed focus restoration, late provenance, entire inventory disappearance/Cancel and no saved-view write.
- [x] **AC-28** — hooks plus actual transport tests pass deferred recovery, deferred MutationCache.onMutate, identity-state reset, actor-keyed caches, retry/transport/late401/JSON fences and Mode A authority replacement. Browser passes30/26 duplicate-label edit/save and one-enabled/one-disabled eligibility.
- [x] **AC-29** — HTTP scoped GET fixture returns only visible catalog groups, never custom singleton groups or hidden/count/capacity metadata.
- [x] **AC-30** — deterministic varied-inventory/property sequences pass distinct-group saves/reset, foreign/hidden/stale relative order, no-op identity, regained-scope omitted append and inserted-ID slot placement without copying the production merge into the test.
- [x] **AC-31** — real DB capacity/release fixture passes fixed200 generic errors/no audit, clear-at-capacity, all-hidden release200 audit, custom/disabled visible retention, second actor unchanged, CAS/no-op/body/auth fences. Browser passes explicit confirmation cancellation and successful retained-draft Retry save.

Desktop/mobile editor and compact404 screenshots are Playwright output artifacts,
not hardcoded product paths:
`/tmp/nyxid-service-preference-inline-final-browser-results/` contains
`inline-desktop.png`, `inline-mobile.png`, `compact-card-mobile.png`, and
`compact-overview-mobile.png`. ROOT independently inspects final visual/source
behavior before publication.
