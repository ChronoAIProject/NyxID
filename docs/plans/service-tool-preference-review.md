# Service preference implementation review

This record tracks the delivery and review of user-controlled AI service preference ordering.

## Requested workflow

- Fable 5.1 designs the implementation and writes strict acceptance criteria.
- GPT-6.1-Sol implements the plan with xhigh reasoning and full access.
- The coordinating agent reviews the implementation directly and returns every substantiated finding for correction, including minor findings.
- Opus 5.5 reviews the completed plan and pull request before final sign-off.
- These three requested heca sessions do not delegate to additional agents.

## Completion gates

- Every plan acceptance criterion has implementation and validation evidence.
- The AI Services interface exposes understandable drag controls and order pills, with equivalent keyboard access.
- Persistence, concurrent edits, and save failures have verified behavior.
- Preferences preserve service visibility, owner boundaries, scopes, approvals, and explicit execution targets.
- Discovery exposes preference information without claiming that external clients must obey it.
- Relevant automated tests, static checks, and browser interaction checks pass.
- All substantiated implementation and review findings are closed.
- Opus 5.5 signs off on the plan and final implementation.

## Baseline

- Branch: `service-tool-preference-order`.
- Starting commit: `ffcd1c59d3e3e5fe4df3413cc93b0f27c4417fa4`.
- Integration base: `f3dc2be94670c5d38fbb9722cbf7c67d52deefa8`, including
  the refreshed grouped Services interface from #1685.
- Existing AI Services page: 20 tests pass.
- Existing frontend lint: zero errors, 29 warnings in unrelated existing files.
- Test database: dedicated local MongoDB replica set, selected with `NYXID_TEST_DATABASE_URL`; no production credentials or data.

## Review status

Fable 5.1 delivered 14 implementation tasks and 24 acceptance criteria. PM review
added four criteria and required corrections to routing, auth, privacy, model
defaults, concurrency, error recovery, accessibility, and validation commands.
The final plan incorporates AC-01 through AC-28. The integrated frontend and
seven pure/database-backed feature tests have passed; the corrected MCP fixture,
CLI checks, Rust static checks and final Opus review are the remaining gates.

## Plan review findings

| Finding | Required correction | Status |
|---|---|---|
| A static key route would take over an existing valid service slug. | Use a dedicated `/service-preferences` route. | In implementation |
| Adding rejection middleware to the existing key-update group could affect catalog-curation callers. | Restrict the new PUT route and verify first-party auth in its handler. | In implementation |
| Raw rank gaps disclose positions of inaccessible entries. | Expose dense ranks from the caller's authorized inventory. | In implementation |
| The minimal model fixture omits required dates, and the absent response timestamp is unspecified. | Require model BSON dates; return a nullable timestamp for no saved order. | In implementation |
| Version and UUID validation need canonicalization, bounds, and overflow rules. | Validate canonical UUID v4 IDs, bounded body/list, safe versions, and unknown fields. | In implementation |
| Loading an empty order after a read failure enables an accidental overwrite. | Block editing until a successful read, with explicit Retry. | In implementation |
| Conflict and stale-ID recovery assume server data the generic error does not contain. | Refetch version/inventory before recovery, preserving local edits. | In implementation |
| Mocked drag callbacks do not prove browser gestures or focus behavior. | Test actual mouse, touch, and keyboard gestures on `/keys`. | In implementation |
| A one-service inventory still needs rank/unrank controls. | Enable single-service editing and enforce the selection limit. | In implementation |
| Unsupported binary-crate test commands and silent database skips weaken validation. | Use valid commands and an explicit isolated database URI. | In implementation |

## Implementation review findings

The PM reviews the implementation directly while it is being completed. These
findings are based on inspected source; correction and execution evidence are
required before closing them. No implementation sign-off is claimed.

| Finding | Required correction | Status |
|---|---|---|
| Conflict recovery calls a submit closure that still sees `recovering = true`, so Overwrite exits without saving. | Separate recovery and mutation guards; prove a 409 followed by Overwrite persists local edits using the latest version. | Closed: browser Overwrite/persistence test passed |
| The initial MCP patch adds rank metadata to `tools/list` and omits it from search results. | Preserve the agreed `tools/list` boundary; add metadata to search matches and verify config/list remain unchanged. | Returned to Sol |
| A legacy preference document defaults to version zero when read, but its missing BSON version cannot match the update CAS filter. | Support missing-version rows in the version-zero CAS and prove the upgrade with a database test. | Closed: mounted HTTP/database legacy CAS regression passed |
| Inventory refresh failures during editing can hide the existing inventory error banner; preference-read failure can disable its own conflict recovery. | Keep edits, expose Retry, and let validated recovery repair failed reads. | Closed: independent 14-scenario browser run |
| Backend UUID validation does not verify the RFC 4122 variant, unlike the frontend UUID v4 schema. | Align validation and cover non-RFC variants. | Closed: pure validation and mounted HTTP regressions passed |
| The draft omits planned toolbar affordances and provenance badges, and uses oversized body typography. | Add the reorder icon, disabled explanation, owner badges, compact design tokens, and loading feedback. | Closed: refreshed source and desktop/mobile visual review |
| Every key-detail poll loads the entire inventory and repeats provider/grant work, including for users with no preference. | Skip unnecessary inventory work, bound dense-rank visibility to saved entries, reuse the live snapshot, and document/measure actual reads. | Closed: database command-monitoring regression passed |
| Search's appended native tool rows lack the advertised rank field, and an older discovery paragraph contradicts the new ordering. | Emit null rank for unrankable native rows and reconcile the existing description. | Returned to Sol |
| Asynchronous conflict recovery can continue after account switching and issue PUT under the new session. | Fence saves/recovery by live identity, reset abandoned editor state, and prove the race with a deferred read. | Closed: deferred account-switch browser test passed |
| The new grid/table test leaves a persisted table preference for following tests. | Reset view storage in setup and assert rendered ordering in both views. | Closed: isolated 31-test regression run passed |
| First-create responses expose nanosecond timestamps that BSON stores at millisecond precision. | Normalize write timestamps to BSON precision and compare first PUT with GET/no-op. | Closed: mounted HTTP/database timestamp equality regression passed |
| Reorder can capture cached inventory/order while a newer read is still in progress. | Wait for current reads before entry and test the disabled loading state. | Closed: loading/refetch unit regressions passed |
| Restoring focus in one animation frame can target a button still disabled by the inventory refetch on editor close. | Restore focus once the intended target is available, cancel stale focus requests, and verify with a delayed inventory response. | Closed: independent delayed Save/Cancel browser regressions |
| The new CLI command enum derives `Debug` for authentication arguments that intentionally do not implement it. | Remove the command enum's unnecessary derive; preserve authentication redaction and prove CLI compilation and command execution. | Returned to Sol |
| The plan's caller table conflates REST preference-route rejection with MCP discovery and incorrectly claims relay callers receive no preference. | Separate REST auth from MCP identity semantics; document scoped relay owner preference, service-account subject defaults, and authorized delegated/OAuth discovery without changing existing authorization. | Returned to Sol |
| Editor entry joins legacy organization provenance, but recovery replaces it with raw `/keys` rows and late provenance never reaches the editor's state snapshot. | Reuse the source join for every inventory recovery and enrich late metadata without resetting ordered IDs, version or draft; verify organization labels survive refresh. | Closed: independent legacy-provenance browser regression |
| The 201-row boundary regression exceeds the default five-second timeout under the full suite and parallel build load. | Scope queries to the relevant rows and verify stable full-suite execution; retain the limit and saved-payload assertions. | Closed: focused and complete frontend reruns passed with the original timeout |
| Inserting the preference sorter leaves the old search documentation attached to the wrong function. | Attach the search contract to the production ranked-search function. | Closed: corrected source inspected by PM |

## Integration review

Fable 5.1 revised the plan after #1685 replaced the page-level card and table
renderers. The PM accepted the revision and returned it to Sol for implementation.
Connection pills use `Discovery #n` to distinguish them from pool `Priority n`.
The collapsed group chip identifies the connection it summarizes, uses the full
authorized group and expands the card. Normal row and group ordering remain as
defined by the existing interface. Reorder edits the complete authorized inventory
while preserving filters, saved views and expanded cards without writing view
preferences. Existing routing, billing and usage behavior must remain intact.

The implementation was checkpointed and rebased onto the integration base above.
Earlier UI execution evidence records the pre-integration implementation. The
final integrated checks below supersede it. The expanded boundary search includes service insights and
billing summaries and finds no preference reads there.

## Validation environment

The user authorized clearing old Rust artifacts and inactive workspaces when
disk space prevents validation. The PM removed only the inactive
`openai-challenge/target` cache, reclaiming approximately 7 GB, then the inactive
`nyxid-managed-key-astra-review/target` and `sandy-crane/target` caches, reclaiming
approximately another 12 GB. Their source was preserved. The `codex-plugin`, `fluffy-comet`, `soft-wolf`, and `happy-maple`
build caches were retained because their workspaces or builds are active.
Backend checks use a task-specific target and an isolated local MongoDB replica
set. The earlier task-owned target was removed after a disk-full compilation;
that unsuccessful compilation is not validation evidence.
As concurrent builds consumed the reclaimed space, the PM also removed nine
inactive dependency caches (`frontend/node_modules`) from old, unregistered
workspaces with no live file references, reclaiming approximately 3.7 GB.
All workspace source was retained.
Six more inactive dependency caches were removed as concurrent builds continued
to consume disk capacity; their source was also retained.

## Earlier PM execution evidence

- `npm test -- src/pages/keys.test.tsx src/schemas/service-preference.test.ts`:
  22 tests passed on the working implementation.
- `npm run lint -- --no-warn-ignored`: zero errors; the same 29 unrelated
  warnings as the baseline, with no warning in feature files.
- Execution-boundary search found no preference reference in `proxy_service`,
  `execution_authority`, `mcp_approval`, or billing services.
- `npx playwright test e2e/service-preference.spec.ts --workers=1`: eight
  browser tests passed in 26.5 seconds, exercising real mouse, keyboard and
  touch gestures, Escape/divider behavior, pills in both views, persistence,
  focus, unsaved-change confirmation, network/conflict/read recovery,
  one-service and older-server states, and deferred account switching.
- PM visually inspected desktop and 390-pixel mobile editor screenshots.
  Controls, instructions, provenance, and numbered pills are visible; long
  organization labels wrap and the tested page has no horizontal overflow.
- `npm run build`: TypeScript, production application, legal prerender,
  credential-accept bundle, and mock-footprint checks all passed.
- The two new preference hook/editor unit files passed all four tests, covering
  cached-read failures, identity-bound writes, the ranking limit, and successful
  inventory refresh before pruning.

At this stage, database, transport, CLI and remaining browser edge-case gates
were still open. Later evidence below supersedes these historical results.

The first full frontend run passed 4,294 tests and failed six: one new test left
view-mode state behind, and five unrelated AgentDetailsSheet tests received 401s
from an existing server at happy-dom's default `localhost:3000` origin. Sol fixed
the view reset. A temporary validation config preserves the repository config and
changes only happy-dom's origin to the unused `127.0.0.1:4629`; both affected
files then passed all 31 tests. The complete isolated run passed all 438 files and
4,300 tests in 202.42 seconds. The existing local server was not modified.

Command: `NODE_ENV=test npx vitest run --config
/tmp/nyxid-service-preference-vitest.config.mts --maxWorkers=2`, from `frontend`.
The temporary config imports the repository's `vite.config.ts`, spreads its
settings unchanged, and sets only
`test.environmentOptions.happyDOM.url = "http://127.0.0.1:4629"`.

The initial PM post-rebase targeted run passed 39 tests and failed one old pill
label assertion. Sol updated the grouped-page expectations and the broader
integration run passed 98 tests across nine files. Its 13-scenario browser run
passed in 39.3 seconds, including complete-group chips, preserved filters and
saved views, overview/table pills, real gestures, delayed focus and error recovery.
The PM inspected the refreshed desktop and 390-pixel mobile screenshots: the
instructions, handles, owner badges, connection identifiers and Discovery pills
are visible and fit the tested viewport. The late-provenance correction and final
Rust checks were still pending at this stage; later evidence supersedes this status.

### Final integrated frontend validation by the PM

- Full isolated suite: all 458 files and 4,694 tests passed in 183.97 seconds.
- Browser suite: all 14 scenarios passed in 39.8 seconds, including late legacy
  organization provenance and stale-inventory recovery without resetting the draft.
- Production build: TypeScript, Vite, legal prerender, credential-accept bundle,
  and mock-footprint assertion passed.
- Lint: zero errors and zero feature warnings; the same 29 unrelated baseline
  warnings remain.
- The 201-row boundary regression initially timed out under concurrent build
  load. Scoped queries corrected its excessive DOM traversal. The focused file
  passed all three tests in 3.45 seconds (2.26 seconds executing tests), and the
  complete rerun passed with the original five-second timeout.
- PM inspected the mobile detail screenshot, including the long service and
  organization labels, Disabled badge, divider, and Save/Cancel controls. Labels
  wrap within the 390-pixel viewport and the browser overflow assertion passes.

Logs are preserved locally in `/tmp/nyxid-service-preference-integrated-{full-recheck,
browser,build,lint}.log`; reviewed screenshots are in
`/tmp/nyxid-service-preference-review/final-*.png`.


### Database and neighboring regression evidence

The integrated feature run compiled successfully and passed seven pure/mounted
HTTP/database tests against the isolated replica set. They cover BSON model
serialization and legacy defaults, UUID/version validation, relevance/cap/order,
concurrent first and later saves, no-op/timestamp equality, chained audits,
scoped reads, stale storage, existing key slugs and MCP config, live organization
visibility revocation, and command-monitored bounded detail/list reads.

Its eighth test initially failed because the MCP fixture double-wrapped
`nyx__call_tool`. Direct dispatch and the correct fixture search word repair the
fixture; that test requires a successful rerun before closing the MCP gate.

The PM independently ran the existing
`curation_router_scoped_discovery_history_and_route_confinement` regression
against the same explicit replica-set URI and rebased backend test binary.
It passed (one test, 2.04 seconds), preserving the neighboring curation
routing and authorization contract. Log:
`/tmp/nyxid-service-preference-curation-regression.log`.

The PM completed the integrated production-source review and is publishing a
draft PR to start the repository CI while final local Rust checks continue.
Draft publication does not close the remaining checks or Opus sign-off.
