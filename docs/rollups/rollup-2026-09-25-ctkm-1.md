# Rollup: 2026-09-25 ctkm-1

This rollup starts from `main` commit
`1b031c77062880e572ec375041a4c029241a86f1` and presents the configurable
billing analytics integration for review before it lands in `main`.

## Summary

- Add the production admin Usage page at `/admin/usage`.
- Replace the preview-only analytics direction with a persisted workspace backed
  by real NyxID usage data.
- Provide Dashboard and List views with Operations, Overview, and Explorer
  templates. Operations is the recommended default.
- Allow administrators to configure filters, measures, breakdowns, chart
  types, Top 5/Top 10 or aggregate views, time intervals, panel order, panel
  width, panel height, and named saved views.
- Preserve the user-facing `/billing` Billing & Usage page and its Billing and
  Usage tabs from the rollup base.

## Included changes and provenance

The rollup has one direct source PR. Its source branch was validated before it
was squash merged into this branch. The source PR remains the authoritative
implementation discussion and test record.

| Source PR | Reviewed source head | Landed squash | Intended behavior / scope |
| --- | --- | --- | --- |
| [#1662](https://github.com/ChronoAIProject/NyxID/pull/1662) | `99a063906bedd56874e1ff89ee3b1cbaccb7031f` | `2018b7a9b10966b710a126c3bb3efc7e13bc5cdd` | Add the configurable admin Usage analytics workspace, persisted workspace state, saved views, three layout templates, Recharts visualizations, draggable/resizable panels, real usage aggregation, and the supporting route, API, tests, and documentation. |

The starting `main` commit already contains the separately merged hosted
service/channel work from [#1664](https://github.com/ChronoAIProject/NyxID/pull/1664)
and the tabbed user Billing & Usage page from
[#1663](https://github.com/ChronoAIProject/NyxID/pull/1663). Those PRs are
inherited base history; they are not repeated in this rollup's diff. The rollup
record itself was introduced in `0286d437` and expanded with this provenance
table in the follow-up documentation commit.

## Problem and resulting behavior

Administrators previously had no durable, configurable analytics workspace for
exploring service usage. The new page loads the existing NyxID usage rollups and
lets an administrator choose how to group and visualize them without inventing
metrics or replacing the existing Billing & Usage experience.

The workspace supports:

- Dashboard/List tabs with an Operations default and Overview/Explorer templates.
- Real service, acting-user, billing-account, credential-class, metric, and
  interval dimensions exposed by the backend reporting contract.
- Requests, billing events, exact/legacy/uncosted event counts, token classes,
  gross cost, wallet/grant/allowance funding, and billed-unit measures.
- Line, bar, pie, and combination visualizations through the existing Recharts
  dependency.
- Top 5, Top 10, and aggregate views, with panel-level measure and breakdown
  settings.
- Dragging, resizing, panel visibility, chart height, ordering, filters, and
  saved views persisted to the user's workspace.
- Clear treatment of UTC bucket boundaries, partial windows, unknown-cost
  events, and unsupported metrics rather than fabricated data.

## Implementation and safety review

The backend analytics aggregation and workspace persistence live in the admin
usage and usage workspace services. The frontend uses the existing NyxID
components, Recharts, TanStack Router, and query hooks. The implementation
keeps `/billing` and `/admin/usage` as separate routes and retains the current
user Billing & Usage behavior while adding the admin analytics surface.

The source PR's review verified that analytics read existing usage fields,
workspace writes are scoped to the acting administrator, stale revisions are
handled, and demo/sample data is limited to development and browser-test
fixtures. No secrets, billing credentials, or fabricated production metrics are
introduced.

## Validation evidence

The source PR was checked against the current `main` base before this rollup was
created. The final source-PR workflow run was
[CI run 36115024967](https://github.com/ChronoAIProject/NyxID/actions/runs/36115024967),
which passed the frontend, backend, CLI, Rust feature, coverage, wizard
freshness, CodeQL, and release-integrity checks.

Targeted local verification also passed:

- 40 frontend tests covering analytics workspace persistence, analytics data
  helpers, admin usage routing/page behavior, and Billing & Usage.
- Rust formatting with `cargo fmt --all -- --check`.
- CLI wizard bundle freshness with
  `cargo test -p nyxid-cli --test wizard_bundle_freshness --quiet`.
- `git diff --check` and a repository-wide conflict-marker scan.

## Rollup acceptance

- [x] The rollup branch starts from current `main` and contains the source PR's
  squash integration.
- [x] The source PR is merged into this rollup and its implementation record is
  listed above.
- [x] Both `/billing` and `/admin/usage` remain registered routes.
- [x] The rollup has no unresolved merge entries or conflict markers.
- [x] Source CI, coverage, CodeQL, release integrity, and local targeted checks
  pass.
- [ ] Complete the required review and merge this rollup into `main`.
