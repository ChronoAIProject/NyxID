# CRM implementation completion

This checklist tracks the approved 4 November 2026
[CRM and dynamic artifacts proposal](https://gist.github.com/ctkm-aelf/f0928ec823b8cb408df38bb24c4bcd34)
and the parent's personal review. The baseline evidence below predates the
5 November CMS/artifact completion pass; G2 evidence is recorded separately
below and in the external G2 implementer report.
The local proposal copy is
`/tmp/nyxid-crm-implementation-approved-plan.md`.
The work stays on the existing `feat/crm-records` checkout, original HEAD
`5f45ccd69`. Phase 0 of the CMS/artifact pass preserved the inherited work in
nine ordered commits before implementation changes. No additional agents,
worktrees, pushes, deployments, or production flag changes were created.

The six files that were dirty before the inherited baseline were preserved:
`errors/mod.rs`, `errors/voice_start.rs`, `models/crm_saved_view.rs`,
`services/crm/query.rs`, `services/crm/records.rs`, `services/crm/views.rs` (under
`backend/src`). Their original contents are captured in
`/tmp/nyxid-crm-review-baseline/working-tree.patch`. They include prior work;
this document does not attribute their whole diff to the completion pass.

## Inherited baseline status

The following results describe the earlier baseline, not the current checkout.
Implementation and review fixes were present. The corrected serial backend build
passed in 44m 25s, and real HTTP fixture regeneration passed (1/1, 0 ignored).
The final CRM run passed **55/55, 0 ignored** including no-update fixture parity.
The independent backend rerun also passed **55/55**, and the full frontend
selection passed **30 files / 161 tests**. Final production build, lint (60 changed
files), rustfmt and whitespace checks passed. **The approved baseline is
implemented and all personal review findings are resolved, with no known open implementation or
verification findings.** Logs live
under `/tmp/nyxid-crm-implementation-logs/`. Parent review evidence lives in
`/tmp/nyxid-crm-review-findings.md`.

## Baseline audit and verification gates

| Capability | Initial audit | Implemented completion and executable checks |
| --- | --- | --- |
| Typed schema | Four seeded objects, field/value validator and owner fence existed. | Shared typed mapping, blank required-text rejection, immutable keys/types/targets, archived fields/options, 100-field/100-option bounds, required pipeline stage and 50-stage cap. `fields` tests, HTTP fixtures, schema/form/field registry tests. |
| Records and links | Transactional revisions/receipts/links existed. | Protected human values; server run-order fences with skipped-stale results; ordinary stage PATCH history; sparse updates preserve omitted values; owner/schema/target transaction invariant preserved. New Mongo `regression_tests` cover source order, protection, stage history and schema fencing. |
| Human authorization | First-party routes and owner read/work/schema checks existed. | Explicit owner context throughout tables, forms and artifacts; live membership/person eligibility for labels; batched assignments; current role enforced on writes. New people aggregate parity, org viewer and revoked-member tests. |
| Queries and reports | Bounded table keysets/aggregates existed; correctness review found edge cases. | Null vs zero including Other/total; overflow refusal; exact safe-integer cents; declared percent units; date/datetime day/week/month buckets with IANA timezones and UTC calendar dates; active records only on boards. New Mongo aggregate/date tests and rendered chart tests. |
| Default CRM UI | Tables, detail/forms, filters and board existed. | Company/contact/opportunity/task workflow; dirty delta patches and stable revision baselines; archived records/values; member and relationship pickers; pagination for lanes/pickers/history; visible filter/count errors; uncertain move/note/archive retry identities. Component/domain-hook regressions and desktop/mobile mocked-API browser workflows. |
| Schema editing | No usable metadata editor at audit. | Owner/schema-authorized editor exposes all field kinds, percent/plain and one/many; uses useAppForm; dirty conflict/reload, successive saves, bounded key/label/options, stable uncertain retry keys. Schema editor unit and browser tests. |
| Saved workspaces | Closed pinned json-render envelope and validator existed. | Paginated lists; early 64 KiB/12-query/60-node/60-edge/8 KiB-prop work bounds; shared metadata snapshot; transaction-fenced save validation; projection compatibility; record open/edit; stable Router owner/object/view/record/surface URLs; draft and revocation handling. Validator, renderer, panel/navigation and editor tests. |
| AI generation | No generation route/service at audit. | Platform-only pinned CRM route, else platform utility route; validated admin authoring; acting-person billing; bounded inference and one repair; explicit save; durable separate generation claims/results, concurrent at-most-once dispatch, successful replay before new-attempt preflight, retryable predispatched failures and no blind uncertain rebilling. Mongo/wiremock generation, route/admin and billing tests. |
| Native specialists | No CRM inventory/grants/dispatch at audit. | Operation-specific closed schemas; explicit owner/object Read/Write/Schema grants and human UI; logical mutation_key independent of RPC correlation; scoped relation/view access; guest/shared refusal; live confirmation/key/session/turn/deadline/member/grant fences; metadata audits; API DTOs and actionable current/errors; existing schedules supply server run order. New native/direct/universal MCP and transaction-fence tests. |
| Import/export | No routes/UI at audit. | Bounded 100-row/100-column/512,000-byte CSV/JSON preview, mapping, exact matching/dedup and atomic commit; snapshot binds schema/input/exact matched ID+revision+action; explicit clear vs omitted preservation; export envelope imports directly; displayed query drives export. Mongo drift/rollback/replay tests and parser/UI browser coverage. |
| Freshness/lifecycle | Erasure inventory and polling existed. | Identifier-only CRM change projection, personal direct delivery, org membership batches and 250 ms coalescing; live delivery authorization; reconnect invalidation and polling fallback; actor-scoped caches; 403/404 eviction, recoverable-error draft retention; TTL/index/erasure handling for generation jobs. Live/recovery/generation tests. |

Optional broad app building, email sync, arbitrary joins/formulas, advanced
collaboration, predictive analytics and historical pipeline snapshots remain
outside the approved baseline. Dated reports describe current records; they do
not manufacture history from current values.

## Personal review resolution map

| Findings | Resolution / regression evidence |
| --- | --- |
| 1,13 draft keys/revisions and async races | Identity-based record/view editors, stable dirty baseline, explicit conflicts, pending controls and stale-validation guard. Record sheet, view editor and recovery tests. |
| 2 history and notes | Bounded infinite history, older-page recovery, live first-page refresh; caught note failures retain draft/key. Activity hook and sheet/browser write tests. |
| 3 archived fields | Detail read-only archived-value section; stored values remain visible. |
| 4 ordinary stage changes | Shared patch path appends from/to stage activity for forms, imports and native writes. Mongo source/stage regression. |
| 5,17 owner/schema fencing | Schema/record/link/view/import validation and effects share owner-fenced transactions. Mongo schema-fence and import drift/atomicity regressions. |
| 6,7 monetary/percent charts and pie | Exact shared currency formatter; unit-aware axes/tooltips and accessible rendered values; count/sum-only nonnegative pie. Safe-boundary, percent and negative-pie tests. |
| Literal chart group identities | Type-prefixed real group identities distinguish null, strings and booleans; only the trailing point of a truncated result uses the synthetic remainder identity. Archived literal `__other__` keeps its marker. Adapter and rendered Bar/Pie collision regressions pass; public query values are unchanged. Date-only report keys cannot contain the sentinel strings. |
| 8 pagination | Pipeline lanes, record picker and history expose bounded navigation. Board component tests keep counts authoritative across pages. |
| 9 people usability/performance | Authorized member picker, unavailable placeholder, explicit owner context, one batch for a 200-row table. People table request-count test and Mongo group-label eligibility parity. |
| 10,18 artifacts/projections | Shared detail navigation with Router record search; explicit columns validated against select; board requests label/stage. Renderer/panel/board tests and browser Back/Forward. |
| 11,19 work bounds | Early structural rejection, per-page shared metadata, paginated views, bounded fields/options. |
| 12,14–16 typed editing/accessibility | Invalid nonempty numeric input errors; exact decimal currency parsing; required blank text rejected on server; optional boolean tri-state; useId label/error/trigger associations. Registry/form/MCP tests. |
| 20,21 archives/stages | Main archived-record toggle; required/unarchivable stage schema; 50 stage definitions; explicit count errors and uncertain move reconciliation. Board tests/browser workflow. |
| Generation route/claims/admin | Platform fail-closed routing without personal fallback; acting-person billing, admin save/clear; separate generation namespace; durable dispatch/result states and UI new-attempt identity. Generation/one-shot/admin tests and browser retries. |
| Native scoped grants/security | Exact typed operation evaluator runs twice around confirmation; reference grants, view Write authority and text-only policy; conflict redaction including MCP error current/errors; write-only admission with live key/session/turn expiry/member fences. Native/direct/universal tests. |
| Native deletion replay | Owner-scoped tombstone helper permits unchanged receipt replay after soft deletion. Exact committed results recover without re-consuming a single-use confirmation or re-running an effect; schema receipt responses retain API DTO conversion. Current grants/turn/creator-admin checks remain; new delete keys still fail in the delete service. Approval/replay/new-key/revocation regression. |
| Native discovery/DTO/provenance | Typed bounded tool shapes/option schemas; ObjectApi schema results and receipt replay; metadata-only activity names/audits; readable action cards including schema changes; human and specialist names/source RFC3339 times. |
| Shared-agent metadata | Shared runtime clears CRM grants; creation audit includes bounded grants; roster names object/capabilities. Shared projection and native roster/audit tests. |
| Import snapshot/sparse roundtrip | Exact ID/revision/action/schema expectations; no silent fresh-revision overwrite; absent JSON columns preserve values; explicit null clears; real task match field and union columns; importable export envelope. Mongo/parser tests and browser import. |
| Revocation/transient/owner switches | Actor-keyed caches, refusal-specific eviction, transport refresh warning with dirty drafts retained, owner/account identity changes reset private surfaces. Real domain-hook regressions and browser live/revocation/outage tests. |
| Option color parity | Empty color accepted consistently; client and native color strings bounded to 64. Schema regression. |
| Frontend regressions and formatting | Updated mocks retain real behavior assertions; generation payload retry state/mock isolation fixed; activity waits for Query notification; changed CRM modules/tests formatted with installed Prettier. Unrelated Router/assistant-schema formatting was restored. |

## Inherited baseline verification

Logs below are under `/tmp/nyxid-crm-implementation-logs/` unless an absolute
parent-review path is given. Counts are per suite, with overlapping filters
reported separately; they are not a sum of unique tests.

### Backend build and real MongoDB execution

MongoDB **8.0.34**, replica set **nyxidrs**, is a writable primary at
`mongodb://127.0.0.1:27019/?directConnection=true`. Port 27017 is standalone and
was not used. Health evidence: `mongo-replica-set.log`. No missing-service skip,
ignored test or zero-test selection counts as success.

The final build command, from the repository root, was:

```sh
CARGO_BUILD_JOBS=1 \
NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27019/?directConnection=true' \
cargo test -p nyxid --bin nyxid-server \
  --config 'profile.test.package.nyxid.debug=0' crm --no-run
```

**PASS in 44m25s, no warnings or errors** (`backend-runtime-fixes-build.log`).
This is the standard Cargo test build with only this package's debug symbols
disabled for host memory. Executable:
`target/debug/deps/nyxid_server-76e2b300eed10499`, SHA256
`c6fdda0f1483357774f72e3d00c0bc756765d97442f5576d25a305a6f617dce3`
(`backend-final-binary.sha256`). No Rust source changed during this build.

A prior Cargo invocation redundantly recompiled unchanged source through the
existing worktree-sensitive build scripts. Runtime checks therefore execute the
fresh Cargo-built test binary directly, rather than triggering another build.
This executes the actual test harness and Mongo transactions, not metadata-only
checking. Every worker runtime row below uses
`NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27019/?directConnection=true'`, this
executable, and `--test-threads=1`. `UPDATE_CRM_FIXTURES` is absent except in the
explicit regeneration row.

| Test filter / additional environment | Result | Log |
| --- | --- | --- |
| `handlers::crm_fixture_tests::crm_http_contract_fixtures_match_real_routes`, `UPDATE_CRM_FIXTURES=1` | **1 passed / 0 failed / 0 ignored**, 24.32s; actual HTTP route regeneration. | `backend-final-fixtures-update.log` |
| `crm` | **55 passed / 0 failed / 0 ignored**, 65.05s; includes no-update fixture parity, direct/universal MCP, generation, native authority, import, dates, aggregates and transactional fences. | `backend-final-crm-tests.log` |
| `services::assistant_oneshot_inference::tests` | 29 passed / 0 failed / 0 ignored. | `backend-oneshot-tests.log` |
| `services::utility_inference_service::tests` | 3 passed / 0 failed / 0 ignored. | `backend-utility-tests.log` |
| `handlers::admin_utility_inference::tests` | 1 passed / 0 failed / 0 ignored. | `backend-admin-inference-tests.log` |
| `services::assistant_live::tests` | 4 passed / 0 failed / 0 ignored. | `backend-live-tests.log` |
| `services::assistant_authority_tests` | 24 passed / 0 failed / 0 ignored. | `backend-authority-tests.log` |
| `services::assistant_access_mode_tests` | 17 passed / 0 failed / 0 ignored. | `backend-access-mode-tests.log` |
| `services::shared_agent_tests::shared_agent_crew_projection_keeps_skills_notes_and_rejects_other_surfaces` | 1 passed / 0 failed / 0 ignored. | `backend-shared-projection-tests.log` |
| `services::org_agent_tests` | 28 passed / 0 failed / 0 ignored. | `backend-org-agent-tests.log` |
| `handlers::assistant_team::tests` | 40 passed / 0 failed / 0 ignored, 226.05s. | `backend-team-handler-tests.log` |

Exact argv/results are in `backend-final-crm-results.json`,
`backend-impacted-results.json` and `backend-team-handler-results.json`. The
impacted/team suites executed the earlier successful Cargo build; their modules
were unchanged by the final CRM activity/test corrections. An earlier misspelled
team filter selected zero tests and was rejected with exit99; only the corrected
40-test run above counts.

The initial CRM run was 50 passed / 4 failed. Corrections preserve the intended
behavioral assertions: a newer admitted turn for value-validation coverage,
authorized label-projection expectations, `measure_count` in the overflow
fixture, and early graph-bound rejection. Logs remain in `backend-crm-tests.log`.
The final single-event activity regression covers stage-only, stage plus amount,
and ordinary patches. Stopped/obsolete build attempts remain in the log directory
and are not counted as verification.

The parent independently passed **41 impacted backend tests, 0 failed/ignored,
27.95s**, on the same replica set, covering one-shot/utility routing, admin,
shared projection, grants and native cards/authority:
`/tmp/nyxid-crm-review-backend-integration.log`. All **36 `errors::` tests** also
passed with 0 failed/ignored, including preserved voice-start behavior and
CRM recovery fields: `/tmp/nyxid-crm-review-error-contracts.log`.
The parent independently reran the same final binary with
`crm --test-threads=2`, the 27019 test URL and `UPDATE_CRM_FIXTURES` explicitly unset:
**55 passed / 0 failed / 0 ignored in 83.61s**,
`/tmp/nyxid-crm-review-backend-final.log`. This overlaps the worker's CRM run.

### Regenerated HTTP fixtures

All **25 fixture files** were regenerated through the actual HTTP test; **16**
have changes in the final checkout, including the new
`record_blank_required_text.json`. The complete changed fixture manifest is
listed below. `fixture-inspection.json` records the inspected manifest and checks:

- History contains one `stage_changed` event, with `changed_fields: ["stage"]`
  and `from/to`, alongside its note and create event.
- Empty and whitespace-only required text return the same 422 fixture with
  `values.name: required text cannot be blank`.
- Invalid view projection and missing-field errors are both retained at
  `props.columns` and `props.columns.0`.

Fixtures were not edited by hand. The subsequent no-update CRM run passed parity.

### Frontend and formatting

The final full frontend selection passed independently with two workers:

```sh
npm test -- --maxWorkers=2 src/components/crm src/lib/crm src/hooks/use-crm \
  src/schemas/crm src/lib/assistant/nyxagent-live.test.ts \
  src/hooks/use-account-panel.test.tsx \
  src/lib/assistant/account-panel-search.test.ts \
  src/components/assistant/nyxbot-agent-details.test.tsx
```

**30 files / 161 tests passed in 32.51s**:
`/tmp/nyxid-crm-review-frontend-tests-final.log`, including real server fixture
parity and the final blank-text, activity and chart-identity regressions.
Targeted final chart checks passed **2 files / 18 tests** with one worker:
`npm test -- --maxWorkers=1 src/lib/crm/views/chart-adapters.test.ts src/components/crm/workspace/renderer.test.tsx`
(`frontend-chart-identity-tests.log`). The activity cases passed **1 file / 2
tests** (`frontend-activity-tests.log`). These counts overlap the full selection.

The final production `npm run build` after the chart-identity fix **passed with
exit 0** (parent session `42292`): TypeScript, main/credential Vite builds, legal
prerender and the mock-footprint assertion. Log:
`/tmp/nyxid-crm-review-build-final.log`.
The closing changed-file ESLint run passed **60 files** with
`--max-warnings=0`, zero errors or warnings:
`/tmp/nyxid-crm-review-lint-final.log`. The chart fix also passed its targeted
zero-warning ESLint check (`frontend-chart-identity-lint.log`).

Installed Prettier formatted the changed CRM modules/tests, including the final
chart files (`prettier.log`, `prettier-runtime-fixes.log`,
`prettier-chart-identity.log`). `cargo fmt --all -- --check` passed on the final
Rust source (`rustfmt-final-check.log`); the parent's independent check also
passed (`/tmp/nyxid-crm-review-rustfmt-final.log`). Closing `git diff --check`
passed (`diff-check-final.log`). Unrelated broad formatting was restored.

### Browser evidence and limits

Browser scripts use actual frontend UI with **mocked APIs**, separately from
transactional backend evidence. The parent reports PASS for desktop and 390px
mobile company/contact/opportunity/task create/link/assignment/note/stage
workflows (six keyed writes), saved-workspace/detail navigation,
Back/Forward/reload/account-panel coexistence, owner switches, dirty delta edits,
conflict/protection/note retry, 404 masking, 503 draft retention, sequential schema
revisions1/2/3, import preview/roundtrip/retry, generation explicit save and
payload-aware retry, live invalidation and reconnect. Scripts:
`/tmp/nyxid-crm-browser-review-*.cjs`; review index:
`/tmp/nyxid-crm-review-findings.md`.

Generation tests use mock providers while exercising the actual claim/routing/
billing services. No production deployment, flag activation, external provider
session, commit or push was performed. This evidence covers the approved CRM
baseline and impacted paths; it is not a claim that every repository test or
external-provider/browser combination was executed.

## Inherited baseline verification gates

- [x] Compile final Rust source/tests and record the actual binary/build evidence.
- [x] Pass CRM Mongo/fixture/generation/native/admin/live tests on the replica set.
- [x] Pass impacted one-shot/utility/authority/shared/team suites.
- [x] Regenerate and inspect HTTP fixtures; pass no-update server parity.
- [x] Pass final full frontend selection, changed-file lint and formatting checks.
- [x] Pass final production frontend build after the chart-identity fix.
- [x] Record final independent rerun results, manifest and final whitespace check.

## Changed-file manifest

This was the **147-file inherited checkout diff: 69 backend, 60 frontend and 18 documentation/
fixture files**, including the six preserved initial dirty files.
Small additions in existing non-CRM test modules initialize the new typed grant/source fields; they do not represent new subsystem implementations.

### Backend

- `backend/src/errors/mod.rs`
- `backend/src/errors/voice_start.rs`
- `backend/src/handlers/admin_utility_inference.rs`
- `backend/src/handlers/assistant_group_tests.rs`
- `backend/src/handlers/assistant_nyxagent.rs`
- `backend/src/handlers/assistant_nyxagent_tests.rs`
- `backend/src/handlers/assistant_team.rs`
- `backend/src/handlers/assistant_team_tests.rs`
- `backend/src/handlers/async_service_policy_tests.rs`
- `backend/src/handlers/crm.rs`
- `backend/src/handlers/crm_fixture_tests.rs`
- `backend/src/handlers/mcp_crm_tests.rs`
- `backend/src/handlers/mcp_transport.rs`
- `backend/src/handlers/nyxbot_custom_event_tests.rs`
- `backend/src/handlers/nyxbot_delegated_image_tests.rs`
- `backend/src/handlers/nyxbot_guest_channel_tests.rs`
- `backend/src/handlers/nyxbot_guest_model_tests.rs`
- `backend/src/handlers/nyxbot_image_delivery_tests.rs`
- `backend/src/handlers/nyxbot_media_tests.rs`
- `backend/src/handlers/nyxbot_mentions_tests.rs`
- `backend/src/handlers/nyxbot_source_queue_tests.rs`
- `backend/src/handlers/nyxbot_tests.rs`
- `backend/src/handlers/trigger_scheduler_tests.rs`
- `backend/src/models/assistant_agent.rs`
- `backend/src/models/crm_record.rs`
- `backend/src/models/crm_saved_view.rs`
- `backend/src/models/platform_settings.rs`
- `backend/src/routes.rs`
- `backend/src/services/assistant_access_mode_tests.rs`
- `backend/src/services/assistant_account_tools.rs`
- `backend/src/services/assistant_acknowledgement_service.rs`
- `backend/src/services/assistant_agent_credential_service.rs`
- `backend/src/services/assistant_agent_learning.rs`
- `backend/src/services/assistant_agent_learning_review_tests.rs`
- `backend/src/services/assistant_authority_tests.rs`
- `backend/src/services/assistant_live.rs`
- `backend/src/services/assistant_nyxagent_tests.rs`
- `backend/src/services/assistant_oneshot_inference.rs`
- `backend/src/services/assistant_oneshot_inference_tests.rs`
- `backend/src/services/assistant_service_execution_managed_tests.rs`
- `backend/src/services/assistant_team_service.rs`
- `backend/src/services/assistant_team_tools.rs`
- `backend/src/services/assistant_upload_service.rs`
- `backend/src/services/crm/activities.rs`
- `backend/src/services/crm/fields.rs`
- `backend/src/services/crm/generation.rs`
- `backend/src/services/crm/generation_tests.rs`
- `backend/src/services/crm/mod.rs`
- `backend/src/services/crm/objects.rs`
- `backend/src/services/crm/people.rs`
- `backend/src/services/crm/query.rs`
- `backend/src/services/crm/records.rs`
- `backend/src/services/crm/regression_tests.rs`
- `backend/src/services/crm/tools.rs`
- `backend/src/services/crm/tools_tests.rs`
- `backend/src/services/crm/transfer.rs`
- `backend/src/services/crm/transfer_tests.rs`
- `backend/src/services/crm/views.rs`
- `backend/src/services/machine_access_tests.rs`
- `backend/src/services/org_agent_tests.rs`
- `backend/src/services/org_service.rs`
- `backend/src/services/personal_erasure_service/local.rs`
- `backend/src/services/platform_manager_journey_tests/owner_management.rs`
- `backend/src/services/platform_provisioning_service.rs`
- `backend/src/services/shared_agent_service.rs`
- `backend/src/services/shared_agent_tests.rs`
- `backend/src/services/utility_inference_service.rs`
- `backend/src/services/voice/credential_tests.rs`
- `backend/src/test_utils.rs`

### Frontend

- `frontend/src/components/assistant/nyxbot-agent-details.test.tsx`
- `frontend/src/components/assistant/nyxbot-agent-details.tsx`
- `frontend/src/components/crm/activity-timeline.test.tsx`
- `frontend/src/components/crm/activity-timeline.tsx`
- `frontend/src/components/crm/crm-page.test.tsx`
- `frontend/src/components/crm/crm-page.tsx`
- `frontend/src/components/crm/date-report.tsx`
- `frontend/src/components/crm/field-editor.test.tsx`
- `frontend/src/components/crm/field-editor.tsx`
- `frontend/src/components/crm/filter-controls.tsx`
- `frontend/src/components/crm/grant-picker.tsx`
- `frontend/src/components/crm/import-export.tsx`
- `frontend/src/components/crm/object-table.tsx`
- `frontend/src/components/crm/people-picker.test.tsx`
- `frontend/src/components/crm/people-picker.tsx`
- `frontend/src/components/crm/pipeline-board.test.tsx`
- `frontend/src/components/crm/pipeline-board.tsx`
- `frontend/src/components/crm/record-form.test.tsx`
- `frontend/src/components/crm/record-form.tsx`
- `frontend/src/components/crm/record-picker.tsx`
- `frontend/src/components/crm/record-sheet.test.tsx`
- `frontend/src/components/crm/record-sheet.tsx`
- `frontend/src/components/crm/schema-editor.test.tsx`
- `frontend/src/components/crm/schema-editor.tsx`
- `frontend/src/components/crm/workspace/crm-views-panel.test.tsx`
- `frontend/src/components/crm/workspace/crm-views-panel.tsx`
- `frontend/src/components/crm/workspace/renderer.test.tsx`
- `frontend/src/components/crm/workspace/renderer.tsx`
- `frontend/src/components/crm/workspace/view-editor.test.tsx`
- `frontend/src/components/crm/workspace/view-editor.tsx`
- `frontend/src/hooks/use-crm-recovery.test.tsx`
- `frontend/src/hooks/use-crm-views.test.tsx`
- `frontend/src/hooks/use-crm-views.ts`
- `frontend/src/hooks/use-crm.ts`
- `frontend/src/hooks/use-nyxagent-live.ts`
- `frontend/src/lib/assistant/access-authority-copy.ts`
- `frontend/src/lib/assistant/nyxagent-live.test.ts`
- `frontend/src/lib/assistant/nyxagent-live.ts`
- `frontend/src/lib/crm/field-kinds.test.tsx`
- `frontend/src/lib/crm/field-kinds.tsx`
- `frontend/src/lib/crm/import.test.ts`
- `frontend/src/lib/crm/import.ts`
- `frontend/src/lib/crm/location.ts`
- `frontend/src/lib/crm/money.ts`
- `frontend/src/lib/crm/owner-context.ts`
- `frontend/src/lib/crm/pipeline-query.test.ts`
- `frontend/src/lib/crm/pipeline-query.ts`
- `frontend/src/lib/crm/query-visibility.ts`
- `frontend/src/lib/crm/record-navigation.ts`
- `frontend/src/lib/crm/views/catalog.tsx`
- `frontend/src/lib/crm/views/chart-adapters.test.ts`
- `frontend/src/lib/crm/views/chart-adapters.ts`
- `frontend/src/lib/crm/views/schema.ts`
- `frontend/src/lib/crm/views/validation.ts`
- `frontend/src/router.tsx`
- `frontend/src/schemas/assistant-nyxagent.ts`
- `frontend/src/schemas/crm-fixtures.test.ts`
- `frontend/src/schemas/crm.test.ts`
- `frontend/src/schemas/crm.ts`
- `frontend/src/test/crm-router.tsx`

### Documentation and regenerated fixtures

- `docs/CRM.md`
- `docs/fixtures/crm/activities.json`
- `docs/fixtures/crm/activity_note_create.json`
- `docs/fixtures/crm/record_blank_required_text.json`
- `docs/fixtures/crm/record_company_create.json`
- `docs/fixtures/crm/record_contact_create.json`
- `docs/fixtures/crm/record_move.json`
- `docs/fixtures/crm/record_opportunity_create.json`
- `docs/fixtures/crm/record_patch.json`
- `docs/fixtures/crm/record_revision_conflict.json`
- `docs/fixtures/crm/related.json`
- `docs/fixtures/crm/table_page_1.json`
- `docs/fixtures/crm/table_page_2.json`
- `docs/fixtures/crm/view_create.json`
- `docs/fixtures/crm/view_get.json`
- `docs/fixtures/crm/view_validate_invalid.json`
- `docs/fixtures/crm/views_list.json`
- `docs/plans/crm-implementation-completion.md`

## CMS/artifact verification (5–6 November 2026)

The G pass first preserved the inherited tree in nine subsystem commits, then
implemented the default-off `cms:poc-v1` gate, bounded live conversation artifact
references, custom objects, `crm_present`, channel links, and a grouped artifact catalog for custom datasets and saved
views. D1's selected relationship labels and D2's text controls/table headers
are corrected. The contract and rollout decisions are in [CRM.md](../CRM.md).

The inherited G frontend verification below predates the G2 merge and its final
fixes. It is historical evidence, not verification of the current tree. It used
`PATH=/opt/homebrew/bin:$PATH` in `frontend/`:

| Command | Result | G log |
| --- | --- | --- |
| `npx tsc -b` | Passed | `G-review-tsc-3.log` |
| `npx vitest run` | 670 files, 7,215 tests passed; 0 failed | `G-review-vitest-2.log` |
| `npm run lint` | 0 errors; 33 existing warnings, no new warnings | `G-review-lint-3.log` |
| `npm run build` | Passed, including typecheck and build postchecks | `G-review-frontend-build-2.log` |

The production output separates `artifacts-CQHHt4o1.js` (3.66 kB, gzip 1.44 kB),
`crm-page-CItcE_PA.js` (40.03 kB, gzip 11.81 kB),
`renderer-BVpmmJit.js` (133.24 kB, gzip 39.66 kB), and
`grant-picker-CgMs19tI.js` (2.56 kB, gzip 1.24 kB).
`assistant-BSo52A09.js` dynamically imports artifacts and has no static import of
the artifact or renderer chunks. `G-review-frontend-chunking-1.json` records that check.

G logs live under
`/Users/chronoai/.heca/plans/dynamic-crm-artifacts/logs/`; the external G2 report records the merge, review maps, current cheap checks and
explicit verification limits.


## G2 merge and review continuation (6 November 2026)

Merged `origin/main` at `e5153f6c85b8acefafcb1873f329175dc7596ac6` in
`4eac272da`. The 26 conflict resolutions preserve main's generated Sandbox
artifacts, execution tasks, artifact recovery and channel identity fields while
keeping CRM references, routes and lazy components distinct. Generated files
and CRM cards can coexist in a reply; artifact-only group replies render in the
agent branch. The redesigned main grants sheet sends CRM only after an explicit
CRM edit while enabled. New org-channel fixtures preserve existing CRM grants.
CRM typography now follows main's shared roles and shared Label implementation,
including its current text-12 table-header role.

The R1 fixes use a tagged reference enum, one atomic bounded/deduplicating update
with sticky Created roles, best-effort optional recording after committed
mutations, explicit transactional creation results, bounded metadata/scalar
summaries, and an empty-reference fast path before any flag read. Custom-object
caps and unique keys use the owner fence; prospective schema grants are exact
keys. The grouped Artifacts catalog counts custom datasets and saved views.
Chat queries inherit a no-poll context; 401 follows session expiry, resource
refusals mask cached values, and Open uses Router links. The external report maps
every R1 item to commits, test names and source evidence. Test presence or type
checking is not a claim that Mongo-backed regressions executed in this pass.

### Stack boundaries (R2-1)

Historical G binaries overflowed during personal/shared group admission, guest
model setup, Lark attachment/discovery dispatch and async automation tests. The
logs identify the failing CRM-branch tests but provide no controlled comparison
against main and do not prove which future caused the growth. G2 does not
attribute those failures to main. Rebuilding/running main and the full test
binary would exceed this continuation's cheap-local-verification scope.

The broad wrapping changes introduced by eight boxing commits (`029253913`,
`3d3dc35c4`, `df8554d6b`, `fd3d45679`, `4b7b24d6c`, `b125e79cf`,
`ed7d7320d`, `d6cb32a8a`) are removed from existing authority, credential,
shared-access, HTTP-proof, async-poll,
group-admission and guest-model call sites. Main's own existing boundaries,
including `for_key` boxing `for_key_with_access`, remain intact. New CRM calls
are isolated instead: virtual-service availability, specialist grant validation,
settled-turn flag resolution, and personal/org group artifact publication (plus
the existing boxed CRM MCP execution). No nextest stack exemption was added
without proof that a failure is test-only. The report gives each old commit's
observed test/log and the removal decision.

This narrows the implementation change but does not establish normal-stack
runtime safety or main-versus-CRM causality. CI must execute the named historical
regressions and CRM tests; a passing `cargo check --tests` cannot prove 2 MiB
Tokio worker behavior. Any remaining overflow needs a measured future boundary
at the growing CRM call rather than another broad speculative wrapper.

### Current G2 local verification

All commands use the default stable toolchain and normal build flags. Cargo
checks are detached, logged and polled, with one cargo process at a time. Frontend
commands run in `frontend/` with `PATH=/opt/homebrew/bin:$PATH`.

| Check | G2 result | Log under the external logs directory |
| --- | --- | --- |
| First `cargo check -p nyxid --tests` | Six merge initializer errors reported; stale process stopped after source repairs (not a successful check) | `G2-check-1.log` |
| `cargo fmt --all -- --check` | Passed, exit 0 | `G2-fmt-final.log`, `G2-backend-final.log` |
| Final `cargo check -p nyxid --tests` | Passed, exit 0, 22m 27s | `G2-check-2.log`, `G2-backend-final.log` |
| `npx tsc -b` | Passed | `G2-tsc-4.log` |
| `npm run lint` | Passed, zero errors, 34 existing main warnings; typography guard passed | `G2-lint-2.log` |
| Main ESLint warning baseline | Same 34 warnings on the 21 warned main source files | `G2-lint-main-baseline.log` |
| Full `npx vitest run --maxWorkers=2` after merge | 717 files / 7,769 tests passed; one typography-policy test failed | `G2-vitest-1.log` |
| CRM/assistant/hooks/schema/flag suites plus typography policy after fixes | 226 files / 2,549 tests passed, zero failures | `G2-vitest-2.log` |

The full run covered all merged frontend files. The focused repeat covers the
files changed afterward and the failed typography guard. Earlier type-check
failures (refreshed dependency and new test typing) and initial lint failure are
retained in the logs, not counted as passes. `npm ci` refreshed dependencies from
the merged lockfile. No dependency source changes were necessary.

External report:
`/Users/chronoai/.heca/plans/dynamic-crm-artifacts/reports/G2-implementer.md`.
Logs:
`/Users/chronoai/.heca/plans/dynamic-crm-artifacts/logs/`.

The original heavy verification requirements are superseded by G2's explicit
cheap-check instructions. No backend test codegen/runtime, clippy, full backend
suite, production build, POC rebuild/restart, browser screenshot run or live
NyxBot model loop was run in G2. Old browser screenshots and chunk sizes above
must not be used as current evidence. All replicas and grant writers must be
upgraded and old writers drained before enabling `cms:poc-v1` or authoring
nonempty CRM grants. Nothing was pushed, deployed or enabled by this pass.
