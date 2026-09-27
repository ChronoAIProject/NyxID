# Configurable billing analytics

The existing `/admin/usage` page contains Dashboard and List tabs. Dashboard uses
Recharts 3.8, the visualization library already used by NyxID, with an Operations
grid and multiple private saved views. List reuses the existing service,
credential-class, and user ranking tables. Both tabs share service, acting-user,
billing-account and time-range filters. There is one Usage navigation entry;
`/admin/analytics` redirects to it.

## Design and library decision

Fable 5.1 was consulted on 2026-09-23 using a tool-free consultation. It recommended
Recharts for its declarative pie, bar, line, and composed charts, and advised against
introducing visx for these conventional, bounded reports. visx is suitable when
custom geometry or interaction justifies owning scales, axes, tooltips, legends,
and accessibility. Recharts fits this report's small bucket counts and existing
React integration without a second visualization dependency.

Three layout approaches remain available in the automated design fixtures. The live
Dashboard uses Operations, preserving panels from previously saved layouts:

- **Overview**: GA-inspired summary metrics, prominent trend, service mix, and ranking.
- **Operations**: Grafana-inspired compact grid with six starting panels, a connected
  summary strip and standard NyxID dashboard cards. Each panel has
  independent metrics, chart types, breakdowns, and Top 5/10 controls. Add, duplicate,
  remove, resize, and reorder panels. The grid adapts from one column on phones to
  two columns, then a maximum of three columns. Drag a header handle to reorder;
  Space, arrow keys, and Space provide the keyboard equivalent. Move up/down
  buttons remain available. The editor offers one-, two-, or three-column widths
  and Compact, Standard, or Tall chart heights. Order and sizes autosave with the
  workspace and are included in named views. Legacy `wide` panels remain full width.
- **Explorer**: PostHog-inspired query controls beside a large visualization and
  accessible data table. Turn the current exploration into an Operations panel.

The templates use the existing NyxID typography, colors, density, and light/dark
themes from DESIGN.md. They are starting configurations, not separate applications.
Existing saved workspaces keep their panels and filters. The live page uses the
Operations grid; the layout selector is confined to the design fixtures.

All three layouts share the chart treatment in `visualization.css` and the Recharts
renderer. Following the Genex and LabsAI references, the two leading trend curves
use vertical color-to-transparent area gradients beneath crisp strokes; remaining
series use finer lines to avoid overlapping fills obscuring the trends. The palette
stays in violet, blue, lavender, and slate tones, with a neutral Other series.
Ranking bars use a restrained horizontal gradient. Combined charts use muted tonal
stacked bars and a lavender request line. Donuts use slim tonal segments. Grid lines
are quiet and legends use small color dots. Hover markers and tooltips expose exact
values; unknown costs still leave gaps. Every gradient is scoped to its chart
instance so duplicated panels render independently.

Panels retain NyxID's shared card background, text, border, radius, shadow, and
heading styles in both themes. Chart colors cannot recolor cards, headers, or
controls. The same rendering applies to existing views, new panels, and all three
layout fixtures. Chart styling does not change settlement or underlying values.

## Implementation plan and acceptance criteria

1. Extend the existing bounded rollup reporting engine to retain UTC time buckets
   for analytics queries, preserving raw-edge, pending-tail, and snapshot behavior.
   Distinguish actor filters from billing-owner filters. Rank complete populations
   on the server; never derive analytics from a page of the usage ranking.
2. Add an authenticated private workspace store with structural validation, bounded
   configurations, and revision-based conditional writes. Autosave the working
   draft; explicitly save named views so exploring does not overwrite a saved view.
3. Build one Recharts renderer and panel specification, then the three layouts.
   Each panel chooses its measure, breakdown, chart, and aggregate/Top 5/Top 10 view.
   Label credit units and UTC buckets. Unknown costs remain gaps or unavailable
   values; combined charts label each axis. Every chart has a data table.
4. Use authenticated usage and workspace APIs for all normal previews. Keep
   deterministic fixtures restricted to the browser-test server. Wire analytics
   into admin navigation and billing.
5. Review the implementation directly, test real Mongo aggregation and concurrent
   persistence, run frontend tests/build/lint, and exercise all samples at desktop
   and mobile sizes. Fix findings and rerun affected verification.

Shared dashboards and arbitrary query languages are outside
this design. Private views contain settings only; all usage reads retain their
server-side authorization. Financial settlement and wallet mutation are unchanged.

## Run the unified Usage page

Start a backend built from this checkout with a reachable MongoDB 8 replica set
and the configuration described in [ENV.md](ENV.md). An older backend can serve
`/health` successfully while lacking the analytics endpoints or having lost its
database connection. Verify `/api/v1/public/config` responds and an unauthenticated
`/api/v1/users/me` returns 401 promptly before opening the frontend.

From `frontend/`, explicitly point Vite at that backend. For example, with the
backend on port 4630 and its `FRONTEND_URL` set to `http://127.0.0.1:4628`:

```sh
BACKEND_URL=http://127.0.0.1:4630 FRONTEND_URL=http://127.0.0.1:4628 \
  npm run dev -- --host 127.0.0.1 --port 4628 --strictPort
```

Open `http://127.0.0.1:4628/admin/usage` and sign in as an admin or operator
on that backend. Dashboard and List use the same real usage data and selected filters. Existing
saved settings are restored.
An empty local database shows empty usage states. Verify a layout or time-range
change survives reloading the page to confirm database-backed persistence.

The isolated local review instance at `http://127.0.0.1:4628/admin/usage` can also
be populated with explicitly requested seed records. Its ignored
`.context/local-analytics/seed-usage.cjs` script is restricted to the local preview
database, requires billing to be disabled, uses existing catalog services, and
inserts finalized reporting history. Reruns preserve existing and folded rows
and add missing hours through the current hour. The local launcher performs this
refresh on startup so short relative windows stay populated after a restart.
The launcher sets `VITE_USAGE_SEEDED=true` to show a development-only “Local seed
data” note. `seed-report.json` records the generated range and expected totals.
These records exercise the real reporting APIs and saved workspace, including
Dashboard/List filter parity; they are not a frontend response fallback.

Normal development and production never automatically substitute generated usage. API failures
show errors; empty windows show empty states. All values come from the current
usage contract: costs, requests, tokens, quantities, and active identities. The
references supply visual direction only; their health scores, portfolio values,
percentage changes, and other unrelated metrics are not copied into billing.

The existing Card, Button, Select, Input, Popover, and other shared UI primitives
supply the platform styling. The time-range label and selector sit on a single line
within the filter toolbar; controls wrap as groups on narrow screens. Filter pickers
hold draft selections until Apply; Cancel and dismiss leave applied filters intact.
Applied selections are displayed below the controls using the audit log's shared
`DataTableFilterChips`. Each chip can reopen its picker or clear its filter group.
Names resolve independently of the search results, including after reload. The
custom time range starts with the currently selected relative window.

Synthetic data is restricted to `import.meta.env.DEV && import.meta.env.MODE ===
"test"`. Playwright launches Vite with `--mode test` and exercises its fixtures at
`/admin/usage?mock=1&sample=overview|operations|explorer`. The test fixture banner
labels those values as synthetic. The normal preview server does not import the
fixture renderer. The production build assertion rejects the fixture-data markers.

## Workspace behavior

Filters and time ranges belong to the persisted workspace. The URL retains the
active tab and List sorting/pagination; obsolete time/user/service parameters are
removed so the address does not disagree with the saved filters.

The active draft autosaves after 600 ms of inactivity, including filters, layout,
panel order, widths, heights, intervals, titles, measures, metric units, chart types, breakdowns, and
Top N selections. Saving a named view makes a separate snapshot. Loading and
exploring that view changes the draft; **Update saved view** explicitly replaces
the snapshot. **Save as new view** creates another one.

The server stores one `usage_workspaces` document per authenticated admin,
using the admin UUID as `_id`, a BSON `updated_at`, and a revision. A write must
match the previous revision. Stale writes return HTTP 409; the workspace offers
retry, reload saved, and keep my changes after fetching the current revision.
Autosaves are serialized so a slow request cannot discard subsequent edits.

Unsaved edits have a per-tab session backup and a user-scoped browser recovery
copy. A completed save only clears recovery data it owns. When a recovered draft
has a stale revision or an incomplete time range, the dashboard renders the
server's saved view and preserves the draft without autosaving. **Use saved view**
dismisses that recovery copy; **Restore recovered draft** loads it using the
latest server revision. Editing stays disabled until either choice, and an
incomplete restored range must be completed before queries or autosaves resume.
Valid drafts with a matching revision recover automatically. Browser storage
failures are visible. Admins can save settings; operators can explore
usage but cannot write workspace settings, following the existing admin-write
policy. Regular users cannot access the admin analytics API.

## Insight controls and valid data

The September 25 refinement follows PostHog's separation of measure, breakdown,
time interval, visualization, and filters ([Trends documentation](https://posthog.com/docs/product-analytics/trends/overview)).
It uses the existing billing report contract rather than adding product-event,
session, funnel, or retention data. Fable 5.1 reviewed that contract in a tool-free
consultation; the implementation lead checked its recommendations against the
actual reducers and reviewed the implementation directly.

Each trend panel offers Auto, Hourly, Daily, Weekly, and Monthly intervals. Auto
retains the existing behavior: hourly through 48 hours, daily beyond that. Weeks
start Monday at 00:00 UTC and months start on the first. A custom window remains
bounded to 31 days, allowing at most 745 hourly buckets and 11 series; no interval
is silently changed. Calendar buckets intersect the selected, end-exclusive window.
Partial axis labels carry an asterisk; tooltips and data tables state the actual
included UTC range.
Totals, filters, and whole-window Top N membership are independent of the interval.

| Available measure | Meaning |
| --- | --- |
| Requests | One count from the primary platform metering record; component and resale rows do not count again. |
| Billing events | Metering records; one request can generate several. |
| Total, input, output, cache-read, cache-write tokens | Provider-reported token fields on the primary record. Total is input + output; caches may overlap and are not added again. |
| Billed units | The selected metered quantity: requests, tokens, input/output/cache tokens, bytes, or images. Billed input can differ from provider-reported input. |
| Gross, wallet, grant, allowance cost | Existing microcredit amounts, displayed as credits. These are usage costs, not fiat revenue or wallet balances. |
| Exact-cost, legacy, uncosted events | Cost provenance counts. Legacy and uncosted can overlap; they are not three mutually exclusive shares. |
| Active users and services | Existing distinct whole-window totals in the summary. Do not sum distinct counts across groups or periods. |

Panels can break down by service, acting user, billing account, or credential class.
Credential classes use the same names as the platform's List view. Service/user/
account legends support drilldown using the shared filters; credential classes
are reporting groups without an identity-filter action.

Safe future derived measures include cost per request and tokens per request,
calculated from matching populations with a null result for zero denominators.
Cost coverage must account for overlap between legacy and uncosted counts.
Cache-hit rates require provider-specific accounting. Model metadata exists in
some meter and rollup records, but a model breakdown needs an explicit reporting
contract and missing-value rules; it does not require inventing model data.
Agent/API-key and route breakdowns require a review of retained rollup dimensions.
Latency percentiles, HTTP error rates, funnels, retention, and fiat revenue need
additional reporting inputs and are not offered as fabricated metrics.

The drag implementation uses dnd-kit with pointer and keyboard sensors. It stores
ordered panels and spans, preserves DOM order, and never uses dense grid packing.
Cards stay in place while dragging, with an outline on the drop target, so mixed
widths do not distort the grid preview. Dropping commits the new order.
Widths collapse to fit smaller screens. Recharts continues to own visualization;
tooltip item colors are explicitly themed because its default black item color
does not inherit the tooltip container color. Axis text uses theme foreground,
and the pale light-theme series have been strengthened. Browser tests measure
tooltip and axis text contrast in both themes and require at least 4.5:1.

## Template contract

`newView` and `newPanel` in `frontend/src/lib/usage-analytics.ts` define the three
starting templates. `frontend/src/schemas/usage-analytics.ts` validates the
shared contract. Adding a template means supplying another configuration for the
existing workspace and renderer. For example, this is a valid new workspace
request for a combined Top 5 service-cost chart:

```json
{
  "revision": 0,
  "config": {
    "version": 1,
    "draft": {
      "id": "ec35002b-a14f-4356-a789-12063eab943d",
      "name": "Weekly service spend",
      "layout": "operations",
      "filters": {
        "period": "7d",
        "from": null,
        "to": null,
        "services": [],
        "actors": [],
        "owners": []
      },
      "panels": [
        {
          "id": "7b8e9c76-e564-4e50-b953-d4fc97fb3b84",
          "title": "Service spend and requests",
          "chart": "combo",
          "measure": "cost",
          "metric": "tokens",
          "breakdown": "service",
          "top": 5,
          "wide": true
        }
      ]
    },
    "saved_views": []
  }
}
```

Use the revision returned by GET or PUT for subsequent updates. `chart` accepts
`line`, `bar`, `pie` (rendered as a donut), or `combo`. Measures are `cost`,
`requests`, `total_tokens`, `quantity`, `wallet_cost`, `grant_cost`, and
`allowance_cost`. `quantity` selects an existing billing metric through `metric`.
Breakdowns are `service`, `user` (acting person), or `owner` (billing account).
`top` is 0 for aggregate, 5, or 10. Combined charts show the selected measure as
stacked bars and requests on a labeled right axis; requests alone use a single
axis. Standard-width panels follow their layout's grid; full-width panels span it.

## API and reporting semantics

| Endpoint | Behavior |
| --- | --- |
| `GET /api/v1/admin/usage` | Existing List report, including the same optional `services`, `actors`, and `owners` multi-selection filters. |
| `GET /api/v1/admin/usage/analytics` | Query a bounded usage snapshot; returns totals, UTC buckets, globally ranked slices and series, units, and freshness. |
| `GET /api/v1/admin/usage/workspace` | Read the authenticated user's private workspace and revision. |
| `PUT /api/v1/admin/usage/workspace` | Validate and conditionally save the authenticated admin's workspace. |

Analytics accepts `period=24h|7d|30d` or RFC 3339 `from` and `to`, plus optional
comma-separated `services`, `actors`, and `owners`. `measure`, `metric`,
`breakdown`, and `top` match the panel contract. `actors` and `owners` are separate
UUID filters combined with AND. Owners can be personal or organization accounts.
Service filters accept catalog IDs or slugs. The service picker lists the visible
active catalog; chart legends also support filtering historical service identities.

Windows are positive, at most 31 days, and end-exclusive. Auto uses hourly buckets
for windows up to 48 hours and daily otherwise; panels can explicitly choose
hourly, daily, weekly, or monthly buckets. Relative windows follow the existing
usage report's hour-aligned start. Partial first/last buckets contain only usage
within the requested window. Hourly charts use hourly summaries even when daily
rollups are available. The engine combines folded summaries, raw boundary rows,
and pending rows under the existing snapshot and timeout rules.

Ranking uses the complete selected population. Top 5/10 plus Other keeps the same
groups throughout a time series. Empty buckets are zero; cost buckets or groups
containing unpriced usage are unavailable, and line charts leave gaps. The UI
reports unknown-cost coverage and historical cached-rate estimates. Credit values
are converted from microcredits only for display. Duplicate display names and real
entities named Other are disambiguated in both slices and series.

There is no fixed panel-count limit. The server limits each filter to 20 values,
each workspace to 20 saved views and 1 MiB of configuration, and titles to 100 UTF-8
bytes. The frontend shows validation feedback if the overall configuration exceeds
that size. Charts first mount as their panels approach the viewport; offscreen
panels suspend query refreshes, and identical queries share cached results.
Operations also uses CSS content visibility to avoid rendering offscreen contents.
Queries use the existing 20-second database deadline and an outer timeout. Errors
are shown per panel with a retry action. Every chart includes a table with its
plotted values and units.

## Verification

The implementation is exercised with real MongoDB 8 aggregation tests for Top N,
Other, actor/owner separation, selected services, unknown costs, grouped time
series, raw edges, and hourly/daily rollup parity. Workspace tests cover private
ownership, concurrent revision writes, BSON timestamps, validation, and
admin/operator/user authorization. Large-board tests accept 1,000-panel
configurations, persist 40 panels through MongoDB, and reject configurations
exceeding the overall size bound. The existing admin usage regression suite is
also included; its two large performance benchmarks remain opt-in.

Frontend tests cover canonical query keys, units and custom ranges, Unicode panel
duplication, actual sample filtering, autosave races, per-user/per-tab recovery,
conflicts, and operator behavior, plus existing billing and usage-page regressions.
The Playwright flow exercises all templates, named save/reload, filters, Top N,
chart changes, panel duplication, and the Explorer-to-Operations action. Three
additional browser cases check mobile rendering and horizontal overflow. Another
adds and restores 18 panels and checks that charts load as they enter view. A live-path
case verifies rendering from intercepted API responses, inline time-range controls,
and an API error without a synthetic-data fallback. Desktop
screenshots were reviewed in both themes. Production build, mock-footprint check,
targeted lint, Rust formatting, and whitespace checks are part of the final gate.

The September 25 verification passed 17 analytics/usage tests, 5 workspace tests,
33 targeted frontend tests, and all 12 Playwright cases. Calendar tests cover
leap years, a 31-day window touching three months, end-exclusive boundaries,
token classes, and matching raw/folded totals. Browser tests cover pointer and
keyboard reordering, mixed panel widths, cancellation, saved sizes and intervals,
and measured text contrast in both themes. Production builds, the sample-footprint
assertion, targeted frontend lint, Rust formatting, and `git diff --check` passed.
The two existing opt-in large aggregation benchmarks were not run.

The seeded local preview also passed real-browser sign-in, all six populated
panels, filtered Dashboard/List parity, light/dark and mobile rendering, and a
named-view/filter/panel save-and-reload check. Its generated requests span eight
catalog services, 12 demo people, and three demo organizations. The latest live
check used an isolated review account and verified database persistence of drag
order, width, height, and interval; all four intervals returned the same input-token
total. The browser reported no JavaScript errors or failed authenticated API calls.
An idempotence check inserted zero seed rows on rerun.

Relevant commands from `frontend/`:

```sh
npm test -- src/hooks/use-usage-workspace.test.tsx src/hooks/use-usage-analytics.test.tsx src/lib/usage-analytics.test.ts src/pages/billing.test.tsx src/pages/admin-usage.test.tsx src/pages/admin-usage.router.test.tsx
npx playwright test e2e/billing-analytics.spec.ts
npm run build
```

Backend tests require `NYXID_TEST_DATABASE_URL` pointing at a disposable MongoDB 8
replica set. From the repository root:

```sh
cargo test -p nyxid --bin nyxid-server services::admin_usage_service::tests --no-default-features
cargo test -p nyxid --bin nyxid-server usage_workspace --no-default-features
```
