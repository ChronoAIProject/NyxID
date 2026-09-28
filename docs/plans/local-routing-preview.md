# Local routing preview with production metadata

Refreshed 28 September 2026 from main `bef3511b` (frontend v0.30.2), including
the service icon registry and per-connection icon overrides from #1681.
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
6. Pools keep the existing separate tab. Priority ordering there remains a local
   preference preview; no production strategy change is implied by this UI edit.

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

The current backend does not expose per-connection live readiness. `GET /keys`
sets `connected: true` during response construction; `status: active`, a recent
`last_used_at`, online node presence, and shared OAuth application credentials
are not evidence of a working upstream credential. Even an expired OAuth token
may be refreshable. The preview never upgrades these metadata facts to Ready.

Production pools currently use round-robin or weighted selection. Their resolver
filters enabled members and active services, but does not verify credential or
provider health at member selection. The proposed Priority strategy and the
shared readiness resolver described in `ai-service-connection-user-flow.md` are
backend follow-up work. The local preference order is not an effective routing
order until those checks exist.

No live provider probes or execution requests are made by this preview. The
current user session may also differ from an agent key's access. Final selection
must check the execution caller, exact bindings, policy, approvals and funding,
then report the actual selected source. It must not replay an already-dispatched
request through a second identity.

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
through the local preview's read-only gateway. Its production backend must deploy
this endpoint before these fields populate: an older server shows Not reported,
with an explanatory inline message. It never fabricates payer or caller data.
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
