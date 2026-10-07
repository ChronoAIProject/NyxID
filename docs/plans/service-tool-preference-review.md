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

The user clarified that connection choice and reordering must appear inside
each actual service group, such as the expanded Anthropic connection table.
The global editor does not satisfy that requirement. The revised inline
implementation is complete and PR #1796 remains draft through final validation
and review. Fable completed the revised plan; ROOT authorized Part A with
31 original acceptance criteria and personally reviewed the revised source and behavior.
The revised plan adds AC-32 for sticky controls and AC-33 for CLI transport security.
All 33 Part A acceptance criteria now have local implementation and validation
evidence. ROOT personally reviewed the coverage/Opus corrections, sticky
controls and spacing, protected CLI release transport, final documentation,
browser screenshots and completed validation logs. The additional findings
from published `0ea6cfa3e41eea4cbc03d65e9c4e8d24da5d3fbb` are corrected locally,
including the 201-row coverage timeout and credential transport boundary.
Fresh exact-head CI, aggregate CodeQL and Opus sign-off remain publication
gates; local evidence alone does not close them. PR #1796 remains draft;
old checks and approval for `e72036b7` do not satisfy these gates.

## Revised scope: PM plan review

The PM fetched main again and integrated `cac8ce77` (#1797) in `bac66b3f`.
The only conflicts were the generated CLI wizard HTML/hash; main's artifacts
are an interim baseline and the final frontend must regenerate them.

Part A places discovery controls in the existing per-service connection table.
The user's separate question about using that order for implicit LLM gateway
selection is pending; implementation of the confirmed UI/discovery work does
not depend on that answer. No mock preference persistence is active on the
production-backed frontend at port 4630.

| PM finding | Required correction | Status |
|---|---|---|
| Group-anchor sorting relocates unrelated interleaved services, including with an empty order. | Permute only each group's occupied slots, within identical relevance buckets for search; test interleaving and result truncation. | Closed: pure/MCP slot and relevance/cap regressions pass; original loader vector preserved |
| Putting pills beside label/readiness would reintroduce the reviewed text squeeze. | Keep the existing label/readiness geometry; place preference metadata below it. | Closed: final renderer/source and geometry tests place pills below readable identities |
| Removing a group and reinserting a block cannot preserve unrelated absolute positions; it also risks false no-op changes. | Define a deterministic merge with preservation of hidden IDs and unrelated relative order, and prove interleaved no-op behavior. | Closed: deterministic varied merge sequences and real DB CAS/no-op tests preserve hidden/unrelated relative order |
| A bounded request can exceed the account-wide storage bound after a preserving merge. | Enforce the 200-ID resulting-list limit without disclosing or deleting hidden IDs. | Closed: real capacity/hidden-release fixture and browser recovery checks enforce the resulting 200-ID bound |
| UI active counts cannot establish caller discovery or provider executability. | Say enabled/default discovery order; expose execution metadata separately. | Closed: honest enabled/readiness wording and caller-specific rank/executable tests pass |
| Singleton/rank and active-only edit rules contradict scoped callers and disabled-heavy groups. | Immutable custom singleton has no rank; scoped catalog member may rank first; allow editing the complete disabled-inclusive group. | Closed: dense singleton/custom-exclusion fixtures and real full30/26 editor checks pass |
| Dirty guards can run after a filter has already hidden the editor. | Guard before mutation or keep the edited group mounted; verify navigation, refetch and identity races. | Closed: real filters/router/tabs/History and identity-race checks retain or safely close the draft |
| A generic implicit finder change can affect approval reads and omit execution scopes. | If runtime ordering is requested, use an explicit execution entry point/context, preserving owner tiers and exact-authority fences. | Conditional Part B contract specified; unchanged runtime verified for the authorized Part A scope |

The PM also required explicit capacity recovery: normal scoped saves never
prune hidden or stale IDs; a confirmed first-party hidden-release operation
removes only IDs outside the current authorized inventory. Shared GET responses
expose no hidden counts. Accessible custom and disabled entries remain intact.
This is included in Part A acceptance criterion AC-31.

## Revised scope: direct implementation review

The PM reviews source directly while Sol implements; these findings require
correction and fresh regression evidence before the revised PR can be ready.

| PM finding | Required correction | Status |
|---|---|---|
| Abandoned asynchronous recovery can leave the new identity permanently busy. | Fence and reset operation state on identity changes; prove the new identity can navigate and edit after the abandoned read. | Closed: deferred recovery hook and real account-switch browser tests; new identity state resets |
| Draft member enrichment neither appends new rows nor removes deleted or inaccessible rows after a successful inventory refetch. | Reconcile successful inventory snapshots without resetting local order, version or dirty state; preserve failed-read drafts. | Closed: successful-refetch add/remove hook and browser checks preserve draft/version; failed reads retain rows |
| Existing early inventory-error returns unmount the inline card and its recovery controls. | Keep the cached edited card visible with Retry and blocked saves on card and overview surfaces. | Closed: real card/overview read-error recovery scenarios keep editor mounted and saves blocked |
| Filter wrappers discard drafts instead of keeping the edited group mounted under the final plan. | Keep the full edited group expanded and mounted when filters hide it, while filters continue to affect other cards. | Closed: real filtered-group browser checks retain the full edited group without view writes |
| A tab handler and the navigation blocker can confirm the same dirty draft twice. | Use one confirmation per action with cancellation preserving both draft and URL. | Closed: real tab/router/overview History tests confirm once and preserve draft/URL on cancellation |
| Loading or failed preference reads claim no order is saved. | Preserve known key metadata; otherwise show a loading or unavailable state. | Closed: loading/error hook assertions and production-404 disclosure browser checks preserve known pills |
| A 201-row group exceeds the client schema bound and can make Save silently reject. | Expose an actionable limit or supported bounded selection, and verify the boundary. | Closed: 201-row local limit, reset and retry/overwrite boundary tests show actionable validation |
| The full editing table derives pool metadata from filtered rows. | Keep pool associations and row identity readable for every row shown while ordering. | Closed: real filtered full-group metadata test retains explicit pool Priority and provenance |
| TanStack can replace pending mutation options after account switching, and a late invalidation can refetch an old actor's cache using new authentication. | Bind actors in mutation variables and read query keys, prevent old-actor refetches, and verify deferred save/release plus late success across identity switching. | Closed: deferred MutationCache actor tests and real transport tests prevent cross-actor writes/refetches |
| CLI save output combines the new group order with discovery ranks fetched before the save. | Project ranks from the returned order or refresh metadata; verify reversing two ranked rows in an actual subprocess. | Closed: personally reviewed returned-order projection; all four actual CLI subprocess tests pass, including reversed ranks and disabled positions |
| Retry and conflict-overwrite bypass form validation and can throw after successful inventory growth exceeds 200 IDs. | Validate every save path safely, preserve the draft, and render the same actionable local limit message. | Closed: safeParse guards all save paths; growth/Retry/Overwrite tests preserve the draft and display limit |
| CLI rank projection includes active non-HTTP connections that REST discovery excludes. | Derive CLI discovery ranks over active HTTP rows and keep disabled saved positions separate. | Closed: personally reviewed active-HTTP projection; final CLI integration includes non-HTTP rows and verifies they do not consume discovery ranks |
| The newly added inventory recovery can retry an old actor's key query under a new account's authentication. | Bind the list query to its actor and reject mismatched retries before transport; prove late A failure cannot fetch B's inventory into A's cache. | Closed: actor-keyed query retries and real DEV transport race tests reject mismatched authority |
| Initial browser assertions depend on a transient pickup announcement and finish before normal read retries settle. | Verify durable drag activation and the single live region; await bounded read failure before asserting recovery, with real sensors and retries retained. | Closed: independent final browser suite passes 21/21 with real sensors and settled read failures |
| Static merge cases and partial execution checks do not establish every final acceptance criterion. | Add meaningful varied merge sequences, later CAS loser recovery, hidden-release audit/isolation assertions, and paired slug-proxy and implicit-gateway execution evidence. | Closed: canonical final backend run passes 14/14, including varied merge, CAS retry, hidden audit/isolation and paired executions |
| The overview row's History callback bypasses the tab guard and hides a dirty editor. | Guard before changing either the selected history row or tab; prove cancellation preserves the draft and acceptance confirms once. | Closed: real overview History cancellation/acceptance scenarios pass one confirmation and draft retention |
| Requiring a dashboard identity for every inventory query disables Services selection in the authenticated standalone CLI wizard. | Preserve the wizard's explicit local authority without weakening actor fences; verify the real inventory hook and Mode A transport with an empty dashboard auth store, then regenerate the bundle. | Closed: real Mode A shim/empty AuthStore transport and built wizard browser tests pass; regenerated bundle freshness test passes |
| The explanation prints HTTP tool prefixes for enabled non-HTTP connections even though they receive no discovery rank. | Present actual HTTP discovery alternatives and accurately describe unsupported protocol rows; verify catalog SSH or mixed-protocol behavior without changing execution. | Closed: HTTP/SSH renderer regression and collapsed protocol explanation pass; source states actual protocols |
| A successful refetch removing the entire accessible inventory returns the empty state and unmounts the active editor. | Retain the edited group and Cancel action until the draft closes; verify an empty inventory, not only an empty group with unrelated rows remaining. | Closed: real entire-inventory-empty browser scenario retains editor and Cancel |
| The search tool description and an earlier architecture paragraph still describe global preference sorting and a Reorder editor. | Align agent-facing descriptions and architecture documentation with equal-relevance same-group slot permutation and the inline full-group connection table. | Closed: personally inspected agent-facing search description and architecture/API/discovery/chat docs |
| In DEV, the real API client awaits its mock module between the hook's identity check and fetch, so the production-backed preview has an unfenced transport gap. | Bind the opted-in preference and inventory requests at the actual fetch boundary and after module loading; prove old-actor requests never reach transport under the replacement authority, and late old 401s do not clear the new account. | Closed: real api-client tests cover asynchronous DEV import, actual fetch boundary, replacement authority, late401 and lateJSON |
| Frontend and backend canonical catalog-group validation disagree on special and unknown-version UUIDs. | Align the frontend, backend and CLI contract while preserving existing v4/v5 catalog groups; verify nil, max and unknown-version boundaries. | Closed: personally inspected matching RFC UUID1–8 validators; final backend/frontend and CLI canonical boundary tests pass |
| The user's production-preview screenshot shows a repeated wall of explanations above the connections and an unavailable action that is hard to understand. | Keep a compact summary and discoverable Agent order action beside the connection table; collapse the detailed explanation behind How selection works, with a concise truthful server state and no simulated production saves. Verify card, overview, dirty-draft and production-404 disclosure behavior. | Closed: independent card/overview 404 and dirty-draft disclosure scenarios pass compact/collapsed flow |
| The independent compact mobile screenshots show normal row labels reduced to a single letter by readiness badges. | Reserve a useful label width and wrap readiness when necessary in normal and editing rows, with pills below the label and no overlaps; measure normal production-404 and editing layouts at mobile, tablet and desktop widths. | Closed: independently inspected final normal/editing mobile screenshots; actual label spans >=64px and no overlaps at390/1024/1440 |
| The user requests the existing Hide connections chevron and interprets the Agent order availability message as Service Pools being unavailable. | Reuse the same chevron geometry/rotation for How selection works and make the backend requirement specific to saving agent order, preserving readable counts, honest unknown state and existing pool availability. | Closed: same lucide ChevronRight geometry/rotation, targeted saving-backend message; independent final browser and visual checks pass |
| Final Clippy detects a production helper used only by regression tests. | Make the helper private and compile it only in tests, preserving the production discovery path without suppressions. | Closed: ROOT inspected both references and the cfg(test) correction; all-target Clippy compiles production/test variants and exits zero with no warnings |

The revised tests use the isolated MongoDB 8.0.17 replica set at
`127.0.0.1:27029`, replica-set name `nyxidPreferenceReview`. Earlier evidence
below remains historical evidence for unchanged boundaries; every revised
acceptance criterion requires fresh evidence.

## Revised scope: independent final validation and sample preview

ROOT ran the final real-route Playwright suite on port 4644 with two workers and
no retries: 21/21 scenarios passed in 55.6 seconds, including actual gestures,
inline recovery/guards, readable identities, compact disclosure and the built
standalone Mode A wizard. ROOT inspected the final desktop and mobile screenshots;
normal and editing labels remain readable and the chevron matches Hide connections.
Log: `/tmp/nyxid-service-preference-group-pm-final-browser.log`.

ROOT's final `npm run build` exited zero, including TypeScript, production app,
legal prerender, credential acceptance and mock-footprint checks.
Log: `/tmp/nyxid-service-preference-group-pm-final-build.log`.
The final full frontend run passed 466 files / 4,773 tests in 190.34 seconds;
ROOT inspected `/tmp/nyxid-service-preference-inline-final-full.log`.
The final backend UUID recheck executed all 14 feature tests against the isolated
MongoDB replica set, with zero failures or ignored tests (3.18 seconds), including
CAS races, scoped merge, privacy/audit and exact execution invariance.
Log: `/tmp/nyxid-service-preference-inline-backend-uuid.log`.
All five neighboring `search_all_tools` tests passed. The assistant agent-creation search
regression and neighboring curation route-confinement test also passed.
Three CLI unit tests and four actual CLI subprocess tests passed; the latter
exercise grouped reads/saves/reset/release, current output ranks, HTTP-only
discovery projection, confirmation,409 and capacity400 exits. The regenerated
wizard freshness test passed. ROOT inspected the final logs:
`/tmp/nyxid-service-preference-inline-final-cli-unit.log`,
`/tmp/nyxid-service-preference-inline-final-cli-integration.log`, and
`/tmp/nyxid-service-preference-inline-final-wizard-freshness.log`.
The all-target backend/CLI Clippy recheck exited zero after the test-only helper
correction (5m15s, no warnings), as recorded in
`/tmp/nyxid-service-preference-inline-final-clippy-recheck.log`.
Final lint has zero errors and the same 29 unrelated baseline warnings; none are
introduced by the feature.

The user explicitly authorized a seeded mock preview after reviewing the
production-backend limitation. ROOT created a separate temporary Vite/API fixture
at `http://127.0.0.1:4631/keys`, clearly marked LOCAL SAMPLE DATA. It serves
30 Anthropic connections (4 enabled / 26 disabled), 34 sample agent-key grants,
four Google Workspace connections and a separate explicit priority pool.
Sample preferences persist to a local JSON file and never reach production.
The production-backed server at port 4630 and its OAuth bridge remain intact.

ROOT drove the actual sample server with Playwright, without API interception:
mouse drag, keyboard reorder, numbered pills, Save, reload persistence, Cancel,
service-scoped reset, overview and mobile checks passed. The restored seed is
available via the banner. No production API or service execution requests were
made; only the local origin and the app's existing Google Fonts assets appeared.
Fixture, instructions, screenshots and evidence live outside the repository in
`/tmp/nyxid-service-preference-mock-preview/`; its `smoke.log` records the pass.

The following records the completed review of the previous scope:

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

## Within-service revision requested by the user: current delivery status

The user's Anthropic card has 30 connections, 26 disabled connections and
34 agent keys with access. Agent key count is not a connection count. The
requested interaction belongs in that service's existing card and connection
table, with clear per-connection order and an explanation of how an agent
selects a tool. A separate global list is insufficient.

The PM disabled the temporary local ordering emulation after the user rejected
that review flow. Port 4630 continues to use the production metadata/OAuth
bridge, whose production backend does not yet implement preference writes.
No emulated order is delivered to production. The actual branch implementation
must remain honest about unavailable saves on an older backend.

Revision gates:

| Finding | Required correction | Status |
| --- | --- | --- |
| Reorder replaces all service cards with one global list. | Put handles, local order pills and Save/Cancel inside the existing service card's connection table; keep its metadata and surrounding groups visible. | Implemented and personally verified in the card and overview browser scenarios |
| Full replacement cannot safely preserve preferences omitted from a scoped read. | Server-authoritative group writes must retain unrelated and currently inaccessible stored entries, with CAS and no-op behavior. | Implemented; real database merge/CAS/no-op/privacy tests pass |
| Discovery ranks and sorting currently cross service groups. | Derive dense ranks within each immutable service group and preserve relevance and unrelated group ordering with a transitive deterministic algorithm. | Implemented; same-group slot, relevance and truncation regressions pass |
| The group does not explain how discovery, named execution and pools select connections. | Display a concise explanation in the actual group, including agent-key scope, disabled/unavailable state, explicit connection slugs and pool routing. | Implemented and verified; final copy nit tracked in the fresh review below |
| The implicit provider gateway chooses an active connection without saved priority. | Document the current selector and separately decide whether the user's order must govern that route. The PM asked the user for this runtime-scope choice. | Conditional Part B; outside this delivery until the user chooses that runtime scope |
| Production 404 makes all preference controls disappear in the local frontend. | Preserve visible explanatory group UI and an honest unavailable-save state on older backends; do not simulate successful production persistence. | Implemented and verified against older-backend 404; separate local sample preview authorized and running |
| Earlier source sign-off covers the rejected global editor. | Repeat PM review, relevant validation and Opus sign-off on the final revised SHA and plan. | PM reviewed revised implementation; fresh CI and exact-head Opus approval remain required |

Only the original Fable, Sol and Opus heca sessions are reused. Fable completed
the revised plan and Sol implemented Part A after PM plan review. The PM
prepared a dedicated MongoDB 8.0.17 replica set on loopback port 27029 and
personally reviewed the implementation and execution evidence; the fixture
contains no production data or credentials.

## Fresh published-head review and coverage correction

Opus 5.5 returned REQUEST_CHANGES on
`0ea6cfa3e41eea4cbc03d65e9c4e8d24da5d3fbb`. It confirmed scoped storage,
CAS/privacy, caller filtering, discovery slot permutation, execution invariance,
transport authority fences and inline UI placement. The following additional
findings required correction and fresh validation. ROOT reviews each
correction directly; Sol handles source and plan changes without new agents.

| Finding | Required correction | Status |
|---|---|---|
| Backend capacity failure appears twice, and the browser fixture uses different server text. | Render one capacity message and exercise the actual backend text with an exact single-occurrence browser assertion. | Closed locally: ROOT inspected the separated capacity message state, exact backend fixture and single-occurrence browser proof |
| An unrelated sticky-header outline was edited after the recorded local gates. | Remove the unrelated edit and run final frontend/browser gates on frozen source. | Closed: outline/comment removed; ROOT inspected fresh browser, unit, build and lint evidence |
| Agent-key count explanation is unclear. | Explain that the card counts agent keys with access rather than service connections. | Closed: ROOT reviewed the specific card-count wording |
| Retry save visibility depends on message prose. | Drive the action from explicit recovery state and verify successful release/retry behavior. | Closed locally: draft-token-bound readyToRetry state controls the action; release/confirmation/retry browser proof passes |
| CLI help still describes cross-service preferences. | Describe discovery order within one service's connections. | Closed: ROOT inspected the corrected help and completed fresh compilation, unit/subprocess and all-target Clippy evidence |
| Plan disagrees with actual action location, guards, disabled pills, summary states and changed-file map. | Align every relevant section and acceptance criterion with the final behavior and complete the file map. | Closed: ROOT reviewed the final plan, API/discovery/architecture docs, complete CLI/frontend file map and per-criterion evidence |
| This review record's closing revision section still describes unimplemented work. | Update implemented Part A gates and distinguish conditional Part B from delivery. | Closed: ROOT updated the section and current status; final CI/Opus remain explicit gates |
| Frontend coverage times out in the 201-row validation/reset test. | Remove unnecessary test work while retaining all 201 IDs, visible local validation, no PUT, and confirmed reset; reproduce with coverage. | Closed locally: full V8 coverage passes 466 files / 4,773 tests with the existing 15% threshold; no timeout or coverage relaxations |
| Local 201-row invalid submission also duplicates the limit paragraph. | Render the validation message once while retaining the real form error and actionable reset. | Closed: one form error; native sticky-submit hook and real 201-row browser checks retain every ID, reject PUT and confirm reset |
| The user finds Agent order hard to recognize and wants Save/Cancel in the sticky section. | Use a clear idle CTA, then one Save/Cancel action set in the sticky action bar while editing; preserve validation, recovery and focus, and prove scrolled desktop/mobile usability. | Closed locally under AC-32: ROOT inspected source, six card/overview width scenarios and mobile/desktop screenshots; deliberate gap spacing replaces justify-between |
| The first sticky-action candidate wraps unrelated row panels in the order form, making their existing untyped buttons submit the draft. | Keep the table and its panels outside the dedicated order form; native sticky Save targets that form explicitly. Verify panel interaction leaves the draft unsaved. | Closed: dedicated form excludes the table; actual Show all/fewer keys browser interactions retain the dirty order with zero PUTs |
| The first error-scroll calculation looks for an explicit region role that neither surface provides and can miss sticky-header occlusion or a repeated identical failure. | Measure the actual action bar's bottom bound and reveal each failed attempt; verify first and repeated scrolled failures remain below the sticky cover and inside the scrollport. | Closed locally: associated native submit control measures the action bar; submit count repeats error reveal; browser geometry/hit-testing verifies first and repeated failures |
| CodeQL reports two new high-severity cleartext-transmission alerts on the added CLI JSON DELETE helper, including its refresh retry. | Establish an actual secure transport boundary for credential-bearing requests, retain safe local fixtures, and prove remote cleartext and downgrade redirects cannot transmit credentials. No alert dismissal or scanner suppression. | Corrected locally: ROOT reviewed URL validation, verified HTTPS/no-redirect client binding, protected preliminary reads/refresh/retry, retained profile fences and four passing real subprocess security cases; fresh exact-head aggregate CodeQL remains required |
| Scrolled overview screenshots show passing row text in the main top gutter above the new sticky bar. | Extend the sticky surface background over the existing mobile/desktop gutter without obscuring normal metadata; inspect fresh screenshots and recheck geometry/error visibility. | Closed: ROOT inspected corrected 390/1440 screenshots; all three width browser checks, seven overview unit tests and final build/lint pass |

CI run: `https://github.com/ChronoAIProject/NyxID/actions/runs/37571777616`.
The frontend coverage job ran 4,772 passing tests and one timeout, with 465 files
passing and one failing. Its failure is at
`src/hooks/use-service-group-order.test.tsx:225` (5,000 ms), rather than the
coverage threshold. The normal Frontend, Rust Clippy, CLI Test, wizard freshness,
billing smoke and feature-combination jobs passed on this head. Full new-head
CI and a fresh Opus review are required after correction; these partial passes
do not close either gate.

The CodeQL workflow completed successfully, but its separate aggregate security
check `112631937301` failed on new alerts
`https://github.com/ChronoAIProject/NyxID/security/code-scanning/465` and
`https://github.com/ChronoAIProject/NyxID/security/code-scanning/466` at
`cli/src/api.rs:566` and `:576`. A successful scanner execution does not establish
a clean security verdict. Both findings require implementation and fresh exact-head
CodeQL evidence before sign-off.

### Final local correction review

ROOT inspected the corrected sticky controls on cards and the service overview,
including the mobile/desktop screenshots and all six width scenarios. Agent
order is a primary CTA; editing shows a readable label, outline Cancel and
primary Save in one group with explicit gaps. The existing chevron remains.
The dedicated native form excludes unrelated row panels; its associated submit
button supplies the actual sticky action-bar bound used to reveal errors.
Repeated identical failed submissions reveal the error again. ROOT also found
and returned the overview gutter issue, then inspected its corrected screenshots
and three passing browser rechecks.

The CLI release command constructs one existing read-only authenticated client,
then validates and binds transport to that client's actual destination before
any request. Remote transport requires verified HTTPS. Local HTTP is restricted
to exact loopback hosts, bypasses proxies and pins localhost to loopback. All
release reads, refreshes and DELETE attempts refuse redirects. Explicit keys
never refresh; saved-profile destination and login-generation fences remain.
ROOT found and returned a first candidate's duplicate destination resolution;
the final constructor resolves once. The DELETE helper independently applies
the same transport policy to its initial and refresh-retry request.

ROOT reviewed the final source, documentation and completed logs below. Plan
§17 contains the commands and artifact paths. Existing backend/database and
execution-boundary evidence remains applicable because these corrections do
not change backend source or discovery/execution semantics.

| Local validation reviewed by ROOT | Result | Evidence |
|---|---|---|
| Full frontend and full V8 coverage | 466 files / 4,773 tests passed in each; 73.16% lines with the existing 15% coverage threshold | `/tmp/nyxid-service-preference-sticky-full-frontend.log`, `/tmp/nyxid-service-preference-sticky-full-coverage.log` |
| Complete feature and regenerated wizard browser execution | 29 passed, no retries; final gutter-only change adds three passing overview browser and seven passing unit rechecks | `/tmp/nyxid-service-preference-sticky-frozen-browser.log`, `/tmp/nyxid-service-preference-sticky-gutter-browser.log`, `/tmp/nyxid-service-preference-sticky-gutter-unit.log` |
| Final production build and lint | Build passes including mock-footprint assertion; lint has zero errors or feature warnings, with 29 unrelated existing warnings | `/tmp/nyxid-service-preference-sticky-gutter-build.log`, `/tmp/nyxid-service-preference-sticky-gutter-lint.log` |
| Fresh CLI compilation, formatting and all-target Clippy | All pass; compiler-artifact paths identify the freshly rebuilt executables | `/tmp/nyxid-service-preference-sticky-cli-compile-recheck.log`, `/tmp/nyxid-service-preference-sticky-cli-artifacts.json`, `/tmp/nyxid-service-preference-sticky-final-fmt.log`, `/tmp/nyxid-service-preference-sticky-final-cli-clippy.log` |
| CLI preference/API/TLS unit tests | 3 preference, 5 API and 17 TLS tests passed; zero ignored | `/tmp/nyxid-service-preference-sticky-cli-unit.log`, `/tmp/nyxid-service-preference-sticky-cli-api.log`, `/tmp/nyxid-service-preference-sticky-cli-tls.log` |
| Real CLI transport, preference and network/profile subprocess tests | 4 transport, 4 preference and 5 network tests passed; verified CA/refresh and zero requests at downgrade targets | `/tmp/nyxid-service-preference-sticky-cli-transport.log`, `/tmp/nyxid-service-preference-sticky-cli-integration.log`, `/tmp/nyxid-service-preference-sticky-cli-network.log` |
| Regenerated wizard freshness | Fresh rebuilt-source check passes; all 171 manifest inputs and recorded hash match | `/tmp/nyxid-service-preference-sticky-cli-freshness.log`, `/tmp/nyxid-service-preference-sticky-gutter-closure.json` |

All substantiated local implementation findings are corrected. Fresh published
head CI, its separate aggregate CodeQL verdict and Opus's plan/PR sign-off remain
required. Their final results belong in the PR evidence so that adding evidence
does not change an approved source head. No merge or deployment is performed.

### Renewed Opus review and CI boundary-test correction

Opus reviewed published `00eef8d7bfb0c44e8bad4ae85659f1abc257ffd7` and confirmed
all seven prior corrections plus the sticky layout, external form isolation,
explicit recovery state and CLI credential transport. It returned three nits.
ROOT accepted all three, including the optional wording correction, and sent
them to Sol. ROOT then inspected their minimal source/document diff and the
completed 45-test unit, production build/type, lint and wizard-closure evidence.

| Renewed finding | Closure |
|---|---|
| The panel retains an unused action prop and render slot after controls moved to the sticky bar. | Removed the prop, unused ReactNode import and slot; both callers already omitted it, so DOM geometry and existing browser evidence are preserved. |
| The plan's exact-chevron requirement is a sentence fragment. | Rewritten as a complete requirement preserving the existing Hide connections chevron. |
| The latest evidence section implies an unpublished worktree and calls the prior reviewed head the published head. | Removed the worktree-only heading; explicitly distinguishes prior `0ea6cfa3` review from corrections published in `00eef8d7`, with final binding in the PR body. |

Evidence: `/tmp/nyxid-service-preference-opus-nits-unit.log` (3 files / 45 tests),
`/tmp/nyxid-service-preference-opus-nits-build.log`,
`/tmp/nyxid-service-preference-opus-nits-lint.log` (zero errors/feature warnings),
and `/tmp/nyxid-service-preference-opus-nits-wizard-closure.json` (171 inputs,
unchanged recorded hash).

CI on `00eef8d7` passed normal Frontend, CLI Test, workspace Clippy and wizard
freshness, but Coverage (Frontend) again timed out in the 201-row hook test at
the unchanged five-second limit (4,772 passing / one failing). ROOT downloaded
the completed job's exact log from the API even while the workflow remained
running: `/tmp/nyxid-service-preference-00e-ci-frontend-coverage.log`, job
`112646436406`, workflow `37576468011`. The earlier local pass and first query
optimization did not establish CI stability. ROOT returned this failure to Sol
for bounded hook/form testing without 201 unrelated heavy row subtrees, retaining
every ID, validation/no-write/reset assertions and the full-table 201-row browser
proof. Fresh focused and full coverage execution and final-head CI are required;
no timeout, threshold, exclusion or retry policy is relaxed.

During concurrent builds, ROOT recovered from a transient shared-disk ENOSPC by
removing only this task's completed 1.6 GB Rust incremental cache. No active
Cargo process used that target; the latest CLI checks already disabled
incremental compilation. Source, logs, compiler-artifact executables and both
previews were preserved. Completed nit/CLI logs contain no disk failure.

The corrected transport was also scanned on published `00eef8d7` (merge ref
`080a279e`). All four jobs in CodeQL workflow `37576467932` succeeded, and all
four merge-ref analyses have zero results and empty warning/error fields. Alerts
465 and 466 are fixed, not dismissed, and the PR merge ref has zero open alerts.
The separate aggregate `112646549519` is **neutral**, with no new alerts, solely
because main retains the obsolete pre-matrix category
`.github/workflows/codeql.yml:codeql`. ROOT inspected main's analysis history:
the last legacy upload is `0c314264` on September 21; subsequent September 28 and
October 5 uploads use all four language categories. Workflow history confirms
the matrix migration in rollup `567dd3ed` (#1669 via #1670). Opus independently
verified the same inherited neutral warning on other merged and open PRs. The
old feature head still failed for its two new high alerts despite that same
warning, so the inherited category does not mask the feature's new findings.

ROOT accepted Opus's requirement to make AC-33's security pass rule explicit:
successful final-head CI/wizard/scanner jobs, no new aggregate alerts, both
transport alerts fixed without dismissal, and zero results/errors/warnings
across the final-head analyses with no open PR alerts. Neutral is recorded
literally only for this proven obsolete category; any additional missing
configuration or scan error blocks delivery. Final published-head evidence is
still required. No configuration, analysis or alert is deleted or suppressed.

ROOT reviewed the completed coverage correction directly. The 201-ID test uses
the real ordering hook, form/schema validation and shared sticky actions with
a lightweight native form fixture. Every ID, the exact permutation, visible
validation, zero invalid PUTs and confirmed reset body/version are retained.
A separate three-row case exercises the actual table, row movement, native
external Save association, pristine/dirty gating and table exclusion. The full
201-row browser test is preserved. The obsolete tooltip mock is removed, so
the small renderer cases use their real UI primitives.

Final focused V8 execution passes all ten hook tests; the 201-ID boundary is
92 ms, and the actual three-row table/form case is 208 ms. The complete V8
recheck passes 466 files / 4,774 tests in 215.80 seconds with 73.17% lines,
the existing 15% threshold and default five-second test timeout. Final build,
type checking and lint pass, and the 171-source wizard closure is unchanged.
Evidence: `/tmp/nyxid-service-preference-201-hook-focused-recheck.log`,
`/tmp/nyxid-service-preference-201-hook-full-recheck.log`,
`/tmp/nyxid-service-preference-201-hook-build-recheck.log`,
`/tmp/nyxid-service-preference-201-hook-lint-recheck.log`, and
`/tmp/nyxid-service-preference-201-hook-wizard-closure.json`.

The first local full-coverage attempt failed with ENOSPC and is retained as
failed evidence in `/tmp/nyxid-service-preference-201-hook-full-coverage.log`;
the passing recheck uses a separate log/report path after space recovery. A
strict indexed-access type error in the fixture was corrected before the final
build and coverage recheck. Neither failed attempt is counted as a pass.
All three renewed nits and the second CI rendering timeout are corrected
locally. ROOT reviewed the explicit AC-33 security rule and provenance;
fresh final-head CI and the literal aggregate/analysis/alert evidence plus
Opus sign-off remain required before marking PR #1796 ready.

### Interim discovery summary placement revision

CI workflow `37578725903` completed successfully on published `a60afb9c`,
including backend tests and both backend coverage jobs, normal Frontend,
Coverage (Frontend), CLI tests and coverage, wizard freshness and CI Pipeline.
ROOT inspected the completed checks; Opus independently confirmed the same
result. Together with CodeQL workflow `37578725940`, the four zero-result
merge analyses and literal neutral aggregate `112653487531`, this closes the
previous coverage/security validation on that head. These are historical
results because the user then asked to move Agent order beside the discovery
summary. They do not substitute for checks on the revised publication.

ROOT personally reviewed the new shared summary/disclosure split and both
call sites. The compact summary and one action group sit below Hide connections
on cards and below the overview tabs. Explicit gaps keep controls next to the
summary; narrow layouts wrap them directly below it. The shared summary owns
the existing loading/read-error/404 feedback and Retry. The full explanation
remains in normal flow, so opening it cannot cover the connection table with
a tall sticky header. Both surfaces retain their CTA refs and focus effects.
The native external Save still targets the dedicated order form, and its
nearest `data-service-order-actions` ancestor now covers the complete sticky
surface used by the existing repeated-error reveal calculation. The card's
ResizeObserver measures that same complete header. The original Hide
connections chevron and row interactions are unchanged.

ROOT inspected actual idle desktop-card and mobile-overview screenshots and
scrolled mobile-card and desktop-overview editing screenshots from
`/tmp/nyxid-service-preference-discovery-row-final-browser-results/`.
They show the CTA or Agent order/Cancel/Save beside the discovery summary,
natural wrapping with aligned left edges, readable controls and a visible
table beneath the sticky cover. The real-route width cases also check physical
proximity, open explanation scrolling, native form association, raw mouse Save,
keyboard Cancel, focus return and repeated failed-save visibility. The initial
browser attempt's missing working directory and the initial unit harness's
obsolete panel usage are recorded as failed attempts, not passing evidence;
their corrected executions use separate recheck logs.

Opus's additional wording nit on `a60afb9c` was accepted: the plan must refer to
the published nit corrections by their commit, rather than calling them an
unpublished follow-up. The updated plan and architecture contract reflect the
new discovery-row placement. Fresh frozen-source validation, exact published
head checks and final Opus plan/implementation/delivery approval remain the
publication gates. Their completed evidence will be recorded in PR metadata
without a source-only evidence commit after approval.

Opus's early review independently confirmed the revised implementation and
returned two plan corrections: the old body placement in §5.2 and the error
reveal's reference to an action bar rather than the complete sticky section.
ROOT accepted both. Opus also observed the existing expanded mobile card
header's height; ROOT verified that the requested placement still leaves
visible connection rows, operable actions and unobscured repeated errors at
390px. The open explanation scrolls away. No unrelated header compaction is
needed to satisfy this change. An initial screenshot caught a view-transition
snapshot; ROOT inspected the final-run replacement screenshots, which are clear.

### Compact settings row and on-demand help

The user subsequently rejected the dense explanation, the separate summary
and help rows, and the combined inline disclosure. Those intermediate layouts
and their passing checks above are historical. ROOT directed a compact shared
settings row: preferred connection and counts, one Agent order or editing
action group, and a quiet How it works popover trigger. The long inline
explanation is removed from the connection table. The new help contains three
short rules, the existing Service Pools link and brief conditional protocol
and gateway notes. Technical discovery, rank, credential and execution
contracts remain in the architecture/API documentation and unchanged code.

ROOT personally reviewed the consolidated `ServiceAgentOrderPanel`, both
sticky call sites and the updated tests. One semantic section now owns the
summary, controls, status/Retry and help. The popover uses the existing shared
Radix primitive, a labelled heading, viewport width and available-height bounds,
and normal Escape/outside-click dismissal. Its portal contributes no height to
the sticky section. The original Hide connections chevron is retained; the
help trigger uses the same ChevronRight geometry and open rotation. The prior
join-specific borders/shadows and separate body disclosure are removed.
Dedicated native order forms, external Save, complete-cover error measurement,
focus restoration and shrinking-inventory Cancel remain intact.

ROOT inspected dark-mode idle desktop-card, open desktop-card help, open mobile
editing help and scrolled mobile-overview screenshots in
`/tmp/nyxid-service-preference-settings-row-focused-browser-results/`. The
default view presents one compact settings row directly above the table,
with no permanent help row or explanation wall. Mobile editing keeps Agent
order, Cancel, Save and How it works together. The bounded help is readable
and stays inside the viewport; closing it restores the table view.
Final frozen-source logs, exact-head CI/security evidence and Opus approval
remain required before publication is marked ready.

### Info tooltip and explicit discovery CTA

The user's next screenshot review supersedes the popover requirement above:
the primary entry is **Reorder discovery**, the editing context is **Discovery
order**, and help uses the usual Info icon and Tooltip. Hide connections keeps
the existing ghost button's hover appearance while expanded, scoped to this
page's card call site. The exact connection chevron and shared Button remain
unchanged. The former popover evidence is historical and cannot close the
latest UI criteria. Noninteractive tooltip copy replaces the popup-only pool
link; existing Service Pools navigation and routing remain available through
their original controls.

ROOT fetched and integrated main `f0d06a72` (plugin/MCP update #1789) in merge
`631b4994`, preserving the uncommitted UI changes and user attachments. Personal
review of the auto-merged MCP source confirms search and list-connected still
use the preferred loader, which retains the original loader vector, scoped
guest filtering and native rows with null rank. New tool titles and bearer
session restrictions retain their main implementation. Fresh merged-source
backend feature execution and final UI/build/wizard checks are in progress;
publication, exact-head security evidence and Opus approval remain pending.

After the daemon restart, ROOT checked the disconnected implementation worker
and sent a recovery prompt. The provider restarted successfully and the worker
confirmed it could retain implementation and local-validation ownership. The
user's requested Opus 5.5 fallback was therefore unnecessary. Missing temporary
validation resources were recreated; old log references remain historical.

Opus reviewed the merged working tree independently and confirmed the compact
settings row, CTA names, status live regions, tooltip copy and scoped Hide
connections styling. ROOT accepted its tooltip interaction finding: mouse
clicks after hover and keyboard activation after focus must keep help open;
only touch taps toggle it. The worker implemented that distinction and added
real-route hover/click and focus/Enter assertions beside the touch coverage.
ROOT personally inspected the fix, assertions and documentation. Keyboard
focus wording is explicit. Existing setting/status/form strings deliberately
retain "agent order" while the entry and editing labels use discovery wording.

Fresh focused validation passes 100 tests across ten files. Production build,
lint (zero errors, 29 unrelated warnings), formatting and regenerated wizard
build pass. These are local results, not published-head approval. The earlier
30-scenario browser pass predates the modality fix and remains historical.
The first full coverage attempt hit an unrelated usage-router timeout; that
failure is preserved and does not count as passing coverage. Complete frozen
browser/coverage, merged backend/CLI execution, wizard freshness, published-head
CI/security evidence and renewed Opus delivery approval remain pending.

ROOT independently verified the completed recovery evidence in §18: complete
Node 22 unit and V8 runs pass all 4,777 tests across 467 files, with 73.17%
line coverage and unchanged thresholds/timeouts. All 30 frozen browser cases
pass, including the corrected desktop and touch tooltip interactions. ROOT
inspected the final 390px idle card and 1440px scrolled overview editing/help
screenshots; controls, rows and bounded help remain readable. Node 22 production
and wizard builds and lint pass; the wizard bytes match the browser-tested
bundle. Backend preference tests pass 14/14, neighboring filters pass 5+1+1,
and CLI unit/subprocess/transport/freshness checks pass 3+4+4+1, all with zero
ignored tests. Formatting and backend/CLI all-target Clippy with `-D warnings`
pass. The earlier failed coverage run and interrupted timestamp-only rebuild
remain separately recorded; neither is represented as passing evidence.

The worker's final source binding matches the reviewed tracked changes; only
this ROOT-owned evidence addition follows it. ROOT closes the accepted source
findings and local validation gates. Publication, CI/security on that exact
published head, and Opus delivery approval remain pending. Attachments remain
outside the commit.

### Main service-card motion merge and final local review

GitHub retained stale base and test-merge metadata after publication of
`19fffb3a`. Reapplying the existing `main` base exposed current main `b90f07ac`,
including service-card motion/pool visuals, reporting identities and billing
visualizations. ROOT started the merge and delegated conflict resolution to
the recovered implementation worker. Generated wizard conflicts were resolved
by rebuilding, not selecting either side.

ROOT and Opus reviewed the combined source. Main's motion sequence and pool
rows remain. Each connection's main, pool and animated details rows form one
sortable tbody; the keyboard getter aligns complete group centers. Dirty
ordering guards run before sequence requests. Editing exclusively expands its
own card, and a before-paint update reconciles ephemeral expansion without an
account-default write. This preserves Save/Cancel focus after view restoration.
The complete sticky header remains the error-reveal measurement boundary.

ROOT inspected the final 390px idle card and 1440px scrolled editing/help card
screenshots under `merge-b90/browser-refreshed-results/`. The discovery controls
and bounded help remain readable. ROOT accepts the resolved source and local
gates documented in §19: 4,789 tests across 468 files pass in full unit and V8
runs, with 73.25% line coverage and unchanged thresholds/timeouts; all 41
Chromium cases and two WebKit grouped-drag cases pass. Chromium native touch
is covered; genuine WebKit touch dragging is not claimed. Production/wizard
builds, the recomputed wizard closure, lint, formatting, backend preference
14 tests and neighboring 5+1+1 tests, CLI 3+4+4+1 tests and all-target Clippy
pass. Backend and CLI tests have zero ignored cases. Failed/interrupted interim
attempts remain separately recorded and are not represented as final passes.

No unresolved merge entries or unstaged tracked changes remain before this
review addition. Attachments remain untracked. ROOT will publish the merge
and bind remote CI, full backend/billing execution, security analysis and
renewed Opus sign-off to its exact commit. Earlier published-head approvals
remain historical and do not grant final delivery approval.
