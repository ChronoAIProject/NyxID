# Telegram Sol implementation evidence

Implementer: persistent Heca agent `01a17afc-03c3-74f3-8206-1235e8b89954`, session `01a17afc-03c3-74f3-8206-125edb599f7c`. Actual Heca status verified `codex / gpt-6.1-sol / x_high / full-access`, exact worktree `01a17a96-885c-7042-bf1c-6e33531a2117`, branch `fix/telegram-onboarding-2045-1974`. No subagents, delegation, Claude/Opus calls, external Telegram requests, production sends, deployment, merge, publication or issue closure.

Approved plan v2 hash verified initially: `8eaf1ef5d3e584715496114ddccc2255edf112716c2ccbffdfb6283de958be6e`. Sol reported the absent hosted-origin/provenance ambiguity; Astra assessed and approved v3, verified in full at SHA-256 `cec3fc2c74c3ec14013ceeac7c7d2da154629d8bb7539ff3325b7e6aedd4f69b`. PM owns plan/review/status; source/tests and this evidence are Sol-owned.

Current checkpoint: `SOURCE_REVIEW_COMPLETE` for the exact A-R21 two-file patch in `telegram_new_connect.rs` and `telegram_new_tests.rs`, personally accepted by Astra. `LOCAL_VALIDATION_COMPLETE=true` for the authorized narrow A-R21/A-R22 follow-up on source commit `da9662a4980b852df8543c588cc1ebe631f3bf1b`: same-config fresh build, executable/source identity and inventory, both exact regressions (1 passed / 0 failed / 0 ignored each), and required all-target warnings-denied Clippy passed. Exact commands and evidence are appended below. The historical 387 unique passing tests and Clippy proof remain bound exclusively to feature commit `1b078ab9345bcd1d0eecf5e555ae8d7ee4de42ca` and its captured executable/source identity; those suites were not repeated. Both tickets and final exact-commit implementation approval remain BLOCKED, including native L1–L3 and I-07. Fixture/browser success is not Telegram client acceptance or actual reply proof. Historical failures, interrupted runs, results and identities are preserved chronologically below.

## Historical A-R21/A-R22 checkpoint before authorized revalidation

This section preserves the source-ready checkpoint before the narrow backend window. The completed revalidation on `da9662a4980b852df8543c588cc1ebe631f3bf1b` is appended at the end of this document.

The 387-test and Clippy proof below is preserved and bound exclusively to feature commit `1b078ab9345bcd1d0eecf5e555ae8d7ee4de42ca` and its captured executable/source identity. Astra's exact-commit review found A-R21, a registration-completion copy gap, and A-R22, an unexecuted existing current-row projection regression. Astra accepted the exact A-R21 source/test correction after personal review (`SOURCE_REVIEW_COMPLETE`); it has not been rebuilt or runtime-tested (`LOCAL_VALIDATION_COMPLETE=false`). G2 current-row execution and final validation remain OPEN. Telegram remains under the coordinator backend hold; no build/test/check/Clippy is started or queued for this follow-up.

A-R21 changes only the automatic registration success message in `telegram_new_connect.rs` and the existing `telegram_new_creation_webhook_completes_without_consent_or_browser_connect` assertion. Registration is stated separately from replies: an AI agent must be linked first; an existing specialist flow returns to its setup to finish/check the link; only when no setup is underway and no agent is linked does the copy direct a new link in Bot settings. After linking, the user opens the same bot, presses Start and sends a message to verify the first reply. Recipient, both inline buttons/URLs, connection lifecycle and secrets are unchanged. The existing test selects the completion by its existing button and asserts registration, prerequisite, existing-flow/no-duplicate condition and first-reply wording, retaining all prior lifecycle/effect/button/recipient assertions. No new trivial test was added.

Lightweight `cargo fmt --manifest-path backend/Cargo.toml --all` and final `cargo fmt --manifest-path backend/Cargo.toml --all --check` passed exit0 (`.tmp/telegram-r21-fmt.log`); `git diff --check` passed exit0 (`.tmp/telegram-r21-diff-check.log`). Pending authorized revalidation: rebuild the corrected candidate with the same shared resource config/target/jobs, capture fresh executable/source identity, execute the exact existing completion test and A-R22 `services::nyxbot_channel_query_service::tests::current_channels_rank_live_then_latest_before_limiting_and_order_ties` with Mongo27457/thread2, verify compiled nonzero counts, and rerun required corrected-placement all-target warnings-denied Clippy. The current-row test is confirmed once at line7497 of the prior compiled list but has not been executed. Frontend source is unchanged. Astra owns sign-off and final approval; native L1–L3/I-07 remain blocked/open.

A-R22's reviewed fixture proves the visible per-owner/per-bot current-row winner, retained failed history, ties, disconnection and limit behavior; it does not prove remote duplicate-create prevention. The prior shared-path reconnect fixture uses Lark and is not live Telegram proof. I-07 remains an integration evidence gap: `start_http` manually mounts Telegram routes and the artifact fixture calls `owner_response` directly. A real `build_router_with_state` / `private.with_state` `oneshot` fixture with real credentials is needed on an authorized combined commit. Astra's source trace found no production mount defect; no speculative integration code or imports were added.

## Actual commands and interim results

- `npm ci --no-audit --no-fund` in this worktree's frontend: exit 0. Log `.tmp/telegram-npm-ci.log`. No sibling dependencies changed.
- Initial existing focused frontend suite: 5 files / 60 tests passed. Expanded component/schema suite: first run found stale recovery race (1 failed / 78 passed); corrected by checking the current actor-scoped Query cache before navigation. Latest run: 6 files / 81 tests passed, log `.tmp/telegram-vitest.log`. Expiry and delayed-result tests included. Counts will be refreshed after final edits.
- `npx tsc -b`: initial run exposed nullable recovery-state narrowing (corrected); subsequent exit 0, log `.tmp/telegram-typecheck.log`. Final run pending.
- Initial normal Rust compile: `CARGO_TARGET_DIR=/Users/chronoai/Desktop/projects/nyxid/target CARGO_BUILD_JOBS=2 cargo test --manifest-path backend/Cargo.toml telegram_new --no-run`, log `.tmp/telegram-rust-build.log`. Compiled differing lockfile dependencies and reached backend test codegen; no compiler diagnostic before interruption. At 1.8 GiB free, PM's 2 GiB floor required stopping own Cargo PID 97204 with SIGINT. Exit **130**, rustc 99323 exited, free disk recovered to 4.7 GiB. No compile/test/link pass claimed. Coordinator notified and owns resource policy; no target/profile/cache/lock change made by Sol. All Rust verification remains OPEN pending a safe build window.

## Review findings under repair

- A-P11: approved v3 tagged context distinguishes new standalone/hosted requests; legacy absence refuses new recovery. Hosted snapshot captured transactionally and returned from persisted row.
- A-R01: context now gates both initial prompt admission and pre-keyboard dispatch, beyond the optional anchor. Exact watch/link deletion/retarget and between-send fixtures added; Rust execution pending.
- A-R02: generation 0 filter accepts missing/null legacy generation while preserving exact nonzero generations. Raw BSON absence fixture added; execution pending.
- A-R03: bounded gateway error whitelist retains configuration/auth/permission/availability in tool/watch reporting. Watched failure assertion pending.
- A-R04: rejected/unknown copy says NyxID has not yet confirmed creation, with no assertion about remote bot existence.
- A-R05: recovery click/result/cache render check known expiry; UI expiry timer removes action/anchor with stale polling data. Regression tests pass in latest focused frontend run.
- A-R06: intentional closed context/authority before dispatch is acknowledged; database/infrastructure failure remains retryable. Handler fixture uses a real MongoDB collection validator to reject admission writes. Execution pending.

## Remaining verification

Focused real MongoDB Rust tests; dedicated endpoint/middleware cases; complete gateway/release/continuity/introspection boundaries; browser API fixture journeys at 390/1440 and screenshot inspection; targeted ESLint/typecheck/production build; fmt/diff; clippy all-targets as resources permit; conventional implementation commit and exact final changed-file/command/count record; Astra personal review.

## Source-ready milestone (before Rust verification)

Latest approved plan v6 read in full and verified at SHA-256 `7cebb035bde823e2a78290393ec2f61cc982fb8a38d6ac3112abacb2f05edec7`. Intermediate approved v4 was `1b96853398e20c33fbf93ce06fdc546da8c3e91dd27fa343347d9fea8f130558`; v5 was `43158abb55c362ca66711b1432a83c6a3bc464ced4362f3f1473b4b30d1c4df1`. v4 adds modern fresh-event consumption fences (A-R07), v5 distinguishes saved link creator from initiating admin (A-R09), v6 documents the old-writer rollout/rollback boundary (A-R10). PM plan/review/status files remain unstaged and unmodified by Sol.

The first exclusive compile retry used the unchanged normal-profile command `CARGO_TARGET_DIR=/Users/chronoai/Desktop/projects/nyxid/target CARGO_BUILD_JOBS=2 cargo test --manifest-path backend/Cargo.toml telegram_new --no-run`, log `.tmp/telegram-rust-build-retry.log`. It emitted a real Send/HRTB lifetime error at `telegram_manager_e2e_tests.rs:113` (`post(manager_http)`, references to Value/TelegramNewService). Stopped own remaining obsolete second-target codegen with SIGINT: exit130, Cargo18051/rustc18126 both gone, 12GiB free. No pass claimed. Replaced the new recovery issuance async transaction callback with an explicit transaction and shared awaited abort/commit finish, preserving real conditional writes. Verification pending next assigned window; no unsafe, test removal, profile/flag/cache change or second target.

Latest actual frontend checks:

- Six-file component/schema run: 81 tests passed (`.tmp/telegram-vitest.log`). After the primitive expiry-dependency lint correction and hosted label normalization, `npm test -- src/components/channels/telegram-new.test.tsx src/pages/channel-connect-link.test.tsx`: 2 files /32 passed (`.tmp/telegram-vitest-final-affected.log`).
- `npx tsc -b`: exit0 (`.tmp/telegram-typecheck-final.log`). `npm run build`: exit0, including TypeScript, main production app, legal prerender, credential-accept build and mock-footprint assertion (`.tmp/telegram-frontend-build.log`). Existing large chunk and Node deprecation notices remain informational.
- Targeted ESLint on all edited TS/TSX source/test/e2e files: exit0, zero warnings (`.tmp/telegram-eslint-final.log`).
- `npx playwright test --config ../.tmp/telegram-playwright.config.ts channel-telegram-new.spec.ts`: initial 18/18 passed36.1s (`.tmp/telegram-playwright.log`). Initial added hosted cases exposed an invalid fixture attempt UUID; corrected fixture, four hosted/full-page cases passed7.6s (`.tmp/telegram-playwright-hosted.log`). Final affected `--grep 'recovery'` run after hosted display fix: 6/6 passed14.2s (`.tmp/telegram-playwright-final.log`). No meaningful check disabled.
- Real Chromium app with isolated API routes at port4317, widths390/1440, t.me intercepted locally; no external Telegram request. Keyboard Enter opens the same request's native URL, reload retains request, full-page and hosted flows return to the original group source. Hosted UI names the saved requester; exact target/provenance authority is exercised in Mongo fixtures, not inferred from UI text.
- Screenshots under ignored `.tmp/telegram-ui/`: `rejected-{390,1440}.png`, `unknown-{390,1440}.png`, `accepted-{390,1440}.png`, `recovery-focused-{390,1440}.png`, `full-page-recovery-{390,1440}.png`, `hosted-recovery-{390,1440}.png`, `hosted-source-return-{390,1440}.png`. Sol personally inspected focused390, hosted390 and source-return1440, plus initial rejected390. PM personally reran six journeys and inspected images; final PM review remains its own gate.

Source/tests now cover intentional guard failures vs real validator/transaction contention, monotonic and overlapping prompt claims, post-send metadata failure, raw legacy BSON generation absence, exact hosted mutation/source/group authority, human/empty-body/rate/private headers and real HTTP auth middleware, native recovery event replay/concurrent workers, org creatorA/initiatorB, valid installation recovery exclusion and hosted gateway configuration vs503. These remain unverified until actual Rust execution. New bounded log capture checks private/token sentinels absent. Rollout/rollback instructions preserve modern request and uncertain-dispatch fences before old writers resume.

## Additional personal-review corrections (execution pending for Rust)

- A-R11: Connect/Reconnect alerts translate the exact `Validation error: gateway_*` API message (or bare stable code) using the closed safe mapping. Ordinary validation prose is preserved; unknown gateway codes are generic. Eight rejected mutation cases use the actual `ApiError(400, {error: "validation_error", error_code: 1008, message: ...})` envelope. The direct `connect_tool` wrapper adds fixed guidance for the four known codes, while the hosted watch retains inner codes. The real hosted fixture now covers missing config, 401, 403 and 503, including direct-tool guidance.
- A-R12: `attempted_at` now means the current dispatch phase start. Both instruction and keyboard dispatch renew only the exact still-live sending attempt; a final age guard reserves the full bounded 20-second API call within its 45-second horizon. Initial admission CAS also compares observed timestamp, phase and outcome, preventing a stale expired read from replacing a concurrently renewed same-ID claim. DTO unknown projection uses the same horizon constant. Fixtures cover held instructions/keyboard across the original horizon, expired-claim refusal and explicit fresh retry, the 25s/25.001s budget boundary, and an uncommitted Mongo renewal read outside the transaction followed by commit and actual stale admission refusal.
- A-R13: diagnostic construction drops HTTP statuses outside100..599 rather than persisting frontend-invalid metadata. API and real request/DTO fixtures exercise600/700 with preserved rejected classification and available recovery. No raw upstream body is exposed.

Actual R11 checks: `npm test -- src/components/assistant/nyxbot-channels.test.tsx` passed46/46 (`.tmp/telegram-r11-api-envelope-vitest.log`). Targeted ESLint on the component, its test and error helper exited0 (`.tmp/telegram-eslint-r11-envelope.log`). Earlier bare-code draft passed44, but the46-test actual-envelope result supersedes that draft. `npx tsc -b` and `npm run build` also exited0 after the exact-prefix correction (`.tmp/telegram-typecheck-r11-envelope.log`, `.tmp/telegram-frontend-build-r11-envelope.log`). `cargo fmt --manifest-path backend/Cargo.toml --all --check` and `git diff --check` exited0 after the phase/CAS fixes (`.tmp/telegram-fmt-final.log`). No Cargo started/queued while the coordinator's heavy window belongs elsewhere.

A-R14 removes impossible connected/not-connecting fallbacks from the two already-touched connecting status blocks. Existing affected check `npm test -- src/components/channels/telegram-new.test.tsx src/components/channels/telegram-new-connect.test.tsx` passed31/31 (`.tmp/telegram-r14-vitest.log`). The consumption exact-snapshot fixture moved into `telegram_new_recovery.rs`'s private child test module, preserving production helper privacy and all assertions; its local credential collection import is explicit.

Static filter inventory: the file `nyxbot_tests.rs` is mounted as `handlers::nyxbot::tests`, and `nyxbot_release_tests.rs` as its `release_tests` child. Filenames in plan filters may therefore match zero; use compiled test listing and actual module paths to establish nonzero release/continuity and handler counts. Mongo78366 has72 open lsof entries against8192 soft descriptors, with no FD failures; retained per PM direction and will be monitored each batch.

Final frontend source after A-R14: `npm run build` exited0 (`.tmp/telegram-frontend-build-final.log`, includes `tsc -b`, main Vite, legal prerender, credential-accept build/mock-footprint assertion); targeted component ESLint exited0 with zero warnings (`.tmp/telegram-eslint-r14.log`). Final Rust fmt check and diff check exited0. Mongo remained72 descriptors while waiting; no shared resource changes by Sol.

## Resumed backend window

Coordinator handoff initially encountered active Permissions Cargo42211/rustc42278; Sol verified their sibling cwd and did not queue a build. Coordinator subsequently cleared the overlap. Own preflight verified no heavy processes and5.4GiB free. Actual Heca settings/worktree remained codex/gpt-6.1-sol/x_high/full-access and the exact authorized worktree.

First resumed command: `CARGO_TARGET_DIR=/Users/chronoai/Desktop/projects/nyxid/target CARGO_BUILD_JOBS=2 cargo test --manifest-path backend/Cargo.toml telegram_new --no-run`, log `.tmp/telegram-rust-build-current.log`. Compiler reported four E0433 uses from a missing local `AppError` import in the new delivery test module. Stopped own obsolete remaining target by PTY SIGINT; tool exit was **1**, not130. Cargo46038/rustc46143 verified gone, df8.9GiB. Added the explicit test enum import, no production behavior change or check removal. PM personally assessed the diagnostics.

Retry uses the same exact command/settings, log `.tmp/telegram-rust-build-import-fix.log`. A task-specific resource guard samples disk every5s and SIGINTs only the identified own Cargo PID below2GiB; no cache/profile/lock/target change. Guard logs `.tmp/telegram-resource-current.log` and `.tmp/telegram-resource-import-fix.log`. Retry currently running; no pass claimed.

The import-fix retry did **not** complete: at epoch1793070414 the guard measured2,068,398,080 free bytes (~1.93GiB), below the2GiB floor, and sent SIGINT only to own Cargo46785. The tool returned actual exit **1**. Cargo46785 and rustc46875 were verified gone; no cargo/rustc/cc/ld/clang remained. Free disk recovered to6.9GiB. The retry log contains only the two Compiling lines and no newer compiler diagnostic; neither compilation nor the prior Send fix is claimed verified. No Rust test, compiled listing or clippy batch was started, and no additional Cargo was queued. PM notified immediately.

Read-only post-stop sizes: shared target/debug/incremental12G, deps12G, build755M; swap total4096MiB/used3312.06MiB; df6.9GiB. This retry consumed roughly7GiB of headroom before completion; linker peak was not measured. No safe identical-retry capacity is established merely by the recovered6.9GiB. Coordinator owns cache/headroom disposition; Sol changed no flags/profile/target/locks/cache and awaits explicit resolution before more Cargo. Local Rust validation, final implementation commit/review and live L1–L3 remain OPEN.

## Approved resource-profile retry

After coordinator-owned reclaim and explicit Astra exclusive-window clearance, the prior unchanged-profile restriction was superseded by the shared operational configuration `/Users/chronoai/Library/Application Support/heca/worktrees/eafaaf47/arctic-eagle/docs/plans/2026-10-27-nyxid-workstreams/cargo-resource-profile.toml`. Its only settings are package `nyxid` dev/test `debug=0` and `incremental=false`; dependencies, optimization, assertions, overflow checks, test scope and release remain at their defaults. Sol changed no shared configuration or cache.

Preflight verified no Cargo/rustc/linker processes,25,083,572,224 free bytes, and Mongo27457 writable primary in `nyxidTelegramReview` with71 open lsof entries. Actual Heca settings remain `codex / gpt-6.1-sol / x_high / full-access` and the authorized worktree. Started exactly once with `CARGO_TARGET_DIR=/Users/chronoai/Desktop/projects/nyxid/target CARGO_BUILD_JOBS=2 cargo --config '/Users/chronoai/Library/Application Support/heca/worktrees/eafaaf47/arctic-eagle/docs/plans/2026-10-27-nyxid-workstreams/cargo-resource-profile.toml' test --manifest-path backend/Cargo.toml telegram_new --no-run`. New log `.tmp/telegram-rust-build-resource-profile.log`, guard `.tmp/telegram-rust-build-resource-profile-resource.log`; own Cargo54308 sampled every5s with own-process-group SIGINT below2GiB.

Captured actual rustc54459 `--crate-name nyxid_server --test` argv in `.tmp/telegram-rust-build-resource-profile-rustc-options.log`: selected `-C` values are `embed-bitcode=no`, metadata, extra-filename and the existing Swift rpath link argument. No `debuginfo`, `incremental`, `opt-level`, `debug-assertions` or `overflow-checks` option is passed. Thus rustc defaults remain debuginfo0, no incremental, opt-level0, enabled debug assertions and enabled overflow checks. No new compiler diagnostic at capture; compilation result remains pending.

The approved resource-profile compile **passed, exit0 in9m47s**, producing `target/debug/deps/nyxid_server-a7bf57b9c4cb166a`. Guard never triggered. Runner verified `remaining_owned_children=[]`, free23,193,755,648 bytes; independent process check found no Cargo/rustc/linker and subsequent df showed23,698,788KiB free (~22.6GiB). PM notified immediately before subsequent commands. Cargo's workspace-level `[unoptimized + debuginfo]` summary also covers dependencies, whose settings remain unchanged; actual captured backend argv proves the package override. This validates the previous Send/import fixes at compilation; Rust test execution remains pending the serial batches.

### Compiled inventory and execution identity

The subsequent same-config `cargo test --manifest-path backend/Cargo.toml -- --list` unexpectedly started a full CLI/backend rebuild. Both existing build scripts watch missing package-local `.git/HEAD` and `.git/refs`. Stopped only redundant own Cargo58268 process group: Popen -2 / actual tool exit130, no remaining owned children or other compiler/linker; free24,265,089,024 bytes. Failed/interrupted log `.tmp/telegram-rust-test-list.log` is retained; the early inventory parser refused its zero count and no inventory/test pass was claimed.

Astra personally inspected the build-script issue and approved direct serial execution of the unchanged successfully built binary. Identity `.tmp/telegram-test-binary-identity.json`: SHA256 `5fd2b8d96c5fce15fd0de3d353a7b6a6b2a1f371677b522cc7bef477c4f6cc07`, size756,187,696 bytes, mtime_ns1793071716545770526. Backend source hashes are preserved in `.tmp/telegram-compiled-source-sha256.json`; any source edit requires a rebuilt candidate. All fixture executions use backend package cwd, `NYXID_TEST_DATABASE_URL=mongodb://127.0.0.1:27457/?directConnection=true`, `--test-threads=2`, and the original binary compiled with the exact approved config. Native loader rpath is already in the captured link options; no runtime override needed. An initial root-cwd list (no test execution) was superseded by the package-cwd list.

`/Users/chronoai/Desktop/projects/nyxid/target/debug/deps/nyxid_server-a7bf57b9c4cb166a --list` at backend cwd passed exit0:10472 tests/0 benchmarks. Log `.tmp/telegram-rust-compiled-list-backend.log`, inventory `.tmp/telegram-rust-inventory.txt`. Exact nonzero filters: `telegram_new`94; `telegram_manager`22; `channel_connect_link`19; `platform_provisioning`12; `nyxbot_gateway`16 including `services::nyxbot_gateway_service::creator_tests`6; `handlers::nyxbot::tests`219 including `handlers::nyxbot::tests::release_tests`32 and its `continuity_`7. These are inventory counts, not execution passes. Clippy still runs through Cargo with the shared config.

### First original-candidate execution and diagnostics

All direct commands below use the exact binary path above at backend cwd with the explicit Mongo URI and `--test-threads=2`. First `telegram_new` batch **failed:91 passed/3 failed/0 ignored**,77.35s, exit101; `.tmp/telegram-rust-tests-telegram-new.log`. Failures: same-update first-rejection retry unwrap returned ChannelPlatformError10005 at `telegram_new_tests.rs:240`; org-manager HTTP creation expected200/received409 at `telegram_manager_e2e_tests.rs:482`; public manager copy asserted removed `latest Telegram app` at `telegram_manager_public_tests.rs:82`. All new lease/CAS/authority/context/consumption regression cases shown in this run passed. Overall pass is not claimed.

Unchanged exact reproductions with `--exact --test-threads=2 --nocapture`: first rejection failed0/1 in0.91s (`.tmp/telegram-rust-retry-first-rejection-unchanged.log`); org manager failed0/1 in2.20s (`.tmp/telegram-rust-org-e2e-unchanged.log`), both exit101/zero ignored. Astra and Sol inspected WireMock0.6.5: MockSet sorts mocks by priority in place but guard IDs retain original insertion indexes. Scoped priority1 rejection therefore deactivates the wrong mock after sorting, leaving rejection active. Planned narrow one-shot provider fixture preserves real retry/accepted/three-send assertions. Org409 cause remains under investigation; do not broaden expected behavior without exact error/effect/redelivery proof. Public copy expectation will assert approved bounded retry guidance while retaining chat/recovery/keyboard checks.

After the94 fixtures, Mongo78366 remained healthy but had5925 lsof entries (5908 regular WiredTiger files), near its8192 soft limit. Astra approved restarting only this idle Mongo. Verified zero test binaries and only its27457 LISTEN socket; copied prior log to `.tmp/telegram-mongo8/mongod-before-fd-restart.log`; graceful SIGTERM exited. A task-specific launcher raised its own process soft limit8192→65536 under unchanged hard9223372036854775807, then spawned the same exact mongod args/data/replica/port/cache. No global limit, sibling Mongo or database deletion. New PID61027,67 descriptors after startup, healthy writable `nyxidTelegramReview`; unique probe insert/transaction abort passed. High-limit startup emits no low-rlimit warning; actual launcher limits captured in tool output, privileged launchctl inspection unavailable and not attempted with escalation. Server connection capacity changed3276→26214, consistent with the inherited limit. Preserved existing data/fixture isolation.

Astra approved finishing remaining unchanged original-candidate filters before batching understood fixes and rebuilding once. Any subsequent source changes will make these explicitly historical candidate results; final affected verification requires a new binary.

### Complete original-candidate serial batch (historical, before test corrections)

Every command used the pinned SHA above at backend package cwd, explicit Mongo27457 URI and `--test-threads=2`. Exact runner calls were `python3 -u .tmp/run-telegram-backend.py <label> --binary /Users/chronoai/Desktop/projects/nyxid/target/debug/deps/nyxid_server-a7bf57b9c4cb166a <filter> --test-threads=2`; each label names its `.tmp/<label>.log` and resource log.

| Filter | Label | Result | Duration |
|---|---|---|---|
| `telegram_new` | `telegram-rust-tests-telegram-new` | 91 passed/3 failed/0 ignored, exit101 | 77.35s |
| `telegram_manager` | `telegram-rust-tests-manager-original` | 21 passed/1 failed/0 ignored, exit101 | 14.77s |
| `channel_connect_link` | `telegram-rust-tests-hosted-original` | 19 passed/0 failed/0 ignored, exit0 | 2.45s |
| `platform_provisioning` | `telegram-rust-tests-provisioning-original` | 12 passed/0 failed/0 ignored, exit0 | 26.86s |
| `nyxbot_gateway` | `telegram-rust-tests-gateway-original` | 16 passed/0 failed/0 ignored, exit0, including6 creator tests | 24.80s |
| `handlers::nyxbot::tests` | `telegram-rust-tests-handlers-original` | 218 passed/1 failed/0 ignored, exit101 | 638.59s |

Union:360 unique tests executed,356 passed/4 failed/0 ignored; overlapping filters do not add unique coverage. All32 release and all7 continuity cases passed in the219 handler batch. Mongo61027 stayed healthy; roughly21k descriptors peak under65536, no FD errors. No compiler/test children remained after the batch.

The fourth failure was `hosted_watch_reports_gateway_configuration_separately_from_outage`: `links::complete` at613 requires `bot.platform == link.platform`; fixture synthetically changed the link to telegram-new while inserting a telegram bot. NotFound1003 at helper972 occurred before any gateway assertion. Corrected fixture uses actual `setup_link_tool(...,"telegram",...)` in a manager-unconfigured setup, asserts the exact manual platform and retrieves that link's watch. All configuration/401/403/503 event, safe-code, tool-guidance and no-sentinel assertions remain.

Coherent test-only corrections also replace WireMock's scoped sorted-priority guard with a one-shot rejection (`up_to_n_times(1)`, `expect(1)`), retain same-update Accepted/three-send checks and assert instructions/instructions/keyboard sequence; public welcome expects approved bounded retry guidance. Org HTTP fixture now injects a command-observed transaction DB. Test-only task-local update15 correlates synchronous Mongo callbacks with the creation HTTP task. At most32 pending/16 failure records retain only fixed command/collection/statement categories, numeric request/update/transaction IDs, numeric error code and a known-label boolean; no raw BSON, private text, provider body, token or Mongo error prose is logged. Retry remains conditional on both the exact Conflict1004/key/message and correlated owner-org-fence update failure112/TransientTransactionError. No runtime transaction changes. Final200/Ready, same proof IDs/date, unchanged original identity/provenance, duplicate-event revision and unique child/token/webhook effects are required. Root cause is still OPEN until this is actually executed. Source modifications make all original-binary results historical; a fresh config-bearing build precedes affected execution.

### Corrected candidate compilation and root-cause execution

Astra personally inspected the coherent test-only correction/observer diff and approved continuing the already-running rebuild. The same exact shared-config `cargo test --manifest-path backend/Cargo.toml telegram_new --no-run` passed exit0 in10m24s, log `.tmp/telegram-rust-build-fixture-corrections.log`; resource/options siblings preserve the own-process guard and actual unchanged rustc codegen options. Cargo67461/rustc67566/linker children exited, `remaining_owned_children=[]`, free21,643,866,112 bytes. No production source changed from the original successful compilation. Only five test files differ from its source manifest: nyxbot_tests, telegram_manager_e2e_tests, telegram_manager_public_tests, telegram_new_delivery_tests, telegram_new_tests.

Fresh binary identity `.tmp/telegram-test-binary-corrected-identity.json`: same executable path, SHA256 `cd6f7105e84334a4fffd4bc1e67e6e80fd0115ea3698b3d4580a358e65cc215f`, size767,218,592,mtime_ns1793074138924362983. Fresh backend source manifest `.tmp/telegram-corrected-compiled-source-sha256.json`. Original manifests are retained for historical results. Fresh backend-cwd `--list` exit0,10472 tests/0 benchmarks (`.tmp/telegram-rust-compiled-list-corrected.log`), unchanged nonzero filter inventories94/22/19/12/16/219 with6creator/32release/7continuity.

First exact corrected test command: `NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27457/?directConnection=true' /Users/chronoai/Desktop/projects/nyxid/target/debug/deps/nyxid_server-a7bf57b9c4cb166a services::telegram_new_tests::manager_e2e::telegram_new_manager_org_channel_http_chat_reply_and_creation_end_to_end --exact --test-threads=2 --nocapture`, backend cwd. **1 passed/0 failed/0 ignored**,2.94s,exit0, `.tmp/telegram-rust-org-e2e-corrected.log`. Measured root A-R16: exact envelope Conflict1004/key conflict/message `Conflict: Security state changed; restart the operation`; correlated actual Mongo command request423/update15/transaction29, `update`/`users`/`owner_org_fence`, code112, `TransientTransactionError=true`. This proves the originating transaction statement rather than inferring it from an HTTP status. Identical update/date/payload redelivery succeeded; Ready/Connected, duplicate event revision unchanged, original request identity/actor/owner/date/start10/sender700 and unique child/token/webhook effects passed. No broad conflict acceptance or production transaction change.

Other exact corrected commands use the same binary/env/cwd and `--exact --test-threads=2 --nocapture`: `services::telegram_new_tests::delivery_tests::telegram_new_bound_first_rejection_can_retry_same_update` passed1/1,0ignored,0.81s (`.tmp/telegram-rust-first-rejection-corrected.log`); `services::telegram_new_tests::manager_public::telegram_manager_public_start_welcomes_chat_without_hiding_creation` passed1/1,0ignored,0.93s (`.tmp/telegram-rust-public-guidance-corrected.log`). Final focused batches and clippy remain pending below.


The exact corrected hosted-watch run failed0/1,4.51s,exit101 (`.tmp/telegram-rust-hosted-gateway-corrected.log`): configuration case passed,401 saved `invalid` instead of `gateway_not_authenticated`. Initial suspicion of a remaining production-code propagation defect was premature. Source traced the actual additional fixture precondition: test_app_state defaults jwt_issuer to `nyxid`, while real `connect_gateway` requires HTTPS or localhost HTTP and returns `issuer_unsupported` before any provider POST. Existing real gateway fixtures explicitly set a supported issuer. Narrow correction sets only this test's issuer to `https://nyxid.test` and requires each configured case to have actually sent POST `/channels`; all four last_error/event/tool/sentinel checks remain. Production code unchanged. This source change requires another rebuilt candidate before affected execution; clippy runs first to collect any static findings before that expensive rebuild.


Required clippy first invocation: `CARGO_TARGET_DIR=/Users/chronoai/Desktop/projects/nyxid/target CARGO_BUILD_JOBS=2 cargo --config '/Users/chronoai/Library/Application Support/heca/worktrees/eafaaf47/arctic-eagle/docs/plans/2026-10-27-nyxid-workstreams/cargo-resource-profile.toml' clippy --manifest-path backend/Cargo.toml --all-targets -- -D warnings`, log `.tmp/telegram-rust-clippy-all-targets.log`. Runtime capture revealed the external cargo-clippy boundary drops global --config placed before its subcommand: cargo-clippy71641 argv lacked it, and actual backend clippy-driver79099/79129 (metadata-only, both non-test/test targets) showed debuginfo2/split-debuginfo unpacked/shared incremental. Built-in test had honored the same supplied config. Reported immediately; Astra independently confirmed and escalated exact options. Current guarded check allowed to collect warnings while headroom remains ample; no independent flag/config/cache changes. Coordinator operational alignment for passing the identical file after clippy is pending. No resource-config compliance claimed for this first clippy invocation.


### Complete lint findings and approved corrections

The first all-target warnings-denied Clippy finished exit101, with all owned children gone and20,059,254,784 free bytes. Full diagnostics remain in `.tmp/telegram-rust-clippy-all-targets.log`. Its six distinct findings were two owned changes (large `CreationContext` enum variant and unused socket read amount) plus four pre-existing `nonminimal_bool` expressions. No suppressions, dependency edits or check reductions were used.

Astra approved transparent `Option<Box<CreationWatch>>` plus the sole `Box::new` snapshot constructor; existing raw BSON/hosted exact-snapshot tests must prove compatibility after rebuilding. The deliberately interrupted-response fixture now asserts the actual socket read count is nonzero as a request-arrival barrier, retaining the incomplete-body `InvalidResponse` and uncertain-outcome assertions. The hosted gateway fixture uses a valid local HTTPS issuer and counts exact POST `/channels` separately after watch and direct tool execution, so cleanup calls cannot masquerade as gateway-path coverage.

Coordinator and Astra approved exactly four equivalent baseline boolean rewrites in `login_code_service.rs`, `service_pool_routing.rs`, `lark_base_writes/ledger.rs` and `nyxbot_gateway_release_service.rs`. Astra personally reviewed complete diffs and surroundings, including short-circuit, Utc, None and malformed UUID behavior. These will be isolated in a prerequisite commit containing only those four files after validation. Required existing login10/Lark15/pool1/node-exclusion1 tests plus handler release32 remain mandatory. The pool legacy-fallback test alone does not reach node eligibility; the exact runtime excluded-node test does.

Coordinator corrected external-command placement: built-in Cargo test/check/build keep global `--config` before the subcommand, while Clippy uses `cargo clippy --config <same-file> ...`. The same shared values/scope remain unchanged. Corrected command, started once after the first fully exited: `CARGO_TARGET_DIR=/Users/chronoai/Desktop/projects/nyxid/target CARGO_BUILD_JOBS=2 cargo clippy --config '/Users/chronoai/Library/Application Support/heca/worktrees/eafaaf47/arctic-eagle/docs/plans/2026-10-27-nyxid-workstreams/cargo-resource-profile.toml' --manifest-path backend/Cargo.toml --all-targets -- -D warnings`. Log `.tmp/telegram-rust-clippy-corrected-profile.log`, resource/options siblings. Actual nyxid_server non-test/test clippy-driver argv captures only embed-bitcode=no, metadata, filename and existing Swift rpath codegen options: no debuginfo/incremental/optimization/assertion/overflow override, preserving debug0/no incremental/opt0/assertions+overflow enabled. Source is stable during this command; result pending.


Corrected-placement Clippy **passed exit0 in6m22s**, all targets with `-D warnings`, zero diagnostics (`.tmp/telegram-rust-clippy-corrected-profile.log`). Actual backend bin/test driver options comply with shared profile as captured above. Guard did not trigger; all owned children exited,20,008,153,088 free bytes. This supersedes the first six-finding run without discarding its failed log. Final same-config no-run rebuild then started once under `.tmp/telegram-rust-build-final.log` with resource/options siblings, own Cargo84714. Source manifest `.tmp/telegram-final-compiled-source-sha256.json` captures backend sources, manifest, lock and build script before this build; final executable identity and inventory will follow its result.


### Final executable and focused execution

Final same-config no-run compilation **passed exit0 in9m41s** (`.tmp/telegram-rust-build-final.log`). Resource guard never triggered; `remaining_owned_children=[]`,20,008,542,208 free bytes. Actual backend test rustc options again omit debug info/incremental/optimization/assertion/overflow overrides. Fresh binary `.tmp/telegram-test-binary-final-identity.json`: SHA256 `1060f46cade1f49edbd68242d003eab3cb90f52609a12b0fb5eaaedf412f9ee1`, size757,234,992,mtime_ns1793076008793457273 at the same approved executable path. `.tmp/telegram-final-compiled-source-sha256.json` binds1288 backend/source/build inputs, unchanged through compilation. Historical identities/logs remain preserved.

Final direct `--list` at backend cwd **passed exit0**,10472 tests/0benchmarks (`.tmp/telegram-rust-compiled-list-final.log`). Inventory `.tmp/telegram-rust-final-inventory.json` confirms telegram_new94,telegram_manager22,channel_connect_link19,platform_provisioning12,nyxbot_gateway16/creator6,handlers::nyxbot::tests219/release_tests32/continuity_7; prerequisite login10,lark15,pool1,exact runtime node-exclusion1. No filename-only zero-match filter is treated as verification.

Exact first final execution: `NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27457/?directConnection=true' /Users/chronoai/Desktop/projects/nyxid/target/debug/deps/nyxid_server-a7bf57b9c4cb166a handlers::nyxbot::tests::hosted_watch_reports_gateway_configuration_separately_from_outage --exact --test-threads=2 --nocapture` at backend cwd. **1 passed/0failed/0ignored**,8.45s,exit0 (`.tmp/telegram-rust-hosted-gateway-final.log`). None/local-config,401,403,503 saved errors,watch events,direct-tool guidance and sentinel exclusion assertions passed, including separate exact `/channels` POST counts before and after direct-tool dispatch. This resolves the fixture issuer/transport precondition without any production-policy relaxation.

Final serial batches are launched by ignored `.tmp/run-telegram-final-batch.py`, which checks the compiled nonzero inventory, exact binary SHA/mtime and all captured source hashes before each filter; asserts Mongo27457 writable primary; records descriptor count; and invokes the same owned-process guarded runner serially. Each command is direct executable at backend cwd with the explicit Mongo URI and `--test-threads=2`. No source changes occur during the batch. `.tmp/telegram-rust-final-batch.log` and per-filter logs/summary preserve exact commands/results.


## Final local verification and delivery

All ten final serial filters passed: **409 test executions /387 unique tests, zero failed, zero ignored, zero measured**. The separately executed first hosted diagnostic also passed1/1 and overlaps the handler batch. Six primary filters cover360 unique tests; the four prerequisite filters add27. Every executed name was reconciled against its log and compiled inventory, including32 release,7 continuity and6 creator/introspection tests. Explicit Mongo27457 was authoritative, transaction-capable unique databases were used, and no fixture skip was accepted. Source and executable hashes remained exact before every filter and after the full batch.

All commands used backend package cwd, `NYXID_TEST_DATABASE_URL=mongodb://127.0.0.1:27457/?directConnection=true`, and executable `/Users/chronoai/Desktop/projects/nyxid/target/debug/deps/nyxid_server-a7bf57b9c4cb166a`. Actual execution template was `<executable> <filter> --test-threads=2`; the fully qualified node-exclusion filter additionally used `--exact`. The ignored serial runner and resource runner record these actual commands, normal environment, process IDs, disk floor and child exit without changing test behavior. Final summary `.tmp/telegram-rust-final-batch-summary.json`, full orchestration `.tmp/telegram-rust-final-batch.log`, reconciled names/resources `.tmp/telegram-final-verification-summary.json`.

| Actual filter | Passed | Duration | Exact log |
| --- | ---: | ---: | --- |
| `telegram_new` | 94 | 87.73s | `.tmp/telegram-rust-final-telegram-new.log` |
| `telegram_manager` | 22 | 16.43s | `.tmp/telegram-rust-final-manager.log` |
| `channel_connect_link` | 19 | 2.99s | `.tmp/telegram-rust-final-hosted.log` |
| `platform_provisioning` | 12 | 31.33s | `.tmp/telegram-rust-final-provisioning.log` |
| `nyxbot_gateway` | 16 | 23.25s | `.tmp/telegram-rust-final-gateway.log` |
| `handlers::nyxbot::tests` | 219 | 620.75s | `.tmp/telegram-rust-final-handlers.log` |
| `login_code_service` | 10 | 172.96s | `.tmp/telegram-rust-final-login.log` |
| `lark_base_writes` | 15 | 10.28s | `.tmp/telegram-rust-final-lark.log` |
| `service_pool_routing` | 1 | 0.20s | `.tmp/telegram-rust-final-pool.log` |
| `handlers::service_pool_proxy_tests::runtime::pool_proxy_excluded_node_member_is_skipped_before_credential_materialization` | 1 | 22.51s | `.tmp/telegram-rust-final-pool-node-exclusion.log` |

Required corrected-placement `cargo clippy --config '/Users/chronoai/Library/Application Support/heca/worktrees/eafaaf47/arctic-eagle/docs/plans/2026-10-27-nyxid-workstreams/cargo-resource-profile.toml' --manifest-path backend/Cargo.toml --all-targets -- -D warnings` passed exit0 in6m22s on this exact source. Actual bin/test options prove package debug0/no incremental with default opt0/assertions/overflow enabled; dependency settings remain unchanged. Final `cargo fmt --manifest-path backend/Cargo.toml --all --check` passed exit0 (`.tmp/telegram-fmt-delivery.log`); `git diff --check` passed exit0 (`.tmp/telegram-diff-delivery.log`). No allowances, unsafe, removed dependencies or meaningful check reductions.

At final batch completion:20,572,274,688 free bytes,19262 Mongo open entries under65536, writable primary `nyxidTelegramReview`, no Cargo/rustc/cc/ld/clang/clippy processes. Every owned guard reports no disk stop and no surviving children. Peak sampled batch-boundary descriptors21696 remained below the limit; no new FD errors occurred. Coordinator independently reconciled387/387 and quiescence, closed the exclusive window and prohibited further heavy Cargo until renewed clearance. No subsequent build/test/check/Clippy is queued. Existing missing package-local `.git` build-script watches remain a known build-infrastructure limitation: direct execution of the pinned compiled candidate was explicitly assessed/approved by Astra; source edits would require a fresh build.

Frontend remained unchanged during backend verification. Exact six-file command was `npm test -- src/components/channels/telegram-new.test.tsx src/components/channels/telegram-new-connect.test.tsx src/components/channels/telegram-new-resume.test.tsx src/pages/channel-connect-link.test.tsx src/components/assistant/nyxbot-channels.test.tsx src/schemas/telegram-new.test.ts`:81/81. Later affected commands separately passed32/32 (Telegram/page),46/46 (assistant actual ApiError envelope) and31/31 (Telegram/connect after R14). These overlap and are not summed. Final `npx tsc -b`, targeted edited-file ESLint and `npm run build` exited0 in the logs already listed. Existing Vite chunk-size and Node deprecation notices are informational. No unchanged unit checks were rerun merely for review.

Actual browser command `npx playwright test --config ../.tmp/telegram-playwright.config.ts channel-telegram-new.spec.ts` passed18/18; added hosted/full-page cases passed4/4; final affected `--grep 'recovery'` passed6/6. Existing repository Chromium harness drove this worktree app at4317,390/1440, with local API interception and intercepted t.me navigation. Screenshot paths above include focused keyboard recovery, full-page, hosted target and original source return. No browser request reached Telegram. API fixtures and Rust local HTTP/Mongo integration are separate kinds of evidence; neither is live client/provider acceptance.

| Plan item | Demonstrated local evidence | Acceptance boundary |
| --- | --- | --- |
| T1 |94-test Telegram batch: bounded diagnostics/status normalization, provider rejection, malformed/interrupted/oversized/timeout responses, redirect refusal, safe Debug/DTO/log sentinels | Accepted API call does not establish rendering/creation |
| T2 | Bound outcomes, first-send one-shot retry, duplicate monotonic boundary, overlapping commands and durable post-send write failure cases in94 | Explicit fresh command required for uncertain delivery |
| T3 | Phase-renewal45s/full20s reserve, stale observed CAS, lifecycle/authority/context mutations and consumption races in94 | Revocation/expiry/terminal state retains priority |
| T4 | Human middleware/empty body/rate/private headers; exact standalone/hosted/source/admin/manager snapshots, legacy absence and installation exclusions in94 | Advisory DTO is not authority; issuance/consumption revalidate |
| T5 | Concurrent duplicate events/workers, one consumed bot, org creatorA/initiatorB, unchanged identity/provenance plus hosted19 and handler219 | Remote provider may create extras; NyxID consumes at most one |
| T6 | Six-file81 plus affected32/46/31; expiry, delayed result/cache/account switch, popup fallback, optional legacy DTO, safe error/unknown/accepted states | Overlapping counts reported separately |
| T7 | Actual Chromium18+4+6 runs and390/1440 screenshots, keyboard activation/reload/hosted original-source return | Local fixture UI, no live Telegram claim |
| G1 | Gateway16 including6 creator/introspection, plus owner/org authority in94 and handler219 | #1984 exact confidential-client/creator boundary retained |
| G2 | Hosted exact1/1 and handler219; gateway16; frontend46 actual error-envelope cases |401/403/configuration/availability guidance remains distinct; explicit Reconnect retained |
| G3 | Handler219 including release32/continuity7, platform12, hosted19, partial registration/native94 | #1991 exact remote release/continuity preserved |

Astra personal exact-commit/full-diff acceptance remains OPEN. Native L1–L3 remain OPEN and did not waive coding/local/integration/UI checks. Required future live evidence is one explicitly authorized named supported client/OS/version, dedicated manager/account and existing specialist/source, exact deployed versions and confidential introspection client association, one bot creation and owner-link action if needed, one unique harmless owner message and actual reply, then same-account scoped retry. No such live authority or action was inferred.

Residual operational/provider limits: issued native form URLs are not revocable and carry no NyxID request ID; Telegram may allow deliberate extra remote forms, but fresh private event/date/update proof and exact transactional fences still admit at most one bot. Bot API acceptance proves neither rendering nor creation/reply. Old writers ignore new context/dedup fences, so documented admission pause/drain/all-writer upgrade/version verification and terminal-row/uncertain-attempt rollback handling are required before production use. Legacy absent context retains its old flow and is intentionally ineligible for new recovery; platform-installation recovery remains excluded. Initial resource/Send/import/lint/fixture failures and exact failed logs are retained above; no unresolved local code/test/infrastructure failure remains in the final source. No deployment, merge, push, issue closure, production send or external Telegram effect occurred.

Validated prerequisite commit: `336405f8e667ea2eb2c6513effec3d15fd960b02` (`chore(backend): simplify equivalent boolean guards`), exactly four approved files/eight additions/eight removals, full diff sent to Astra before peer import. The feature commit accompanying this evidence contains the remaining32 files. The delivery's complete36-file list follows; PM-owned implementation/review/status documents are preserved unstaged/unmodified by Sol.

```text
backend/src/handlers/nyxbot.rs
backend/src/handlers/nyxbot_tests.rs
backend/src/handlers/telegram_new.rs
backend/src/models/telegram_bot_request.rs
backend/src/routes.rs
backend/src/services/channel_connect_link_service.rs
backend/src/services/lark_base_writes/ledger.rs
backend/src/services/login_code_service.rs
backend/src/services/nyxbot_gateway_release_service.rs
backend/src/services/nyxbot_gateway_service.rs
backend/src/services/platform_provisioning_service.rs
backend/src/services/service_pool_routing.rs
backend/src/services/telegram_manager_e2e_tests.rs
backend/src/services/telegram_manager_public_tests.rs
backend/src/services/telegram_new_api.rs
backend/src/services/telegram_new_claims.rs
backend/src/services/telegram_new_delivery_tests.rs
backend/src/services/telegram_new_prompt.rs
backend/src/services/telegram_new_recovery.rs
backend/src/services/telegram_new_service.rs
backend/src/services/telegram_new_tests.rs
docs/API.md
docs/CHANNEL_EVENT_GATEWAY.md
docs/TELEGRAM_NEW.md
docs/plans/2026-10-27-telegram-sol-evidence.md
frontend/e2e/channel-telegram-new.spec.ts
frontend/src/components/assistant/nyxbot-channels.test.tsx
frontend/src/components/assistant/nyxbot-channels.tsx
frontend/src/components/channels/telegram-new.test.tsx
frontend/src/components/channels/telegram-new.tsx
frontend/src/components/channels/telegram-prompt-status.tsx
frontend/src/hooks/use-telegram-new.ts
frontend/src/lib/assistant/nyxbot-channel-errors.ts
frontend/src/pages/channel-connect-link.tsx
frontend/src/schemas/telegram-new.test.ts
frontend/src/schemas/telegram-new.ts
```

## A-R21/A-R22 authorized narrow validation on da9662a4

Astra explicitly assigned the exclusive narrow backend window for source commit `da9662a4980b852df8543c588cc1ebe631f3bf1b`, branch `fix/telegram-onboarding-2045-1974`, in the same persistent session/worktree/settings. No production source or tests changed during this window. Initial status was clean; the later identity snapshot observed only PM-owned `telegram-status.json` being updated by Astra. Sol did not edit PM files. Historical 387-test/Clippy results above remain bound to `1b078ab9345bcd1d0eecf5e555ae8d7ee4de42ca`; no historical suite or frontend check was repeated.

Preflight found no Cargo/rustc/Clippy/linker processes and approximately15.8GiB free. Own Mongo8 PID61027 remained at `127.0.0.1:27457`, writable primary in `nyxidTelegramReview`. A uniquely named database passed an actual transactional insert/abort probe, verified the aborted row absent, then removed only that probe database. The retained Mongo process is the same PID previously launched with captured process-local soft nofile65536; current connection capacity26214 remains consistent with that inherited limit. Privileged live limit inspection was unavailable; no limit, restart, sibling Mongo or topology change was made. Open entries were396 after the two exact tests and228 at release, below the retained limit.

The existing ignored runner `.tmp/run-telegram-backend.py` was reused with its child-argv capture extended to include `clippy-driver`. It set the required target/jobs/own Mongo environment, used normal Cargo locking, started each command serially, sampled disk/owned children every5s, and would SIGINT only its owned process group below2GiB. Each invocation had a distinct log/resource/options file. Every command exited0, no guard fired, and every runner reported no surviving owned children. No cache, dependency, flags, shared profile or target changes were made.

Fresh build command from the worktree root:

```sh
CARGO_TARGET_DIR=/Users/chronoai/Desktop/projects/nyxid/target CARGO_BUILD_JOBS=2 cargo --config '/Users/chronoai/Library/Application Support/heca/worktrees/eafaaf47/arctic-eagle/docs/plans/2026-10-27-nyxid-workstreams/cargo-resource-profile.toml' test --manifest-path backend/Cargo.toml telegram_new --no-run
```

This passed exit0 in10m13s, log `.tmp/telegram-rust-r21-build.log`, producing `/Users/chronoai/Desktop/projects/nyxid/target/debug/deps/nyxid_server-a7bf57b9c4cb166a`. No old shared binary was executed before this fresh build. The first build result was sent immediately to Astra with options/log/headroom and verified child exit. Sampled build minimum free space was12,657,274,880bytes (~11.79GiB); completion free space was15,701,807,104bytes. The guard covered code generation and linking.

Fresh identity `.tmp/telegram-test-binary-r21-identity.json` binds this executable to source commit `da9662a4980b852df8543c588cc1ebe631f3bf1b`: SHA-256 `edc1de2486ad2319de2f3087abf17ec98e46194040d52276cb8e2b3cc3a157ec`, size765254528bytes, mtime_ns1793102613951993863. The1288-input backend manifest `.tmp/telegram-r21-backend-input-sha256.json` has SHA-256 `3b0badec4785596286d242df871df990ad0a96a5c231236b29b94c6f58cd624c`. Compared with the historical387 source manifest, only `telegram_new_connect.rs` and `telegram_new_tests.rs` differ. All1288 source hashes plus executable hash/mtime were checked stable after build, between exact tests, and after Clippy. A separate all-tracked snapshot `.tmp/telegram-r21-compiled-source-sha256.json` retains the broader initial file state; the backend manifest supplies the stable source binding independently of PM documentation edits.

The following direct commands ran serially with cwd `backend`, `NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27457/?directConnection=true'`, and the freshly identified executable. Direct execution of the fresh candidate preserves the previously assessed Cargo unit-test cwd while avoiding the known package-local `.git` rebuild trigger.

```sh
NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27457/?directConnection=true' /Users/chronoai/Desktop/projects/nyxid/target/debug/deps/nyxid_server-a7bf57b9c4cb166a --list
NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27457/?directConnection=true' /Users/chronoai/Desktop/projects/nyxid/target/debug/deps/nyxid_server-a7bf57b9c4cb166a services::telegram_new_tests::telegram_new_creation_webhook_completes_without_consent_or_browser_connect --exact --test-threads=2
NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27457/?directConnection=true' /Users/chronoai/Desktop/projects/nyxid/target/debug/deps/nyxid_server-a7bf57b9c4cb166a services::nyxbot_channel_query_service::tests::current_channels_rank_live_then_latest_before_limiting_and_order_ties --exact --test-threads=2
```

| Command | Actual result | Log |
| --- | --- | --- |
| Compiled inventory | Exit0;10472tests,0benchmarks; each requested exact name appears once | `.tmp/telegram-rust-r21-list.log`, `.tmp/telegram-r21-exact-inventory.json` |
| A-R21 registration completion | Exit0;1passed,0failed,0ignored,0measured;10471filtered;1.29s | `.tmp/telegram-rust-r21-completion.log` |
| A-R22 current-row ranking | Exit0;1passed,0failed,0ignored,0measured;10471filtered;1.99s | `.tmp/telegram-rust-r21-current-rows.log` |

These are two actual executed regressions with no fixture skips or zero-match passes. A-R21 now has execution proof for registration/agent-prerequisite/existing-setup/first-message wording and retained lifecycle/button/recipient assertions. A-R22 supplies the previously omitted G2 visible current-row execution: live precedence, per-owner/per-bot winner, failed history, ties, disconnection and50limit. Its scope remains visible-row selection, not remote duplicate-create prevention or live Telegram reconnect behavior.

Required final lint command from the worktree root, with config after the external Clippy subcommand:

```sh
CARGO_TARGET_DIR=/Users/chronoai/Desktop/projects/nyxid/target CARGO_BUILD_JOBS=2 cargo clippy --config '/Users/chronoai/Library/Application Support/heca/worktrees/eafaaf47/arctic-eagle/docs/plans/2026-10-27-nyxid-workstreams/cargo-resource-profile.toml' --manifest-path backend/Cargo.toml --all-targets -- -D warnings
```

Clippy passed exit0 in6m06s with no diagnostics, log `.tmp/telegram-rust-r21-clippy.log`; no allowances, suppressions or reduced checks. Actual backend test rustc and binary/test clippy-driver argv are captured in `.tmp/telegram-rust-r21-build-rustc-options.log` and `.tmp/telegram-rust-r21-clippy-rustc-options.log`, with checked projection `.tmp/telegram-r21-compiler-options-summary.json`. All three invocations have no debuginfo/incremental/opt-level/debug-assertions/overflow-checks overrides: rustc defaults are debug0, no incremental, opt0, enabled debug assertions and enabled overflow checks. Selected `-C` arguments are only embed-bitcode=no, metadata, extra-filename and the existing Swift rpath. The workspace summary's `+ debuginfo` still reflects unchanged dependencies; actual backend argv proves the package override.

Final machine summary `.tmp/telegram-r21-validation-summary.json` records exact results, logs, stable source/binary identity and resource samples. After Clippy, an independent process scan found no Cargo/cargo-clippy/rustc/clippy-driver/cc/ld/clang; Mongo remained writable primary,228open entries, free16,760,188,928bytes (~15.61GiB). Astra received the complete results and explicit window release immediately, before this evidence update. No more heavy commands are queued. Only this Sol evidence file was edited for the handoff; source/tests/PM files were untouched.

`LOCAL_VALIDATION_COMPLETE=true` is limited to this authorized A-R21/A-R22 follow-up. Astra owns exact-commit review and per-ticket sign-off. Native L1–L3 and I-07 remain OPEN, both tickets/final approval remain BLOCKED, and no live Telegram creation, message, reply, merge, deployment, push, PR or issue closure is claimed.
