# Public and personal NyxBot acceptance evidence

The primary maintains this evidence record with Part 5. The converged design is
authoritative; an implementation, a passing test and a live observation are
different evidence states. Record the tested commit and actual commands/results
before changing any row to verified. Mocked upstream calls cannot satisfy A13
or the external portion of A1/A15.

Current checkpoint: all implementation slices are integrated through
`df91ba159` on [draft PR #1901](https://github.com/ChronoAIProject/NyxID/pull/1901).
Identity, personal execution/settlement/Stop, ingress, Telegram transport, account
lifecycle recovery and frontend controls are integrated. Personal event admission,
the durable saved-reply hook, card continuation and original delayed-result
routing are also integrated. Public keys, exact budget/funding, restricted
FAQ/signup tools, encrypted disposable replies, model callbacks and cleanup are
implemented, but public activation remains unavailable until the deployed
upstream contract can be verified.

CI [38731500507](https://github.com/ChronoAIProject/NyxID/actions/runs/38731500507)
on `f1234a53a` passed all-target strict Rust Clippy, formatting, frontend
tests/coverage, CLI tests/coverage, wizard freshness and separate AWS/GCP builds.
CodeQL completed all four languages. Backend coverage compiled and executed the
full suite: **8,516 passed, 10 failed, 6 ignored**. The failures exposed acceptance
of array-shaped public-tool arguments, malformed or incomplete test fixtures,
and a missing public-callback route smoke case. Docker input validation also
found the FAQ missing from builder inputs. The combined AWS/GCP build was
interrupted by runner shutdown. No passing full backend run is claimed.

`71759e243` stages the embedded FAQ and exercises public callback credential
refusals through the real mounted route, checking that no sponsor meter opens.
`df91ba159` requires object-shaped public-tool arguments, tests both direct and
wrapped calls, and corrects response-ID, live-turn, revision and card-collision
fixtures without relaxing production authority checks.
[CI 38736949991](https://github.com/ChronoAIProject/NyxID/actions/runs/38736949991)
validated strict Clippy, all three AWS/GCP combinations, frontend/CLI
tests and coverage, wizard freshness, billing route smoke and Docker embedded
inputs on that revision. Full backend tests and coverage remain pending.

A focused local nextest run at `df91ba159` compiled in 10m02s and executed 24
tests against the transaction-capable MongoDB below: **23 passed, 1 failed**.
All Telegram transport journeys, public-tool tests, machine authority regressions
and billing route smoke passed. The remaining async-routing test progressed
past its corrected revision assertion and exposed a reused operation ID in its
three-case fixture. With a unique ID per operation, the focused rerun passed
all three cases in 2.669 seconds after a 9m42s rebuild. The 23 passing tests plus
this final focused pass verify all 24 selected regressions. Logs:
`/tmp/nyxbot-manager-ci-fixes-test.log` and
`/tmp/nyxbot-manager-async-fixture-test.log`. These tests use local HTTP fixtures;
they do not establish live Telegram or deployed public-runtime proof.

The final exact-settlement and usage consistency correction is integrated as
`23508ea09`. Astra reviewed it without a confirmed blocker. Its runtime
regressions passed in the full coverage test execution above; the ten unrelated
failures still prevent a green integrated result. Four precise lint expectations
identify intentionally unavailable verifier inputs and the Ready constructor;
there is no blanket lint suppression or fabricated readiness.

Earlier missing webhook arguments, transaction callback `Send` diagnostics and
four merge-related test-target errors have been corrected. Main `828009878` was
merged in `8e33386d6`; specialist report/card provenance is retained alongside
platform origin. The settings test now scopes Retry to its section and mocks its
separate personal-inventory request; all ten settings tests, focused ESLint and
TypeScript passed after the merge. The canonical wizard build and an independent
check of all 175 manifest inputs matched the regenerated source hash.

A local nextest run of `platform_manager_transport::journey_tests` on `f8f623a42`
was stopped after roughly an hour of host paging before tests executed. It used
the explicit transaction-capable MongoDB URI below, one Cargo build job, no
incremental compilation and line-table-only test debug information. The compiler
reported unused-code warnings and no type errors before interruption. This is
not a successful build or runtime result. Log:
`/tmp/nyxbot-manager-journey-1.log`. Runtime validation moves to CI after the
production consumer fixes. That interrupted local attempt supplies no runtime evidence; subsequent CI
and focused results are recorded above. Full acceptance gates remain open.

Main advanced again to `616a9131c`; merge `bf1ae6211` retains its inherited request
span and turn-start trace in the extracted runtime. Public budget recovery fixes
and public-intent minting are integrated through `c06123c47`. Opus's follow-up
source review found P3-1/P3-2 corrected; it does not replace executed regressions
or real upstream proof. Astra independently reviewed the integrated personal
event/hook/card and public signup-send changes at `75fc04d4a`, plus budget fixes
`cdfe61b12` and `457783115`, and found no high-confidence regression in that
bounded source review. Its subsequent public review found that a failed
preparation could retain an active session until the 24-hour session expiry;
`280e86b86` adds exact failed-preparation and expired-turn recovery. Astra checked
that correction without finding a remaining blocker. Its final transport review
at `5c9dcec9b` found no confirmed blocker in original destination checks,
event/reply deduplication, recovery, card rendering or public dispatch consumers.
Astra reviewed the public execution bridge and initially reported a timeout
recovery gap, then explicitly withdrew it after checking the concrete settlement
wrapper, which already revokes and schedules cleanup in the same transaction.
The actual-settlement regressions and protection against a stale failure
revoking a successor turn are integrated in `23508ea09` and passed Astra's bounded
source review. None of these reviews ran tests or
endorsed merging. Captured delayed-result
destinations currently expire with the original 2,100-second turn lease; later
results remain in web history. The focused Telegram regression run now passes that limit check; live
acceptance is still pending.

Main advanced to `897a89c2b` during verification. Merge `f00faf456` preserves its
composer/model-label/manual-CLI-login changes and the feature's Telegram labels
and origin decoding. The only conflicts were generated wizard HTML/hash; the
canonical `npm --prefix frontend run build:wizard` regenerated them on Node
22.21.1 and passed TypeScript checking. Independent hashing of all 175 manifest
inputs and extras matched
`3852abf799a366390be2b284ccc4712ed34eae9eb18bc56153971f4ab40fce26`.
The four affected frontend suites passed 115 tests after the merge. All four
personal Chromium journeys passed again on that merged source against HTTP
fixtures. Workspace formatting was checked again. Logs: `/tmp/nyxbot-manager-wizard-main897.log`,
`/tmp/nyxbot-manager-frontend-main897.log`.

Frontend worker checkpoint `8ac676907`, integrated as `678409e8b`, passed 187
tests across 12 files, focused ESLint, TypeScript checking, production and
credential-accept builds, legal prerender and mock-footprint checks. Four
Chromium journeys passed against HTTP fixtures on an isolated port-4611 server:
signup invitation navigation; confirmation/reload/intent clearing; retained web
history after unlink; and a fresh review after ten-minute challenge expiry.
Command: `node_modules/.bin/playwright test --config
playwright.platform-personal.config.ts` from `frontend`. These results support
A3/A5/A11/A16/A17 but do not prove integrated backend or live Telegram behavior.

The primary's full frontend run passed 5,611 tests and timed out once in the
unchanged `platform-anonymous-adoption.test.tsx` file under host load. That file
then passed both tests unchanged in isolation. Full ESLint passed; workspace
formatting passed at `98a360c87`. The full run is recorded as a timeout followed
by a successful focused retry, not as an uninterrupted full-suite pass.

| Gate | Owners | Required evidence | Status |
| --- | --- | --- | --- |
| A1 Public tool boundary | 3, 2 | Direct/wrapped MCP, resources/prompts, proxy/LLM and native-tool denial tests; pinned real NyxAgent containment proof | Open |
| A2 Operator isolation | 1, 2, 3, 5 | Two owners plus operator/sponsor; no history reads, prompt/result logs or public account capabilities | Open |
| A3 Proof and consent | 1, 4, 5 | Wrong account/chat, old onboarding link, signup alone, expiry/replay, parallel confirm | Open |
| A4 Personal parity | 2, 3 | Existing Auto/Ask/Full behavior, explicit denies, actual connected-resource scope, separate user credentials | Open |
| A5 History | 2, 4 | Same personal thread in web/Telegram; no public or old-home import; all adoption/recovery paths exclude platform threads | Open |
| A6 Revocation | 1, 2, 5 | Transaction races; exact origin/turn stop; no late send/card; independent tasks and web turns continue | Open |
| A7 Replay and races | 1, 3, 5 | Retry across link/unlink and process restart has one durable audience/charge; concurrent claims have one winner | Open |
| A8 Billing | 2, 3, 5 | Sponsor public, person personal inference, existing org BYOK semantics; no fallback payer or duplicate ledger posting | Open |
| A9 Full-loop limits | 3 | Exact concurrent caps over all priced hops, streaming, continuations, timeout/crash and UTC rollover | Open |
| A10 Groups | 5 | Linked sender and copied confirmation in group yield only private-chat handoff | Open |
| A11 Simple setup | 1, 3, 4 | Browser source + two budgets + Enable journey; defaults/preserved limits; failed readiness remains disabled | Open |
| A12 Existing product | All, 5 integrates | Existing assistant, channel, BotFather child setup/activation, grant and billing regressions; required CI | Open |
| A13 Live proof | 5, primary | Pinned NyxID/NyxAgent builds; two real accounts and Telegram identities; FAQ through billing/history/unlink | Open; needs reviewed build and authorized test environment |
| A14 Stable consent/capacity | 1, 5 | Guide edits/onboarding expiry/same-bot rotation preserve consent; public exhaustion leaves personal capacity; emergency stop blocks both | Open |
| A15 Public retention | 3, 5 | Sender/session isolation; live 24h expiry; linking cleanup; exact upstream delete/expiry proof; cleanup key cannot execute | Open |
| A16 Signup completion | 1, 4 | Signup beyond ten minutes gets fresh challenge; invitation policy intact; stale/replayed/mismatched intents denied | Open |
| A17 Personal lifecycle | 1, 2, 4, 5 | Atomic thread deletion/recreation, web continuation/cards after unlink, no public fallback on personal failures | Open |

## Verification environment

Source baseline `b6f11c1d3`; CI pins Rust `1.98.1` and cargo-nextest `0.9.146`.
CI uses MongoDB 8.0 as a replica set with an explicit
`NYXID_TEST_DATABASE_URL`. New race tests must call
`test_utils::connect_transaction_test_database`, which verifies a real
write-and-abort transaction and fails if MongoDB is absent or standalone.

Relevant required checks, as defined by `.github/workflows/ci.yml`:

- `cargo +1.98.1 fmt --all -- --check`
- `cargo +1.98.1 clippy --workspace --all-targets -- -D warnings`
- `cargo +1.98.1 nextest run -p nyxid -p nyxid-service-adapters --profile ci`
  with transaction-capable MongoDB; CI wraps this with resource sampling and
  uses line-table-only test debug information.
- Backend billing smoke and other required jobs selected by changed paths.
- Frontend `npm ci`, `npm run lint`, `npm run test`, `npm run build`.
- Relevant Playwright journeys using this worktree's server. The existing
  configuration uses port 4611 and may reuse a server outside CI; verify checkout
  identity or disable reuse so another worktree cannot supply false evidence.

Run focused checks after meaningful slices, then the required integrated suite.
Do not rerun unchanged suites without a new failure, change or unresolved risk.
Backend builds are serialized by the primary; workers request the build slot.
At the opening environment check, unrelated worktrees were compiling and the
volume had approximately 12 GiB available. No unrelated processes were stopped
and no unrelated artifacts were removed. Reassess before granting a build slot.

The primary provisioned an isolated local MongoDB 8.0.34 replica set on
`127.0.0.1:27188` named `nyxbot_manager_test`. Its explicit test URI is
`mongodb://127.0.0.1:27188/?replicaSet=nyxbot_manager_test&directConnection=true`.
An actual transactional insert followed by abort left zero documents, and the
temporary probe database was dropped. This verifies the test environment only,
not feature behavior. The temporary data directory and process belong to this
task and must be shut down after validation; other local MongoDB instances are
unrelated.

## External evidence to resolve before activation

- Exact NyxAgent repository/build and public profile contract: native tools,
  extra MCP servers, bounded execution, credential propagation for every priced
  model call, and isolated upstream session deletion/expiry.
  A [pinned upstream source review](2026-10-15-public-personal-nyxbot-upstream-evidence.md)
  now records the existing restrictions and concrete missing public-contract,
  cost-bound and cleanup support. It does not identify the deployed version or
  satisfy real runtime acceptance. Permission for changes to the separate
  NyxAgent repository has been requested; NyxID implementation continues.
- Sponsor authorization/funding, enforced invitation policy, and two distinct
  test people/Telegram identities for A13.
- All serving auth/MCP/proxy/webhook/worker replicas support the new mode before
  enabling it. Additive deserialization alone is insufficient.

The test results above do not establish production readiness, external runtime
proof or live acceptance. Each completed gate must link to its evidence.
