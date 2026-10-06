# Service preference order for agent discovery

Branch: `service-tool-preference-order`. Planner: Fable 5.1. Status: implemented and locally verified against `868ce0b7`, including release 0.66.0 and the latest frontend readability changes. All 28 acceptance criteria and every substantiated review finding are closed. §17 records the acceptance matrix; §19 records backend/CLI/Clippy validation; §20 records frontend/wizard validation; §21 records the final label-layout correction and its validation. Final remote CI and Opus 5.5's verdict are recorded on [PR #1796](https://github.com/ChronoAIProject/NyxID/pull/1796), bound to its published head. Earlier integration and validation history is retained below.

## 1. Problem and scope

Agents discover NyxID service operations through `nyx__search_tools`,
`nyx__list_connected_services` and `nyx__call_tool`
(`backend/src/services/mcp_service.rs`, dispatched from
`backend/src/handlers/mcp_transport.rs::handle_meta_search` /
`handle_meta_list_connected`). Today `search_all_tools` ranks by
`(matched words desc, words in name desc, loader order asc)` and truncates to 25;
`list_connected_services` returns loader order. Loader order is `UserService`
rows sorted `created_at` descending (`load_callable_user_services`) first, then
platform (`DownstreamService`) rows. A person who connected
both `api-twitter` and a Composio-style aggregator that also exposes tweet
operations cannot tell NyxID which one agents should see first.

This plan adds a **personal, user-controlled preference order over connected
services**, edited by drag and drop on the AI Services page (`/keys`, External
Services tab), and applied as a deterministic tiebreak inside NyxID's own
discovery surfaces. It is discovery metadata only.

Out of scope, deliberately:

- No execution effect. Preference never changes which service a `nyx__call_tool`
  call hits (the tool name is `<slug>__<operation>` and binds the service), never
  retries or substitutes another provider after a failed call, and never touches
  proxy resolution, approvals, execution authority, platform grants or billing.
- No cross-provider broker. NyxID cannot make an independent client (Claude Code
  with its own Composio or browser tools, Cursor, OpenClaw) prefer a NyxID
  service over tools NyxID does not serve. The guarantee is exactly: *within a
  NyxID discovery response, preferred services sort first at equal relevance,
  and every response row carries `preference_rank` so a client that wants to
  honor it can.*
- No org-level order. Preferences are per acting identity (see §2.2). An org
  admin surface for org-wide ordering is a separate feature.
- `tools/list`, `GET /api/v1/mcp/config` and the delegated exact-operation view
  are **not reordered**. Their `catalog_digest` sorts by `service_id`, exact
  approvals bind digests, and consumers map the catalog into their own types.
  Changing their row order buys nothing and risks digest/evidence drift.
- No change to execution routing defaults: service pools (strategy, member
  `priority`, failover), the personal → organization → platform credential
  cascade for shared slugs, agent-key credential overrides, and the *proposed*
  "Connection order" in `docs/plans/service-route-resolution-flow.md` (not
  implemented) are untouched. §2.5 states how the UI keeps them apart.

## 2. Product semantics

### 2.1 Ranked, unranked, legacy

- A **preference order** is an ordered list of `UserService.id` values
  (UUID v4 strings). After authorization and stale filtering, position `i` (1-based) is the service's dense `preference_rank`.
- A visible service in the list is **ranked**; every other visible service is
  **unranked**. Unranked services sort after all ranked ones in the existing
  loader order, so a person who never saves an order sees exactly today's
  behavior (**legacy default**: no document, empty list).
- Ranking is total across the person's whole visible inventory: personal rows,
  org-inherited rows, auto-connected rows and disabled rows share one list. The
  editor lists that complete authorized inventory as one flat list, independent
  of the catalog grouping and of the saved view filters, and keeps the service
  name, slug and owner (person, organization, platform) on every item so
  provenance stays visible.

### 2.2 Whose preference

The document is keyed by the identity whose inventory a request runs under:
`AuthUser.user_id` for REST and `McpAuthContext.user_id` for MCP. Consequences,
stated so they are not rediscovered:

REST preference API authorization is separate from MCP discovery authorization:

| Caller | REST `GET /service-preferences` | REST `PUT /service-preferences` |
|---|---|---|
| Human session / first-party JWT | authorized metadata | verified first-party human only |
| Person/org-owned agent key | metadata within its live allowlist, excluding viewer-org rows | rejected |
| Delegated token | metadata with exact `account:read` under existing GET policy | rejected |
| OAuth application token | metadata under existing management GET policy | rejected |
| Service account / relay | rejected | rejected |

For requests that existing MCP authentication and proxy scopes authorize:

| Caller | MCP `user_id` | Discovery preference applied |
|---|---|---|
| Human session / first-party JWT | the person | the person's document |
| Person-owned agent key (`nyxid_ag_`) | the owning person | the person's document after the key's service allowlist filter |
| Assistant chat key, owner turn | the owner person | owner document over authorized metadata, including acknowledgement-required services |
| Assistant chat key, guest turn | the owner person | owner document restricted to granted guest-visible services |
| Org-owned agent key | the org user | normally legacy order: org users cannot author a human document |
| Relay token | verified owner person | owner's order restricted to the relay's live service/node scope |
| Delegated / OAuth application token | the verified MCP subject | subject's order when existing proxy scope authorizes MCP |
| Service account | service-account subject ID | normally legacy order: no human preference document |

Preference never widens or narrows permission. It is applied **after** every
existing visibility, scope and node filter, by reordering an already-authorized
list.

### 2.3 Identity: connection IDs, not slugs

Entries store `UserService.id`. Slugs are rejected because a deleted row keeps
its slug as a tombstone while a new active row may reuse it
(`docs/AI_SERVICES_ARCHITECTURE.md`, "Two listings"). Platform-source services
that have no `UserService` row (auto-connected `DownstreamService` fallbacks in
MCP) cannot be ranked: they are not on `/keys`, and their `service_id` is a
catalog id. They always sort as unranked. Platform-key *connections* that do
have a `UserService` row (auto-provisioned rows) are ordinary entries.

### 2.4 Stale entries

A ranked service can be deleted, un-shared by its org, or filtered out by an
agent key's allowlist after the order was saved. Rule: **stale ids are inert**.
Readers intersect the stored list with the caller's already-authorized inventory
and ignore the rest; the stored list is never returned raw. Disabled connections
retain their rank in the management inventory/editor (§2.1) but are absent from
MCP discovery under its existing active-service rules. No cascade write happens
on delete/disable/membership change (no hot-path or cross-collection coupling).
The next human `PUT` replaces the whole list, which prunes stale ids implicitly;
the UI only ever shows ids it can resolve.

### 2.5 Discovery preference versus execution defaults

The refreshed AI Services UI (#1685) shows several things that already decide
*which connection runs a request*. Preference order decides none of them. The
table is normative for wording in UI, docs and tool hints.

| Mechanism | Where it is decided | What the UI shows | Effect of preference order |
|---|---|---|---|
| Exact addressing | `/proxy/s/{slug}`, `/proxy/{id}`, `nyx__call_tool` with `<slug>__<operation>` | Slug in the connection row, "Configure" link | none; the call names the connection |
| Service pools | `ServicePool.strategy` (round-robin, weighted, priority), member `priority` (lower first), failover policy | `ServicePoolSummary`, `ServicePoolRoutingPanel`, row label `Priority n` | none; pools are addressed by their own slug |
| Shared catalog slug cascade | proxy resolution: personal, then organization (`primary_org_id`, then earliest membership), then platform | DEV-only routing preview (`service-routing-preview.ts`, `personal < org < platform`) | none; proxy resolution never reads `service_preferences` |
| Agent credential override | `AgentServiceBinding` per agent key | insight panels "overrides" | none; it changes the credential, not the listed order |
| Proposed "Connection order" | `docs/plans/service-route-resolution-flow.md`, not implemented | nothing | not this feature; must not share its name |

UI rules that follow:

- The rank pill reads **`Discovery #n`** (aria-label `Discovery preference n`),
  never `#n` or `Priority n`, because `ServiceConnectionTable` already prints
  `Priority {member.priority}` for pool members in the same row.
- The editor banner states that the order only changes how NyxID lists tools to
  agents at equal relevance and names the three execution mechanisms it does not
  change (slug called, pool routing, credential cascade).
- The editor and pills never appear inside the pool routing panel, the pool
  cards, or the routing preview's candidate ordering.

## 3. Ranking rules (normative)

Let `rank(s)` be the 1-based position of `s.service_id` in the resolved order,
or `u32::MAX` when unranked. Let `loader(s)` be the index of `s` in the loader
output (today's "original order").

1. **Service order** (`nyx__list_connected_services`, and the service vector fed
   to search): stable sort by `(rank(s), loader(s))`.
2. **Tool search** (`nyx__search_tools`): candidates keep today's relevance keys
   and gain preference as the third key:
   `(matched desc, in_name desc, rank(service) asc, candidate_index asc)`.
   Relevance wins over preference: a tool matching every query word from an
   unranked service still outranks a partial match from rank 1. Preference only
   decides ties, including the empty-query listing where every tool ties.
3. **Result limit** stays `MAX_SEARCH_RESULTS = 25`, applied after the sort, so
   at equal relevance ranked services' tools survive truncation first.
4. **Determinism**: every key is a total order; two runs over the same loader
   output and document produce the same list. Loader order itself is unchanged.
5. Chat-only native tool extras appended in `handle_meta_search`
   (`machine_access_service::definitions`, upload tool) are unaffected.

Implementation point (matches the code on this branch): the pure sort
`mcp_service::order_services_by_preference(&mut [McpToolService], &HashMap<String, u32>)`
is applied by `service_preference_service::order_discovery(db, user_id, services)`,
which builds the visible set from user-managed `service_id`s, reads the
document once, and returns `(services, ranks)`. `mcp_transport` keeps two
loaders: `load_all_services_for_meta_tools` (unranked; unchanged apart from the
guest-turn allowlist filter) and `load_preferred_services_for_meta_tools`,
which wraps it with `order_discovery` and is the only caller for
`nyx__search_tools` and `nyx__list_connected_services`. Search receives the
rank map through `search_all_tools_ranked(services, query, ranks)` and the
listing through `list_connected_services_ranked`; the empty-map wrappers
`search_all_tools` and `list_connected_services` are `#[cfg(test)]` so the
production binary has no unused entry points while existing tests compile
unchanged.

## 4. Data model

New file `backend/src/models/service_preference.rs`:

```rust
pub const COLLECTION_NAME: &str = "service_preferences";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServicePreference {
    #[serde(rename = "_id")]
    pub user_id: String,                 // person or org user id
    #[serde(default)]
    pub ordered: Vec<String>,            // UserService ids, position = rank
    #[serde(default)]
    pub version: i64,                    // optimistic concurrency; first save writes 1
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}
```

- `_id` is the owner id, like `assistant_settings`. No secondary index, no
  migration, no startup task. Absent document means legacy behavior.
- No `skip_serializing` anywhere. No secrets, so derived `Debug` is fine. New timestamps are normalized to BSON milliseconds before returning a write response.
- Bounds: at most 200 entries; each entry must be a canonical lowercase UUID v4; no duplicates. Constants and validation live in services. Request bodies are bounded at 16 KiB and reject unknown fields. Versions are integers from 0 through 2^53-2, safely incrementable in JavaScript.
  200 is above any realistic `/keys` inventory and keeps the document under a
  few KiB.

## 5. Backend API

### 5.1 `GET /api/v1/service-preferences`

Dedicated GET route in the shared authenticated API, rejecting service accounts
and relay tokens. Ordinary API keys and delegated exact `account:read` GETs
retain authorized metadata access. Existing slug-based key routes are untouched.
Missing documents return `ordered: []`, `version: 0`, `updated_at: null`.

Response:

```json
{ "ordered": ["<user_service_id>", "..."], "version": 3, "updated_at": "2026-10-07T09:00:00Z" }
```

`ordered` is the stored list **filtered to the caller's visible inventory**.
Read the preference document first; absent/empty orders return immediately.
For saved IDs, reuse the detail-rank helper's bounded (at most 200) selected-ID
source/membership/org-scope visibility check, API-key service scope and viewer-org
filter, then projected endpoint existence. These are the same live visibility
rules as `/keys`, without rendering or decrypting its full metadata. A restricted
agent key cannot learn UUIDs outside its allowlist. `version` is the raw document
version (0 when absent). No provisioning or OAuth reconciliation runs.

### 5.2 `PUT /api/v1/service-preferences`

Dedicated PUT route with rejection middleware for delegated, API-key, service
account and relay tokens on this route only. The handler additionally calls
`login_client_context::require_first_party_human` using verified `AuthUser`;
OAuth application tokens cannot write. Key updates and curation are unchanged.

Request: `{ "ordered": ["<id>", ...], "expected_version": 3 }`.

Validation (all `AppError::ValidationError`, HTTP 400, code of that variant):
length > 200; any noncanonical or non-v4 UUID string; invalid or exhausted version; duplicates; any id not in the caller's
visible inventory (the same live visibility rules as 5.1; PUT validates against
the existing read-only full inventory; an id belonging to
someone else and a nonexistent id produce the identical message, "unknown
service id", so ownership is not probeable).

A no-op at the current version returns unchanged metadata and emits no audit.
Both first-insert and later compare-and-swap races have exactly one winner.

Persistence (service layer `service_preference_service::replace`):

- Read the current document and reject a mismatched version before considering
  a no-op. Absent/empty no-ops keep the document absent and return null metadata.
- `expected_version == 0` with no current row: `insert_one` of a fresh document with `version: 1`.
  A duplicate-key error (document appeared concurrently) maps to
  `AppError::Conflict`.
- With a current row: `find_one_and_update` with filter
  `{ _id, version: expected_version }`, update
  `{ $set: { ordered, updated_at }, $inc: { version: 1 } }`, `ReturnDocument::After`.
  No match means `AppError::Conflict` (HTTP 409, code 1004).
  For an existing legacy row whose deserialized version is zero, the CAS filter
  accepts either `version: 0` or an absent `version`; it upgrades the row while
  retaining `created_at`. This differs from a first insert against no row.
- Response: same shape as GET (filtered list, new `version`, `updated_at`).
- Audit: `audit_service::log_actor_event` with event type
  `service_preference_updated` and metadata `{ "count": n, "version": v }` only.
  No service ids in audit (they are not secrets, but the event must stay small
  and the list is reconstructible from the document).
- No `ServiceChangeEvent`/history row: this is a person-level setting, not a
  service mutation, and history readers expect per-service lineage.

### 5.3 `GET /api/v1/keys` and `GET /api/v1/keys/{id}`

`KeyResponse` gains `preference_rank: Option<u32>` (serialized always; `null`
when unranked). List ranks reuse its authorized inventory and one preference read.
Detail adds one preference read; absent/empty/unranked orders add no inventory work.
For a ranked detail, visibility queries only saved IDs (<=200), reusing the shared
membership/org scope and retired-catalog resolver, plus projected endpoint IDs.
It does not render/decrypt other credentials or reload providers; one listing
membership snapshot is shared across the selected personal/org walk. Each management read adds
a preference point read; ranked detail additionally performs the bounded visibility work above; proxy, LLM gateway, MCP `tools/call`, approvals and
billing paths read nothing new.

### 5.4 MCP meta-tools

- `nyx__list_connected_services` rows gain `"preference_rank": <n|null>` and
  are sorted per §3. The tool description gains one sentence: "Rows are sorted
  by the owner's preference order; `preference_rank` 1 is the most preferred."
- `nyx__search_tools` matches gain `"preference_rank"` (the service's rank) and
  the response `hint` gains: "At equal relevance, tools from the owner's
  preferred services are listed first." Chat-key `chat_access` fields are
  unchanged.
- `nyx__discover_services` (not-yet-connected catalog) is unchanged.

### 5.5 CLI

`nyxid service list` adds a `Pref` column (rank or `-`) read from
`preference_rank`. New `nyxid service preference show` prints the resolved order
as a table (rank, slug, label, id) and `nyxid service preference set <ID_OR_SLUG>...`
replaces the order: slugs are resolved against `/keys` preferring the active row
(the CLI already does this in `api_key.rs` bind), the current `version` is read
from GET and sent as `expected_version`, and a 409 prints "preference order
changed elsewhere; re-run" and exits non-zero. Both support `--output json`.

## 6. Frontend

Baseline is the #1685 AI Services UI on `origin/main`: `pages/keys.tsx` renders
`GroupedServiceCards` (one collapsed card per catalog group, expandable into a
`ServiceConnectionTable`), `ServiceViewToolbar` (Organization/Service
multiselects, search, Personal/All source toggle, Auto-connected toggle, active
filter pills, `ServiceSavedViews` persisted through
`PUT /users/me/preferences/services` into `profile_config.services_view`),
`useServiceView` over the `useServiceCardView` zustand store, per-connection
billing/usage insights (`useServiceInsights`, `GET /service-insights`), pool
routing summaries (`useServiceRoutingPools`), table mode via `renderTable`, the
`/keys/services/$groupId` overview page, and a DEV-only routing preview
(`?view=routing`). All of that is preserved unchanged except for the additive
pill and the editor entry described here. The former `KeyCardContent`,
`ServiceTableRow`, `groupKeysBySource` and the page-level auto-connected
`Switch` no longer exist and must not be referenced.

Files: `pages/keys.tsx` (Reorder button, editor swap),
`components/dashboard/grouped-service-cards.tsx` (collapsed-card rank chip),
`components/dashboard/service-connection-table.tsx` (row pill),
`components/dashboard/service-preference-editor.tsx` (editor),
`hooks/use-service-preference.ts`, `schemas/service-preference.ts`,
`types/keys.ts` (`preference_rank`). No change to `service-view-toolbar.tsx`,
`service-saved-views.tsx`, `service-filter-multiselect.tsx`, `lib/service-view.ts`,
`schemas/service-view.ts`, `stores/service-card-view-store.ts`,
`hooks/use-service-view.ts`, insight/billing/pool components, `service-overview.tsx`
or `key-detail.tsx`. `@dnd-kit/core` 6.3, `@dnd-kit/sortable` 10,
`@dnd-kit/utilities` 3.2 are already dependencies; `analytics-canvas.tsx` and
`sortable-panel.tsx` remain the sensor/keyboard/announcement precedent.

### 6.1 Normal view: pills attached to connection IDs

- **Row pill** in `ServiceConnectionTable`: for a row whose `KeyInfo.preference_rank`
  is non-null, render `<Badge variant="accent" aria-label="Discovery preference n">Discovery #n</Badge>`
  in a wrapping metadata row below the existing connection-label/readiness
  header, within the Connection / Slug cell. The pill must not consume the
  label's horizontal space or overlap its text or icons. No new column.
  The component reads `preference_rank` from
  the row it already receives, so the pill appears everywhere this renderer is
  used: table view mode (`renderTable`), an expanded group card, the
  `/keys/services/$groupId` Connections tab, and the DEV routing preview.
- **Collapsed-card chip** in `GroupCard` (`grouped-service-cards.tsx`): when any
  connection in `group.connections` (the complete group, not the filtered
  `matches`) has a rank, render a `Badge variant="accent"` button next to the
  `n disabled` badge reading `Discovery #best` with
  `aria-label="Discovery preference best · <connection label>"` and a `title`
  listing every ranked connection in the group as `#n · label`. Activating it
  expands the card (`onToggle`) so the per-row pills are visible. A group is a
  catalog service, not a connection; the chip always names the connection it
  summarizes.
- Pills show the **dense authorized rank** from `/keys`, never a visible
  position. Filters (Personal/All, Organization, Service, search, Auto-connected,
  older state/type pills) hide rows but never change the text of a visible pill.
- **No reordering of groups or rows.** Groups keep the upstream alphabetical
  order and rows keep the `/keys` order inside each group; the rank is
  cross-service, so the pill, not position, carries it. This keeps upstream
  grouping tests and view transitions intact.
- **Reorder button** (`ArrowUpDown`, `variant="outline"`, label `Reorder`) sits
  in the tabs row beside `ViewToggle`, services tab only, hidden while the
  routing preview is active or when the preference GET returned 404. Disabled
  while keys or preference are loading or refetching (even with cached data),
  on either read error, with zero authorized services, or while the editor is
  open; `title` explains the reason.

### 6.2 Reorder mode

Entering reorder mode (state in `KeysPage`, not URL):

- `ExternalServicesTab` renders `ServicePreferenceEditor` **instead of**
  `GroupedServiceCards`. The sticky toolbar, saved views, filters, group cards,
  table, insight and pool queries unmount; `CodexConnectionSection` and
  `ArchivedServiceHistory` stay. Nothing writes to `useServiceCardView` or to
  `PUT /users/me/preferences/services`; on exit `GroupedServiceCards` remounts
  and the store restores the same filters and expanded card.
- The editor's inventory is the complete authorized `/keys` list the tab already
  holds (with `credential_source` joined from `/user-services` as today):
  personal, organization (including viewer rows), auto-connected and disabled
  connections, regardless of the filters that were active. The editor header
  reads "Editing your complete inventory · N connections. Filters and saved
  views are not applied here and are not changed."
- One flat vertical sortable list in both view modes (roomier item spacing in
  grid mode, compact in table mode). Each item shows `ServiceIcon`, label, slug,
  the catalog service name, `ServiceOwnerAvatar` with `connectionSourceLabel`,
  and `Auto-connected` / `Disabled` badges where applicable. The editor fetches
  no insights, billing or pools.
- Items are not links while reordering (no accidental navigation on touch); the
  Connect Service CTA stays reachable in the header.
- An instruction banner (info callout per DESIGN.md "Banners") reads: "Drag the
  handle to set the order agents see first when tools tie on relevance.
  Keyboard: focus a handle, press Space, use the arrow keys, press Space again.
  Items below the divider are unranked. This does not choose which connection
  runs a request: that is the slug the agent calls, pool priority or rotation,
  and the personal → organization → platform credential cascade." Visible at
  all times in reorder mode, not a tooltip.
- A **divider item** with id `__unranked__` splits the list: everything above
  it is ranked and shows a live `Discovery #n` pill; everything below is
  unranked and shows a muted `Unranked` pill. The divider is itself sortable so dragging an
  item across it ranks or unranks it, and dragging the divider moves the cut.
  Each item also has `Rank`/`Unrank` buttons (move to the end of the ranked
  zone / to the top of the unranked zone) so touch and keyboard users never
  need a long drag.
- Drag affordance: `GripVertical` handle button (`setActivatorNodeRef`,
  `touch-none cursor-grab active:cursor-grabbing`, `aria-label="Drag <label>"`),
  dragged item at `opacity-0.35`, drop target outlined with
  `data-drop-target` like `sortable-panel.tsx`, `DragOverlay` for the card
  ghost.
- Sensors: `PointerSensor` with `activationConstraint: { distance: 6 }` (mouse
  and touch; the handle is the activator), `KeyboardSensor` with
  `sortableKeyboardCoordinates`. `accessibility.announcements` alone produce
  "Picked up <label>, position n of m", "Moved to position n", "Dropped at n",
  "Cancelled", using dnd-kit’s single live region, without duplicate announcements.
- Save/Cancel bottom-right of the editor (DESIGN.md interaction rules). Save is
  `variant="primary"` and disabled until dirty. The list is a single field
  `ordered: string[]` of a `useAppForm` form with
  `zodResolver(servicePreferenceRequestSchema)`; drag/button changes call
  `setValue("ordered", next)` (default `shouldDirty: true` enables Save), reset
  from server data uses `{ shouldDirty: false, shouldTouch: false }`.
- Cancel discards local changes and leaves reorder mode. Leaving the tab or
  toggling view mode while dirty asks "Discard unsaved order?" (plain confirm
  dialog; no `beforeunload` handler).
- Save/Cancel restore focus to Reorder once the closing inventory refetch finishes
  and the button is available. Identity changes, navigation, or a later user
  interaction cancel pending focus restoration.

### 6.3 Save outcomes

- Success: invalidate `["service-preference"]` and `["keys"]`, exit reorder
  mode, toast "Preference order saved".
- 400 validation (should not happen from the UI; stale id because a service was
  deleted in another tab): banner "Some services are no longer available.
  Refresh services before saving again." then refresh `/keys` successfully before the form drops ids absent from that refreshed inventory
  and stays dirty for the user to re-save.
- 409 conflict: banner "Your preference order changed in another tab" with two
  actions: **Reload order** (refetch, reset form, stay in reorder mode) and
  **Overwrite** (re-submit with the server's current `version`, obtained by a successful refetch (the generic 409 body has no version contract)).
- Network/5xx: `ErrorBanner` with Retry; local order kept.

### 6.4 Hooks and schema

- `useServicePreference()` query `["service-preference", identity]`, `staleTime: 0`,
  `refetchOnMount: "always"` (matches `useKeys`).
- `useSaveServicePreference()` mutation `PUT /service-preferences`; on 409 preserves the error and local edits for explicit refetch recovery; on success invalidates
  as above.
- `useDeleteKey`/`useUpdateKey` additionally invalidate `["service-preference"]`
  so a deletion in the detail page refreshes resolved ranks on return.
- `schemas/service-preference.ts`: `servicePreferenceResponseSchema`
  (strict object, canonical lowercase RFC4122 UUID-v4 IDs, at most 200 unique IDs,
  safe integer `version`, nullable string `updated_at`), `servicePreferenceRequestSchema`
  (the same ordered-ID rules and `expected_version` from 0 through 2^53-2).
  `KeyInfo` gains `preference_rank?: number | null`.

### 6.5 States

| State | Behavior |
|---|---|
| Loading keys | existing skeleton; Reorder disabled |
| Keys error | existing `ErrorBanner`; Reorder disabled |
| 0 services | upstream empty state; Reorder disabled |
| 1 service | Editing enabled for ranking/unranking |
| Preference query error | Ordinary editing/saving blocked; ErrorBanner with Retry; explicit conflict recovery remains callable and requires a successful fresh GET |
| Inventory query error during editing | Inventory ErrorBanner with Retry; preserve the draft and block ordinary editing/saving |
| Preference GET 404 | Reorder hidden for older-server compatibility; pills still render from `/keys` if present |
| View mode switch | allowed when not dirty; confirm when dirty |
| Active filters / saved view while editing | not applied to the editor, not modified; restored on exit |
| Filters hiding a ranked row | row hidden, no renumbering; collapsed chip still summarizes the group's best rank |
| Routing preview (`?view=routing`, DEV) | Reorder hidden; row pills still render through the shared table |
| `/keys/services/$groupId` | row pills render; no editor there |
| Org viewer rows (`allowed: false`) | rankable like any visible row (ranking is personal and discovery already shows them) |
| Disabled services (`is_active: false`) | rankable; MCP never loads them, so the rank is inert until re-enabled |
| Pool member rows (`Priority n`) | pill and pool label coexist; wording per §2.5 |

## 7. Security and privacy review

- Writes are human-only; GET follows the `/keys` auth class and filters to the
  caller's visible inventory, so restricted agent keys, delegated readers and
  OAuth readers cannot enumerate ids outside their scope.
- Unknown and foreign ids are indistinguishable in PUT errors.
- Documents contain only UUIDs, counts and timestamps. No names, slugs,
  credentials or ciphertext. Audit carries counts and version only.
- Preference is applied strictly after scope/visibility/node filtering and
  never consulted by `execute_tool`, `proxy_service`, approvals, execution
  authority, grants or billing. A deliberate `grep` acceptance criterion (AC-14)
  guards this.
- Rate limiting: global per-IP limiter applies; the dedicated PUT adds no new
  limiter and is bounded by body size, entry count and verified human auth.

## 8. Performance

- Discovery meta-tools: +1 `find_one` by `_id` per call (already several
  queries per call). `/keys` list: +1 `find_one`, reusing inventory. Preference
  GET: one preference `find_one` only for absent/empty orders, with no inventory,
  provider or credential reads. A saved order adds the shared live membership
  snapshot, selected service-ID queries per personal/org owner (each at most 200),
  bounded retired-catalog-ID projections where applicable, org scope/source reads
  and one projected endpoint-existence query when selected services survive.
  It never renders/decrypts keys or loads providers. Detail adds
  one preference read and, only for a saved ranked connection, a bounded
  selected-ID visibility walk and projected endpoints as described in §5.3. No new reads on
  proxy, LLM, MCP `tools/call`, approvals or background sweeps.
- No new index (primary key lookup), no migration, no startup work.
- Page aggregate: a fresh `/keys` page reading both `/keys` and
  `/service-preferences` adds two preference-document lookups. An absent/empty
  order adds no second inventory walk; a saved preference GET adds only the
  bounded selected-ID/source/endpoint visibility reads above. The existing hook
  behavior is retained without a tab-only query gate.

## 9. Documentation changes

- `docs/API.md` "Unified Keys": document `GET`/`PUT /service-preferences`,
  `preference_rank` on `/keys` rows, error mapping (400 / 409 code 1004), and
  the inventory-filtering guarantee.
- `docs/API_DISCOVERY.md` "Tool search semantics": add the third sort key, the
  25-cap interaction, `preference_rank` on meta-tool rows, and the explicit
  non-guarantee about independent clients.
- `docs/AI_SERVICES_ARCHITECTURE.md`: new "Service preference order" section
  (identity, stale rule, no-execution-effect, org-key limitation) plus one
  paragraph under the upstream "Service cards and saved filter defaults"
  section: the `Discovery #n` pill, the editor's independence from filters and
  saved views, and the §2.5 distinction from pool `Priority n`, the credential
  cascade and the proposed connection order.
- `docs/chat/08-nyxagent-engine.md`: one sentence that chat-key discovery rows
  carry `preference_rank` and follow the owner's order for guests too.
- `CLAUDE.md` Rule 8: one bullet: "`service_preferences` is per acting identity,
  discovery-only (`nyx__search_tools`/`nyx__list_connected_services`, `/keys`
  `preference_rank`); never read on execution paths; `tools/list`/`/mcp/config`
  unchanged."
- CLI: `nyxid service --help` text; no separate CLI reference doc exists.

## 10. Rollout and compatibility

- Additive everywhere: new collection, new optional response fields
  (`preference_rank`, meta-tool fields), new routes. Old frontends ignore the
  field; old CLIs ignore the column. Old backend replicas during a rolling
  deploy return `/keys` rows without `preference_rank` and 404 the new routes;
  the frontend treats a 404 on GET as "no order" and hides the Reorder button
  when the first GET 404s (feature detection, no flag). Rollback needs no data
  action; the collection is inert for old binaries.
- No release version bump solely for this feature. Regenerate the embedded CLI wizard only if its producer inputs change.

## 11. Tests and how to run them

**Targeted checks:**

```bash
source "$HOME/.cargo/env" 2>/dev/null
cargo test -p nyxid service_preference            # pure + HTTP/MCP DB tests; set DB URI below
cargo test -p nyxid search_all_tools
cargo test -p nyxid-cli service_preference
cd frontend && PATH=/opt/homebrew/bin:$PATH npm run test -- keys service-preference grouped-service-cards service-connection-table && npm run lint && npm run build
cd frontend && PATH=/opt/homebrew/bin:$PATH npx playwright test e2e/service-preference.spec.ts --workers=1
```

The Playwright fixture must serve the upstream page's requests
deterministically: `/api/v1/users/me` with `profile_config: { onboarding: { ai_services_completed_at: <timestamp> }, services_view: <saved filters> }`,
`/api/v1/service-insights` → `{ "connections": [] }`, pool listings → empty,
`/api/v1/catalog?include_all=true` → `{ "entries": [] }`, and it must record
any `PUT /api/v1/users/me/preferences/services` so AC-18 can assert none
happened.

**MongoDB-dependent:** never silently skip for sign-off. Use the dedicated reachable
replica set explicitly, `NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27127/?replicaSet=service-preference-rs&directConnection=true'`.
Run one Cargo build at a time with task-owned target `/tmp/nyxid-service-preference-target`,
`CARGO_PROFILE_TEST_DEBUG=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0`, jobs 1.
PM recovered disk space by removing authorized inactive task/build caches; local backend checks now use the task target above. Other active builds and all daemons are left untouched.
No external processes are stopped/restarted or other projects’ artifacts deleted.

Test fixtures to reuse: `test_utils::connect_test_database`,
`test_utils::test_app_state`, `handlers/keys.rs` test helpers
(`insert_user`, `insert_key_fixture`, `test_auth_user`),
`services::assistant_authority_tests::{fixture, ordinary_key, connected}` with
`mcp_chat_authority_tests::direct_call`, and `mcp_transport` unit helpers
`api_key_auth`, `user_managed`, `platform`.

## 12. Implementation checklist (code present; baseline checks in §17; latest integration checks in §19)

Backend

1. `models/service_preference.rs`: struct per §4 and `COLLECTION_NAME`; `mod`
   entry in `models/mod.rs`; bson roundtrip and dated-legacy-defaults tests.
2. `services/service_preference_service.rs` (actual signatures):
   `MAX_ORDERED_SERVICES = 200`, `MAX_EXPECTED_VERSION = 2^53 - 2`,
   `MAX_REQUEST_BYTES = 16 KiB`;
   `get(db, user_id) -> AppResult<Option<ServicePreference>>`;
   `resolve_visible(ordered: &[String], visible: &HashSet<String>) -> Vec<String>`
   (visible-only, first occurrence wins);
   `rank_map(ordered: &[String], visible) -> HashMap<String, u32>` (dense 1-based);
   `filter_inventory(views: Vec<KeyView>, scope: Option<&[String]>, api_key: bool)`
   and `visible_inventory(db, encryption_keys, user_id, scope, api_key)` over
   `list_keys_read_only_with_grants` with live grants/providers for PUT validation;
   `visible_ordered_ids(db, user_id, ordered, scope, api_key) -> HashSet<String>`
   for bounded saved-ID GET/detail visibility without credential/provider loads;
   `validate_order(ordered, expected_version, visible)`;
   `replace(db, user_id, ordered, expected_version, visible) -> Replacement { preference, changed }`
   (insert-or-CAS per §5.2, no-op detection);
   `order_discovery(db, user_id, services) -> (services, ranks)`;
   `detail_rank(db, user_id, service_id, scope, api_key) -> Option<u32>`
   for the bounded `GET /keys/{id}` path (§5.3).
3. `mcp_service.rs`: `order_services_by_preference(&mut [McpToolService], &ranks)`,
   `search_all_tools_ranked(services, query, ranks)` with the third key,
   `list_connected_services_ranked(services, query, ranks)` emitting
   `preference_rank`; `search_all_tools` and `list_connected_services` are
   `#[cfg(test)]` empty-map wrappers; updated meta-tool descriptions.
4. `mcp_transport.rs`: `load_all_services_for_meta_tools` stays the unranked
   loader (only change: guest-turn allowlist filtering);
   `load_preferred_services_for_meta_tools` wraps it with `order_discovery` and
   is called by `handle_meta_search` and `handle_meta_list_connected`, which
   pass `ranks` to the ranked helpers, add `preference_rank` to search match
   rows and the hint sentence. Execution (`nyx__call_tool`, `tools/list`,
   `/mcp/config`) keeps the unranked operation catalog.
5. `handlers/service_preference.rs`: `get` reads the document first and resolves
   saved IDs through `visible_ordered_ids`; `put` uses `visible_inventory` for
   validation. Dedicated DTOs use `resolve_visible`; `put` additionally runs
   `login_client_context::require_first_party_human`, calls `replace`, and
   emits the audit event only when `changed`; `utoipa` annotations.
6. `routes.rs`: `service_preference_reads` (GET, merged into the shared
   authenticated group with SA/relay rejection) and `service_preference_writes`
   (PUT with delegated/API-key/SA/relay rejection and a
   `MAX_REQUEST_BYTES` body limit), leaving key routes unchanged.
7. `handlers/keys.rs`: `preference_rank` on `KeyResponse`; list ranks from
   `rank_map` over the request's own authorized inventory, detail from
   `detail_rank`; `KeyAuthorizationEvidenceResponse` unchanged. On the #1685
   base, add the field beside `can_edit_configuration`/`oauth_app_source` in
   both constructors (§18).
8. CLI: `Pref` column in `service list`, `service preference show|set`.
9. Docs per §9; compatibility per §10.

Frontend

10. `types/keys.ts` and `schemas/service-preference.ts` (+ `.test.ts`).
11. `hooks/use-service-preference.ts` (query + mutation, 404 feature detection,
    invalidations), plus the extra invalidation in `useDeleteKey`/`useUpdateKey`.
12. `components/dashboard/service-preference-editor.tsx`: DnD list with divider,
    handles, `Discovery #n` pills, Rank/Unrank buttons, announcements,
    `useAppForm` field, Save/Cancel, conflict/validation/network banners,
    complete-inventory header, §2.5 banner wording; items carry service name,
    slug and `ServiceOwnerAvatar`; no insight/pool queries.
13. `pages/keys.tsx` (on the #1685 version): Reorder button in the tabs row
    beside `ViewToggle` (services tab, not in routing preview, hidden on GET
    404), editor swap inside `ExternalServicesTab` in place of
    `GroupedServiceCards`, dirty confirm on view/tab change; no writes to
    `useServiceCardView` or saved views.
13a. `components/dashboard/service-connection-table.tsx`: row pill in wrapping
    metadata below the label/readiness header, from `KeyInfo.preference_rank`;
    preserve the existing label's available width and prevent overlap.
13b. `components/dashboard/grouped-service-cards.tsx`: collapsed-card chip from
    `group.connections` that expands the card; no change to group or row order.
14. Tests: `pages/keys.test.tsx` (on the upstream harness, with
    `useServicePreference` mocked), `components/dashboard/service-preference-editor.test.tsx`,
    pill tests for `ServiceConnectionTable` and `GroupCard`, and
    `e2e/service-preference.spec.ts` updated for the grouped UI (§11 fixture).

## 13. Acceptance criteria

Each criterion names the test that proves it. Pure model/ranking/CLI tests need
no database; the combined `service_preference` filter also selects DB tests.

- **AC-01** (local, `models/service_preference.rs`): a legacy document with `_id`, `created_at`, and `updated_at`
  deserializes to `ordered = []`, `version = 0`; a full document round-trips
  through BSON with `created_at`/`updated_at` as BSON dates.
- **AC-02** (local, `service_preference_service` tests): `validate_order`
  rejects 201 entries, a non-UUID entry, a duplicate, and an id outside the
  visible set, each with `AppError::ValidationError`; the unknown-id message is
  byte-identical for a foreign id and a random id.
- **AC-03** (local, `mcp_service` tests): `order_services_by_preference` is a
  stable sort: ranked services appear in rank order, unranked keep loader
  order after them, platform-source services (catalog ids) are unranked, and a
  rank for an id not in the list changes nothing.
- **AC-04** (local): `search_all_tools_ranked` keeps `search_all_tools_matches_words_in_any_order_and_ranks_full_matches_first`
  green and adds: at equal `(matched, in_name)` a rank-1 service's tool precedes
  a rank-2 and an unranked one; a full match from an unranked service precedes a
  partial match from rank 1; the empty query lists rank-1 tools first; with 40
  tying tools across two services the 25 survivors are the ranked service's
  tools first.
- **AC-05** (local): `search_all_tools(services, query)` (empty rank map)
  returns exactly what it returned before this change for the existing test
  inputs; `assistant_account_tools::search_tests::asking_for_an_agent_finds_agent_creation_first`
  stays green.
- **AC-06** (local): `list_connected_services` rows carry `preference_rank`
  (`1`, `2`, or `null`) and are ordered by §3 rule 1; `count` unchanged.
- **AC-07** (DB, `handlers/service_preference` tests): `PUT` with
  `expected_version: 0` creates `version 1`; a second `PUT` with
  `expected_version: 1` yields `version 2`; a `PUT` with `expected_version: 1`
  after that returns 409 with code 1004 and the document is unchanged; two
  concurrent first `PUT`s produce exactly one 200 and one 409.
- **AC-08** (DB): `GET` by a restricted agent key whose allowlist covers one of
  three ranked services returns `ordered` with exactly that one id and the raw
  `version`; the same GET as the human returns all three.
- **AC-09** (DB): after the owner deletes a ranked service via
  `DELETE /keys/{id}`, `GET` omits its id without any write to
  `service_preferences` (assert `version` and `updated_at` unchanged), and
  `/keys` rows for the remaining ranked services receive dense authorized ranks with no gaps.
- **AC-10** (DB): `PUT` is rejected before the handler for API-key, service
  account, delegated and relay tokens (401/403 per the existing rejection
  layers); `GET` succeeds for an API key and for a delegated `account:read`
  token.
- **AC-11** (DB): `GET /keys` and `GET /keys/{id}` include `preference_rank`
  equal to the dense authorized position for ranked services and `null` otherwise; a
  command-monitoring run of `GET /keys` shows exactly one `find` on
  `service_preferences`.
- **AC-12** (DB, MCP transport via `assistant_authority_tests::fixture`): with
  two connected services and a saved order placing the second first,
  `nyx__search_tools` with a nonempty relevance-tie query (transport requires a query) lists the preferred service's
  tools first and each match carries `preference_rank`;
  `nyx__list_connected_services` returns them in preference order; a guest-turn
  chat key sees the same order restricted to its allowed services.
- **AC-13** (DB): `nyx__call_tool` on a tool from an unranked service behaves
  identically with and without a saved order (same response and execution audit
  event data/actor/target; each call has its own audit ID, timestamp and chain fields);
  `GET /api/v1/mcp/config` `catalog_digest` and service order are identical
  before and after saving an order.
- **AC-14** (local, repo grep in the PR checklist):
  `grep -rn "service_preference" backend/src/services/proxy_service.rs backend/src/services/execution_authority.rs backend/src/services/mcp_approval.rs backend/src/services/billing backend/src/handlers/service_insights.rs backend/src/services/service_insights_activity.rs`
  returns nothing, and `execute_tool*` functions do not reference the
  collection or rank map.
- **AC-15** (DB): the audit log after a successful PUT contains one
  `service_preference_updated` row whose `event_data` has only `count` and
  `version`, and no service ids.
- **AC-16** (local, CLI): `nyxid service list` renders the `Pref` column from
  `preference_rank` (`-` when null); `service preference set` resolves slugs
  preferring the active row and sends `expected_version` from the prior GET;
  a mocked 409 exits non-zero with the "changed elsewhere" message.
- **AC-17** (local, `keys.test.tsx` on the upstream harness, plus
  `ServiceConnectionTable`/`GroupCard` pill tests): a ranked connection row
  shows `Discovery #n` with `aria-label="Discovery preference n"` inside an
  expanded group card, in table view mode, and on `/keys/services/$groupId`
  (same renderer); a collapsed group card shows the `Discovery #best` chip
  naming its best-ranked connection and expands the card when activated;
  unranked rows and groups show neither; with Auto-connected hidden, a search,
  an Organization/Service selection or Personal/All toggled, visible pill text
  is unchanged and no row or group changes position relative to the upstream
  order; a pool-member row shows both `Priority n` and `Discovery #n`.
  At a 1440-pixel viewport, the rendered connection-label text retains usable
  width and overlaps neither the discovery pill nor the readiness badge.
  Tablet (1024 pixels) and mobile (390 pixels) retain visible rank metadata
  without introducing page-wide horizontal overflow.
- **AC-18** (local, `keys.test.tsx`): Reorder is disabled with no authorized
  services, while loading or refetching, and on error, and hidden in the
  routing preview and after a preference GET 404; clicking it replaces the
  toolbar and grouped cards/table with the editor, whose item count equals the
  full `/keys` length while a Personal-only view with auto-connected hidden and
  a Service selection is active; items are not links; the banner contains the
  §2.5 wording; after Save or Cancel the grouped view returns with the same
  `useServiceCardView` filters and expanded card, and no request was made to
  `PUT /users/me/preferences/services`.
- **AC-19** (real-route Playwright with real sensors, plus editor button tests): dragging item C above item A
  updates pills to C=`Discovery #1`, A=`Discovery #2`; moving an item below the divider makes it
  `Unranked`; moving the divider up unranks everything below it; `Rank`/`Unrank`
  buttons behave as §6.2; the single dnd-kit `aria-live` region receives the announcement
  text.
- **AC-20** (local, editor test): Save is disabled until a change, enabled after
  a move, disabled again after Cancel resets; Save sends exactly the ids above
  the divider in order plus the current `version` as `expected_version`.
- **AC-21** (local, editor test): a 409 renders the conflict banner with
  **Reload order** and **Overwrite**; Reload resets to the refetched order and
  clears dirty; Overwrite re-submits with the server version. A network error
  shows `ErrorBanner` with Retry and keeps the local order. A 404 on the initial
  GET hides the Reorder button.
- **AC-22** (local, `schemas/service-preference.test.ts`): request schema
  rejects 201 ids, a non-UUID, duplicates and a negative `expected_version`;
  response schema accepts `version: 0` with `ordered: []`.
- **AC-23** (local): `npm run lint` passes with no `console.log`, no raw
  `useForm`, and `npm run build` type-checks; `cargo clippy -p nyxid -p nyxid-cli --all-targets -- -D warnings` passes.
- **AC-24** (docs review): the four docs in §9 and `CLAUDE.md` contain the new
  sections; `docs/API_DISCOVERY.md` states the non-guarantee for independent
  clients verbatim from §1; `docs/AI_SERVICES_ARCHITECTURE.md` carries the
  §2.5 distinction next to the upstream service-card section.

## 14. Tradeoffs recorded

- **Relevance over preference.** Chosen so a precise query never loses its best
  match to a preferred-but-irrelevant service. The cost is that preference is
  invisible for queries with a single clear winner, which is the desired
  behavior.
- **No cascade on delete.** Stale ids stay in the document and are filtered on
  read. This avoids touching the delete/disable/membership paths and any
  cross-collection transaction. The cost is one extra `find_one` per read
  instead of zero, and dense ranks computed after each caller’s authorization; UI-only auto-connected hiding does not renumber ranks.
- **Per-identity, not per-org.** Keeps ownership simple and human-only. Org
  agent keys get legacy order; the plan records this and leaves an org surface
  to a later feature.
- **Catalog order untouched.** `tools/list` and `/mcp/config` keep their
  digest-sorted contract. A client that wants preference in those surfaces reads
  `preference_rank` from `/keys` or the meta-tools.
- **Divider as a sortable item.** One container keeps the dnd-kit wiring simple
  and gives keyboard users a single linear order; the Rank/Unrank buttons cover
  users who cannot or do not want to drag.
- **Pills, not repositioning, in the grouped UI.** The rank is cross-service
  while the page groups by catalog service and sorts groups alphabetically, so
  moving rows would mirror the agent order only partially while breaking the
  upstream grouping and view-transition behavior. The pill carries the exact
  dense rank instead.
- **Editor replaces the grouped region.** Hiding the toolbar while editing
  avoids two conflicting notions of "what is shown" (saved filters versus the
  complete inventory) and guarantees the saved view and store are never
  mutated by a reorder. The cost is that filters cannot be used to find an item
  inside the editor; the list is bounded (at most 200 ranked, inventory-sized)
  and items carry service name, slug and owner.
- **`Discovery #n` wording.** One extra word on the pill prevents it from being
  read as pool `Priority n` or as a default execution connection, which the
  same table rows also display.
- **Protocol limitation.** MCP has no server-to-client "prefer this tool" field.
  Ordering and `preference_rank` are the strongest signals the protocol allows;
  honoring them is the client's choice.

## 15. Handoff

Plan file: `docs/plans/service-tool-preference-order.md`. Design: a personal
`service_preferences` document keyed by acting identity, holding ordered
`UserService` ids with optimistic versioning; applied as a deterministic
tiebreak after relevance in `nyx__search_tools` and as the primary order in
`nyx__list_connected_services`, surfaced as `preference_rank` on `/keys` and
meta-tool rows, edited by an accessible dnd-kit list with a ranked/unranked
divider that replaces the grouped region while editing, and `Discovery #n`
pills on connection rows and collapsed group cards of the #1685 AI Services
UI; no execution, approval, billing, routing-default or catalog-digest effect.
28 acceptance criteria, including all PM corrections in §16 and the UI
revisions in §18.

## 16. PM review corrections before implementation

The coordinating agent reviewed this plan before implementation. These corrections
supersede the affected clauses above. The implementer must incorporate them into
the normative sections and acceptance checklist so the final plan is coherent.

1. Use `GET/PUT /api/v1/service-preferences`, a dedicated route that does not
   consume a slug under `/keys/{id}`. Existing UUID and slug-based key routes retain their
   behavior. Put write rejection middleware on the new route only. Do not change
   the auth classes of unrelated key-update or catalog-curation routes. The PUT
   handler also calls the existing verified
   `login_client_context::require_first_party_human` guard. GET rejects service
   accounts and relay tokens; ordinary API keys and delegated `account:read`
   readers retain authorized metadata access.
2. Missing documents produce `ordered: []`, `version: 0`, `updated_at: null`.
   The stored model requires BSON dates. AC-01's minimal legacy fixture includes
   `_id`, `created_at`, and `updated_at`, with only `ordered` and `version` absent.
   Validate canonical UUID v4 strings, duplicates, and a nonnegative version that
   can safely increment and round-trip through JavaScript. Bound the request body
   and reject unknown fields. Keep constants and business validation in services.
3. Service visibility and rank calculation belong in the service layer. Reuse the
   existing read-only inventory resolver, current grants, and providers without
   reconciliation or provisioning. Apply authorization before preference. Exposed
   ranks are dense positions among the caller's authorized ranked services, so
   hidden or deleted entries disclose no cardinality through rank gaps. The web
   auto-connected toggle does not change ranks; the full authorized inventory is
   the rank basis. No preference write is needed to prune a stale read.
4. Both concurrent first saves and concurrent later saves have one winner. A no-op
   at the current version does not bump version or create an audit event. The audit
   service appends through its existing chain path. The unchanged catalog and
   explicit execution-target behavior must be proved by real handler/MCP tests,
   not solely by searching source text.
5. A preference read failure blocks editing and offers Retry. It must never seed
   an empty order and allow an accidental overwrite. A 404 disables editing for
   compatibility with older servers. Conflict recovery refetches the current
   version before an explicit overwrite; the existing generic 409 body does not
   promise a version field. Keep local edits until the user chooses a recovery.
   Refresh inventory before removing stale IDs after a validation error.
6. A single connected service can be ranked or unranked; enable editing with one
   service. Show a clear empty state with none. Explain that preference breaks
   relevance ties. Preserve focus, unsaved changes, identity-separated caches,
   useful labels, and bounded selection behavior at the 200-service limit.
7. Avoid duplicate screen-reader announcements. Real Playwright tests must drive
   mouse dragging, touch interaction, keyboard dragging and cancellation on the
   actual `/keys` route, including grid/table modes, order pills, persistence on
   reload, save failure, conflict recovery, and a narrow mobile viewport without
   horizontal overflow. Mocking dnd-kit callbacks alone is insufficient.
8. This backend is a binary crate: remove unsupported `--lib` test commands.
   Run database-dependent tests with an explicit reachable
   `NYXID_TEST_DATABASE_URL` so missing infrastructure cannot silently pass. Use
   targeted checks plus required PR CI checks. Do not bump release versions solely
   for this feature. Maintain any generated CLI wizard artifact required by CI.

Additional acceptance criteria:

- **AC-25:** existing slug-based key reads and catalog-curation writes retain their
  prior auth and routing behavior. OAuth application tokens cannot modify preferences.
- **AC-26:** scoped REST and MCP responses expose no hidden IDs or rank gaps.
  Stale read filtering performs no write. Canonical ID, version, body-size, unknown
  field, and no-op behavior pass explicit tests.
- **AC-27:** real browser tests on the grouped UI prove mouse, touch, and
  keyboard reordering, cancellation, persistence, pills in an expanded group
  card, in table mode and on `/keys/services/$groupId`, the collapsed chip,
  failure/conflict recovery, mobile layout, focus, unsaved-change handling, and
  that the saved view and active filters survive an edit unchanged (no
  `PUT /users/me/preferences/services`, same filter pills after exit).
- **AC-28:** preference read failure cannot enable saving an empty replacement;
  one-service ranking, the 200-service bound, cache identity changes, and stale
  inventory recovery have tests.

The final implementation handoff includes an AC-01 through AC-28 evidence matrix,
exact check commands and outcomes, and every resolved review finding.

## 17. AC implementation and check evidence

The matrix records successful implementation and local validation. §19 lists
backend/CLI/Clippy commands and results on the `566ca5f9` integration committed
as `0390ad00`; §20 supersedes frontend/wizard evidence with final `868ce0b7`
integration results. Backend/CLI Rust source is unchanged between these bases.
All local gates passed. Remote CI and Opus's final verdict are separate gates,
recorded on the PR; no remote pass is inferred from local checks. Earlier-base
results are retained below as historical evidence.

- [x] AC-01: `models/service_preference.rs::service_preference_bson_dates_and_legacy_defaults`: BSON dates and dated legacy defaults passed in the merged 8-test backend feature run.
- [x] AC-02: `service_preference_validation_and_dense_visibility`: 201 IDs, canonical UUID-v4/variant, duplicate, safe-version and unknown-ID parity assertions passed in the merged 8-test backend feature run.
- [x] AC-03: `mcp_service::tests::service_preference_stable_order_relevance_cap_and_legacy`: stable dense preference order, inert stale/platform IDs; passed in the merged 8-test backend feature run.
- [x] AC-04: The same pure MCP regression covers ties, relevance priority, empty query and the 25-result cap; passed in the merged 8-test backend feature run.
- [x] AC-05: Empty-map compatibility assertion passed in the feature test; all 5 existing `search_all_tools` regressions and `asking_for_an_agent_finds_agent_creation_first` passed from the fresh merged test binary. No ignored tests.
- [x] AC-06: Pure MCP regression checks connected-service count and rank/null fields; passed in the merged 8-test backend feature run.
- [x] AC-07: `service_preference_http_cas_noop_legacy_and_chain`: both first-insert/later CAS races, no-op equality, BSON timestamp equality and missing-version legacy upgrade; passed against the dedicated replica set through mounted HTTP middleware/auth.
- [x] AC-08: `service_preference_http_scopes_stale_slug_config_and_validation`: verified scoped API-key GET preserves raw version and exposes only one ranked ID; passed in the merged 8-test backend feature run.
- [x] AC-09: The same HTTP test deletes through `/keys/{id}`, compares the untouched stored preference document and dense surviving detail rank; passed in the merged 8-test backend feature run.
- [x] AC-10: Mounted HTTP test exercises API-key, service-account, relay, delegated and OAuth identities on GET/PUT; passed in the merged 8-test backend feature run.
- [x] AC-11: `service_preference_command_monitoring_bounds_detail_and_single_listing_read`: one preference find per endpoint, absent/empty preference GET with zero inventory reads, page aggregate two preference reads, saved GET/detail selected-ID bound at 200, endpoint projection and no credential/provider metadata loads; passed in the merged 8-test backend feature run.
- [x] AC-12: `service_preference_discovery_guest_dense_and_explicit_target_unchanged` passed: real MCP search/list, native null metadata and scoped/guest/relay dense filtering.
- [x] AC-13: Real MCP regression passed the same unranked named tool through `nyx__call_tool` before/after preference: exactly two upstream effects and identical audit event type/data/actor/target/count. Mounted HTTP test passed full `/mcp/config` equality; MCP passed byte-identical `tools/list`. The first tools/list snapshot follows baseline execution; the audit comparison selects `mcp_tool_call` rows independently of initial activation/request audits.
- [x] AC-14: Fresh `rg` returned no preference/rank references in proxy, execution authority, approvals, billing, `handlers/service_insights.rs` or `services/service_insights_activity.rs`. Source review confirms only search/connected metadata calls use the preferred loader; execution keeps the operation catalog.
- [x] AC-15: HTTP CAS test checks exactly two changed-write audits across both races/no-op, count/version-only event data and successful chain verification; passed in the merged 8-test backend feature run.
- [x] AC-16: CLI unit checks passed 2/2 (0.01s): active slug/display and prior GET-version→PUT/typed409. Fresh merged process integration passed 2/2 (1.52s): precise table `1`/`-` rank cells, show/table/JSON equality, active slug and actual nonzero409 exit. Parser splits outer `│` and internal `┆`; a missing row prints the rendered table. No timeout increase or weakened assertion.
- [x] AC-17: Merged full suite passed (4,741 tests): shared table/overview pills follow links and precede readiness; Gamma→Alpha API order is unchanged. Real grouped browser scenario passed: chip derives hidden Beta from complete group, names it, lists all ranks in title and expands; org/service/search/Personal filters retain dense pill text, pool row shows independent Priority 7, overview and DEV use the shared renderer.
- [x] AC-18: Fresh merged grouped browser scenario passed: full three-item inventory under Personal/service/search/auto-hidden filters, toolbar unmounted, no form links, saved filter text and expanded card restored after both Save and Cancel, no service-view PUT. Page tests cover loading/refetch entry gates; browser covers empty/404 and hidden DEV entry.
- [x] AC-19: PM's merged 14-scenario real-route browser run passed, including mouse/touch dragging, keyboard movement/Escape, the divider, Rank/Unrank and live rank updates. The keyboard test waits for actual Escape layout animation and the sensor render frame before the next lift; all movement/announcement assertions remain.
- [x] AC-20: Fresh merged browser tests pass dirty-gated Save, Cancel, exact ordered IDs/version payload, grid/table transitions and Save/Cancel focus after a delayed closing inventory GET.
- [x] AC-21: Fresh merged real-route browser tests pass network retry with edits retained, actual 409→Overwrite→successful persistence, Reload resets dirty, recovery after ordinary preference refetch failure, and initial-404 compatibility.
- [x] AC-22: Request/response schema checks passed in the merged full frontend suite; canonical RFC4122 UUID-v4, duplicate/201 bound, safe version, unknown-field and missing-document/null timestamp cases.
- [x] AC-23: Fresh merged frontend passed 463 files/4,741 tests (168.33s), production build and lint passed (0 errors/29 unchanged unrelated warnings, no feature warnings), browser 14/14 passed (36.1s), and regenerated wizard freshness passed 1/1 (0.04s). The 201-row regression retains its default 5-second timeout. Fresh local all-target Clippy with `-D warnings` passed on Rust 1.94.1 (4m15s), log `/tmp/service-preference-merge-566ca5f9-clippy.log`; remote Rust 1.98.1 CI is recorded on the PR at its published head.
- [x] AC-24: API/OpenAPI, discovery, service-card architecture, NyxAgent and CLAUDE reviewed against revised UI. Docs distinguish Discovery from pool Priority/cascade, preserve normal order, state independent-client limits and one relevance/name/preference/stable contract. REST auth and MCP identity application are separate (including scoped relay order). Upstream release 0.66.0 is retained with no feature-specific bump. Guest Internal exclusion and active-MCP versus full-UI rank examples are documented.
- [x] AC-25: Mounted HTTP test passed the original `preference-order` slug read and OAuth preference-write rejection. Existing `curation_router_scoped_discovery_history_and_route_confinement` passed against the dedicated replica set (1.45s), preserving curation auth/routing.
- [x] AC-26: Fresh merged backend feature run passed scoped/guest/relay HTTP/MCP privacy and live org revocation: no hidden IDs or rank gaps, unchanged stale storage, canonical IDs/version/body/unknown-field/no-op checks.
- [x] AC-27: PM's fresh merged browser run passed 14/14 in 36.1s on merged 0.66 source: mouse/touch/keyboard/Escape, grouped/table/overview/DEV pills, complete-group chip, preserved filters/expanded card/no saved-view write, stale400, late legacy-org provenance enrichment and refreshed provenance, identity switch, delayed exit focus and desktop/mobile screenshots. Log: `/tmp/nyxid-service-preference-066-browser.log`.
- [x] AC-28: Merged full frontend suite and fresh browser cases prove 200 bound, fail-closed cached reads, 404, one-service rank/unrank, separate identity caches/late mutation rejection, deferred recovery identity switch, inventory-failure retry and actual400 inventory-first recovery. New metadata-only enrichment regression preserves draft IDs/version when source arrives late and when a legacy inventory is refreshed.


Passing baseline check commands on the prior rebased source (`35af1701`):

- `NODE_ENV=test npx vitest run --config /tmp/nyxid-service-preference-vitest.config.mts --maxWorkers=2`: PM's final recheck passed 458 files / 4,694 tests (183.97s), log `/tmp/nyxid-service-preference-integrated-full-recheck.log`. Isolated happy-dom origin is the only temporary config override. This supersedes the earlier 4,693-test pass and the subsequent concurrent run's 201-row timeout.
- The same command with `src/pages/keys.test.tsx src/pages/service-overview.test.tsx src/schemas/service-preference.test.ts src/hooks/use-service-preference.test.tsx src/components/dashboard/service-preference-editor.test.tsx src/components/dashboard/service-routing-preview.test.tsx src/components/dashboard/service-pool-routing-panel.test.tsx src/hooks/use-service-view.test.tsx src/lib/service-view.test.ts`: 99 passed (12.09s), log `/tmp/service-preference-integrated-targeted.log`.
- `npx playwright test e2e/service-preference.spec.ts --workers=1`: PM's final run passed 14/14 (39.8s), log `/tmp/nyxid-service-preference-integrated-browser.log`. This supersedes the earlier keyboard-timing failure and includes legacy provenance race/recovery.
- After the singular-copy correction, `npx playwright test e2e/service-preference.spec.ts --workers=1 --grep 'read failure fails closed'`: 1/1 passed (5.1s), including the exact `1 connection` header, read Retry, one-service unranking/persistence, 404 and empty state. Log `/tmp/service-preference-singular-browser.log`; ESLint on the two changed files passes with no warnings/errors. The full frontend/browser evidence above remains applicable to this copy-only delta.
- The isolated Vitest command targeting `src/components/dashboard/service-preference-editor.test.tsx`: 3/3 passed (3.46s), then the verbose run passed again (3.14s; boundary test 572ms) after row-scoped queries and mock reset, with the normal 5-second timeout. PM's final full recheck also passed this file (2.26s execution). ESLint on this changed test file passes with no warnings/errors.
- `npm run build`: PM's fresh production run passed, including TypeScript, shipped app/prerender/credential-accept bundles and mock-footprint assertion. The wizard uses the separate freshness gate below.
- `npm run lint -- --no-warn-ignored`: exit0, no errors, 29 existing unrelated warnings; no changed-feature warnings. Log `/tmp/service-preference-integrated-lint.log`.
- `cargo fmt --all` and `git diff --check`: passed. Static boundary scan includes both new service-insights files and returns no matches.
- `cargo clippy -p nyxid -p nyxid-cli --all-targets -j 1 -- -D warnings`: final run passed, exit0 (5m38s), log `/tmp/service-preference-final-clippy.log`, local Rust 1.94.1. The earlier E0283 collection type and CI 1.98.1 single-clone lint are corrected without suppressions. CI's Rust 1.98.1 recheck is scheduled for PM's next push; no remote pass is inferred.
- `cargo test -p nyxid-cli --bin nyxid service_preference -j 1 -- --test-threads=1`: 2/2 passed (0.01s; build 2m43s), log `/tmp/service-preference-final-cli-unit.log`. The first process integration run passed the real 409/nonzero-exit scenario and failed only the table separator assumption. `/tmp/service-preference-cli-table-diagnostic.log` captures the actual correct rank cells and `┆` delimiters.
- `cargo test -p nyxid-cli --test service_preference -j 1 -- --test-threads=1`: corrected final run passed 2/2 (1.99s), log `/tmp/service-preference-final-cli-integration.log`, including actual CLI table/show/JSON and real409 exit; no ignored tests.
- `cargo test -p nyxid-cli --test wizard_bundle_freshness -j 1`: 1/1 passed (0.06s), log `/tmp/service-preference-final-wizard-freshness.log`; no ignored tests.
- `cargo test -p nyxid --bin nyxid-server service_preference -j 1 -- --test-threads=1`: final run passed 8/8 with zero ignored tests (3.15s; build 12m34s), log `/tmp/service-preference-integrated-backend-recheck.log`. DB URI and bounded task-target/debug/incremental environment from §11 are set explicitly. The superseded first attempt passed 7 and failed the MCP fixture (3.67s), log `/tmp/service-preference-integrated-backend-tests.log`: its nested `nyx__call_tool` wrapper was corrected to one direct dispatch, and the unmatched `proxy` query was replaced with the actual generic operation name `request`. All original execution/effect/audit and ranking assertions now pass.
- While that fixture-only rebuild ran, the successfully compiled rebased binary `/tmp/nyxid-service-preference-target/debug/deps/nyxid_server-c7db09510f7e99fe` was invoked directly, with the explicit §11 DB URI and `--test-threads=1`, for unchanged existing regressions: `search_all_tools` (5 passed, 0.06s), `asking_for_an_agent_finds_agent_creation_first` (1 passed, 0.06s), and `curation_router_scoped_discovery_history_and_route_confinement` (1 passed, 2.00s). No second Cargo build or skipped DB test. Logs: `/tmp/service-preference-search-tests.log`, `/tmp/service-preference-agent-search-tests.log`, `/tmp/service-preference-curation-tests.log`.
- PM independently ran the same curation regression against the explicit isolated Mongo URI and compiled binary: 1 passed (2.04s), log `/tmp/nyxid-service-preference-curation-regression.log`.
- The same direct binary/DB invocation passed neighboring assistant regressions: `chat_mcp_lists_ungranted_tools_and_allow_retries_execute_without_bypassing_denial` (1 passed, 2.65s), `guest_access_follows_spec_markers` (1 passed, 1.71s), and `chat_discovery_does_not_write_request_audits_but_execution_refusals_do` (1 passed, 2.34s). Logs: `/tmp/service-preference-assistant-discovery-tests.log`, `/tmp/service-preference-guest-scope-tests.log`, `/tmp/service-preference-assistant-audit-tests.log`.

Checkpoint `abc2cfde` rebased from `ffcd1c59` onto `f3dc2be9`, producing `d32f6111`. PM committed/pushed the integrated source as `caed88dfa5e449af062b77d2fd264b3c05e4d1b1` and opened draft PR [#1796](https://github.com/ChronoAIProject/NyxID/pull/1796) for remote CI; this is not sign-off. PM owns the review record and publication; the implementer performs no commit/push/PR actions. Browser screenshots use Playwright output paths and attachments; PM retained reviewed copies at `/tmp/nyxid-service-preference-review/integrated-*.png`.

### Implementation findings reconciled

- Owner chat discovery intentionally includes acknowledgement-required services
  (`mcp_chat_authority_tests` existing listing assertions). Owner tests retain that
  contract; scoped ordinary keys and guest discovery prove hidden-ID/dense-rank
  filtering. Guest filtering affects metadata only; grant/approval/execution gates
  are unchanged.
- `search_all_tools` and the unranked connected-list wrapper are now test-only
  compatibility helpers in this binary crate; production uses ranked helpers.
- First saves use BSON millisecond precision so PUT, GET and no-op timestamps agree.
- The search doc comment belongs to production `search_all_tools_ranked`; the sorter documents its stable ranked/unranked order separately.
- CLI table integration splits the preset's outer `│` and internal `┆` characters, verified against actual CLI output, and retains precise `1`/`-` last-column assertions plus table/show/JSON cases. A missing row includes the full rendered fixture table.
- Rust 1.98.1 CI's `cloned_ref_to_slice_refs` fixture finding is corrected with `std::slice::from_ref(&b)`; no lint suppression or production behavior change.
- Editor grid/table modes share one vertical list with roomier/compact spacing;
  after #1685 the saved pills live on `ServiceConnectionTable` rows and the
  `GroupCard` chip (§6.1), not on the deleted card/row components.
- Empty onboarding fixture objects trigger the real first-run takeover. The API
  fixture supplies `ai_services_completed_at` so the real services route is
  available without introducing a test-only product flow.
- Editor metadata uses the tab's same source join on initial data, every refresh,
  and rendering. Late provenance enriches metadata only; it never resets draft
  IDs or the optimistic version.
- Keyboard browser input waits for actual Escape layout animations to settle
  and the activation render frame before issuing the next movement. Assertions
  still require the real sensor, destination, one live region and persisted rank.

## 18. Upstream integration delta (`origin/main` `f3dc2be9`)

Commits since the plan baseline `ffcd1c59`: #1790 (26 fixed-endpoint OAuth
connectors), #1792 (pool accounting by billing request), granular display
controls and Studio breadcrumbs, #1791 (LinkedIn OAuth), #1794 (MCP
registration / consented grants), #1685 (AI Services refresh). Only #1685
changes this plan's surface. The discovery-only backend design and the
semantics of AC-01..AC-28 are unchanged; the deltas below are what the
implementer must rebase onto.

Backend (expected merge points, no design change):

- `handlers/keys.rs`: `KeyResponse` gained `can_edit_configuration` and
  `oauth_app_source`; add `preference_rank` beside them in
  `key_response_from_view` and `key_response_from_result`.
- `handlers/mcp_transport.rs`: `McpAuthContext` gained `oauth_client_id` and
  `api_key_credential_id` (also in the `api_key_auth` test helper), and
  `mcp_exec_context` now carries request attribution for service insights. The
  preferred loader for search/list-connected is independent of both; keep the
  execution path on the unchanged operation catalog.
- `routes.rs`: upstream added `/me/preferences/services` (PUT) and
  `/service-insights` (GET); mount `/service-preferences` beside them per §5.
- New upstream `GET /service-insights` reads connection metadata only and must
  not consult `service_preferences` (covered by AC-14's grep; add
  `backend/src/handlers/service_insights.rs` and
  `backend/src/services/service_insights_activity.rs` to that grep list).

Frontend (re-implement on the upstream files rather than merging the old
`keys.tsx`; the upstream rewrite deleted every pre-#1685 render path this branch
touched):

| Pre-#1685 integration point on this branch | Upstream replacement | Action |
|---|---|---|
| `KeyCardContent` badge row pill | `GroupCard` collapsed chip + `ServiceConnectionTable` row pill | move (§6.1) |
| `ServiceTableRow` Name cell pill | `ServiceConnectionTable` Connection / Slug cell | move (§6.1) |
| `groupKeysBySource(...sort by preference_rank)` | `groupServiceConnections` + `matchingConnections`, alphabetical groups | drop the sort; pills only |
| page-level `showAutoConnected` `Switch` forced on in reorder mode | toolbar `Auto-connected` toggle inside `ServiceViewToolbar`, part of saved filters | do not touch; editor uses the complete inventory and unmounts the toolbar |
| `ExternalServicesTab` flat grid/table | `GroupedServiceCards` with `renderTable` | swap in the editor in place of `GroupedServiceCards` while editing |
| `keys.test.tsx` (pre-#1685 mocks) | upstream harness with `QueryClientProvider`, `use-pools`, `use-service-routing-pools`, `use-orgs` mocks | re-add preference tests on the upstream file; add a `use-service-preference` mock |
| `types/keys.ts` `preference_rank` | upstream added `can_edit_configuration`, `oauth_client_id`, `connection_id`, `oauth_app_source` | keep additive field |
| e2e fixture (`/keys`, `/user-services`, `/nodes`, `/orgs`, `/catalog`) | page also requests `/service-insights`, pools, `/catalog?include_all=true`, saved view in `/users/me` | extend fixture per §11 |

Preserved upstream behavior that the integration must not alter: default
`source: "personal"` and `show_auto_connected: false` filters, Organization and
Service multiselects, search-on-submit, filter pills and Clear filters, saved
default views and their 404 compatibility note, alphabetical group order,
single expanded card with view transitions, sticky toolbar measurements,
insight/billing/usage panels, pool summaries and routing panel,
`/keys/services/$groupId`, key-detail tabs, `?view=routing` DEV preview,
`?pool`/`?org` deep links to Service Pools, and the Connect Service CTA
placement (toolbar when keys exist, header otherwise).

Root-owned review document `docs/plans/service-tool-preference-review.md` is
not modified by this revision.

## 19. Release 0.66 integration (`566ca5f9`, #1795; committed as `0390ad00`)

PM requested `git merge --no-commit origin/main` on the already published branch
at `35af1701`, preserving its plan-only working diff and the PM-owned review
record. After all local gates passed, PM committed the merge and corrections as
`0390ad001d23c1b18fcfe7de997f84382de58d6c`; it was not pushed at the start of §20.
PM owns publication, renewed CI and final review of draft PR
[#1796](https://github.com/ChronoAIProject/NyxID/pull/1796).
The results below establish the backend/CLI/Clippy evidence used in §17. The
later frontend-only integration supersedes UI evidence in §20.

Actual conflicts were limited to `handlers/mod.rs`, `models/mod.rs`, the generated
wizard `assets/index.html`, and `bundle-meta/index.hash`. Both preference and
upstream concurrency module declarations are retained. The wizard is regenerated
from the merged frontend source instead of choosing either generated side.
The original plan-only working changes are retained. The PM review record was
byte-identical at the merge boundary; subsequent PM-owned review updates are
preserved, and the implementer has not edited that file.

MCP transport/service merged automatically and were inspected against upstream:

- Both discovery catalog construction sites retain async
  `assistant_account_tools::virtual_service_for(&state.db, chat).await`, including
  upstream skill-authoring flag, role and failure behavior. Guest service and
  platform allowlist filtering still precedes dense preference ranking.
- Only search and list-connected call the preferred loader. `tools/list`, config,
  operation discovery and explicit execution retain their existing catalogs and
  execution gates; native search rows remain explicitly unranked.
- Upstream's verified `caller_token`, `scope`, resource claims, acting identity,
  delegation restriction projection and scheduled-key route admission are
  retained. Preference does not replace or alter the new execution context.
- Direct/node delegation and credential-owner parity, concurrency admission,
  typed 429/Retry-After and response-body lease lifetimes remain upstream's
  implementations. Preference adds no execution retry or provider substitution.
- Release 0.66.0 version changes are retained from upstream; this feature adds no
  separate version bump. No preference UI source needed a conflict resolution.

Fresh checks on this merged source (one Cargo build at a time, `-j 1`, explicit
§11 replica-set URI, task target `/tmp/nyxid-service-preference-target`, both debug
profiles 0 and incremental disabled):

- [x] `npm --prefix frontend run build:wizard`: exit0; TypeScript and wizard
  production build passed, regenerated 168-file source closure and embedded
  assets/hash. Log `/tmp/service-preference-merge-566ca5f9-wizard-build.log`.
- [x] Conflict-marker scan: no markers or unmerged paths. `cargo fmt --all`
  applied module-order formatting; `cargo fmt --all -- --check` and staged/working
  `git diff --check` pass. The exact
  `service_preferences|preference_rank` execution-boundary scan has no matches;
  unrelated existing HTTP `preference-applied` headers are not feature reads.
- [x] `cargo test -p nyxid --bin nyxid-server service_preference -j 1 --
  --test-threads=1`: 8 passed, 0 failed/ignored (3.21s; compile 6m09s), log
  `/tmp/service-preference-merge-566ca5f9-backend-feature.log`. Includes the new
  mounted GET command-monitoring assertions and all CAS/auth/privacy/audit/MCP
  execution invariants. Fresh binary: `nyxid_server-a548fe5994bb0fe3`.
- [x] Neighboring search, assistant guest/discovery/audit and curation regressions:
  10 passed, 0 failed/ignored, using the fresh merged binary and explicit DB URI.
- [x] Upstream MCP delegation/proxy parity, async skill-authoring discovery,
  concurrency and org-agent revocation: 17 passed, 0 failed/ignored, from the same
  merged binary. Exact filter/results table below.
- [x] `cargo test -p nyxid-cli --bin nyxid service_preference -j 1 --
  --test-threads=1`: 2/2 passed (0.01s; compile 32.26s), log
  `/tmp/service-preference-merge-566ca5f9-cli-unit.log`.
- [x] `cargo test -p nyxid-cli --test service_preference -j 1 --
  --test-threads=1`: 2/2 passed (1.52s; compile 21.98s), log
  `/tmp/service-preference-merge-566ca5f9-cli-integration.log`; exact saved-rank
  table/show/JSON and actual nonzero409 exit retained.
- [x] `cargo test -p nyxid-cli --test wizard_bundle_freshness -j 1`: 1/1 passed
  (0.06s; compile 21.09s), log
  `/tmp/service-preference-merge-566ca5f9-wizard-freshness.log`.
- [x] `cargo clippy -p nyxid -p nyxid-cli --all-targets -j 1 -- -D warnings`:
  exit0, no warnings/errors (4m15s), local Rust 1.94.1, log
  `/tmp/service-preference-merge-566ca5f9-clippy.log`.
- [x] PM-owned full isolated frontend suite: 461 files / 4,723 tests passed
  (168.67s), exit0, log `/tmp/nyxid-service-preference-066-full-frontend.log`.
  This is fresh merged-0.66 evidence, including the final alias/dead-branch cleanup.
- [x] PM-owned browser: 14/14 passed (37.3s), log
  `/tmp/nyxid-service-preference-066-browser.log`. Fresh desktop/mobile artifacts
  visually reviewed at `/tmp/nyxid-service-preference-review/066-*.png`.
- [x] PM-owned production build: exit0, log
  `/tmp/nyxid-service-preference-066-build.log`.
- [x] PM-owned lint: exit0, zero errors/feature warnings and the same 29 unrelated
  baseline warnings, log `/tmp/nyxid-service-preference-066-lint.log`.
- [ ] Renewed remote CI after PM's merge commit/push; no remote pass is inferred.

Opus preliminary findings on `35af1701` are incorporated before Rust compilation:

- Preference GET reads its document first and uses the extracted
  `visible_ordered_ids` helper shared with detail rank. Absent/empty orders and
  scope-excluded saved IDs return before inventory reads. The command-monitoring
  test now asserts mounted GET's exact one-read/zero-inventory behavior, raw
  empty-document metadata, 200-ID saved visibility bound, endpoint projection
  and no provider/credential metadata reads. All assertions passed in the fresh
  eight-test backend feature run above.
- Guest connected search/list's granted UserManaged/Platform-only behavior and
  exclusion of Internal catalog entries are explicit in chat 08 and 09; native
  virtual tools retain their separate guest authorization.
- API discovery and AI Services architecture explain active MCP versus full UI
  rank bases, including disabled UI #1 causing active UI #2 to become MCP #1.
- Normative text corrects the user-before-platform loader order, shipped stale
  400 banner, latest integration status and obsolete #1685 rerun statement.
  §5.1/§8 document the bounded GET work rather than a full metadata inventory walk.
- `keys.tsx` drops the uncalled compact/icon branch and redundant inventory alias.
  The optional tab-only query gate was not added, preserving existing query,
  focus, compatibility and identity behavior. Focused isolated Vitest passed
  3 files/33 tests (4.62s), log
  `/tmp/service-preference-merge-566ca5f9-frontend-targeted.log`.

PM committed the validated source and wizard as `0390ad00`. PM inspected the merged MCP,
shared GET visibility helper, docs and UI cleanup. The eight-test feature run,
all neighboring/upstream backend filters, CLI/wizard/Clippy and all fresh frontend
gates have passed. All AC-01..AC-28 now have current local evidence. There are no
known unresolved implementation findings; PM's final sign-off and renewed remote
CI remain separate gates.

After these checks, PM fetched frontend-only `868ce0b7d01a6193ad7a6cff77147bd5163bac9a`
(76-file readability change, no Rust source). It was held until PM committed the
0.66 integration. Its separately authorized merge and fresh UI evidence are now
tracked in §20; completed backend evidence remains applicable while Rust source
is unchanged.

The first Cargo invocation built the merged test binary successfully. A later
cached Cargo invocation retriggered the build scripts because their relative
`.git` watch paths are absent in this linked worktree. The redundant task-owned
compile was stopped; no source changed after the successful build. Remaining
backend regressions were run directly as
`/tmp/nyxid-service-preference-target/debug/deps/nyxid_server-a548fe5994bb0fe3 <filter> --test-threads=1`,
with the same explicit §11 DB/task-target environment. No old-base binary or
zero-test result is counted.

| Fresh merged backend filter | Passed | Test time | Log suffix under `/tmp/service-preference-merge-566ca5f9-` |
|---|---:|---:|---|
| `search_all_tools` | 5 | 0.00s | `search.log` |
| `asking_for_an_agent_finds_agent_creation_first` | 1 | 0.01s | `agent-search.log` |
| `curation_router_scoped_discovery_history_and_route_confinement` | 1 | 1.45s | `curation.log` |
| `chat_mcp_lists_ungranted_tools_and_allow_retries_execute_without_bypassing_denial` | 1 | 1.55s | `assistant-discovery.log` |
| `guest_access_follows_spec_markers` | 1 | 1.55s | `guest-scope.log` |
| `chat_discovery_does_not_write_request_audits_but_execution_refusals_do` | 1 | 1.44s | `assistant-audit.log` |
| `mcp_delegation_` | 6 | 1.92s | `mcp-delegation.log` |
| `mcp_proxy_` | 4 | 3.12s | `mcp-proxy-parity.log` |
| `skill_authoring_discovery_tests` | 2 | 0.59s | `skill-discovery.log` |
| `handlers::mcp_transport::tests::service_concurrency_` | 2 | 0.35s | `mcp-concurrency.log` |
| `handlers::proxy::proxy_resolution_integration_tests::service_concurrency_` | 2 | 1.16s | `proxy-concurrency.log` |
| `org_agent_mcp_tests` | 1 | 1.34s | `org-agent-mcp.log` |

## 20. Frontend readability integration (`868ce0b7`)

PM authorized `git merge --no-commit origin/main` from committed integration head
`0390ad001d23c1b18fcfe7de997f84382de58d6c` onto fetched
`868ce0b7d01a6193ad7a6cff77147bd5163bac9a`. PM inspected the merged source and
completed fresh full frontend/browser/build/lint checks before committing and
publishing the integration. The PM review record is preserved and is not edited
by the implementer.

The only conflicts were generated `cli/src/wizard/assets/index.html` and
`cli/src/wizard/bundle-meta/index.hash`; both are resolved by regeneration from merged source.
Grouped cards, the shared connection table, keys page and architecture docs
merged automatically and were inspected against upstream. Preserve upstream
`text-primary-text`/control-height readability tokens, pool strategy icons,
wrapping and billing/usage changes. Discovery pills remain beside the connection
link, the collapsed chip still derives from complete `group.connections`, and
the editor retains its inventory/filter/identity/recovery/focus behavior.

The editor's information icon explicitly adopts `text-primary-text` to follow
the upstream readability contract. AC-12's text now says "a nonempty" query.
No backend/CLI Rust source, execution semantics or preference visibility logic
changed. The completed §19 backend/CLI/Clippy evidence is retained without a
repeat backend build. Only wizard freshness is affected by the rebuilt embedding.

Fresh evidence on this frontend merge:

- [x] `npm --prefix frontend run build:wizard`: exit0; TypeScript and wizard
  production build passed, merged embedded assets and 168-file source closure
  regenerated. Closure hash prefix `c960ab3c6057`; log
  `/tmp/service-preference-merge-868ce0b7-wizard-build.log`.
- [x] No unmerged paths or conflict markers. Staged/working `git diff --check`
  passed. `git diff 0390ad00 --name-only -- backend cli ':!cli/src/wizard'`
  has no output: backend/CLI Rust source and manifests are unchanged. PM review
  record was byte-identical to its pre-merge backup at this verification boundary;
  subsequent PM-owned updates remain preserved.
- [x] `cargo test -p nyxid-cli --test wizard_bundle_freshness -j 1`: 1 passed,
  0 failed/ignored (0.04s; focused CLI rebuild 26.71s), log
  `/tmp/service-preference-merge-868ce0b7-wizard-freshness.log`. Uses the §11
  explicit DB URI/task target/debug=0/incremental=0 environment and checks the
  rebuilt merged 168-file source closure.
- [x] PM-owned full isolated frontend suite: 463 files / 4,741 tests passed in
  168.33s. Log `/tmp/nyxid-service-preference-868-full-frontend.log`; the same
  origin-only temporary config described in §17 preserves the repository setup.
- [x] Real-route browser suite: all 14 scenarios passed in 36.1s, including
  actual mouse/touch/keyboard input, persistence, complete-group/table/overview
  pills, preserved views, delayed focus, conflict/stale recovery and account
  switching. Log `/tmp/nyxid-service-preference-868-browser.log`. PM inspected
  fresh desktop and 390-pixel mobile screenshots under
  `/tmp/nyxid-service-preference-review/868-*.png`; labels wrap without overflow.
- [x] Production build and lint passed, with zero lint errors/feature warnings
  and the same 29 unrelated baseline warnings. Logs
  `/tmp/nyxid-service-preference-868-{build,lint}.log`.
- [x] PM personally inspected the final source diff and all validation logs;
  every substantiated plan, implementation and preliminary Opus finding is closed.

PM publishes this integration on PR #1796. Required remote CI and Opus's final
review are recorded there against the published head, so their final verdict does
not require another source or evidence-only commit.

Source/wizard are complete and ready for PM's review. PM inspected the merged UI
and accepted preservation of upstream readability/service-table/pool behavior,
preference pills/editor and the information-icon token correction. All fresh
frontend/browser/build/lint checks passed. No frontend source edits remain. The focused CLI wizard
freshness check passed; backend regressions, CLI preference tests and Clippy are
not repeated because Rust source is unchanged and the only CLI delta is the
regenerated embedding. No known unresolved source findings remain. The fresh
§20 UI results supersede §19's UI baseline; backend evidence remains applicable.

## 21. Follow-up normal-page comparison and label spacing

The PM freshly verified that current main `868ce0b7` is included in the branch
and rendered main and the PR with identical sample data. The normal External
Services page retains main's grouped cards, filters, connection table, billing
and routing presentation. Editor screenshots must be identified as the Reorder
editing state.

The inline rank pill could consume all horizontal space for the connection-label
text. Task 13a and §6.1 now place it in metadata below the existing label/readiness
header. AC-17 additionally requires usable rendered label width, no badge overlap
at 1440 pixels, and visible rank metadata with contained table scrolling at
1024/390 pixels. The new browser regression measures the actual label span rather
than its icon-bearing link. Shared-renderer unit tests assert rank text in the
correct connection cell; the saved-view scenario checks semantic filter values
and absence of writes.

- [x] PM directly reviewed the source and all follow-up test corrections.
- [x] Focused frontend: 3 files, 60 tests passed in 4.53 seconds.
- [x] Real-route browser suite: 15/15 passed in 38.8 seconds, including the expanded
  grouped-table geometry and narrow-table containment regression.
- [x] Production build passed; changed-file ESLint has zero errors/warnings;
  whitespace checks passed.
- [x] Main/PR visual comparison: 2/2 passed in 6.7 seconds; PM inspected the
  corrected collapsed and expanded screenshots with identical sample data.

The follow-up changes no Rust source or wizard dependency, so the existing
backend/CLI proof remains applicable. The PM records the revised head's required
CI and Opus plan/PR sign-off in PR #1796. Full evidence and the temporary,
production OAuth-backed local review setup are in the PM review record.
