# Telegram ticket sign-off — Astra personal adversarial review

Review checkpoint: **2026-10-27 12:16 UTC** (narrow local validation complete; historical checkpoints retained in the ledger). Reviewer: **Astra, accountable Telegram PM**, Heca agent `01a17a9b-2cf3-79e3-9177-1aa759f5eb61`. This is the PM's review record, separate from the [Sol implementation evidence](2026-10-27-telegram-sol-evidence.md).

| Ticket | Explicit verdict | Reason |
| --- | --- | --- |
| [#2045 — Create bot button missing after NyxID handoff and /start retry](https://github.com/ChronoAIProject/NyxID/issues/2045) | **BLOCKED** | Local diagnostics, recovery and authority tests pass, but native rendering/form/event/actual reply acceptance L1–L3 is unexecuted. A-R21 wording correction and its exact regression now pass on the published candidate. Combined router acceptance I-07 is open. |
| [#1974 — Verified Personal Bot cannot connect to a specialist: gateway_not_authenticated](https://github.com/ChronoAIProject/NyxID/issues/1974) | **BLOCKED** | Local creator/gateway/owner/reconnect tests pass. A-R21 completion regression and A-R22 current-row regression now pass on the published candidate. Actual deployed gateway connection, owner link and reply are unproven, as is I-07. |

**ASTRA_IMPLEMENTATION_APPROVED has not been issued.** The user replaced both Opus gates with the pure Astra–Sol workflow. They remain **SUPERSEDED_BY_USER**, never passed. No external adversarial reviewer sign-off exists. The historical Opus agent `01a17aa3-1dae-7e80-b8c0-c4fd9a511f2b`, session `01a17aa3-1daf-7a31-9ba3-c86a3fece909`, failed authentication before any review; this is preserved in the [review ledger](2026-10-27-telegram-review.md).

## Current review stages

| Stage | Exact scope and result |
| --- | --- |
| **SOURCE_REVIEW_COMPLETE** | Astra's prior full review of feature `1b078ab9345bcd1d0eecf5e555ae8d7ee4de42ca`, plus the current two-file A-R21 patch and surrounding notification paths. No additional production defect found in this follow-up. |
| **LOCAL_VALIDATION_COMPLETE=true** | Published candidate `da9662a4980b852df8543c588cc1ebe631f3bf1b`: fresh build, both exact regressions and required all-target warnings-denied Clippy passed. Unchanged earlier evidence is carried forward with its original commit identity; the historical 387-test batch was not repeated. |
| **Final ticket approval: BLOCKED** | Both #2045 and #1974 also require native L1–L3 and combined production-router I-07 evidence. No `ASTRA_IMPLEMENTATION_APPROVED`. |

The coordinator-authorized narrow window is complete and released after actual exit/guard/child-quiescence verification; Permissions now owns the slot. No Telegram heavy command is started or queued. The shared executable may be overwritten by another stream; the archived source/binary identity records bind this completed run. Draft PR [#2057](https://github.com/ChronoAIProject/NyxID/pull/2057) remains blocked for final acceptance.

## Exact reviewed identities

| Identity | Value |
| --- | --- |
| Common base | `b1dd6548ade04ec4f0051203e1353796331c7f87` |
| Isolated prerequisite | `336405f8e667ea2eb2c6513effec3d15fd960b02` — exactly four equivalent boolean-guard rewrites, eight additions/eight removals |
| Historical full-batch implementation | `1b078ab9345bcd1d0eecf5e555ae8d7ee4de42ca` — parent is the prerequisite; 387 unique passing tests and earlier Clippy proof |
| Current reviewed and locally validated implementation | `da9662a4980b852df8543c588cc1ebe631f3bf1b` — two-file A-R21 correction plus Sol evidence; fresh build/two exact tests/required Clippy passed |
| Approved plan | [v6](2026-10-27-telegram-implementation.md), SHA-256 `7cebb035bde823e2a78290393ec2f61cc982fb8a38d6ac3112abacb2f05edec7`; **ASTRA_PLAN_APPROVED** |
| Worktree / branch | `01a17a96-885c-7042-bf1c-6e33531a2117` / `fix/telegram-onboarding-2045-1974` |
| Sole implementer | Heca `01a17afc-03c3-74f3-8206-1235e8b89954`; session `01a17afc-03c3-74f3-8206-125edb599f7c`; actual `codex / gpt-6.1-sol / x_high / full-access` verified |
| Historical full-batch executable | `nyxid_server-a7bf57b9c4cb166a`; SHA-256 `1060f46cade1f49edbd68242d003eab3cb90f52609a12b0fb5eaaedf412f9ee1`; 757,234,992 bytes; mtime_ns `1793076008793457273` |
| Current narrow-batch executable | Same target filename; SHA-256 `edc1de2486ad2319de2f3087abf17ec98e46194040d52276cb8e2b3cc3a157ec`; 765,254,528 bytes; mtime_ns `1793102613951993863` |

Prior PM/evidence documentation commits `24827c641c755c79f5d1562a3242a8f5d509d3fb` and `a71492b9cb347ecbc28e77bf8c73a16e1921ec8e` did not change implementation. A-R21 was then committed as the current published candidate above, without source expansion. This follow-up updates evidence and PM records only. Astra compared all 1,288 fresh backend inputs to that exact commit and the working source; only `telegram_new_connect.rs` and `telegram_new_tests.rs` differ from the historical full-batch input manifest. Their reviewed hashes match. The 387-test proof remains bound to its original commit; current correction proof is the separately archived narrow batch.

For historical implementation `1b078ab9345bcd1d0eecf5e555ae8d7ee4de42ca`, Astra personally read the full candidate diff and relevant surrounding code throughout the implementation iterations, sent every accepted finding to the same Sol, and rechecked the final commit against the personal-review file fingerprints: all 35 recorded source/domain-document fingerprints match. The additional Sol evidence document was read separately. Astra independently compared **all 1,288 captured build inputs to the exact Git commit**, with zero differences, and reconciled every individual final test result. Final adversarial reinspection covered the transaction error conversion/ACK boundary, phase lease takeover, exact hosted provenance, gateway error layers, actual HTTP error envelope, actor-scoped browser cache, route mounting and operator rollout/rollback instructions. That last review found A-R21 and A-R22 below; green tests were not treated as sufficient acceptance.

## #2045 criteria and evidence

The [archived issue snapshot](2026-10-27-telegram-evidence/issue-2045.json) had no comments when captured for the original review. The issue records Telegram for macOS **11.5 (`ru.keepcoder.Telegram`)** as one observed reproduction, with a separate screenshot whose client version is unverified. The inspected historical deployment was `0.73.1 / d102570afaab`; that is not the current candidate deployment.

| Issue / plan criterion | Evidence and challenge | State |
| --- | --- | --- |
| Expose keyboard delivery outcome separately from waiting for creation; retain only sanitized diagnostics (T1–T2). | `telegram_new` 94/94: first/second sends, rejection vs uncertainty, valid message ID, 400/401/403/429/503 and nonstandard status normalization, timeout/interrupted/oversized/malformed responses, redirect refusal, raw-payload/token sentinels absent from DTO/Debug/logs. Frontend optional legacy DTO and accepted/rejected/unknown cases pass. | Local PASS on the reviewed commit. |
| `/start` and recovery preserve the same owner, assistant and source; expiry/revocation/races must fail safely (T3–T5). | Exact live actor/admin/manager, link creator vs request initiator, watch/agent/source and request-generation transaction fences; explicit standalone vs absent legacy context; actual creation consumption CAS; renew each dispatch phase and reserve the full provider timeout. Tests cover deleted/replaced/retargeted rows, stale observations, source/group identity, org membership loss, duplicate completion and one local bot/token/webhook effect. | Local PASS. L3 still OPEN. |
| Provide an actionable native form alternative without arbitrary redirects or broader authority (T4, T6–T7). | Human-only empty-body POST, shared five-per-minute actor limit, private response headers, bounded t.me URL builder, no request/target/redirect input. Web fixtures exercise popup blocking, late results, account switch, expiry/completion, keyboard action and source return at 390/1440 pixels. | Local PASS; actual native form/event semantics OPEN. |
| Verify native Create bot rendering and creation on named supported clients; API mocks alone are insufficient (L1). | No candidate client run performed. Bot API acceptance proves neither rendering nor creation. The swallowed keyboard error/state gap is confirmed; the blank bubble's root cause and client compatibility remain unproven. No unsupported-client upgrade claim is made. | **BLOCKED**. |
| Complete the journey with the original specialist, owner link and actual reply (L2). | Registration, webhook, local gateway and owner fixtures pass. No actual child-bot message/reply has occurred. A-R21 correction is source-reviewed and its exact completion regression passes; actual reply acceptance is separate. | **BLOCKED**. |

The native link is Telegram's documented `https://t.me/newbot/{manager}/{suggested_username}?name=...` contract, sourced in the approved plan from [Telegram managed-bot creation request links](https://core.telegram.org/api/links#managed-bot-creation-request-links), [managed bots](https://core.telegram.org/api/bots/managed-bots), and [ManagedBotCreated](https://core.telegram.org/bots/api#managedbotcreated). It carries no NyxID request ID and cannot be revoked remotely. Issuance is not creation proof. NyxID still requires the existing authenticated private `managed_bot_created` event with the bound sender, update/date/manager observation and live authority. A deliberate second remote form submission may create an extra Telegram bot; the local one-consumer claim does not promise exactly-once remote creation. Actual direct-link event binding remains an acceptance gate.

## #1974 criteria and comment evidence

The [archived issue and both comments](2026-10-27-telegram-evidence/issue-1974.json) were reread for this review. The original delegated-token analysis is explicitly provisional. #1984 creator-token/grant work and #1991 release/continuity behavior already exist in the base and must be preserved; this work does not claim to newly implement them or to establish the historical production root cause.

| Issue / comment / plan criterion | Evidence and challenge | State |
| --- | --- | --- |
| Personal owner can create the gateway connection without broad CMA roles; preserve exact registered confidential client, token lifetime, actor and live authority (G1). | Gateway 16/16 includes all six creator/introspection tests: ordinary owner, exact eligible confidential client/catalog association, unrelated tokens/clients, reserved actor and invalid lifetime boundaries. Telegram and handler suites cover live owner/admin loss and intended destination. | Local PASS; actual deployed contract/versions OPEN. |
| Distinguish local configuration, authentication, permission and availability; provide useful reconnect/tool/watch guidance (G2). | The real hosted fixture reaches gateway POSTs separately from tool POSTs for missing config/401/403/503; stored safe code, emitted watch guidance, direct-tool error and sentinel exclusion all pass. Frontend 46/46 covers actual `ApiError` validation prefix and ordinary/unknown messages. | Local PASS. The incident traces have not been correlated with real upstream logs. |
| Official `telegram-new` flow must distinguish bot registration from specialist setup and verified replies — [comment 6310623341](https://github.com/ChronoAIProject/NyxID/issues/1974#issuecomment-6310623341). | Watched completion says a reply is unverified. A-R21 now separates registration from the agent-link prerequisite, sends an existing specialist flow back to its setup, and requests first-message verification. Astra accepted the exact two-file source correction after reviewing all notification paths. | Local PASS on `da9662a4`; actual reply remains separate. |
| Owner-not-linked retry must preserve correct connection, pairing and current-row presentation — [comment 6311869799](https://github.com/ChronoAIProject/NyxID/issues/1974#issuecomment-6311869799). | Release/continuity suite includes explicit reconnect with a fresh owner link and exact lineage. Existing current-row query ranks live rows over historical failures and hides superseded disconnections. Its dedicated A-R22 exact test now passes 1/1 on the published candidate after the earlier omission was identified. The comment did not capture connection IDs and does not prove duplicate database records. | Local PASS; actual owner-link acceptance **OPEN**. |
| Same intended specialist receives the owner's message and replies in the Telegram chat (L2). | Webhook registration, Active status, inbound activity and gateway row status do not meet this criterion. No live reply proof exists. | **BLOCKED**. |
| Preserve rollback, partial setup, reconnect and release boundaries (G3). | Handler 219/219 includes 32 release and seven continuity tests. Platform provisioning 12/12 and hosted-link 19/19 pass. Four-file prerequisite release rewrite is truth-equivalent and personally reviewed. | Local PASS. |

## Historical full-batch checks and durable evidence

[Evidence manifest](2026-10-27-telegram-evidence/manifest.json) records hashes and original paths for archived logs, issue snapshots, binary identity, compiler options and the source manifest. Archived logs normalize only trailing whitespace and extra final blank lines; the manifest retains both original and archived hashes, and raw originals remain under `.tmp`. Earlier failed/interrupted attempts remain in the local `.tmp` logs and the chronological review/Sol evidence records; they are not counted as passes. No live action is represented by the local fixture logs.

For historical implementation `1b078ab9345bcd1d0eecf5e555ae8d7ee4de42ca`, the final compilation command from the worktree root was:

```sh
CARGO_TARGET_DIR=/Users/chronoai/Desktop/projects/nyxid/target \
CARGO_BUILD_JOBS=2 \
cargo --config '/Users/chronoai/Library/Application Support/heca/worktrees/eafaaf47/arctic-eagle/docs/plans/2026-10-27-nyxid-workstreams/cargo-resource-profile.toml' \
test --manifest-path backend/Cargo.toml telegram_new --no-run
```

**PASS, exit 0, 9m41s** — [build log](2026-10-27-telegram-evidence/telegram-rust-build-final.log). Required lint command:

```sh
CARGO_TARGET_DIR=/Users/chronoai/Desktop/projects/nyxid/target \
CARGO_BUILD_JOBS=2 \
cargo clippy --config '/Users/chronoai/Library/Application Support/heca/worktrees/eafaaf47/arctic-eagle/docs/plans/2026-10-27-nyxid-workstreams/cargo-resource-profile.toml' \
--manifest-path backend/Cargo.toml --all-targets -- -D warnings
```

**PASS, exit 0, 6m22s** — [Clippy log](2026-10-27-telegram-evidence/telegram-rust-clippy-corrected-profile.log). The coordinator-approved package-only dev/test configuration sets `nyxid` debug=0 and incremental=false. Captured actual compiler arguments confirm it, retaining default optimization, debug assertions and overflow checks; dependencies and test scope were unchanged. The generic Cargo summary's “+ debuginfo” is not the package's actual rustc setting. No lint allowance or test-only authority bypass was introduced.

The existing build scripts unnecessarily rebuild unchanged source when Cargo lists/runs tests in this worktree. Coordinator and Astra approved direct execution of the successfully compiled executable, with source/binary identity checked before each filter. At `backend/` cwd, each table filter was run serially as:

```sh
NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27457/?directConnection=true' \
/Users/chronoai/Desktop/projects/nyxid/target/debug/deps/nyxid_server-a7bf57b9c4cb166a \
'<filter from table>' --test-threads=2
```

The final exact pool-node filter also uses `--exact`. The fixture MongoDB is 8.0.17, replica set `nyxidTelegramReview`, distinct port 27457 and test-local databases. Transaction tests ran, with no skipped fixture paths or ignored tests. The compiled inventory contains 10,472 tests; that is an inventory count, not an execution claim.

| Executed filter | Passed | Seconds | Log |
| --- | ---: | ---: | --- |
| `telegram_new` | 94 | 87.73 | [log](2026-10-27-telegram-evidence/telegram-rust-final-telegram-new.log) |
| `telegram_manager` | 22 | 16.43 | [log](2026-10-27-telegram-evidence/telegram-rust-final-manager.log) |
| `channel_connect_link` | 19 | 2.99 | [log](2026-10-27-telegram-evidence/telegram-rust-final-hosted.log) |
| `platform_provisioning` | 12 | 31.33 | [log](2026-10-27-telegram-evidence/telegram-rust-final-provisioning.log) |
| `nyxbot_gateway` | 16 | 23.25 | [log](2026-10-27-telegram-evidence/telegram-rust-final-gateway.log) |
| `handlers::nyxbot::tests` | 219 | 620.75 | [log](2026-10-27-telegram-evidence/telegram-rust-final-handlers.log) |
| `login_code_service` | 10 | 172.96 | [log](2026-10-27-telegram-evidence/telegram-rust-final-login.log) |
| `lark_base_writes` | 15 | 10.28 | [log](2026-10-27-telegram-evidence/telegram-rust-final-lark.log) |
| `service_pool_routing` | 1 | 0.20 | [log](2026-10-27-telegram-evidence/telegram-rust-final-pool.log) |
| `handlers::service_pool_proxy_tests::runtime::pool_proxy_excluded_node_member_is_skipped_before_credential_materialization` | 1 | 22.51 | [log](2026-10-27-telegram-evidence/telegram-rust-final-pool-node-exclusion.log) |

Every command exited 0, with 0 failures/ignored/measured. **409 executions reconcile to 387 unique passing tests**, including 360 unique main-workstream tests and 27 prerequisite regressions. The six creator, 32 release and seven continuity tests are subsets. The separate exact hosted diagnostic 1/1 run is also included in the handler suite and is not added again.

Frontend checks ran in `frontend/`:

- `npm test -- src/components/channels/telegram-new.test.tsx src/components/channels/telegram-new-connect.test.tsx src/components/channels/telegram-new-resume.test.tsx src/pages/channel-connect-link.test.tsx src/components/assistant/nyxbot-channels.test.tsx src/schemas/telegram-new.test.ts` — 81/81. Subsequent affected groups passed 32/32, 46/46 after the actual HTTP-envelope fix, and 31/31 after UI cleanup. These counts overlap. Logs are archived in the evidence directory.
- `npx tsc -b`, targeted edited-file ESLint with zero warnings, and `npm run build` — exit 0. The build includes typechecking, main/credential-accept bundles and mock-footprint assertion. The log retains nonfatal Node deprecation, existing CSS optimization and bundle-size warnings; build success is not a claim of a warning-free Vite run.
- `npx playwright test --config ../.tmp/telegram-playwright.config.ts channel-telegram-new.spec.ts` — 18/18; added hosted/full-page cases 4/4; final affected `--grep 'recovery'` cases 6/6. Astra personally ran the six recovery cases: 6/6 in 11.4s ([log](2026-10-27-telegram-evidence/telegram-astra-ui.log)). Chromium drove the local app at port 4317 at 390/1440 pixels with API fixtures and intercepted t.me navigation. Screenshots remain under `.tmp/telegram-ui/`; Astra inspected them. This is browser UI evidence, not native Telegram acceptance.
- `cargo fmt --manifest-path backend/Cargo.toml --all --check` and `git diff --check` — exit 0 before the candidate commits; archived logs. No production deployment or side effect was involved.

## Current candidate narrow validation

The [follow-up manifest](2026-10-27-telegram-evidence/followup-r21/manifest.json) archives 16 artifacts with raw/archived hashes. [Astra source verification](2026-10-27-telegram-evidence/followup-r21/telegram-r21-astra-source-verification.json) independently matches all 1,288 backend inputs to `da9662a4980b852df8543c588cc1ebe631f3bf1b`. [Binary identity](2026-10-27-telegram-evidence/followup-r21/telegram-test-binary-r21-identity.json), [exact inventory](2026-10-27-telegram-evidence/followup-r21/telegram-r21-exact-inventory.json) and [validation summary](2026-10-27-telegram-evidence/followup-r21/telegram-r21-validation-summary.json) preserve the source binding, exits, resources and Mongo readiness. The inventory still contains 10,472 tests; only the requested two exact cases were executed in this batch.

| Required current-source check | Actual outcome | Evidence |
| --- | --- | --- |
| Same shared-config `cargo test --manifest-path backend/Cargo.toml telegram_new --no-run` | Exit 0, 10m13s; no compiler diagnostics | [Build log](2026-10-27-telegram-evidence/followup-r21/telegram-rust-r21-build.log) |
| `services::telegram_new_tests::telegram_new_creation_webhook_completes_without_consent_or_browser_connect --exact --test-threads=2` | 1 passed, 0 failed/ignored/measured; exit 0, 1.29s | [Completion log](2026-10-27-telegram-evidence/followup-r21/telegram-rust-r21-completion.log) |
| `services::nyxbot_channel_query_service::tests::current_channels_rank_live_then_latest_before_limiting_and_order_ties --exact --test-threads=2` | 1 passed, 0 failed/ignored/measured; exit 0, 1.99s | [Current-row log](2026-10-27-telegram-evidence/followup-r21/telegram-rust-r21-current-rows.log) |
| `cargo clippy --config SHARED_FILE --manifest-path backend/Cargo.toml --all-targets -- -D warnings` | Exit 0, 6m06s; no diagnostics | [Clippy log](2026-10-27-telegram-evidence/followup-r21/telegram-rust-r21-clippy.log) |

Build and Clippy used the exact absolute shared config and `CARGO_TARGET_DIR=/Users/chronoai/Desktop/projects/nyxid/target CARGO_BUILD_JOBS=2` commands shown in the historical section; config follows `clippy` for its external wrapper. Both tests directly executed the fresh pinned `nyxid_server-a7bf57b9c4cb166a` from the worktree's `backend/` directory, with `NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27457/?directConnection=true'`, the full exact names above, `--exact --test-threads=2`. Each name appeared exactly once in the fresh inventory. The existing Mongo8 replica passed a unique insert/abort transaction probe; no fixture was skipped.

Astra personally inspected the actual build/test/lint results, live child command/config forwarding and captured compiler arguments, and independently hashed the fresh binary. The [compiler options](2026-10-27-telegram-evidence/followup-r21/telegram-r21-compiler-options-summary.json) confirm package debug0/no incremental with default opt0/assertions/overflow for the test build and both Clippy targets. The source and binary remained stable through checks. The guard never triggered; minimum build headroom was 12,657,274,880 bytes, and the final owned children lists were empty. Astra's separate release scan found no Cargo/compiler/test/linker processes and 16,760,160,256 bytes free; Mongo remained healthy. The coordinator accepted release immediately and assigned Permissions before report bookkeeping.

**A-R21 and A-R22 are resolved locally. SOURCE_REVIEW_COMPLETE and LOCAL_VALIDATION_COMPLETE=true apply to this exact candidate**, using the current narrow proof plus unchanged earlier evidence at its recorded identities. No historical 387-test or frontend suite was repeated. Both ticket verdicts remain BLOCKED for native L1–L3 and combined I-07; this is not `ASTRA_IMPLEMENTATION_APPROVED`.

## Finding dispositions

The [chronological ledger](2026-10-27-telegram-review.md) preserves the reasoning, unsuccessful attempts and prior plan changes. “Resolved” here means resolved on the identified local candidate, not issue closure or live acceptance.

| Finding | Resolution / evidence | State |
| --- | --- | --- |
| A-R01 | Exact hosted origin checked at retry and again before keyboard; deleted/replaced/retargeted authority and between-phase revocation fixtures pass. | Resolved |
| A-R02 | Legacy absent/zero connection generation and optional BSON metadata remain readable and usable on the legacy path. | Resolved |
| A-R03 | Closed gateway codes survive inner connect and actual hosted watch; configuration/401/403/503 transport assertions pass. | Resolved |
| A-R04 | UI separates sending, rejected, unknown and API-accepted states; no inferred render/creation success. | Resolved |
| A-R05 | Late recovery result checks current cache/request/revision/actor/expiry and unmount; stale links disappear. | Resolved |
| A-R06 | Closed authority is intentionally ACKed; raw DB/admission/commit failures retain retry semantics. Actual validator and transaction-contention tests pass. | Resolved |
| A-R07 | Modern fresh-event consumption fences live authority/context/request in one transaction; deletion, expiry, stale CAS and concurrent completion tests pass. | Resolved |
| A-R08 | Hosted headings use Telegram display name and retain exact source return; browser evidence inspected. | Resolved |
| A-R09 | Hosted link creator A is distinct from initiating admin B; successful B flow, foreign actor and membership-loss regressions pass. | Resolved |
| A-R10 | Operator docs require old-writer drain/all-replica upgrade and settlement of modern pending/uncertain work before rollback. | Resolved by personal document review |
| A-R11 | Exact bare/prefixed API gateway code translation; ordinary prose is preserved; actual watch/tool and 46 frontend cases pass. | Resolved |
| A-R12 | Renewed 45s phase lease, full 20s dispatch budget, no expired revival, exact observed timestamp/phase/outcome CAS; deterministic real-Mongo stale-observation tests pass. | Resolved |
| A-R13 | Nonstandard HTTP status normalized before persistence/DTO; 600/700 response and read/recovery tests pass. | Resolved |
| A-R14 | Removed unreachable connecting UI branches; existing 31 tests, lint/typecheck/build pass. | Resolved |
| A-R15 | WireMock scoped priority/index fixture defect fixed with one-shot rejection; same update and exact instructions/instructions/keyboard sequence retained and passed. | Resolved |
| A-R16 | Real org conflict traced through bounded command evidence to update/users/owner_org_fence, Mongo112/TransientTransactionError; only that correlated exact envelope permits identical-event redelivery. Final200/Ready/Connected and unique effects pass; no production transaction weakening. | Resolved |
| A-R17 | Public welcome test asserts intended useful /start guidance instead of an unproven latest-client remedy. | Resolved |
| A-R18 | Optional boxed watch reduces enum size without BSON shape change; clippy and real raw/hosted/legacy persistence tests pass. | Resolved |
| A-R19 | Four baseline boolean rewrites isolated in prerequisite `336405f8e667ea2eb2c6513effec3d15fd960b02`; personal exact diff/surrounding review, 27 relevant tests and release32/Clippy pass. No change to evaluation order, None or malformed UUID semantics. | **Prerequisite APPROVED** for coordinator-authorized peer import |
| A-R20 | Interrupted-body TCP fixture checks actual nonzero read count; uncertainty/redaction assertions and warnings-denied lint pass. | Resolved |
| A-R21 | Exact two-file correction separates registration, existing specialist setup, agent linking and actual first-reply verification; Astra reviewed the surrounding standalone/hosted/installation notification paths and existing assertion. Buttons, recipient and lifecycle remain unchanged. | Resolved on `da9662a4`: exact completion 1/1, build and Clippy pass |
| A-R22 | Existing exact current-row ranking/history test was omitted from the original filters; now executed once on the fresh candidate, 1 passed/0 failed/0 ignored. No new test or production change needed. | Resolved on `da9662a4` |

The initial hosted diagnostic failure occurred before its assertions because the fixture used a mismatched platform, and the next failure was an invalid fixture issuer rejected before the gateway POST. Both were corrected to exercise the actual supported flow. The earlier suggestion that the issuer-stage failure proved a production diagnostic defect was retracted after tracing; the final actual transport assertions passed.

## Focused source review and exact remaining proof

A-R21 reviewed source identities (committed and locally validated in `da9662a4980b852df8543c588cc1ebe631f3bf1b`):

- `backend/src/services/telegram_new_connect.rs`: SHA-256 `e2518da7899d5df5ca37dc895dfb918b54b05ef370e4189ee335e0f1d0b4418a`.
- `backend/src/services/telegram_new_tests.rs`: SHA-256 `e4b87725278874aedfed2d385d1383c5c2bc833bea0a71dde441455d1a22761c`.

Astra reread the official-flow comment 6310623341 against `finish_request` and the surrounding connection finalization. Bot activation and webhook registration precede asynchronous specialist watch completion. The revised registration message therefore describes only registration, makes the agent prerequisite explicit, and directs a user with an existing specialist setup back to that setup instead of initiating another link. Its existing test selects the notification by the unchanged **Open your bot** button and retains recipient, URL, token/webhook and lifecycle assertions. `channel_connect_link_service::complete` and `nyxbot::channel_bot_watch` were also traced: the owner-linked success explicitly says the reply is unverified; the other success gives the owner-verification step; failure distinguishes a created bot from failed linking and retains bounded gateway guidance. Installation authority returns through the separate `ready_for_test` notice. No additional source correction is required by that trace.

For A-R22, Astra read `nyxbot_channel_query_service::list_current` and the complete existing test, then traced `nyxbot::list`, the REST/tool listing and the frontend query/component. The test demonstrates owner filtering, one visible row per bot, a live connection winning over 80 newer failures, pending visibility, latest-failure/tie order, later disconnection suppressing a failed row, and a 50-row limit after grouping. It also preserves the unrelated index. Its deliberate assertion of 87 persisted history rows proves that the projection does not delete history; it does **not** prove absence of historical records or remote duplicate bot creation. The previously passing explicit-reconnect continuity fixture uses the shared Lark gateway path, proving fresh pairing and exact lineage there; it is not native Telegram pairing proof. A-R22 is adequate for the cited visible-current-row criterion and its exact test now passes. No new test or production rewrite is needed.

Native L1–L3 remain distinct from local source correctness. The recovery handler accepts only a first-party human session/access token without an acting client, an empty body, and the shared actor rate limit; response headers are private. The service checks the initiating actor, live owner/manager, exact hosted context and request transaction before issuing the fixed official URL. Actual consumption still requires the authenticated private `managed_bot_created` event with bound sender, fresh update/date/manager observation and live context CAS. Neither browser URL issuance nor a crafted local event proves that a named Telegram client renders the form or emits the same real event. Registration/owner fixtures do not demonstrate the selected specialist's actual reply. These are concrete missing observations, not newly established authentication defects. The one-bot native runbook remains unchanged.

### I-07: missing production-router integration evidence

Astra and the coordinator independently confirmed this **integration test gap**, with no newly established runtime defect:

- Telegram's passing `telegram_new_creation_recovery_http_authentication_and_middleware_boundary` uses `telegram_manager_e2e_tests::start_http`, which manually mounts the recovery route and its guards. It cannot detect that route disappearing from `routes.rs`.
- Artifacts' `generated_artifacts_materialize_history_owner_http_ranges_recovery_and_retention` calls `generated_artifacts::http::owner_response` directly. It covers owner/range/retention behavior below the router, not the production mount or real authentication composition.
- The separately inspected production mounts are present. Telegram recovery joins the human-only API router; Artifacts nests its recovery/download routes under `/assistant`. Existing delegated-read denial covers `assistant`, `channel-bots` and `assistant-attachments`; no speculative deny-list edit is needed.

The smallest combined fixture should reuse existing prepared request/artifact data and real signed or persisted credentials, instantiate `crate::routes::build_router_with_state(state.clone())`, use the returned private router with `private.with_state(state)`, and dispatch through `tower::ServiceExt::oneshot`. Existing `routes/remote_machine_tests.rs` demonstrates that seam. Hand-mounted substitute routes, injected `AuthUser`, or direct handler/helper calls cannot satisfy I-07.

| Production path on the exact combined commit | Required successful-owner proof | Applicable negative/effect proof |
| --- | --- | --- |
| `POST /api/v1/channel-bots/telegram-new/requests/{id}/creation-link` | Empty-body request with real first-party credentials returns the same request ID and bounded official creation URL, with `no-store`/`no-referrer`; issuance alone does not create a bot or replace the request. | Anonymous, foreign initiator, expired request, delegated/API-key/service/relay and acting-client cases retain their existing rejection contracts. No creation URL or provider effect is exposed on denial. Preserve invalid-body and rate-limit behavior already tested below the router. |
| `GET /api/v1/assistant/nyxagent/conversations/{id}/attachments/{attachment_id}` | Real intended-owner request reaches the mounted handler and returns the exact stored fixture bytes and applicable content/range headers; a generic 2xx or non-404 is insufficient. | Unauthorized/delegated-read/foreign-owner and retention-expired cases retain their existing status/envelope contracts, with no artifact bytes disclosed. API-key/service/relay restrictions remain effective at the human-only boundary. |

The companion artifact recovery POST was also source-inspected; its recovery/idempotency behavior remains under the existing Artifacts and shared integration criteria. It is not an additional Telegram I-07 gate.

Use strict Mongo fixture readiness and fail clearly if the database is unavailable. Seed denial cases with otherwise valid fixture identifiers so authorization is actually exercised. Assert each path's established response status and bounded envelope; a catch-all 404, engine-disabled response or invalid credential cannot substitute for an owner-success case. Reuse existing range/retention/recovery assertions and add only the missing router composition coverage. The shared I-07 contract remains authoritative; this matrix makes its missing proof concrete. Both input candidates must first be combined by the coordinator under the existing integration authorization. No sibling feature import or speculative combined code is included in this review.

## Smallest remaining actions

1. **Combined I-07:** after coordinator-authorized integration, mount this exact Telegram candidate with the Artifacts candidate, preserving both route sets and authentication/ownership/expiry/delegated-read boundaries. Use the actual production router with real signed/persisted credentials and the payload/rejection matrix above on that integration commit; both existing PMs review the result. The current manually mounted/direct-helper tests are inputs, not combined route proof. No feature-branch import is authorized merely by this checklist. See the shared `2026-10-27-nyxid-workstreams/integration-acceptance.md`, row I-07.
2. **External L1–L3:** the corrected candidate is now locally validated. Select a candidate-running staging/test destination, one existing disposable personal owner and intended specialist/source, one owner-controlled Telegram account with exact app/OS/version, and an existing dedicated capable manager plus actual gateway/introspection/version evidence. The minimum separately authorized effects are one disposable managed bot, at most two manager setup `/start` commands, one optional owner-link action, and one harmless unique private prompt with an actual received reply. If the keyboard renders, open/cancel its form, then create once through the same-request recovery so both paths are observed without two bots. Record rendering, form, authenticated fresh event, connection, owner link and reply separately. The detailed sequence and metadata-redaction rules are in the review ledger's live runbook. No deployment or production action has been authorized, and this report does not request secrets.

The local correction and validation are complete. The native destination/client and combined integration remain concrete acceptance dependencies. Neither ticket can receive **APPROVED** or `ASTRA_IMPLEMENTATION_APPROVED` until all applicable items above are demonstrated against the final exact candidate.
