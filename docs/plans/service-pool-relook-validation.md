# Service pool relook validation

This change addresses the management and UI follow-up to [issue #1680](https://github.com/ChronoAIProject/NyxID/issues/1680), implemented initially in [PR #1717](https://github.com/ChronoAIProject/NyxID/pull/1717).

## Behavior and compatibility

- Candidate inventory no longer requires guessing an operation path. The dashboard and bare CLI candidate command explicitly request `check_operation=false`. REST callers omitting the flag retain their existing operation-check defaults.
- Selected draft connections are inspected independently of inventory search and pagination. Saved operation checks retain disabled and cooling state; draft inventory does not inherit saved cooldowns.
- The editor presents ownership, routing, Same API or AI chat, and connection order before advanced retry settings. It preserves revision conflicts and compatibility confirmations, generates valid slugs, and clears hidden models when switching to Same API.
- The pool menu and dialogs preserve pointer access and keyboard focus after repeated use. Closed menu content disappears immediately, avoiding lost clicks during rapid reopening. Long names and slugs preserve action visibility on desktop and mobile.
- CLI management resolves reserved and UUID-shaped slugs without masking authorization failures. Authorized organization pool UUIDs work without `--org`. Mutation output distinguishes unchecked health from availability.

## Acceptance evidence

The initial local acceptance run used base `f6cd8f6b51526478ba077354112d12502a0a6af8`, MongoDB 8.0.17 with replica-set transactions, real local CLI/server binaries, and local HTTP providers. Browser checks distinguish HTTP fixtures from the live backend smoke. No production provider credentials were used. The PR checks validate the branch after synchronization with `main`.

| Requirement | Result | Concrete evidence |
|---|---|---|
| A=429, B=200; replay preserved; attempts=2; both audits | Passed | `pool_proxy_retries_429_replays_body_and_respects_cooldown`: method/body/headers, response attribution and both status/member/priority/hash-chain audit rows. Primary real CLI proxy/audit check also exercises this. |
| A=400 is terminal | Passed | `pool_proxy_bad_request_does_not_try_backup`; primary real upstream 400 with nonzero CLI exit and no B request. |
| Pre-first-byte fallback; mid-stream failure without switching | Passed | `pool_proxy_pre_first_body_failure_can_retry_with_explicit_replay`, `pool_proxy_mid_stream_failure_never_switches_members`, native AI stream-error cases and response-gate tests. |
| Cooldown/Retry-After and exhaustion | Passed | 429 second-call skip; all-cooling tests; durable health ordering/reset/rotation tests; HTTP and transport exhaustion tests. |
| Legacy weighted/round-robin single attempt | Passed | Both runtime cases; weighted 2:1 produces 429/429/200/429 with upstream counts 3:1 and zero cooldown rows. |
| No access widening or wrong credential materialization | Passed | Restricted service/node/org/platform cases, revoked platform grant, approval/destination drift, viewer/admin-only, machine declared-pool and streamed-upload refusal. |
| AI gateway/adapters/custom destinations | Passed | Slug/gateway parity, OpenAI/Anthropic/Responses conversion, tools, native paths, Codex local destination/node transport, unsupported-feature refusal and streamed errors. |
| Node routing/cancellation | Passed | Live node-capability loss, excluded-node pre-decryption, Codex node transport; supplemental node manager/dispatch and CLI real-provider cancellation tests. |
| Per-attempt billing, funding and recovery | Passed | Six billed proxy cases plus pool admission and durable recovery/coordinator/lease tests; route-coverage smoke. Mongo8 is required for ledger bulk writes. |
| CLI issue sketch and all management commands | Passed | 31 pool tests pass: issue retry aliases, flags/files/defaults/disable, candidates/show/health/reset, member/strategy edits, revisions, owner/identifier handling and node cancellation. Primary real CLI/backend smoke supplements HTTP mocks. |
| Browser creation/editing/health/ownership | Passed | 24 current component/hook tests; 4,084 full frontend tests before the final DOM/overlay follow-ups; independent 16-step rendered-app browser check and separate desktop inspection. Primary real UI-to-backend create/edit/delete also passed. |
| Repeated dialog cancellation, rapid Actions reopening, keyboard/focus | Passed | Primary final 20-entry Chromium run: original flows with immediate disable/enable, 30 desktop and 10 mobile cancellations, keyboard/direct-name focus and zero console/page errors; separate 10 rapid toggles. Implementation additionally checks keyboard navigation/Tab/Escape for all four dialogs. Six permanent component regressions. |


## Reproduce the committed checks

Start an ephemeral MongoDB 8 replica set as described in [Contributing](../../CONTRIBUTING.md#running-tests), then export its URI as `NYXID_TEST_DATABASE_URL`.

```sh
cargo test -p nyxid service_pool -- --test-threads=2
cargo test -p nyxid pool -- --skip service_pool --skip oracle --skip test_utils:: --skip trigger_scheduler:: --test-threads=2
cargo test -p nyxid services::node_dispatch -- --test-threads=2
cargo test -p nyxid billing_route_coverage_smoke -- --nocapture
cargo test -p nyxid-service-adapters
cargo test -p nyxid-cli pool
cargo test -p nyxid-cli --test wizard_bundle_freshness
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
bash scripts/check-rci-backend-boundary.sh
npm --prefix frontend test
npm --prefix frontend run lint
npm --prefix frontend run build
```

Initial local results: 108 service-pool backend tests, 39 supplemental pool tests, 14 node-dispatch tests, one billing-route test, 15 adapter tests, and 31 CLI pool tests passed. The full frontend suite passed 4,084 tests in 406 files before the final DOM and overlay corrections. The 24 focused component and hook tests, TypeScript, and ESLint passed after those corrections. Workspace Clippy, formatting, the backend boundary check, and the production frontend build passed.

## Independent live checks

The live smoke executed all CLI pool subcommands against real local `nyxid` and `nyxid-server` binaries with a disposable database and two HTTP providers. All 28 checks passed. It covered the issue's 429 fallback example and both chained audit records, terminal 400, persistent cooldown and reset, revision conflicts, member files, strategy transitions, partial-member preservation, personal/organization slug collisions, and actual browser create/edit/delete operations.

The final Chromium check passed 20 assertions against the rendered app with HTTP fixtures. It covered creation, slug generation, increasing priorities, explicit operation checks, AI model validation and clearing, single-paste edits, health resets, usage instructions, and deletion. It also executed 30 desktop and 10 mobile cancellation cycles, keyboard focus restoration, and direct-name editing after prior menu use. A separate check passed ten immediate enable/disable toggles. Both captured zero page and console errors.

Layout checks used 128-character names and 80-character slugs at 390, 768, 1024, and 1440 pixels. Actions stayed inside the viewport; health rows retained the Reset control without horizontal overflow.

The permanent component regressions cover nonmodal menu ownership, repeated cancellation, keyboard navigation into all four dialogs, Tab and Escape behavior, reopening, and focus restoration to another pool's name button. Backend regressions assert both attempt audit rows for 429 fallback and weighted legacy routing with a single attempt.

## Unavailable-credential regression

The follow-up to PR #1731 reproduced a candidate-inventory HTTP 400 with
`Bad request: API key is failed` using a real local backend and a disposable
connection whose stored key status was `failed`. A permanent backend regression
failed with the same error before the correction.

Regression coverage now checks six nonactive credential states, active keys with
missing credential material, and unavailable agent overrides across inventory,
explicit operations, selected draft members, and saved health. It checks healthy
backup selection before dispatch, zero inspection decryptions or last-used
writes, preserved node/platform/no-auth behavior, service/node scope filtering,
and propagation of malformed or missing database records. Direct proxy errors
retain their HTTP 400 payload and telemetry classification. Component tests
cover the disabled credential row, healthy selection, the new-pool connection
requirement, and edits to existing empty drafts.

The searchable multi-select dropdown keeps selections visible, supports
select/deselect without closing, preserves member configuration across search,
and restores trigger focus when Escape closes the dropdown inside the editor.
Component tests cover its 50-member limit, unavailable options, loading, empty
results, retry, and pagination.

The full frontend suite passed 4,113 tests in 411 files after the dropdown change;
the 31 focused pool component/hook tests, lint, and production build also passed.
All 113 backend service-pool tests passed after the credential correction.
After adding name search, all ten inspection tests passed, including literal,
case-insensitive matching before pagination, owner/service scope restrictions,
platform labels, and selected-member independence. The 131 proxy-service,
21 error-contract, and 14 proxy-telemetry tests passed. Workspace Clippy,
formatting, and the backend boundary check passed.
The final independent live server/CLI/browser smoke passed 11 checks covering
the exact HTTP 400 contract, empty drafts, candidate inventory and operation
checks, name search before pagination, saved health, and healthy fallback with
one dispatch attempt. The browser searched displayed names, selected and
deselected connections, handled unavailable rows, repeated three Escape/reopen
cycles, checked the 390-pixel layout, and created/edited/deleted a pool against
the backend, with zero page or console errors.

## Verification scope

The local backend runs target service pools and their integration boundaries; they are not a claim that every backend test ran locally. The PR's CI jobs run the full selected backend, CLI, frontend, feature, and coverage suites.

An early local run used unsupported MongoDB 7 and failed during billing bulk writes. That run was discarded and repeated on MongoDB 8. Browser interaction failures found during review were fixed and rerun on stable source. Temporary test identities, signing keys, databases, and servers were cleaned up after the successful runs.
