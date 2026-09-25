# Rollup: 2026-09-25 ctkm-1

This rollup starts from `main` commit
`1b031c77062880e572ec375041a4c029241a86f1` and combines the configurable
billing analytics workspace, device login compatibility and scoped approval,
simpler service-account catalog grants, and CI improvements for review before
they land in `main`.

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
- Let a platform administrator grant catalog skill access through the normal
  service-account scope form, including both key metadata GET endpoints.
- Restore eight-character device-code compatibility and let users choose
  account access or a scoped Agent Key during web approval.

## Included changes and provenance

The source PRs below provide the implementation discussions and test records.
Each source branch is validated before its squash merge into this branch.

| Source PR | Reviewed source head | Landed squash | Intended behavior / scope |
| --- | --- | --- | --- |
| [#1662](https://github.com/ChronoAIProject/NyxID/pull/1662) | `99a063906bedd56874e1ff89ee3b1cbaccb7031f` | `2018b7a9b10966b710a126c3bb3efc7e13bc5cdd` | Add the configurable admin Usage analytics workspace, persisted workspace state, saved views, three layout templates, Recharts visualizations, draggable/resizable panels, real usage aggregation, and the supporting route, API, tests, and documentation. |
| [#1667](https://github.com/ChronoAIProject/NyxID/pull/1667) | Final head recorded on the source PR | Squash commit linked from the source PR | Grant platform-wide catalog access through administrator-saved SA scopes; remove the separate activation form; preserve token and live authority checks, private connection isolation, and Ornn proxy permissions. |

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

The billing analytics source PR was checked against the current `main` base before this rollup was
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

## Service-account catalog access

Aevatar needs catalog metadata and skill maintenance without a separate catalog
role assignment, activation action, or one grant per catalog service. In
[#1667](https://github.com/ChronoAIProject/NyxID/pull/1667), a full platform
administrator saving `catalog:skills:read` through ordinary SA create/edit
grants both `GET /api/v1/keys` and `GET /api/v1/keys/{catalog_uuid}` across
current and future catalog services. `catalog:skills:write` independently grants
skill recommendation updates. The read endpoints return catalog metadata with
catalog UUIDs and `resource_type: "catalog_service"`; they expose no private
connections or credentials.

The server records the administrator's explicit scope grant and protects the
account. Existing General/Curation scope strings do not silently become a
platform grant. Legacy role-based editors keep their existing authority until
an explicit administrator scope save converts them. Issued-token and live SA
scopes, immediate revocation, transaction write fences, and separate Ornn
proxy roles, credentials, target restrictions, and operations remain enforced.

The ordinary form replaces the separate Apply catalog access flow and sends an
access-state precondition to prevent stale edits from restoring revoked access.
The embedded CLI wizard is rebuilt from the combined rollup sources.

The focused MongoDB regression suite covers both GETs with only the read scope
and no catalog role, private UUID denial, independent read/write scopes,
revocation, non-admin grant prevention, legacy accounts, and Ornn token
continuity. Astra and Fable reviewed the implementation and its CI integration.
The source PR's checks gate its integration into this
rollup, including coverage of the combined sources.

The rollup's CI from [#1669](https://github.com/ChronoAIProject/NyxID/pull/1669)
runs backend head coverage and its base comparison on separate runners. It
preserves the native 73% head threshold, full test suites, coverage reports,
and an exact-commit base comparison with a cache fallback. This isolates the
comparison build that repeatedly lost its runner during the earlier SA checks.

Deploy every backend replica before deploying the frontend and before using
the new scope grant flow. Then save Aevatar's catalog scopes in the normal
admin edit form and verify both GETs using a catalog UUID from the list. The
existing SA, client secret, and tokens already carrying the matching scopes
remain usable.

## Rollup acceptance

- [x] The rollup branch starts from current `main` and contains the source PRs'
  squash integrations.
- [x] The source PRs are merged into this rollup and their implementation records
  are listed above.
- [x] Both `/billing` and `/admin/usage` remain registered routes.
- [x] The rollup has no unresolved merge entries or conflict markers.
- [x] Source CI, coverage, CodeQL, release integrity, and local targeted checks
  pass.
- [ ] Complete the required review and merge this rollup into `main`.

## Device login contribution

[PR #1666](https://github.com/ChronoAIProject/NyxID/pull/1666) adds eight-character
device-code compatibility and web approval-time selection of account access or
a scoped Agent Key. It retains the production implementation and removes the
standalone HTML prototype and its supporting artifacts.

See [the device login contribution record](rollup-2026-09-25-ctkm-1-device-login.md)
for behavior, verification and rollout constraints. Its source PR records the
reviewed revision, final checks and landed squash.

## CI latency contribution

[PR #1669](https://github.com/ChronoAIProject/NyxID/pull/1669) was reviewed at
`3587afd60dc03a232a37f5c1fbb1c4beb8dbe1ea` and squash merged as
`02c86d0c72a91f60b57dc44c5301ba3df887a605`. It addresses the serial
backend validation job that had taken about 34 minutes on recent PRs.

The full backend test suite, standalone billing smoke, and production backend
build plus embedded-input guard now run on separate runners and all remain in
the `CI Pipeline` gate. The PR-head coverage threshold stays at 73%, while an
informational base comparison runs in parallel and reuses a report only for an
exact base commit and CI workflow hash. Rollup pushes measure backend coverage
even for documentation-only changes so each branch tip can seed that cache.
The existing four CodeQL languages scan in parallel. Publish-image digests
build alongside CI, but release tags still wait for the gate and builds.

On the combined billing and device-login tree, the source PR's
[CI run](https://github.com/ChronoAIProject/NyxID/actions/runs/36125774429)
passed in 19m43s, including a 19m27s correctness gate. Its
[CodeQL run](https://github.com/ChronoAIProject/NyxID/actions/runs/36125774417)
passed in 17m35s. Backend line coverage was 87.54% against the unchanged 73%
threshold; CLI was 71.50% and frontend was 70.41%. The coverage comment,
base report, production build guard, and conflict checks passed.

A follow-up stabilizes the expired device-code approval test by disabling the
MongoDB TTL index only in that test's isolated database before asserting the
persisted expired status. Production expiry and test assertions are unchanged.
