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
The final plan incorporates AC-01 through AC-28. All local acceptance checks
passed on the prior integration base, and the PM closed the original findings.
Opus preliminary review returned the additional findings recorded below.
Release 0.66.0 and the subsequent frontend readability revision `868ce0b7`
are integrated and locally verified. The PM personally reviewed the final source
and every substantiated finding is closed. Required remote CI and Opus 5.5's
final verdict are recorded on PR #1796 against its published head; the PR is the
authoritative delivery status for those external gates.

## Plan review findings

| Finding | Required correction | Status |
|---|---|---|
| A static key route would take over an existing valid service slug. | Use a dedicated `/service-preferences` route. | Closed: existing slug and dedicated-route HTTP regressions passed |
| Adding rejection middleware to the existing key-update group could affect catalog-curation callers. | Restrict the new PUT route and verify first-party auth in its handler. | Closed: caller-matrix and neighboring curation regressions passed |
| Raw rank gaps disclose positions of inaccessible entries. | Expose dense ranks from the caller's authorized inventory. | Closed: scoped HTTP, relay/guest MCP and live-org regressions passed |
| The minimal model fixture omits required dates, and the absent response timestamp is unspecified. | Require model BSON dates; return a nullable timestamp for no saved order. | Closed: BSON and missing-document HTTP regressions passed |
| Version and UUID validation need canonicalization, bounds, and overflow rules. | Validate canonical UUID v4 IDs, bounded body/list, safe versions, and unknown fields. | Closed: pure/schema and mounted HTTP regressions passed |
| Loading an empty order after a read failure enables an accidental overwrite. | Block editing until a successful read, with explicit Retry. | Closed: failed-read and cached-read browser/unit regressions passed |
| Conflict and stale-ID recovery assume server data the generic error does not contain. | Refetch version/inventory before recovery, preserving local edits. | Closed: actual409 Overwrite and actual400 inventory-first browser regressions passed |
| Mocked drag callbacks do not prove browser gestures or focus behavior. | Test actual mouse, touch, and keyboard gestures on `/keys`. | Closed: independent fourteen-scenario browser suite passed |
| A one-service inventory still needs rank/unrank controls. | Enable single-service editing and enforce the selection limit. | Closed: one-service browser and 201-row limit regressions passed |
| Unsupported binary-crate test commands and silent database skips weaken validation. | Use valid commands and an explicit isolated database URI. | Closed: eight backend tests executed with explicit replica-set URI |

## Implementation review findings

The PM reviewed the implementation directly and required source correction and
execution evidence for each substantiated finding. The table records all returned
findings and their closure; external Opus sign-off and final CI are separate gates.

| Finding | Required correction | Status |
|---|---|---|
| Conflict recovery calls a submit closure that still sees `recovering = true`, so Overwrite exits without saving. | Separate recovery and mutation guards; prove a 409 followed by Overwrite persists local edits using the latest version. | Closed: browser Overwrite/persistence test passed |
| The initial MCP patch adds rank metadata to `tools/list` and omits it from search results. | Preserve the agreed `tools/list` boundary; add metadata to search matches and verify config/list remain unchanged. | Closed: mounted HTTP config and MCP discovery/execution invariance tests passed |
| A legacy preference document defaults to version zero when read, but its missing BSON version cannot match the update CAS filter. | Support missing-version rows in the version-zero CAS and prove the upgrade with a database test. | Closed: mounted HTTP/database legacy CAS regression passed |
| Inventory refresh failures during editing can hide the existing inventory error banner; preference-read failure can disable its own conflict recovery. | Keep edits, expose Retry, and let validated recovery repair failed reads. | Closed: independent 14-scenario browser run |
| Backend UUID validation does not verify the RFC 4122 variant, unlike the frontend UUID v4 schema. | Align validation and cover non-RFC variants. | Closed: pure validation and mounted HTTP regressions passed |
| The draft omits planned toolbar affordances and provenance badges, and uses oversized body typography. | Add the reorder icon, disabled explanation, owner badges, compact design tokens, and loading feedback. | Closed: refreshed source and desktop/mobile visual review |
| Every key-detail poll loads the entire inventory and repeats provider/grant work, including for users with no preference. | Skip unnecessary inventory work, bound dense-rank visibility to saved entries, reuse the live snapshot, and document/measure actual reads. | Closed: database command-monitoring regression passed |
| Search's appended native tool rows lack the advertised rank field, and an older discovery paragraph contradicts the new ordering. | Emit null rank for unrankable native rows and reconcile the existing description. | Closed: native-row MCP assertion passed and docs inspected |
| Asynchronous conflict recovery can continue after account switching and issue PUT under the new session. | Fence saves/recovery by live identity, reset abandoned editor state, and prove the race with a deferred read. | Closed: deferred account-switch browser test passed |
| The new grid/table test leaves a persisted table preference for following tests. | Reset view storage in setup and assert rendered ordering in both views. | Closed: isolated 31-test regression run passed |
| First-create responses expose nanosecond timestamps that BSON stores at millisecond precision. | Normalize write timestamps to BSON precision and compare first PUT with GET/no-op. | Closed: mounted HTTP/database timestamp equality regression passed |
| Reorder can capture cached inventory/order while a newer read is still in progress. | Wait for current reads before entry and test the disabled loading state. | Closed: loading/refetch unit regressions passed |
| Restoring focus in one animation frame can target a button still disabled by the inventory refetch on editor close. | Restore focus once the intended target is available, cancel stale focus requests, and verify with a delayed inventory response. | Closed: independent delayed Save/Cancel browser regressions |
| The new CLI command enum derives `Debug` for authentication arguments that intentionally do not implement it. | Remove the command enum's unnecessary derive; preserve authentication redaction and prove CLI compilation and command execution. | Closed: CLI compilation, unit and actual subprocess integration tests passed |
| The plan's caller table conflates REST preference-route rejection with MCP discovery and incorrectly claims relay callers receive no preference. | Separate REST auth from MCP identity semantics; document scoped relay owner preference, service-account subject defaults, and authorized delegated/OAuth discovery without changing existing authorization. | Closed: corrected docs inspected; real relay-auth MCP ranking parity passed |
| Editor entry joins legacy organization provenance, but recovery replaces it with raw `/keys` rows and late provenance never reaches the editor's state snapshot. | Reuse the source join for every inventory recovery and enrich late metadata without resetting ordered IDs, version or draft; verify organization labels survive refresh. | Closed: independent legacy-provenance browser regression |
| The 201-row boundary regression exceeds the default five-second timeout under the full suite and parallel build load. | Scope queries to the relevant rows and verify stable full-suite execution; retain the limit and saved-payload assertions. | Closed: focused and complete frontend reruns passed with the original timeout |
| Inserting the preference sorter leaves the old search documentation attached to the wrong function. | Attach the search contract to the production ranked-search function. | Closed: corrected source inspected by PM |
| The CLI output regression parses only the outer table border, while the existing condensed style uses a different internal separator. | Parse the actual column separators and retain exact ranked/unranked last-column assertions, with useful failure output. | Closed: PM inspected diagnostic output/fix; both CLI integration tests passed |
| Rust 1.98 CI Clippy rejects a cloned single-item slice in the MCP fixture. | Use `std::slice::from_ref` without suppressing the lint and pass Clippy. | Closed: PM inspected fixture change; final local Clippy passed |

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

### Prior integrated frontend validation by the PM

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


The corrected feature rerun passed all eight tests (zero failures or ignored
checks, 3.15 seconds after 12m34 compilation). The PM inspected the corrected
fixture and the execution log. It proves preferred search/connected ordering,
null native ranks, owner/scoped/relay/guest visibility, byte-identical
`tools/list`, unchanged explicit tool result/target and exactly two effects for
two named calls with identical execution-audit data. The earlier HTTP test
proves full `/mcp/config` equality. Log:
`/tmp/service-preference-integrated-backend-recheck.log`.

Draft PR: https://github.com/ChronoAIProject/NyxID/pull/1796. Source revision:
`caed88dfa5e449af062b77d2fd264b3c05e4d1b1`. Required remote CI is running.


The PM's final production-source inspection covers persistence/CAS and validation,
route/auth layering, inventory and dense-rank visibility, MCP relevance and scope
application, explicit execution boundaries, CLI identity/slug resolution,
account-bound UI recovery, drag controls, shared group/table pills and view-state
preservation. No additional substantiated production finding remained after the
search-comment correction. `cargo fmt --all -- --check` independently passed.
Final CLI/static-check execution, required CI and Opus sign-off remain open.


The first remote CLI test/coverage runs and the local CLI integration run exposed
the same new-test defect: parsing only `│` does not split the condensed table's
internal `┆` columns. CLI unit tests passed (two tests); the Set/conflict integration
case passed, while the table/JSON case requires the corrected parser and a rerun.
The PM confirmed both separators in the installed table-style source and returned
the correction to Sol. This failed run is not final acceptance evidence.


### Final CLI execution evidence

- CLI unit feature checks: two passed, zero failures/ignored, 0.01 seconds.
- Actual CLI subprocess integration: two passed, zero failures/ignored, 1.99
  seconds after the separator correction. The PM inspected the diagnostic table
  and corrected parser. Assertions retain the exact final-column `1`/`-` values,
  saved table ranks, JSON equality, active-slug selection, versioned PUT and
  nonzero conflict exit with actionable text.
- CLI wizard freshness: one passed, 0.06 seconds.

Logs: `/tmp/service-preference-final-cli-unit.log`,
`/tmp/service-preference-final-cli-integration.log`,
`/tmp/service-preference-final-wizard-freshness.log`.


### Prior local review gate

`cargo clippy -p nyxid -p nyxid-cli --all-targets -j 1 -- -D warnings` passed on
the corrected source (5m38s, no warnings or errors), log
`/tmp/service-preference-final-clippy.log`. This closes the last local gate.
The PM inspected both final fixture diffs and all successful execution logs.
Every substantiated plan and implementation finding above is closed.

The first remote CI run identified the CLI parser and cloned-slice fixture
findings; those failures are superseded only after CI passes on the corrected
revision. Final Opus review will cover the complete plan and PR diff, followed
by confirmation on the final revision after required CI succeeds.


## Opus preliminary review and 0.66.0 integration

Opus 5.5 reviewed committed `35af1701e8c66f1226f883773325000ed5deb28b`
read-only. It accepted storage/CAS, REST auth, list/detail rank visibility,
MCP relevance/order/execution separation and frontend identity/recovery boundaries.
It withheld final sign-off while release #1795 (`566ca5f9`) was integrated.
The PM accepted and returned every additional substantiated finding, including
minor and documentation findings, to the same Sol implementation session.

| Finding | Required correction | Status |
|---|---|---|
| Preference GET repeats a full inventory/render/decrypt walk even for absent/empty saved order; plan understates page reads. | Read preference first; reuse the bounded saved-ID visibility helper for GET/detail; measure absent/empty/bounded reads and correct the plan. | Closed: PM inspected shared helper and plan; merged command-monitoring regression passed |
| Guest search/list visibility tightening is insufficiently documented. | Explain granted UserManaged/Platform discovery and omission of Internal catalog entries, preserving separate native guest authorization. | Closed: PM inspected chat/08 and chat/09 corrections |
| UI ranks include visible disabled connections while MCP ranks reflect eligible active discovery, so numbers can differ. | Explain the dense-rank basis and a disabled-connection example in architecture/discovery docs. | Closed: PM inspected both documentation corrections |
| Plan status, evidence heading, legacy loader order and stale-error copy are outdated. | Reconcile final status/evidence and shipped behavior. | Closed: PM inspected corrected normative text and fresh evidence; publication gates remain explicit |
| Reorder action has an unreachable compact branch and a redundant inventory alias. | Remove both and preserve all entry/loading/focus behavior. | Closed: PM inspected source; fresh full frontend and 14 browser scenarios passed |

The PM fetched release `566ca5f9` when GitHub reported a merge conflict and
scheduled no checks on the updated PR. Sol preserved upstream delegation,
concurrency and asynchronous native catalog construction, resolved generated
wizard conflicts and applied review corrections before fresh validation.
The PM inspected the discovery diff against `566ca5f9`: preferred loading remains
limited to search and connected-service listing; verified caller claims,
delegation projection, async native catalog construction and execution admission
remain upstream's implementation. The shared GET/detail helper resolves at most
200 saved IDs through live source/scope visibility and projected endpoint
existence, avoiding provider loads and credential rendering/decryption. An
absent, empty or scope-excluded order skips inventory work. Plan §8 now counts
both page preference-document reads and bounded nonempty-GET work.
The PM owns merge commit/publication.

### Fresh 0.66.0 frontend validation by the PM

- Full isolated suite: all 461 files and 4,723 tests passed in 168.67 seconds.
- Browser suite: all 14 scenarios passed in 37.3 seconds with actual mouse,
  keyboard and touch input, saved group/table/overview pills, preserved filters
  and saved views, delayed focus, stale/conflict recovery and account fences.
- Production build: TypeScript, Vite, legal prerender, credential-accept bundle
  and mock-footprint assertion passed.
- Lint: zero errors and zero feature warnings; the same 29 unrelated baseline
  warnings remain.
- PM inspected fresh desktop and 390-pixel mobile screenshots: instructions,
  handles, connection identifiers, provenance, numbered pills, divider and
  Save/Cancel controls fit; long mobile labels wrap and overflow assertions pass.

Logs: `/tmp/nyxid-service-preference-066-{full-frontend,browser,build,lint}.log`.
Screenshots: `/tmp/nyxid-service-preference-review/066-*.png`.
The isolated frontend command and origin override are the same as recorded above.
These results supersede the older frontend evidence for the integrated source.

The fresh backend feature run passed all eight tests with no failures or ignored
checks (3.21 seconds after 6m09 compilation). The PM inspected the actual log,
including the extended mounted GET command-monitoring test for absent/empty
orders, bounded saved IDs, endpoint projections and no provider/key reads.
Log: `/tmp/service-preference-merge-566ca5f9-backend-feature.log`.
The PM also inspected all successful merged regression logs: ten neighboring
search/assistant/guest/audit/curation checks and seventeen delegation, proxy
parity, skill discovery, concurrency and org-agent checks passed without failures
or ignored tests. CLI unit and actual subprocess integration passed two tests
each; wizard freshness passed one. Changed-package all-target Clippy with warnings
denied passed in 4m15s. The log prefix for these checks is
`/tmp/service-preference-merge-566ca5f9-`; exact filters and counts are in plan §19.
An independent PM scan of 45 execution/approval/billing/insight files found no
`service_preferences` or `preference_rank` reference.

All substantiated preliminary Opus findings are closed. PM committed the 0.66.0
integration as `0390ad00`, then reviewed the frontend-only readability merge
with `868ce0b7`. Its service-table and pool presentation changes are preserved
alongside Discovery pills and the editor; the information icon follows upstream's
readability token. No backend/CLI Rust source or manifest changed. The completed
backend/CLI/Clippy results therefore remain applicable; wizard generation and
freshness were verified again against the final merged embedding.

### Final local validation on `868ce0b7`

- Full isolated frontend suite: all 463 files and 4,741 tests passed in 168.33s.
- Real-route browser suite: all 14 scenarios passed in 36.1s.
- Production build and lint passed; zero lint errors or feature warnings and
  the same 29 unrelated baseline warnings.
- Wizard build passed with its regenerated 168-file closure; freshness passed
  one test, with zero failures/ignored checks, in 0.04s after a focused CLI rebuild.
- PM inspected the final desktop and 390-pixel mobile screenshots. Controls,
  instructions, provenance and numbered pills are visible; long labels wrap and
  the browser overflow assertions pass.
- PM inspected the final diff against `868ce0b7` and verified no Rust-source
  changes from `0390ad00`, no unmerged paths and clean staged/working whitespace.

Final UI logs: `/tmp/nyxid-service-preference-868-{full-frontend,browser,build,lint}.log`.
Wizard logs: `/tmp/service-preference-merge-868ce0b7-{wizard-build,wizard-freshness}.log`.
Screenshots: `/tmp/nyxid-service-preference-review/868-*.png`.
Plan §20 records exact checks. CI and Opus's final plan/PR sign-off are recorded
in the PR body against the published head, avoiding an evidence-only commit
after the final review.

### Follow-up visual comparison with current main

The PM fetched main again on 7 October 2026. Both `origin/main` and the
production health endpoint identify `868ce0b7`; that commit is an ancestor of
the reviewed branch. No main commits are missing. The PM rendered main and the
PR with identical sample connections and inspected the normal collapsed cards
and expanded connection table. The earlier editor screenshots represented the
Reorder editing state, not the normal External Services page.

The comparison exposed one layout regression: the inline Discovery badge could
squeeze the connection-label text to zero width. Sol moved the badge into a
wrapping metadata row below the existing label/readiness header. The PM reviewed
the correction, updated plan §6.1/task 13a/AC-17, and inspected the corrected
expanded-table screenshot against main. The existing name/readiness layout is
preserved and the rank remains bound to its connection.

The PM also required the regression to measure the actual text span, prevent
geometric overlaps, and prove that narrower tables scroll within their container.
Existing unit/browser checks now locate ranks in the correct connection cell.
The grouped browser scenario compares semantic filter values, including source,
auto-connected visibility, organization/service selections, search, saved-view
count and absence of saved-view writes, rather than transient toolbar labels.
All follow-up source and test findings are closed.

Validation inspected by the PM:

- Three focused frontend files passed all 60 tests in 4.53 seconds.
- All 15 real-route browser scenarios passed in 38.8 seconds, including the new
  1440-pixel label/overlap regression and 1024/390-pixel container-overflow checks.
- Changed-file ESLint passed with no errors or warnings; whitespace checks passed.
- The PM's production build passed TypeScript, both Vite outputs, legal prerender
  and the mock-footprint assertion.
- Two main/PR comparison scenarios passed in 6.7 seconds. Screenshots use identical
  sample data, not the user's production account.

Browser/build logs:
`/tmp/nyxid-service-preference-main-check-{browser,build}.log`.
Comparison harness, log and screenshots:
`/tmp/nyxid-external-services-verification/`.
Unit and lint execution are retained in the existing Sol heca session
`01a11208-5a9b-7221-a009-5cd45b355530`.

For the user's live review, the PM started the same frontend on loopback port
4630 with a temporary configuration outside the repository. It reuses the
existing production CLI/OAuth bridge, returns to normal `/keys`, and permits
production metadata reads plus saving the existing service view. It retains the
access token in server memory for 15 minutes, discards refresh tokens, and sets
an opaque HttpOnly local cookie. The bridge's redirect, session requirement,
invalid-callback rejection and mutation denial were verified. Production still
returns 404 for `/service-preferences`, so the frontend's existing compatibility
gate hides Reorder there. The setup introduces no repository source changes.

Only the existing Sol and Opus heca sessions are reused for this follow-up.
The PM owns publication; final required CI and Opus sign-off bind the revised
published head in the PR body.
