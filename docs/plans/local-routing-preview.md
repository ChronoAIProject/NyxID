# Local routing preview with production metadata

Refreshed 1 October 2026 from main `be1883bd` (frontend v0.39.0), including
priority failover, round-robin/weighted rotation, operation-scoped pool health,
and the latest service icon registry.
Grouped cards and service overview headers show the catalog icon; a group with
one connection also uses its custom icon. Each connection row shows its own icon
override, with the service glyph as fallback. Auto-connected and custom services
can resolve their glyph by service slug without a catalog identifier.
The prior preview is preserved in stash `e71429e8d5d19a1668df4f2e2925abba37625f61`.
Updated 27 September: the actual React frontend now renders one collapsed card
per catalog service, with its real configured connections inside. The same
`GroupedServiceCards` component is used by the normal `/keys` grid and the local
production-data view at `/keys?view=routing`. This is application code, not the
standalone HTML reference. Current main's full detail pages, authorship/history,
org permissions and reconnect flows are retained.

Pool integration validation: 102 focused frontend tests and all 6 backend
pool billing tests passed. The backend tests used an isolated MongoDB 8.0.16
replica set, including a regression joining each attempt's service history to its
billing request ID. TypeScript, the production frontend build, changed-file lint
and whitespace checks passed. Signed-in visual review is still outstanding.

Updated 2 October: collapsed service cards are 288px tall with two description
lines and the existing icon/name/count and footer navigation. The body shows a
billing summary (including disabled connections), pool
member count/strategy and failover, deduplicated agent-key count, last-use time,
and latest recorded edit. Source avatars sit at the body's bottom right. The
expanded table exposes per-connection billing, pool membership, access/use, and
edit history. Clicking billing selects the first billable connection; clicking
last edit opens the affected connection's history. No pool editor is mounted in
AI service cards.

102 focused frontend tests, 14 backend billing projection tests, production/credential-accept
builds and changed-file lint passed for this revision. Signed-in visual review remains unavailable.
The backend now reports configured billing separately from execution availability;
the production-data preview uses published inventory metadata until that backend
change is deployed. Missing data remains unverified.

Updated 5 October after an Opus 5.5 xhigh review: billing checks the service-wide
configuration first, then the connection's credential supplier. Unpriced services
show **—** even when a person supplied an API key. A billable service can contain
NyxID, BYOK and Unverified connections. The connection's fee is a separate fact.
See [the decision table and backend contract](service-billing-labels.md).

Read-only live metadata verification through the actual classifier produced:

- Anthropic: all 30 connections unpriced, so one dash on the card.
- Chrono LLM and Spotify: dash.
- Twitter: expected **3 NyxID · 1 BYOK**: the owner confirms the three personal
  OAuth connections used NyxID's app. Production still omits their app source,
  so the current preview reports three Unverified until the shared backend
  resolver is deployed. It covers unmarked modern keys and legacy provider-token
  provenance. Unresolved metadata must not be guessed as either NyxID or BYOK
  from the presence of a token record.
- DeepSeek: one stored API-key connection on the supplied-key path, so BYOK.
- Custom MacBook SSH: dash. Three other MacBook connections refer to private
  catalog entries unavailable through production discovery, so billing remains
  Unverified until the backend insight projection is deployed.

The backend additions expose OAuth app provenance on `/keys` and the service-wide
billing gate plus selected supplier on `/service-insights`. These are additive,
read-only changes. They have not been deployed to production. Twitter's currently
configured 0.05 credits/request platform-key price does not apply to OAuth under
the existing execution rules; this revision does not change prices or charging.

The dash tooltip remains beside the icon/value with an 8px gap. Longer details
align below the content with viewport collision handling. The local frontend uses
production metadata; signed-in visual inspection is unavailable in this session
because no browser is connected.

Validation for the billing correction: 110 focused frontend tests and four backend
metadata regressions passed, along with TypeScript, production/credential-accept
builds, changed-file ESLint and whitespace checks. The backend test target compiled.

Run from `frontend/`:

```sh
NYXID_ROUTING_PREVIEW=1 \
BACKEND_URL=https://nyx-api.chrono-ai.fun \
FRONTEND_URL=https://nyx.chrono-ai.fun \
npm run dev -- --host 127.0.0.1 --port 4317 --strictPort
```

Open <http://127.0.0.1:4317/keys?view=routing>. If sign-in is required, open
<http://127.0.0.1:4317/__routing-preview/login>. The production site's existing
CLI Authentication flow sends a fresh session to the local preview. The access
token stays in server memory for at most 15 minutes; refresh tokens are discarded.
The browser receives an opaque HttpOnly session. Restarting Vite or session expiry
requires signing in again. Do not use `?mock`; that is an unrelated fixture mode.

The gateway accepts allowlisted metadata GETs, including `/service-pools`,
UUID-addressed pool details, candidates and health, candidate discovery,
service history and Codex connection metadata from current main. Key and node
detail reads require UUID paths. Pool mutation requests, execution, credential
reveal and other mutation endpoints are blocked, except for the authenticated,
same-origin `PUT /users/me/preferences/services` with a validated filter payload. The existing backend's human
`GET /keys` still performs its normal platform auto-provisioning/reconciliation;
the gateway does not change that server behavior. Logout clears the local session.
The gateway and routing-specific diagnostics are development-only. Grouped service
cards and filter controls are also used by the normal production frontend.

## Live frontend walkthrough — 27 September 2026

1. Open **External Services**. Each shared catalog service appears once with the
   real connection count. Custom services remain separate by immutable identity.
   All groups start collapsed on a fresh page load.
2. Click the service title or **View N connections**. The parent card grows to
   contain a compact comparison table: Connection/Slug, Classification, Status,
   Configuration and Activity. Editors have a Configure action; every visible
   connection has a History action.
3. Status reflects known restrictions or missing verification. Activity shows
   the latest recorded configuration change; no successful route or last caller
   is invented.
4. Click a connection to open its existing full detail page and History tab.
   Returning to services preserves group expansion and search in this browser
   session, scoped to the account. Refreshing starts collapsed again.
5. Organization and Service have separate named selectors. Search matches names,
   slugs and owners. Only matching rows appear in an expanded card; the full
   group count and Service details link preserve context.
6. Each collapsed service card shows its saved **Pool** name and strategy, plus
   **Failover**: for example, `Up to 3 attempts`, `Off · single attempt`, or
   `Pool disabled`. Multiple pools show the additional count and how many have
   failover enabled. Hover/focus shows each policy and its pool slug. These
   summaries describe configured policies, not successful health probes.
7. Click the pool summary to see priority/rotation, the pool slug,
   eligibility/cooldown and billing per member inside the card. This inspection
   is read-only. **Manage in Service Pools** opens the selected pool, selects its
   owner and scrolls its expanded card into view. **Configure** and member/policy
   editing live only in Service Pools. The real revision-checked editor allows
   drafting in this production-data preview but disables Save and mutations.

The service-grid changes passed 98 focused frontend tests, TypeScript checking,
targeted lint and the production build. The running Vite server serves the updated
modules and reports the production API URL in runtime config. No browser was
connected for signed-in visual inspection.

## Card sizing and account filter defaults — 28 September 2026

Collapsed summaries now share a 256px minimum height with reserved description
space and aligned footers. Filters cover search, source, enabled/disabled state,
HTTP/SSH type and auto-connected inclusion. They apply to the normal grid/table
and the production-data preview. A partial group match shows only matching rows and
shows their count against the complete group.

The backend implementation stores **Save as default** in
`users.profile_config.services_view`; see
[AI Services Architecture](../AI_SERVICES_ARCHITECTURE.md#service-cards-and-saved-filter-defaults).
The local production-backed preview can filter immediately. Account saving is
available only when its connected backend exposes this field on `/users/me`.
The backend addition in this worktree has not been deployed to production.
There is no local-storage substitute for an account save.

Validation: 159 frontend/gateway/authentication tests and 21 backend profile
tests pass, including real MongoDB preference persistence and sibling-setting
preservation. TypeScript, targeted ESLint, Rust formatting and the frontend
production build pass. The updated live modules return HTTP 200. Signed-in visual
inspection remains unavailable because no browser is connected.

## Readiness and execution boundary

Pool health comes from the saved configuration and the selected operation. Eligible
means the member passed metadata/admission inspection; it is not an upstream probe.
Do not infer working credentials from `status: active`, recent credential preparation
or node presence. Failed inspection shows unverified health, including when cached
results previously said eligible.

Priority pools use the saved failover policy and durable cooldown; round-robin and
weighted pools select once. A null policy uses priority defaults. Direct connection
slugs keep their normal semantics. The UI does not create a routing policy merely
because multiple connections share a service card. See [Service pools](../SERVICE_POOLS.md)
for retry safety, per-attempt billing and supported entrances.

The production metadata gateway remains read-only for pools. Saving settings and
resetting cooldowns use the normal backend endpoints outside this preview. The
preview does not synthesize routes or use local storage as execution configuration.

## Connection tables and standalone selectors — 28 September 2026

Organization and Service are standalone searchable multi-select menus populated
from actual records. Each selected value has its own removable pill. Selections
within a menu match with OR; the two menus combine with AND. Organization ownership
uses circular avatars; platform sources use the NyxID icon.
Additional criteria reuse the audit log's filter panel, Apply/Cancel actions and
editable/removable chips. The saved-default blob now includes
`organization_ids` and `service_group_ids` arrays; older singular selections
migrate to one-item arrays and empty/null selections to empty arrays.
Expanded cards contain a comparison table, with the exact owner, status, latest
change and slug of each matching connection. **Service details** opens
`/keys/services/{groupId}`: Connections (the same table with row disclosures) and
per-connection History. This full page retains all accessible siblings regardless
of list filters. Each connection links to its original configuration page.

Multi-select validation: 81 focused frontend/gateway tests and 22 backend profile
tests passed, including real MongoDB persistence, legacy single-selection reads,
individual pill removal, combined selections and empty-result recovery. The
frontend production build, TypeScript and targeted lint passed. No browser was
connected for signed-in visual inspection.

## Dense connection tables and reader history — 28 September 2026

Expanded cards now have a compact header and take the grid width for the connection
table; collapsed cards retain their 256px minimum height. Native view transitions
animate card resizing and the surrounding grid, with a reduced-motion fallback.
The standalone table uses the same component. Extra dates, permissions, provisioning
source, header names and other metadata open inside a table row. The separate
per-connection information cards have been removed from the service overview.

Editors see targets, auth/routing summaries and configuration links. Members and
viewers see connection identity, ownership, state and activity, with History always
available for authorized connections. Direct detail navigation enforces the same UI
boundary. A failed history request never restores cached history.

The matching backend changes (private configuration projection and scoped reader
history) are local to this worktree and **not deployed**. The production-backed
preview shows the frontend changes, but production still enforces its deployed
history policy until the backend change is released. The preview does not claim
health-based fallback routing or last-caller data.

Validation for this revision: 131 focused frontend tests, including filters,
connection/navigation permissions and history caching; 127 backend tests covering
history (21), keys (74), user-services (14) and endpoints (18). Backend integration
tests used an isolated MongoDB 8 replica set, removed after the run. Production
frontend build and TypeScript checks pass. Targeted ESLint has no findings; full
ESLint has no errors and 27 existing warnings. Signed-in visual animation review
remains unavailable because no browser is connected to this session.

Only one service card can be expanded at a time. Opening a new card closes the
previous card in the same transition. Expanding a service scrolls its header into the main content viewport after the
card animation completes. Collapse and restored expansion state do not trigger
scrolling; reduced-motion users get an immediate reveal. Connection names now
link to their full details page for every authorized reader, including org
members and viewers. A separate chevron opens the inline summary; Configure
remains editor-only.

The filter card is sticky within the dashboard content viewport and contains the
Organization/Service selectors, search, additional filters, selected pills,
Personal/All services view switch, save/default controls, result count, collapse
and refresh actions. The card's measured height sets the expanded service's scroll
margin, including after pills wrap. Personal is the initial view without a saved
account default and excludes organization and platform sources. Selecting an
organization switches to All services; returning to Personal clears organization
selections. Saved account defaults retain the user's chosen view. Clearing other
filters preserves Personal versus All services.

Validation for the Personal default, sticky filters, exclusive expansion and reader
navigation: 104 focused frontend tests passed. The 17 routing-preview tests also
passed after the final test typing correction. Production build, TypeScript,
targeted ESLint and diff whitespace checks passed. The live preview returns HTTP
200; signed-in visual review still requires a connected browser.

Saved views now has a dedicated header control in the sticky filter card, with a
count, a preview of the saved account default and a click to restore it. Save as
default / Update default is visible alongside it; matching the saved default
shows a checked status. This retains the existing single account-default model.
Personal / All services is one pill showing the active source icon and label;
clicking it switches to the other view. No saved default or persistence behavior
was migrated. These changes use the supplied Billing screenshot as a visual
reference for AI Services; the separate Billing checkout is unchanged.

Validation: 47 focused frontend tests passed across routing preview, keys and
service-view state; production build, TypeScript, targeted ESLint and whitespace
checks passed. No browser was connected for signed-in visual inspection.

The Organization and Service triggers now align label, selection and chevron in
fixed columns, with the selected value right-aligned beside the chevron; dropdown
rows reserve consistent checkbox/avatar space. The sticky
filters have an opaque background above the card and a scroll-dependent shadow
in both themes. The background extends to the full dashboard scrollport width,
using measured gutters to hide borders and shadows from scrolled cards. When
stuck, the card keeps its rounded corners and hides Saved views and the entire
results/action footer. Controls stay on one line, with selected filter pills
below, capped at two full rows (three rows total). Extra selections scroll within
that area and snap to complete rows. Pill heights are 44px on mobile and 36px on
desktop; long labels truncate with their full text available on hover. No empty
pill row is rendered. Narrow viewports scroll the controls
horizontally. The Personal / All services pill remains alongside the filters.
Returning to the normal position restores the saved view controls and footer.
There is no fixed padding or opaque band below the sticky card.
The cover now uses an 8px backdrop blur and fades out over 28px below the card;
its mask softens passing connection borders alongside the background instead of
cutting them off at a horizontal edge. A layered shadow keeps the rounded filter
card visually above that cover. This is an overlay only and adds no layout space.
The Vite production build passed after this styling adjustment.
An expanded service's name/count and actions remain sticky below the filters
while its connection rows scroll. Its offset uses the measured filter height
plus the existing 32px reveal gap. It starts moving up when the third-last
connection reaches the header's lower edge, keeping the final three connections
clear as the card scrolls away. The release threshold uses actual connection-row
positions, excluding expanded metadata/history rows. Scrolling back restores the
normal pinned position. Only the table content clips to the card corners, so the expanded section
does not introduce a scroll container that prevents the header from sticking.
When the service header pins, an opaque cover fills its top gap and rounded
corner cutouts across the full scrollport, hiding connection text and borders
that have scrolled above the header. The
cover is absent before pinning and after collapse. Scroll/resize tracking is
limited to the expanded card and cleaned up on collapse. Validation: 54 focused
tests passed, including pin/return/collapse behavior and releasing the header at
the final three connection rows; production build and targeted lint passed.
The search field keeps the same border color and thickness on focus in both
themes. The full service overview now shows the matching catalog icon beside its
title, with the same globe fallback as custom-service cards. The 14 shared filter
control tests and 6 service-overview tests passed after these changes, along with
the production build, TypeScript and targeted lint.
Collapsed service cards no longer list connection names beneath Sources; the
names remain in the expanded connection table. Active search retains its labelled
Matches summary so users can see why a connection was included.
The service grid uses 24px gaps and expanded headers have 20px padding. Expansion
scrolls the dashboard viewport using the live toolbar height and viewport padding,
with an additional 32px reveal offset. This spacing belongs to the scroll position,
so it moves away during manual scrolling. Browser scroll anchoring is disabled
within the changing service grid. Reduced motion remains immediate.
Validation: 60 focused tests passed, including changing-toolbar-height scrolling,
hiding/restoring Saved views and footer, and selecting organization pills while
stuck. Shared data-table controls retain their normal wrapping layout. Production
build, TypeScript, targeted ESLint and whitespace checks passed. The preview is
serving the changes; no browser is connected for visual verification.

## Inline billing and caller insights — 29 September 2026

Cards now show Sources, Latest request (or Your latest), and Billing. Expanding a
card shows a connection table with separate Access & requests and Billing columns.
The inline panels list permitted agent keys and overrides, the latest three exact
requests in 30 days, and the billing account with applicable rates. Billing's For
selector compares the viewer's default with a managed agent key. Configuration
and permitted change history retain their existing access boundaries.

The implementation includes `GET /api/v1/service-insights` and exact request
attribution in the HTTP proxy, both LLM routes, and MCP. The endpoint is allowed
through the local preview's read-only gateway. When an older server returns
404, 405 or 501, the frontend reads existing agent-key inventories, credential
binding metadata and catalog prices. The comparison table shows key names and
configured scope, plus expected payer and configured rates. Its inline billing
flow separates credential supply, expected payer and NyxID charges. Pending or
failed price synchronization stays visible. The compatibility view does not
resolve per-agent billing or claim recorded use; exact caller history and the
managed-key billing selector require the new endpoint. Authorization and network
failures never trigger this fallback. Partial key inventories and unknown
credential overrides are labelled explicitly.
Older request history remains partial after deployment. Rates are current billing
previews; settled transaction history is not added by this revision.

Open `http://127.0.0.1:4317/keys?view=routing` to review the running frontend, or
`http://127.0.0.1:4317/__routing-preview/login` for a fresh preview login. Browser
automation was unavailable in this session, so signed-in visual review remains
outstanding.

Validation: 103 focused frontend tests and 30 backend tests passed, including
real MongoDB privacy, payer/override, exact request capture, metering, and rollup
checks. TypeScript, the production frontend build, Rust formatting, targeted ESLint,
and diff whitespace checks passed. Full ESLint has zero errors and 27 existing
warnings. The committed CLI wizard source-closure hash remains current.

The compact sticky filter now preserves its expanded height in normal page flow.
This prevents shrinking scroll height from clamping the scroll position back
across the sticky threshold. Its visible surface, cover, service-header offset and
card reveal still use the actual compact height; the reserved flow space is
transparent and does not intercept clicks. A regression test models repeated
resize/scroll frames near the bottom of a short filtered list.
Configured agent-key names also appear on collapsed cards when exact request
history is unavailable. Validation for this revision: 124 frontend tests,
TypeScript, production build, and changed-file ESLint passed. The running preview
serves the updated modules; signed-in visual verification remains outstanding.

## Billing and last-used layer — 30 September 2026

Billing, Last used and Agent keys now keep separate positions on collapsed cards
and stay visible in the expanded header. Each table row includes the last-use
summary too. Billing shows payer, rates and separate provider charges; last use
shows recorded layer, exact connection slug, caller/application and time. Clicking
an expanded header summary opens its corresponding inline row panel. The last-use
time has reserved space so a long caller name cannot hide it. All collapsed card
summaries share a 320px minimum height.

The draft backend now projects the layer recorded by each exact request. A change
to today's credential binding cannot rewrite the historical source. Requests
denied before dispatch remain in history but do not count as use. The latest
dispatched request is queried separately from the three recent events, so repeated
denials do not hide it. Production must
deploy this response before its recorded layer can populate locally. The existing
credential timestamp is deliberately not substituted for exact connection use.

Fixed the compatibility inventory parser: absent `expires_at` and `bindings_count`
mean no expiry and zero overrides respectively, per the deployed response contract.
An incomplete inventory is labelled as incomplete rather than showing zero keys.

Validation: 131 frontend tests, TypeScript, production build and changed-file
ESLint passed. The running preview serves the updated summary and schema modules.
Signed-in visual verification remains outstanding because no browser is connected.

## OAuth billing provenance — 5 October 2026

The preview at `http://127.0.0.1:4317/keys?view=routing` still targets production.
The draft backend now records the app source at successful authorization/refresh
and exposes verified `oauth_app_source` through `/keys` and service insights.
Older copied tokens are matched against their original token data without
decryption. Neither a connection ID nor the presence/absence of a retained OAuth
client ID establishes billing source.

Production has not received these changes. Its missing source fields can leave
all four X OAuth rows Unverified after the frontend guesses are removed. The
expected `3 NyxID · 1 BYOK` requires backend evidence; local tests proving that
rendering are not production verification. Rates, charge lanes and free-credit
funding are unaffected by the provenance metadata change.
