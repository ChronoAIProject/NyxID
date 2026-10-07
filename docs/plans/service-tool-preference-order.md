# Per-service agent discovery order (inside the service group)

Branch: `service-tool-preference-order`. Planner: Fable 5.1. Status: Part A
implemented, with the latest Reorder discovery / Info-tooltip settings row recorded
in §18. Coverage/Opus corrections, the prior sticky-action revision and
protected CLI release transport remain recorded in §17.
Earlier results in §16 are historical for changed UI interactions. ROOT owns
direct review and publication; Opus 5.5
renewed its source review on published corrections `00eef8d7`, closed the seven
earlier findings from `0ea6cfa3`, and requested three final nits. The bounded
corrections and validation are recorded in §17; final exact-head CI/review
approval remains ROOT-owned on PR #1796.
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
**default server discovery order**: the caller's personal connections first, then
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
active Anthropic connection that returns. The group help says "Provider gateway
routing is separate from this discovery order" (§5.2); the architecture and
discovery docs retain these implementation details. This behavior is the
subject of the Part B question (§6).

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
- The UI pills are the owner's REST projection. The architecture and discovery
  docs explain that a restricted agent's ranks are dense over what that agent
  is allowed to see; the concise product help states only the enabled HTTP
  access rule (§5.2).

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
  `Badge variant="secondary"` reading `Saved #p · disabled`,
  `aria-label="Saved order position p; disabled connections are not listed to
  agents"`, from `preference_position`. An enabled non-HTTP row instead shows
  `Saved #p · SSH` (or its actual protocol), with an accessible explanation that
  it is outside connected MCP discovery. Moving it retains its saved position
  without making it discoverable; it has no discovery rank or MCP tool prefix.
- Both appear wherever the renderer is used (expanded card, table view mode,
  overview page, DEV routing preview) and coexist with the pool `Priority n`
  row and the readiness badge; the terms are distinct by design.

### 5.2 Unified discovery settings row and info tooltip

For every catalog group with two or more connections in its full inventory,
render one shared `ServiceAgentOrderPanel` section below Hide connections in
an expanded card's sticky header, or below the overview tabs in its sticky
section. An active editor keeps its controls if refreshed inventory falls
below two rows. The table has no permanent explanatory area.

One flex row with explicit gaps and natural wrapping contains a content-sized
preference/counts block, its adjacent primary **Reorder discovery** CTA (or the
one Discovery order label/outline Cancel/primary Save group), and a standard
small Info button next to the preference text. No `justify-between`, expanding spacer, separate help row,
ornamental heading or divider distributes these elements.

The compact block has two lines. Its first shows muted **Preferred in
discovery:** and the known connection name in stronger foreground, inline
when short and naturally wrapped when long. Without a known preferred row,
show **Default server discovery order** only after a successful absent-order
read, or **Saved order · no enabled HTTP preference** when a stored order has
no ranked enabled HTTP row. The second line reads `k enabled · d disabled`
from `is_active`; counts do not claim all enabled rows are listed, callable,
working or verified.

Loading/error/404 reads preserve preferred metadata known from `/keys` and
add a short availability line within the same section. Without known metadata,
show **Loading agent order**, **Agent order could not be loaded**, or **Saved
agent order unknown** (404), never default order from a failed read. A404 says
**Saving agent order requires the backend update** and disables entry with the
same reason; this describes preference saving, not Service Pools. Read errors
retain Retry beside the summary/actions.

The named Info button uses the existing Radix/shadcn Tooltip and project
ghost-button styling, with a size-7 hit target and size-3.5 Info icon. The
tooltip opens on mouse hover or keyboard focus. Mouse clicks and keyboard activation keep the tooltip open. Controlled
toggling applies only to touch; composed Radix handlers must not close it before the tap
toggle. The tooltip has three short rules:

1. Connections that match equally are shown in your order.
2. The AI chooses which connection to use; this order is a preference.
3. Agents see only the enabled HTTP connections they can access.

Mixed-protocol groups add **Other
protocol connections keep their saved positions but are excluded from tool
discovery.** Known LLM/provider groups add **Provider gateway routing is separate
from this discovery order.** Raw paths, tool-prefix examples, database/owner
ordering, executable fields, dense ranks and card-count internals remain in
the architecture/discovery docs and §§3–4 rather than the product help.

The tooltip is short, noninteractive and viewport constrained. It contains no
links, buttons, heading or pool navigation; Service Pools retain their existing
settings routes and controls. Opening help leaves table geometry unchanged.
Keyboard focus remains on the Info button, Escape dismisses it, and blur/outside click
close it. On a real touch390 viewport, tap opens, the next tap closes, and an
outside tap dismisses. All preserve draft, filter/search/view state, URL and
saved-view state, and send no preference write. The preference/count block is
plain; one live status/alert element contains the actual current read-state
text, even when known preferred metadata is retained, without duplicate error
copy or announcements.

The expanded Hide connections ghost button retains `bg-overlay text-foreground`,
matching its standard hover state, while `aria-expanded` is true
(`expanded && !routingOpen`). Its exact existing ChevronRight and hit target
are unchanged. This styling applies only to this card control. Previous native
details, split inline help and text-trigger popover designs are superseded.

The entry CTA is **Reorder discovery** and the editing label is **Discovery
order**. Existing setting, status, save, reset and form messages deliberately
retain **agent order**, preserving their established vocabulary.

### 5.3 Collapsed card chip

Next to the `n disabled` badge: `Preferred: <label>` when a rank-1 connection
exists; activating it expands the card. Nothing otherwise.

### 5.4 Ordering mode (inline, one group at a time)

The CTA/editing labels use discovery vocabulary; existing setting/status and
save/reset/form messages retain agent order as specified in §5.2.

Cards use main's one-card-at-a-time close/reflow/reveal sequence. Every
collapse or group switch checks the draft guard before requesting motion.
The editing group forces the card's expansion, span, layout identity and
CardReveal body open exclusively for that group even when filters or a
saved-view restore change the stored expansion. Reconcile the ephemeral
expansion to that group before paint, without saving account defaults, so
Save/Cancel preserve the expanded card and return focus to Reorder discovery.
Dirty forms and delayed focus stay mounted through reflow.
Each logical connection uses one sortable tbody containing its actual main
row, sibling pool-membership row and animated detail/history panel row; all
remain usable during editing and move together with measured group geometry.

- Control: one obvious `Reorder discovery` CTA (`ListOrdered`, `variant="primary"
  size="sm"`) beside the **compact discovery summary**, below Hide connections
  in the expanded card's sticky header. The overview places the same summary
  and action row below its Connections/History tabs, within the sticky section
  outside the content's `overflow-hidden` wrapper. The action follows the
  summary with an explicit gap, wrapping directly below it at narrow widths;
  no flex spacer or `justify-between` pushes it away. There is exactly one
  action set on each surface. Rendered for every catalog group whose **full
  inventory (enabled and disabled) has two or more connections**, including the 30/26
  case. Loading/refetch, read error, production404 and another group's draft
  disable the idle CTA with the stated reason; read errors retain Retry.
  Table view mixes groups and does not offer an ordering action.
- During editing, replace the idle CTA with the readable **Discovery order** context
  label, primary **Save** and outline **Cancel** beside the same discovery summary.
  Do not render a disabled entry button or a second Save/Cancel set below
  the table. The card bar wraps with a minimum height at narrow widths, retaining
  its ResizeObserver-based sticky geometry. The summary and order action group
  stay together with explicit consistent gaps, never `justify-between` or an
  expanding spacer. Hide connections and Service details retain their own row;
  Service details may align separately at its end. The shared settings row,
  availability and adjacent Info trigger are sticky. The tooltip uses the
  existing portal; it does not expand the header or
  connection table. Preserve
  the exact existing Hide connections chevron. Save/Cancel and their context
  remain visible and operable after scrolling at 390/1024/1440 on both surfaces
  (§12 AC-32).
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
- The sticky Save targets a dedicated native order form by its `form` attribute;
  the DndContext, table and unrelated access/billing panel controls are outside
  that form, so their default buttons cannot accidentally submit. The form
  contains only order actions (empty when the sticky bar owns them). Save `variant="primary"`,
  dirty-gated via `useAppForm` field `ordered: string[]` with
  `zodResolver(servicePreferenceGroupRequestSchema)`; edits call `setValue`
  (default `shouldDirty: true`), resets use `{ shouldDirty: false, shouldTouch: false }`.
  Save sends the group's connection ids (enabled and disabled) in table order
  with the current `version`; success invalidates `["service-preference"]` and
  `["keys"]`, exits ordering, toasts "Agent order saved for <service>" and
  returns focus to `Reorder discovery`.
- Save is disabled when pristine, busy, read-blocked, awaiting stale recovery
  or at backend capacity. Cancel remains available except while busy. Errors
  from a scrolled Save receive focus and are revealed below the measured sticky
  section, inside the main scrollport; visibility is proved by geometry and
  hit-testing, not just a visible class. Local 201-row validation and backend
  capacity each render exactly one truthful message. Release success exposes
  Retry save through explicit recovery state, independently of notice wording.
- Recovery: 409 → in-card banner "Agent order changed in another tab." with **Reload order** (refetch, reset this group's rows, stay
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
- Another group's Reorder discovery is disabled while a draft exists; it never
  discards the draft or prompts. Guards: collapsing the card, switching view
  mode or tab, in-app navigation (TanStack `useBlocker`) and leaving the
  overview page ask "Discard unsaved agent order?"; filter changes do not
  prompt because the card stays mounted; identity change discards the draft
  silently and exits ordering.

### 5.5 Reuse

One renderer (`ServiceConnectionTable`) carries pills and ordering for the
card and the overview page; `GroupCard` adds chip, summary line and sticky discovery summary/action-row
controls; one `service-agent-order-panel.tsx` serves both places;
`useServicePreference()` (GET; 404 → `unavailable` sentinel) and
`useSaveServiceGroupOrder()`; schemas in `schemas/service-preference.ts`;
`KeyInfo.preference_rank` and `preference_position`.

## 6. Part B (conditional): implicit gateway routes follow the saved order

Pending the user's asynchronous answer. If **no**: §5.2 keeps the gateway note,
"Provider gateway routing is separate from this discovery order," and nothing
below is built. If **yes**: Part B is
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
- UI: only after Part B is authorized, replace §5.2's "Provider gateway routing
  is separate from this discovery order" note with concise user-facing copy
  explaining the implicit gateway preference and absence of failover. The
  architecture/discovery docs retain the precise owner-tier/default-selection
  behavior. Part A ships the separate-routing note unchanged.
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
`preference_position`), `api_docs.rs`.

CLI: `cli/src/commands/service.rs` (scoped show/set/reset/release commands),
`cli/src/cli.rs` (group flags and help), `cli/src/api.rs` (versioned DELETE body
and protected release reads/refresh/writes with HTTPS-only remote transport,
safe loopback HTTP and no redirects), `cli/tests/service_preference.rs`,
`cli/tests/service_preference_transport.rs` (real URL/TLS/proxy/initial and
refresh-retry redirect boundaries), and `cli/tests/wizard_bundle_freshness.rs`.

Frontend: `components/dashboard/service-connection-table.tsx` (pill line,
ordering mode, Save/Cancel), `components/dashboard/grouped-service-cards.tsx`
(chip, summary line, sticky discovery summary/action-row controls, `orderingGroupId`, kept-mounted rule,
guards), `components/dashboard/service-agent-order-panel.tsx` (one shared sticky
settings row with summary, actions, status, help trigger and bounded on-demand tooltip),
`components/dashboard/service-order-rows.tsx`,
`components/dashboard/service-order-actions.tsx`,
`pages/service-overview.tsx`, `hooks/use-service-preference.ts`,
`hooks/use-service-group-order.ts`, `hooks/use-keys.ts`, `lib/api-client.ts`,
`components/cli-wizard/client.ts`, `schemas/service-preference.ts`, `types/keys.ts`,
and `e2e/wizard-scope.spec.ts`
(real built standalone wizard/Mode A scope list). Removed:
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
counts and `Saving agent order requires the backend update.`, the on-demand
Info tooltip, and the Reorder discovery control disabled with the same
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
cargo test -p nyxid-cli --test service_preference_transport -j 1
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
   (`Discovery #n`, `Saved #p · disabled`); `ordering` prop with sortable rows, handle,
   Move up/down, `Move disabled to end`, `Reset to default`, live pills,
   Save/Cancel, in-card banners incl. the capacity banner with
   **Release unavailable preferences** confirm and **Retry save**;
   all-connections rendering; draft preservation across `/keys` refetch.
10. `grouped-service-cards.tsx`: chip, summary line (enabled/disabled counts,
    explicit order-saving backend status), sticky discovery summary/action-row controls with the stated states and the
    full-inventory ≥ 2 rule, `orderingGroupId`, kept-mounted-while-ordering,
    guards (collapse, view/tab, `useBlocker`, identity); other-group entry disabled.
11. `service-agent-order-panel.tsx` with §5.2's three short rules and conditional
    protocol/gateway notes in the noninteractive Info tooltip; mount the shared
    settings row in the card and `service-overview.tsx`.
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
tests AC-B1..B7; revise §5.2's separate gateway-routing note only for the
authorized implicit-routing behavior; docs.

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
  label; disabled stored rows show `Saved #p · disabled`; active non-HTTP stored rows show
  their actual protocol's saved-position pill and no MCP prefix/discovery rank;
  unstored and `connection:`
  singleton rows show neither; a pool member shows `Priority n` in its
  sibling `data-service-connection-pools` row and its discovery/saved pill
  in the main row of the same logical connection; two rows labelled "Anthropic" are distinguished by slug and pill;
  filters hide rows but never change visible pill text or any row/group
  position; the collapsed chip reads `Preferred: <label>` only with a rank-1
  connection and expands the card. Verified in the expanded card, table view
  mode and `/keys/services/$groupId`.
- **AC-18** (local, *revised*): one shared settings section contains the
  content-sized two-line preference/counts block, adjacent order actions and
  accessible small **Info** trigger with natural gaps/wrapping. Known names have
  stronger foreground; default/saved/loading/error/unknown state remains honest.
  Counts use `is_active` and never claim working/verified/executable status.
  No permanent help area or separate help row exists above the table. The
  on-demand tooltip shows exactly the three §5.2 rules and only applicable
  short protocol/gateway sentences. It has no interactive links or controls;
  Service Pools retain their existing navigation. Technical contracts remain
  in docs. Tooltip bounds fit390/1024/1440, and opening it does not move rows.
  Mouse hover, keyboard focus and real touch tap open it; Escape, blur, outside
  click/tap and a second trigger tap close it without discarding or saving.
  Keyboard focus stays on the Info button. Dark card/overview idle/editing/
  open-tooltip screenshots prove the unified design.
  Loading/error/404 status, Retry and one disabled CTA with matching reason
  remain discoverable, including30/26. One live element announces read state
  even with a known preference. Tooltip toggles preserve draft, filters,
  search, Personal/All, auto-connected, saved view and URL and send no write;
  table horizontal overflow remains contained at narrow widths.
- **AC-19** (real-route Playwright with real sensors, plus unit tests for
  buttons, *revised*): in the expanded 30-connection card, mouse-dragging a
  disabled stored row above an enabled one updates `Saved #p · disabled` and `Discovery
  #n` pills consistently; keyboard dragging with Escape cancellation; touch
  dragging at a 390px viewport with contained table scrolling and no document
  horizontal overflow; Move up/down,
  `Move disabled to end` and `Reset to default` (with confirm) behave as
  §5.4; pool membership and open detail panels move as one sortable connection
  in mouse, keyboard and touch proofs; the single dnd-kit live region announces; the Slack card, toolbar,
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
  implicit-gateway explanation. User-facing architecture docs describe the
  unified sticky preference/counts/Reorder discovery settings row and its
  brief noninteractive Info tooltip, with technical contracts retained in docs.
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
  kept-mounted and revealed card under filter/Personal-All/saved-view changes
  including reduced motion, guards before collapse/other-card/view/tab/navigation, disabled other-group entry, focus return, and that saved view, filters, store
  and other cards are unchanged (no `PUT /users/me/preferences/services`).
- **AC-28** (*revised*): read failure cannot enable saving; a two-connection
  group (one enabled, one disabled) can be ordered; the 30/26/duplicate-label
  group renders, drags complete connection row groups and saves; main service-card
  scroll alignment/sticky geometry passes in normal and reduced motion; identity change discards the draft and exits
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

- **AC-32** (real-route browser + form boundary tests, new): at **390/1024/1440**
  on the actual expanded card and overview, idle shows exactly one obvious
  primary Reorder discovery CTA beside the discovery summary, below the separate
  Hide connections or overview tabs row. Editing replaces it with one readable
  Discovery order context label and exactly one primary Save/outline Cancel set
  beside that same summary. Real bounding boxes prove the gap is 8–16px when
  beside it, or at most 8px with aligned left edges when naturally wrapped
  below it; no `justify-between` or expanding spacer distributes these controls.
  There is no disabled entry button during editing or duplicate table action set.
  The summary has separate preference and count lines, with a named Info button
  beside the preference text in the same shared settings section. No text help
  button, standalone help row or expanding prose appears above the table;
  the brief tooltip fits the viewport and leaves table positions unchanged.
  Dark idle, editing and open-tooltip captures prove this on card/overview390/1440, while
  all six390/1024/1440 width cases retain strict proximity/sticky proof.
  Real bounding boxes and hit-testing prove the controls remain inside
  the main scrollport and viewport, do not overlap or cause horizontal overflow,
  and work after scrolling; card wrapping preserves ResizeObserver sticky
  geometry and the exact Hide connections chevron. The expanded-only Hide
  connections control retains standard ghost hover `bg-overlay text-foreground`,
  a stable hit target and truthful `aria-expanded`. Overview sticks to main,
  outside the overflow-hidden content wrapper, with an opaque background
  covering main's 16px mobile / 24px wider top gutter. Passing rows cannot show
  above the actions, and the cover leaves normal unscrolled metadata visible.
  Save uses the dedicated form's
  actual schema validation, dirty/busy/read-blocked/stale/capacity gates; Cancel
  is available except busy. Real mouse Save and keyboard Cancel restore focus
  after delayed inventory refetch. A scrolled invalid or failed Save reveals
  one focused error below the sticky cover; all 201 IDs, local no-PUT proof and
  confirmed reset remain. Reset is physically reachable below the full sticky
  cover and hit-tested before a real pointer click; confirmation retains all
  201 rows and the next Save sends exactly `ordered:[]` plus `expected_version`.
  Clicking Show all/fewer keys in the real access panel
  while dirty sends no preference PUT and keeps the order. No unrelated service,
  filter, URL or saved-view write occurs.

- **AC-33** (CLI transport security, new): credential-bearing hidden release
  requires verified HTTPS for remote destinations before any preliminary read,
  token exchange or DELETE. Only exact `localhost`, `127.0.0.1`, `[::1]` HTTP
  destinations are accepted; local HTTP bypasses proxies and localhost resolves
  to loopback. Reject remote/private-network HTTP, deceptive hostname suffixes,
  IPv4-mapped loopback, userinfo, fragments and unsupported schemes. The actual
  client enforces `https_only` remotely and refuses all redirects for initial
  requests, refresh and DELETE retry. Real subprocess fixtures prove: untrusted
  CA rejected; configured CA accepted; saved-profile401 refresh and retried body/
  credentials correct; initial and post-refresh HTTPS→HTTP redirects transmit
  nothing to the target; refresh itself cannot redirect; safe localhost HTTP
  bypasses configured proxies and never follows a redirect. Existing profile
  identity fences, TLS trust/environment, telemetry consent, explicit-token
  no-refresh behavior and unrelated generic helpers remain intact. No scanner
  suppression or dismissal. Final published exact-head security evidence is
  ROOT-owned and requires: successful CI Pipeline including wizard freshness;
  all four current CodeQL scanners (`actions`, `javascript-typescript`, `python`,
  `rust`) successful; all four merge analyses bound to that exact PR head have
  `results_count:0` and empty warnings/errors; the aggregate introduces no new
  alerts; alerts465/466 are fixed, not dismissed; and no PR open alerts remain.
  If the aggregate is NEUTRAL solely because of the inherited obsolete
  `.github/workflows/codeql.yml:codeql` category, record it literally as NEUTRAL,
  with provenance and cross-PR precedent proving the four current language
  categories cover that superseded configuration. Do not label it SUCCESS.
  Any additional missing configuration, analysis error or new alert blocks
  delivery. A local pass cannot establish this remote evidence, and no analysis
  deletion, synthesized SARIF, suppression or discarded gate is authorized.

## 13. Decisions

Decided by ROOT (no further approval needed): disabled connections remain
draggable and keep their saved position with a distinct `Saved #p · disabled` pill; the
unreleased global `PUT` is replaced by the scoped contract and the CLI
contracts updated accordingly; wording uses "enabled/disabled" counts and
"default server discovery order", never listed/callable/working/verified.

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
per-service overview (pill line below the readable wrapping label/readiness header,
  unified sticky two-line summary/Reorder discovery/Info row, compact honest
server-state line and on-demand brief tooltip, inline
drag/keyboard ordering with Save/Cancel, reset and in-banner capacity
recovery), leaving the #1685 grid,
toolbar, filters, saved views, insights and pools unchanged. Part B is
specified conditionally with a dedicated gateway entry point and fences.
14 completed Part A tasks plus one conditional Part B task, 33 Part A acceptance
criteria plus 7 conditional Part B criteria.

## 15. Evidence policy

Historical evidence from `e72036b7` stands for unchanged boundaries: AC-01
model tests, the CAS/no-op/legacy-row mechanics reused by `replace_group`
(re-run anyway as part of AC-07), route rejection layers (AC-10, AC-25), the
`/keys` detail read bound (AC-11), and the `#[cfg(test)]` wrapper arrangement.
Every criterion marked *revised*, every new criterion (AC-29 to AC-33), and
every UI criterion requires fresh execution against the §10 commands and the
MongoDB 8.0.17 review instance, recorded with command and outcome before
sign-off. The §16 matrix checks criteria only from actual passing execution
and the stated source/document review; historical evidence alone does not
complete a revised criterion.


## 16. Part A implementation and fresh review evidence

The global editor has been removed. Part A uses the existing grouped/table/
overview architecture, scoped CAS writes and same-group discovery-slot refill.
The logs below predate the user's final Info-tooltip correction. The current
§5.2 unified settings row and its fresh evidence are recorded in §18; native
details and text-trigger popover evidence here are historical for the changed UI.
The historical source used a single header entry and table-bottom Save/Cancel.
The latest user requirement replaces those controls with the sticky action set
specified in §5.4 and AC-32. Fresh coverage and frozen-source UI gates are being
recorded in §§17–18; §18 carries the current settings-row evidence. The
historical browser result below does not prove the revised AC-32. ROOT owns
publication and the separate review
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

Historical acceptance evidence for AC-01–31 before the coverage/Opus/sticky
revision (changed UI criteria require the fresh §17 execution; this does not
close ROOT's separate review findings or grant Opus sign-off):

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
- [x] **AC-18 historical execution** — the then-current card/overview disclosure passed production404 reason/status, known pills, dirty draft/URL/filter preservation and narrow layout. This does not prove the revised settings-row/Info-tooltip criterion; fresh evidence is recorded in §18. Hook coverage of protocol/gateway/read honesty is retained and rerun there.
- [x] **AC-19** — actual mouse/touch/keyboard sensors pass pickup/move/drop, Escape rollback, single live region, disabled-to-end, confirmed default reset and unrelated Slack/toolbar preservation.
- [x] **AC-20** — browser fixtures assert dirty gating, exact scoped request body/order/version, disabled rows included, reset `ordered:[]`, Cancel and no saved-view writes.
- [x] **AC-21** — browser passes successful409 Overwrite/Reload, actual400 stale-ID refresh + failed inventory read/Retry, network retry, capacity confirmation/cancel/release/refetched-version Retry save and production404 no-write behavior.
- [x] **AC-22** — strict grouped/request/release schema tests pass absent response, unknown fields, duplicates, versions,201 IDs and reset shape; >200 local/retry/overwrite errors remain visible without uncaught Zod errors.
- [x] **AC-23** — final full frontend/production build/lint/wizard regeneration, rebuilt-source wizard freshness and Rust formatting pass. Final backend/CLI all-target Clippy passes with `-D warnings`, exit0, no suppression. Remote CI/final review remains ROOT-owned.
- [x] **AC-24 historical review** — API/discovery/architecture/chat/CLAUDE docs matched scoped merge, relevance-slot/advisory semantics, executable metadata, restricted ranks, protocol distinctions, explicit pools and unchanged implicit gateway. The revised product help and current documentation coherence require the fresh §18 review.
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


## 17. Coverage, Opus and sticky-action revision

Prior reviewed head: `0ea6cfa3e41eea4cbc03d65e9c4e8d24da5d3fbb`.
Those corrections were published on `00eef8d7`. ROOT maintains the final exact
head binding in the PR body. Opus 5.5 requested seven corrections on the prior
head; the user subsequently required sticky Agent order/Save/Cancel actions
and deliberate spacing (AC-32). ROOT identified
new CodeQL cleartext-transmission alerts 465/466 on `delete_with_body`, now covered
by AC-33. This section supersedes changed frontend interaction evidence in §16;
backend production source remains unchanged. ROOT's review record is preserved
and ROOT alone closes findings, publishes and requests renewed review.

The revised AC-18, AC-24 and AC-32 for the final unified row / Info
tooltip require fresh rechecks, recorded in §18. The following sticky-toolbar
and inline-disclosure logs are historical for the changed UI, while remaining
evidence for unchanged coverage, identity, persistence and security boundaries.

Implemented corrections: one exact server capacity message; one local limit
message; explicit ready-to-retry recovery state; unrelated outline change
removed; access-grant explanation tied to the card's count; CLI help specifies
within-service discovery ordering; active plan file map, disabled pills, default
and saved/no-enabled-HTTP summary wording and other-group-disabled behavior
reconciled. Sticky native external Save submits only the dedicated order form;
the DndContext/table and unrelated panel buttons stay outside it. The actual
card and overview action bars keep primary controls together with gaps and
wrapping, retain the Agent order context during editing and contain exactly one
Save/Cancel set. Submission count triggers error reveal for repeated same-error
Save attempts, using the associated external submit control to measure the
sticky cover. Source is frozen for frontend validation; later source changes
must rerun affected gates before claiming completion.

ROOT's screenshot review identified row text passing through the overview's
scrollport gutter above its sticky bar. The final CSS correction extends the
bar's background by the actual main padding (16px mobile / 24px at `sm` and
wider); a 4px wider-layout inset keeps that cover clear of preceding metadata
before scrolling. The three overview browser cases prove the normal metadata
boundary, gutter hit-testing and existing action/error geometry. Fresh mobile
and desktop screenshots were inspected. This final CSS-only change followed
the full frontend/coverage/browser executions below and was verified with the
affected overview unit/browser gates plus production build/lint. It does not
change the wizard's 171-source producer closure.

The new CLI release transport reuses the shared TLS/telemetry/profile builder
and the provider OAuth endpoint policy. It validates the destination and binds
release preliminary reads, refresh and DELETE to a no-redirect HTTPS/explicit
loopback client. This command renews saved sessions on401 using its protected
client rather than the generic preflight; explicit keys never refresh, and live
profile destination/login-generation fences remain. `delete_with_body` itself
also validates and protects its initial and refresh-retry requests. Older
helpers are unchanged apart from shared builder extraction and reuse of the
existing refresh logic with an explicitly supplied client.
The protected constructor first builds the existing read-only authenticated
client, then validates and binds policy to that exact returned base URL; it
does not resolve the profile destination twice or send HTTP before binding.

Coverage investigation preserves the original test in
`/tmp/nyxid-service-preference-coverage-boundary/original-use-service-group-order.test.tsx`
and original CI log `/tmp/nyxid-service-preference-0ea-ci-frontend-coverage.log`.
CI timed out at5516ms with V8 instrumentation. Original focused V8 execution
passed locally at2452ms, so the exact timeout was not reproduced in isolation.
The optimized test uses scoped row/control queries and direct native form
association, and omits unopened, unrelated billing tooltip portal trees in this
hook fixture. All201 IDs, the permutation, visible single local error, zero PUT
and confirmed reset with exact empty-order/version body remain asserted. Final
focused V8 passed9 tests with the boundary at1864ms (24% below the original
focused measurement); no test/global timeout, coverage exclusion or threshold
was changed. Its focused measurement uses the same threshold0 convention as CI;
the full coverage run retains the repository's15% line threshold.

Fresh executions are recorded below only after completion. The unchanged
backend DB/MCP/curation/neighboring evidence in §16 remains applicable; changed
frontend, CLI and generated-wizard gates require this revision's results.

| Gate | Command/result | Log |
|---|---|---|
| Original focused V8 measurement | `NODE_ENV=test npx vitest run --config /tmp/nyxid-service-preference-vitest.config.mts src/hooks/use-service-group-order.test.tsx --coverage --coverage.thresholds.lines=0 --coverage.reportsDirectory=/tmp/nyxid-service-preference-coverage-boundary/original-coverage --maxWorkers=2 --reporter=verbose`:9 passed, boundary2452ms, total15.33s | `/tmp/nyxid-service-preference-coverage-boundary/original-focused.log` |
| Frozen-source focused V8 | Same isolated command, reports `/tmp/nyxid-service-preference-sticky-focused-coverage`:9 passed, boundary1864ms, total8.34s, exit0 | `/tmp/nyxid-service-preference-sticky-focused-coverage.log` |
| Intermediate sticky spacing/geometry | `npx playwright test e2e/service-preference.spec.ts --grep 'sticky|unrelated access|capacity release' --workers=2 --output=/tmp/nyxid-service-preference-sticky-spacing-browser-results`:9 passed30.1s; card/overview390/1024/1440 raw-click/focus/error hit-testing, single capacity copy, real access panel and201 reset | `/tmp/nyxid-service-preference-sticky-spacing-browser.log` |
| Rebuilt wizard producer | `npm run build:wizard`:exit0;171-source manifest, source hash6a459342b100… | `/tmp/nyxid-service-preference-sticky-wizard-build.log` |
| Full V8 coverage, before final overview gutter CSS | `NODE_ENV=test npx vitest run --config /tmp/nyxid-service-preference-vitest.config.mts --coverage --coverage.reportsDirectory=/tmp/nyxid-service-preference-sticky-full-coverage --maxWorkers=2`:466 files/4773 tests passed254.38s, exit0;73.16% lines, normal15% threshold | `/tmp/nyxid-service-preference-sticky-full-coverage.log` |
| Full frontend, before final overview gutter CSS | `NODE_ENV=test npx vitest run --config /tmp/nyxid-service-preference-vitest.config.mts --maxWorkers=2`:466 files/4773 tests passed294.60s, exit0 | `/tmp/nyxid-service-preference-sticky-full-frontend.log` |
| Full feature + rebuilt wizard browser, before final overview gutter CSS | `npx playwright test --config /tmp/nyxid-service-preference-sticky.playwright.config.mts e2e/service-preference.spec.ts e2e/wizard-scope.spec.ts --workers=2 --output=/tmp/nyxid-service-preference-sticky-frozen-browser-results`:29/29 passed1.4m, exit0; includes repeated network/201 validation attempts and physical sticky cover/hit-testing | `/tmp/nyxid-service-preference-sticky-frozen-browser.log` |
| Final overview gutter browser | Same explicit4645 config, `e2e/service-preference.spec.ts --grep 'overview sticky Agent order' --workers=2 --output=/tmp/nyxid-service-preference-sticky-gutter-browser-results`:3/3 passed34.0s, exit0;390/1024/1440, unscrolled metadata, gutter cover, raw Save/keyboard Cancel and repeated-error hit-testing | `/tmp/nyxid-service-preference-sticky-gutter-browser.log` |
| Final overview unit | Isolated Vitest config, `src/pages/service-overview.test.tsx --maxWorkers=2`:7/7 passed7.02s, exit0 | `/tmp/nyxid-service-preference-sticky-gutter-unit.log` |
| Final production build | `npm run build`:exit0, including credential-accept output and mock-footprint assertion | `/tmp/nyxid-service-preference-sticky-gutter-build.log` |
| Final lint | `npm run lint -- --no-warn-ignored`:exit0;0 errors,29 unrelated baseline warnings, no feature warnings | `/tmp/nyxid-service-preference-sticky-gutter-lint.log` |
| Frozen Rust formatting | `cargo fmt --all -- --check`:exit0 | `/tmp/nyxid-service-preference-sticky-final-fmt.log` |
| Rebuilt CLI targets | `CARGO_TARGET_DIR=/tmp/nyxid-service-preference-target CARGO_INCREMENTAL=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_PROFILE_DEV_DEBUG=0 cargo test -p nyxid-cli --bin nyxid --test service_preference --test service_preference_transport --test wizard_bundle_freshness --test network_doctor --no-run -j 1 --message-format=json`:exit0,3m37s including the existing target-lock wait | `/tmp/nyxid-service-preference-sticky-cli-compile-recheck.log`; compiler artifacts in the corresponding `.jsonl` and `/tmp/nyxid-service-preference-sticky-cli-artifacts.json` |
| CLI preference unit | Rebuilt `nyxid-002d4b25df51439a service_preference --nocapture --test-threads=1`:3/3 passed0.02s, exit0 | `/tmp/nyxid-service-preference-sticky-cli-unit.log` |
| CLI API boundaries | Same rebuilt binary, `api::tests --nocapture --test-threads=1`:5/5 passed0.04s, exit0; URL policy, caller-selected credentials never refresh/switch identity, typed errors and neighboring proxy refresh | `/tmp/nyxid-service-preference-sticky-cli-api.log` |
| Shared TLS boundaries | Same rebuilt binary, `tls::tests --nocapture --test-threads=1`:17/17 passed0.13s, exit0; trusted CA, hostname/issuer checks, native fallback, HTTP/WSS and process cache | `/tmp/nyxid-service-preference-sticky-cli-tls.log` |
| New release transport subprocess | Rebuilt `service_preference_transport-01e482d1a7f1a803 --nocapture --test-threads=1`:4/4 passed6.81s, exit0; real private CA/untrusted rejection, protected initial and401-refresh retry, safe local proxy bypass, zero requests at initial/retry/refresh downgrade targets | `/tmp/nyxid-service-preference-sticky-cli-transport.log` |
| CLI preference subprocess | Rebuilt `service_preference-bdaeb69b7981a5b3 --nocapture --test-threads=1`:4/4 passed2.06s, exit0; show/table/JSON, scoped set/reset/release confirmation/capacity/conflict and returned HTTP ranks/disabled positions | `/tmp/nyxid-service-preference-sticky-cli-integration.log` |
| Network/profile diagnostics subprocess | Rebuilt `network_doctor-7c3c69239883b48c --nocapture --test-threads=1`:5/5 passed3.30s, exit0; selected profile, CA/proxy configuration, redaction, fail-fast and help | `/tmp/nyxid-service-preference-sticky-cli-network.log` |
| Rebuilt-source wizard freshness | Rebuilt `wizard_bundle_freshness-41d9c8f58e4486a5 --nocapture`:1/1 passed0.07s, exit0 | `/tmp/nyxid-service-preference-sticky-cli-freshness.log` |
| Final CLI all-target Clippy | `CARGO_TARGET_DIR=/tmp/nyxid-service-preference-target CARGO_INCREMENTAL=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_PROFILE_DEV_DEBUG=0 cargo clippy -p nyxid-cli --all-targets -j 1 -- -D warnings`:exit0,1m49s, no suppression | `/tmp/nyxid-service-preference-sticky-final-cli-clippy.log` |

The direct CLI executions above use fresh compiler-artifact paths under
`/tmp/nyxid-service-preference-target/debug/deps/`; no stale executable was
selected by filename guessing. Each passed with zero failed/ignored tests.
The final gutter edit is outside the wizard manifest: all171 inputs plus
extras still compute `6a459342b1003e3be417e4a0645c2867afffafc5a0e9efb7dc809e688818f860`,
matching the regenerated recorded hash. The closure comparison is preserved
in `/tmp/nyxid-service-preference-sticky-gutter-closure.json`.

All requested local gates passed. Revised frontend interaction evidence for
AC-17–23, AC-27–28 and AC-31 is refreshed by the complete29-case browser run,
full frontend/coverage gates and final affected overview checks; revised CLI
evidence for AC-16 and the new transport boundary is recorded above. Existing
unchanged backend evidence remains in §16.

- [x] **AC-32 local execution** — six real card/overview width cases plus the
  final three overview gutter rechecks prove idle/editing sticky actions,
  deliberate spacing, readable context, native external form validation,
  dirty/busy/read/error gates, physical mouse Save/keyboard Cancel, delayed
  focus, repeated identical errors below the sticky cover, and no unrelated
  access-panel submission. Final390/1440 screenshots were inspected.
- [ ] **AC-33 final exact-head security evidence** — local URL/TLS/subprocess/profile/
  network and static gates pass, including actual initial/refresh/retry
  downgrade refusal and safe loopback proxy bypass. The final exact head still
  requires the literal AC-33 CI/scanner/analysis/alert rule above; it cannot be
  claimed from local tests or historical analyses. No alert was suppressed or
  dismissed. ROOT records final head/merge bindings and facts in the PR body.

Final exact-head CI and review approval remain ROOT-owned. The renewed Opus
source review and its bounded nit corrections are recorded below. No remaining
local failure or implementation limitation was found by the requested executions;
this evidence does not close ROOT's review
record or grant sign-off. Source is frozen with no further source edits pending.
Both4630 production-backed and4631 user-authorized sample preview servers are
preserved; no production writes, new agents, commits or pushes were performed.

The standard4611 port was occupied by another worktree (`fluffy-comet`) when
the first full browser command reused its server. That interrupted run is
invalid evidence: `/tmp/nyxid-service-preference-sticky-final-browser.log`.
The passing frozen run used a temporary4645 config importing the repository
Playwright config, with explicit winter-river cwd, strict port and
`reuseExistingServer:false`. It preserved the unrelated4611 server and both
review servers. Correct-source screenshots are under
`/tmp/nyxid-service-preference-sticky-frozen-browser-results/`, including
`sticky-card-{390,1024,1440}.png` and `sticky-overview-{390,1024,1440}.png`.
The first CLI compile command waited on an existing unrelated backend build
holding `/tmp/nyxid-service-preference-target/debug/.cargo-lock`; the waiting
CLI job was stopped before compiling, preserving that backend process.

### Renewed Opus source-review nits after published corrections `00eef8d7`

Opus's renewed source review on
`00eef8d7bfb0c44e8bad4ae85659f1abc257ffd7` confirmed correctness, security and
execution behavior and closed the seven earlier findings. It returned three
nits, all addressed in `a60afb9c`: remove the unused panel
`action` prop/type/render slot; make the exact Hide connections chevron
requirement a complete sentence; and distinguish the prior reviewed `0ea6cfa3`
from published corrections `00eef8d7` in this section. The architecture text
also specifies one primary Save button and one outline Cancel button.
The final exact published-head binding remains in ROOT's PR body.

Both production callers omitted the action prop. Removing its undefined slot
changes no rendered DOM element, class, spacing or chevron. The existing flex
row remains necessary for the conditional Retry reads button. No visible
geometry changed, so the existing card/overview CTA browser evidence remains
applicable and was not blanket rerun. Only the affected hook/panel and real
page unit files, production type/build checks and lint were executed:

| Gate | New command/result | New log |
|---|---|---|
| Affected panel/hook/card/overview unit | `NODE_ENV=test npx vitest run --config /tmp/nyxid-service-preference-vitest.config.mts src/hooks/use-service-group-order.test.tsx src/pages/keys.test.tsx src/pages/service-overview.test.tsx --maxWorkers=2`:3 files/45 tests passed9.44s, exit0; isolated happy-dom origin4629 | `/tmp/nyxid-service-preference-opus-nits-unit.log` |
| Component/caller types and production build | `npm run build`:exit0; includes `tsc -b`, production/credential-accept outputs and mock-footprint assertion | `/tmp/nyxid-service-preference-opus-nits-build.log` |
| Lint | `npm run lint -- --no-warn-ignored`:exit0;0 errors,29 unrelated baseline warnings, no feature warnings | `/tmp/nyxid-service-preference-opus-nits-lint.log` |
| Wizard closure preservation | Recomputed all171 manifest inputs plus extras: recorded and computed hash both `6a459342b1003e3be417e4a0645c2867afffafc5a0e9efb7dc809e688818f860`; changed panel is outside this closure | `/tmp/nyxid-service-preference-opus-nits-wizard-closure.json` |

Historical logs and freshness evidence above are preserved. This correction
changes no wizard producer input, so no bundle regeneration or unrelated
CLI/Rust/backend/full frontend execution was required. All bounded local gates
passed; no further source edit is pending. ROOT's review record remains
untouched by this work. Both4630/4631 previews and the user's separate sample
persistence remain available; no production writes, agents, commits, pushes,
merges or deployments were performed.

### Second Coverage CI failure and isolated 201-ID hook boundary

Coverage (Frontend) job `112646436406` on published corrections `00eef8d7`
failed the same 201-row hook test at the unchanged5000ms timeout:
4772 tests passed/1 failed across466 files, total301.37s. ROOT downloaded the
completed job log directly from the API:
`/tmp/nyxid-service-preference-00e-ci-frontend-coverage.log` (failure around5270).
No disk error caused this failure. The earlier scoped-query/tooltip
optimization and local full pass did not sufficiently bound CI renderer cost;
that conclusion is superseded by this correction. The preceding logs and
pre-correction test snapshot are preserved, including
`/tmp/nyxid-service-preference-coverage-boundary/00eef8d7-use-service-group-order.test.tsx`.

The 201-ID case now mounts the real `useServiceGroupOrder` hook, its real
`useAppForm`/Zod resolver, the real `ServiceOrderActions` and a native externally
associated form with the hook's actual validation message. It avoids201
connection/Dnd/Radix subtrees. All201 IDs remain in the actual inventory and
draft: the exact moved permutation is asserted; external Save increments the
real form submit count, shows one visible limit message and sends no mutation;
confirmed Reset retains the complete inventory; a second native Save submits
exactly `{ordered:[], expected_version:1}`. Validation is not mocked and hook
logic is not copied into the fixture.

A separate three-row integration case retains the actual table, row movement,
real actions, native form association, pristine/dirty Save gating, table
exclusion from the form and the exact saved permutation/version. The previous
tooltip mock is removed; small-row renderer cases use the actual primitives.
The full real-route201-row Playwright case is unchanged and remains the
full-table integration/physical-error-visibility proof. Production source is
unchanged by this coverage correction; the three-nit source/doc diff above is
retained. No timeout, retry, threshold or coverage exclusion was relaxed.

| Gate | New command/result | New log |
|---|---|---|
| Focused V8 hook/form coverage | `NODE_ENV=test npx vitest run --config /tmp/nyxid-service-preference-vitest.config.mts src/hooks/use-service-group-order.test.tsx --coverage --coverage.thresholds.lines=0 --coverage.reportsDirectory=/tmp/nyxid-service-preference-201-hook-focused-coverage --maxWorkers=2 --reporter=verbose`:10/10 passed7.28s, exit0;201-ID boundary93ms, real three-row table/form143ms; original default timeout | `/tmp/nyxid-service-preference-201-hook-focused-coverage.log` |
| Focused V8 after fixture type correction | Same isolated command, reports `/tmp/nyxid-service-preference-201-hook-focused-recheck-coverage`:10/10 passed8.01s, exit0;201-ID boundary92ms, real three-row table/form208ms | `/tmp/nyxid-service-preference-201-hook-focused-recheck.log` |
| Production build/type recheck | `npm run build`:exit0; component/test types, production and credential-accept outputs, mock-footprint assertion | `/tmp/nyxid-service-preference-201-hook-build-recheck.log` |
| Complete V8 after space recovery, settled fixture | `NODE_ENV=test npx vitest run --config /tmp/nyxid-service-preference-vitest.config.mts --coverage --coverage.reportsDirectory=/tmp/nyxid-service-preference-201-hook-full-recheck-coverage --maxWorkers=2`:466 files/4774 tests passed215.80s, exit0;73.17% lines with the unchanged15% threshold and default5000ms test timeout | `/tmp/nyxid-service-preference-201-hook-full-recheck.log` |
| Final lint recheck | `npm run lint -- --no-warn-ignored`:exit0;0 errors,29 unrelated baseline warnings, no feature warnings | `/tmp/nyxid-service-preference-201-hook-lint-recheck.log` |
| Wizard closure preservation | All171 manifest inputs plus extras recomputed; recorded/computed hash remains `6a459342b1003e3be417e4a0645c2867afffafc5a0e9efb7dc809e688818f860`; changed test is outside the closure | `/tmp/nyxid-service-preference-201-hook-wizard-closure.json` |

The first build found strict indexed-access TS2345 in the new fixture's known
201-item permutation. Explicit bounds assertions fix that type error without
changing runtime JavaScript; the failure log is retained at
`/tmp/nyxid-service-preference-201-hook-build.log`. Focused V8 passed again after
that correction. The first full local coverage run exited1 while writing a
temporary shard with ENOSPC:
`/tmp/nyxid-service-preference-201-hook-full-coverage.log`. It is invalid coverage
evidence, separate from the real CI rendering timeout. Its12MiB intermediates
and log remain preserved. Disk availability recovered to6.1GiB before rerun;
no other target or active process was modified. The full recheck uses the
settled fixture and a separate reports/log path. Existing non-failing React
act warnings in two neighboring hook cases also occur in the preceding focused
log; they are not new diagnostics from the201-ID correction.

The bounded hook/form fixture addresses the second CI rendering-cost finding
and passes the complete local V8 run; no remote pass or ROOT review closure is
inferred from that result.
All requested local gates for this correction passed. The original default
timeout and full coverage threshold remain unchanged; the focused measurement
uses the same existing threshold0 convention as the preceding measurements.
The full201-row browser integration remains unchanged. No unrelated Rust/CLI/
backend or additional browser runs were performed. Source is frozen with no
further code edits pending; ROOT owns publication, current remote CI status,
review closures and the final exact-head binding. ROOT's review record and both
previews/sample persistence remain untouched by this work.

### Historical CodeQL evidence on published corrections `00eef8d7`

ROOT's independent read-only inspection and Opus's read-only nuance audit
reported the following facts for `00eef8d7`, bound to merge `080a279e`:
all four scanner jobs SUCCESS; all four merge analyses `results_count:0` with
empty warnings/errors; alerts465/466 fixed, not dismissed; and PR open alerts0.
Aggregate `112646549519` is **NEUTRAL**, not SUCCESS, solely because of the
inherited obsolete `.github/workflows/codeql.yml:codeql` configuration.
The feature changes no `.github` workflow.

Provenance: the obsolete main category's last upload was2026-09-21 at
`0c314264`. The current matrix categories are `actions`,
`javascript-typescript`, `python` and `rust`, with main uploads on Sept28/Oct5.
Matrix change #1669 landed in rollup `567dd3ed` on Sept27. The same inherited
NEUTRAL condition occurs on merged #1685/#1794/#1795/#1797 and open #1789.
The older `0ea6cfa3` aggregate FAILED for two new high-severity alerts despite
the same obsolete missing category, demonstrating that new-alert detection was
not masked. This is historical provenance, not the final head's security pass.

The final published exact head must satisfy AC-33's literal CI/scanner/merge-
analysis/alert rule. A solely inherited obsolete-category NEUTRAL outcome must
retain that label and its proof; any additional missing configuration, analysis
error or new alert blocks delivery. Final head and test-merge SHA bindings,
job/analysis facts and review approval remain in ROOT's PR body. No old analysis
was deleted, no SARIF synthesized and no scanner finding suppressed or dismissed.


## 18. Restart recovery and final Info-tooltip validation

Fresh local evidence on merged `631b4994`, after the final Info-tooltip and
Reorder discovery correction, was reconstructed on 2026-10-07. The previous
`/tmp` target, config and historical logs were absent after restart. Prior
sections remain historical records; none of those missing logs is represented
as a newly executed result. All new logs below live under
`/tmp/nyxid-service-preference-restart/`.

The accepted modality correction records the pointer type on pointer-down:
mouse clicks and keyboard activation keep help open, while touch taps toggle.
Real card and overview routes assert hover then click and focus then Enter;
the separate touch case retains second-tap and outside-tap dismissal. Existing
agent-order setting/status/save/reset/form wording is deliberate; the entry
CTA is Reorder discovery and the editing label is Discovery order. Keyboard
focus remains on Info; pointer-down does not claim to move mouse focus there.
ROOT accepted the implementation and final screenshots. ROOT owns the review
record, publication, exact-head security evidence and final Opus sign-off.

The isolated Vitest config imports the current repository Vite config, retaining
its plugins, setup, inclusion/exclusion lists, default 5000ms timeout and 15%
line-coverage threshold. It changes only the happy-dom origin to localhost4629
and binds the current frontend root. The temporary `vitest.config.mts` and
`queued-suites.json` retain exact setup/commands; per-gate exit statuses are in
`frontend-results.json` and `rust-results.json`.

| Gate | Fresh result | New evidence under the directory above |
|---|---|---|
| Frozen affected UI/hooks/schema/transport units | 10 files / 100 tests passed, exit0 | `frozen-focused.log` |
| Settled Node22 analytics route | 1/1 passed; test1335ms, unchanged5000ms timeout | `node22-analytics-isolated.log` |
| Complete Node22 unit suite | 467 files / 4777 tests passed;160.44s, exit0 | `node22-full-unit.log` |
| Complete Node22 V8 | 467 files / 4777 tests passed;73.17% lines with unchanged15% gate, exit0 | `node22-full-v8.log`, `node22-coverage/coverage-summary.json` |
| Node22 production build / lint | Both exit0; production and credential-accept build plus mock-footprint assertion; lint0 errors /29 unrelated baseline warnings | `node22-build.log`, `node22-lint.log` |
| Node22 wizard rebuild / source closure | Exit0;171 manifest files plus extras match `6a459342b1003e3be417e4a0645c2867afffafc5a0e9efb7dc809e688818f860` | `node22-wizard-build.log`, `wizard-source-closure.json` |
| Frozen real-route browser | Service-preference plus wizard-scope suites30/30 passed,3.2m, exit0; desktop hover-click/focus-Enter, touch dismissal, all six390/1024/1440 sticky cases and201-row validation proof | `frozen-browser.log`, `frozen-browser-results/` |
| Backend preference / database | 14/14 passed,0 failed/ignored,4.40s after recovered cold compile | `backend-preference.log` |
| Backend search / agent relevance / scoped route confinement | 5/5,1/1,1/1 passed;0 failed/ignored | `backend-search.log`, `backend-agent-search.log`, `backend-curation.log` |
| CLI unit / subprocess / transport / rebuilt wizard freshness | 3/3,4/4,4/4,1/1 passed;0 failed/ignored | `cli-order-unit.log`, `cli-order-subprocess.log`, `cli-order-transport.log`, `cli-wizard-freshness.log` |
| Rust format / backend+CLI all-target Clippy | Both exit0; Clippy retains jobs1 and `-D warnings`, no suppression | `rust-format.log`, `clippy.log` |

The shell initially used native Node26.3.0. Frozen focused/browser evidence
states that runtime; the final build/wizard/lint/full unit/V8 runs use the
repository-pinned major, native Node22.21.1. The Node22 wizard rebuild produced
identical tracked bytes to the browser-tested bundle, verified against the
frozen hashes; no additional browser execution was needed. Final mobile and
desktop idle/open/editing/scrolled screenshots are in
`frozen-browser-results/`, with `discovery-idle-*` and `info-*-dark.png` names.
ROOT independently inspected and accepted the final layout/help artifacts.

No global database was used. Docker was unavailable and the installed Homebrew
MongoDB7 was excluded. An official task-owned MongoDB8.0.0 binary runs with its
own dbpath and localhost27029, replica set `nyxidPreferenceReview`; readiness
confirms version8.0.0, primary=true and port27029 in `mongo-readiness.json`.
Database tests use only the explicit §10 URI and show zero ignored tests.
The recovered Cargo target is `/tmp/nyxid-service-preference-target`, with
incremental off, dev/test debug0 and jobs1. The cold backend build took51m55s
under measured host contention; the selected tests then passed normally.

Failed/interrupted attempts are preserved separately. The first temporary
config had an incorrect root/import path (`focused.log`), and the second still
had the duplicated root (`focused-recheck.log`); corrected frozen execution
passed. The early concurrent Node26 full V8 attempt failed the unrelated
`admin-usage.router.test.tsx` at the unchanged5000ms timeout:466 files/4776 tests
passed and1 failed,353.61s (`full-coverage.log`). This remains failed evidence.
The host had13.5GiB swap use and frequent compiler I/O waits; after own Cargo/UI
compilation settled, the isolated analytics test, complete Node22 unit and
complete V8 suites all passed without any analytics source edit, timeout
change, retry, exclusion or coverage-threshold reduction. Happy-dom navigation
ECONNREFUSED4629 diagnostics alone were not treated as test failures.

Byte-identical wizard regeneration changed timestamps, causing a later Cargo
search command to start a redundant backend rebuild. Only that task-owned
redundant Cargo/compiler attempt was interrupted; its -15 result is preserved
in `rust-timestamp-attempt-results.json`, `backend-search-timestamp-attempt.log`
and `timestamp-rebuild-interruption.json`, never counted as a pass. The actual
remaining backend filters ran against the freshly compiled test binary from
the successful14-test run, copied to `backend-frozen-tests` and hashed in
`backend-frozen-test-sha256.txt`. Frozen source and embedded wizard bytes match;
no production source changed between those executions. CLI targets and
all-target Clippy subsequently rebuilt and passed from the final checkout.

Implementation is locally validated and source-frozen, with no further fix
pending. The evidence-only addition here does not change tested production
inputs. `final-source.json` binds the final tracked diff, including the current
ROOT-owned review-document hash without modifying that document. Attachments
remain untracked. No commit, push, merge, deployment or production write was
performed by this recovery worker. Exact published-head CI/security and final
Opus approval are still required before ROOT marks delivery ready.

## 19. Merge of main b90f07ac and fresh local validation

This merge combines historical preference head
`19fffb3a243c0bd39a680d331a359ded4304d199` with main
`b90f07ac50499f6f654d2fffd9f96f1a83bd0f50`. Earlier published-head approvals
and CI results remain historical. ROOT owns the merge commit, publication,
review record, exact-head CI/security acceptance and final Opus delivery review.
The implementation worker resolves and stages local source and generated assets.
Attachments remain untracked.

Main's motion sequence, sticky connected surfaces, pool rows and icon conventions
are retained. Ordering pins effective expansion exclusively to the editing group
and reconciles the ephemeral expansion before paint; it does not save account
defaults. Save/Cancel keep the card expanded and restore focus to Reorder discovery,
even after a default restore with a different ephemeral expansion. Each sortable
connection is one tbody containing its actual main row, pool row and animated
panel. Variable-height keyboard moves align group centers with the existing
closest-center collision detector. Mouse/touch test destinations also account for
the complete group height. Pool Priority assertions now inspect the sibling pool
row while discovery/saved pills remain in the main row. The browser helper waits
for complete reveal height/opacity and settled layout before geometry measurements.
No timeout, coverage threshold, retry, exclusion or guard was relaxed.

All new logs/configs are under
`/tmp/nyxid-service-preference-restart/merge-b90/`. Final source binding starts at
`source-frozen-final.json`; documentation and regenerated wizard outputs are bound
separately at completion. Node is 22.21.1. Fresh `mongo-readiness.json` confirms
MongoDB 8.0.0, replica set `nyxidPreferenceReview`, writable primary
`127.0.0.1:27029`; Rust tests use that explicit isolated URI and one Cargo process
at a time. The global Homebrew MongoDB 7 instance is not used.

Interim attempts are preserved:

- `focused.log`: 67 passed, one failed pool-text assertion after main moved the
  membership to a sibling row. The assertion was adapted without changing its
  Priority value or discovery-pill expectation.
- `browser-focused.log`, `browser-sensors-2.log`, `browser-debug*.log`: new
  open-panel group-drag assertions exposed unequal-height coordinate handling.
  Center-aligned keyboard coordinates and group-center pointer destinations
  subsequently passed all three actual Chromium sensors.
- `browser-sensors-3.log`: 13 passed, one early 1024px sticky failure after
  opening/theme change. The old settle check could finish before reveal began;
  the helper now polls full reveal and absence of data-moving. The unchanged
  sticky assertions passed the targeted rerun in `browser-guards.log`.
- `browser-guards.log`: the new default-restore setup waited for Saved views while
  main's compact toolbar omitted it; the second case was interrupted. The test
  now scrolls to the full toolbar. `browser-guards-2.log` preserves two failures
  from a fixed 200px scroll that had not reached the card's sticky boundary;
  the test now scrolls from measured card position. Both normal/reduced cases
  passed in `browser-guards-3.log`.
- `browser-expansion.log`: four fresh passes, including dirty default restore
  after changed ephemeral expansion followed by successful Save/Cancel, one-card
  expansion and focus return. These precede extraction of the unchanged keyboard
  helper into its own module; final affected checks refresh on the frozen source.
- `full-unit-before-expansion.log` was interrupted on ROOT's accepted Opus
  expansion/focus fixes. `full-unit-before-helper-extract.log` passed 468 files,
  4,789 tests. The in-flight V8 run was then interrupted to remove a new Fast
  Refresh warning by extracting the keyboard helper; both interruption records
  and the coverage attempt are retained. `lint-targeted-after-extract.log` is clean.
- `browser-webkit.log` preserves a temporary external-config startup failure from
  an omitted webServer cwd. The corrected installed WebKit2336 harness passed
  grouped mouse/keyboard 2/2 in `browser-webkit-2.log`. Genuine WebKit touch-drag
  automation is unavailable through the public Playwright API: WebKit has tap
  support but no CDP/raw touch-move session. No synthetic touch is claimed.

- `build-invalid-test-env.log`: the local serial runner carried NODE_ENV=test
  from Vitest into npm build. The unchanged production mock-footprint guard
  correctly rejected a dev-only chunk (exit1). The runner now scopes that
  environment variable to Vitest commands; production build/wizard/lint are
  rerun on the unchanged frozen source. Passed unit/V8 evidence is retained.

- `browser-40of41.log` preserves the complete first frozen run:40 passed,
  one original390px touch test failed its first-position assertion. The native
  sensor never picked up the handle after scrollIntoViewIfNeeded placed it under
  the complete sticky header. `browser-mobile-isolated-2.log` preserves that
  pickup failure; `browser-mobile-isolated-3.log` passes after the test scrolls
  the handle below the measured header, verifies actual elementFromPoint hit,
  waits initial focus and uses complete group-center coordinates. No production
  source changed; the first-position/pickup/mobile overflow assertions remain.
  The complete browser suite refreshes under `source-frozen-browser.json`;
  comparison proves only its E2E test file changed, so unit/V8/build/wizard
  production-input evidence remains valid.

Final local frontend evidence (all commands under Node22.21.1):

| Gate | Result | Log under merge-b90 |
| --- | --- | --- |
| Reproducible dependency installation | npm ci exit0; main package/lock retained | npm-ci.log |
| Frozen focused unit | 11 files,122 tests passed; exit0 | focused-final.log |
| Frozen full unit | 468 files,4,789 tests passed; exit0,173.36s | full-unit.log |
| Frozen full V8 | 468 files,4,789 tests passed; exit0; lines73.25% vs unchanged15% gate, statements71.49%, branches66.45%, functions68.58% | full-v8.log |
| Production build | exit0; credential-accept output and production mock-footprint check passed | build-final.log |
| Wizard rebuild/closure | exit0;171 producer inputs; recorded/computed hash both ae039241d19e412e04c881ffc0ad2e1644bf74785371b1324af09949b308f012 | wizard-build.log; wizard-source-closure.json |
| Lint | exit0,0 errors and existing29 warnings; refreshed browser file targeted lint also clean | lint-final.log; lint-browser-refreshed.log |
| Complete real-route browser | 41/41 Chromium passed, exit0,211.53s; main scroll4/4, mobile native touch, grouped sensors, tooltip/sticky/focus/error regressions and generated wizard shim | browser-final.log; browser-refreshed-results/ |
| Additional WebKit | grouped mouse/keyboard2/2 passed, exit0,9.14s; native WebKit touch drag is not covered | webkit-final.log; webkit-final-results/ |
| Rust formatting | exit0,5.65s | rust-format.log |
| Merged backend preference | 14 passed,0 failed,0 ignored; fresh Cargo build against merged source and isolated Mongo8 | backend-preference.log |
| Backend search/agent/curation | 5+1+1 passed,0 failed,0 ignored; freshly built SHA-bound executable | backend-search.log; backend-agent-search.log; backend-curation.log; backend-artifact-binding.json |
| CLI unit/subprocess/transport | 3+4+4 passed,0 failed,0 ignored | cli-order-unit.log; cli-integration.log |
| CLI wizard freshness | 1 passed,0 failed,0 ignored | cli-integration.log |
| All-target Clippy | exit0 with -D warnings; serial jobs1,270.61s | clippy.log |

Production/unit/wizard inputs remain byte-identical to source-frozen-final.json.
The browser test-only refresh is bound by source-frozen-browser.json. Complete
old40/41 browser artifacts remain separately available; no old failure is counted
as a final pass. Final screenshot paths are recorded for ROOT's artifact review.
Full backend, billing and usage execution remains an exact-head CI gate owned by
ROOT; local Rust gates cover the feature filters, CLI and all-target Clippy as
requested. Exact published-head CI/wizard/security and final Opus sign-off remain
pending publication by ROOT; this section grants no delivery approval.

After the fresh merged Cargo backend preference build passed, a second filtered
Cargo invocation unnecessarily recompiled the backend. That duplicate build was
interrupted and preserved in backend-search-rebuild-interrupted.log. Remaining
backend filters ran on a copy of the just-built merged executable, SHA256
fe2cc6b1c2b412e2b67eb51ee71dd068c7feafd8753dcad528e847d94ea220ac,
with the explicit isolated Mongo8 URI. No historical executable was used.
All requested local gates now pass; ROOT owns final review-record append and
publication-bound CI/security/sign-off.
