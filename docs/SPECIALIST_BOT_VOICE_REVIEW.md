# Specialist voice: personal review and verification

Baseline: `616a9131c438f4647599f442fd7632eec5cc10aa` (NyxID 0.71.0).
Branch: `fix-specialist-bot-voice`. Review date: 2026-10-15.

Implementation used the requested Heca session with `gpt-6.1-sol`, `x_high`,
and full access. The primary assistant performed the code and adversarial
review personally and sent corrections to that same implementation session.
No reviewer subagents or additional implementation agents were used.

## Findings and resolutions

| Finding | Resolution and evidence |
| --- | --- |
| Platform voice incorrectly required the catalog service in the specialist's tool grants. | The voice-only inference exception now requires exact live private-conversation authority, while allowing platform audio without another service grant or connection. The regression checks empty grants, successful resolution, no new connection, unchanged key contents, and continued refusal of ungranted tool use. |
| BYOK compared a catalog ID against connection grants. | Authorization checks the selected `UserService.id`. A granted connection resolves; an ungranted or subsequently removed connection fails. |
| OpenAI discovery could advertise choices that admission refused, and Grok's catalog-only check could exclude an independently granted connection. | Both providers use source-specific operation checks. The selected connection also uses the same read-only credential/origin snapshot as startup. Tests exercise denied siblings, invalid origins, revoked/missing credentials and propagated data-integrity errors. |
| A broad model-transport exception could accept an ordinary, unbound or different-thread key. | A private `InferenceAuthority` value can be constructed only after normal authentication and exact user/conversation/agent/key matching. Tests include ordinary keys, removed credential bindings, different threads, stale agent snapshots, guest threads, revocation, expiry and destroyed agents. Drafts need no active work turn. |
| OpenAI's internal sideband omitted the existing B1 WebSocket restriction. | Both adapters now enforce it, as explicitly required by `docs/chat/11-voice.md`. An HTTP-only create grant does not authorize a scoped live channel. A separate unscoped granted connection remains usable. |
| The first implementation repeated key, conversation and organization authentication for every source. | Discovery loads bound authority once per request and reuses it before model expansion. A database command observer verifies one key read and one durable binding read across two models and three sources. Startup and live revalidation continue loading fresh authority. |

The review traced callers and neighboring implementations in the voice runtime,
Grok runtime, proxy resolution, operation scopes, normal API-key authentication,
organization-agent access, billing owner selection and duration reservation.
The final source pass found no unresolved defects or actionable nits in this
change. All required local verification and compatibility reruns passed.

## Verification

All database checks explicitly use the isolated MongoDB 8.0.16 replica set:

```sh
NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27019/?replicaSet=voice-test'
```

| Check | Result | Evidence |
| --- | --- | --- |
| New regressions on unchanged production code | 0 passed, 6 failed, 0 ignored; expected authorization/discovery failures | `target/specialist-voice-red.log` |
| Final credential/authorization regressions | 10 passed, 0 failed, 0 ignored | `target/specialist-voice-green.log` |
| Complete voice/provider/duration-billing filter | 76 passed, 0 failed, 0 ignored; 41.01s | `target/specialist-voice-compatibility.log` |
| Operation-scope suite, run by primary reviewer | 26 passed, 0 failed, 0 ignored; 195.94s | `target/specialist-voice-operation-scopes.log` |
| Organization-agent suite, run by primary reviewer | 26 passed, 0 failed, 0 ignored; 30.46s | `target/specialist-voice-org-agents.log` |
| Workspace formatting | `cargo fmt --all -- --check`, exit 0 | `target/specialist-voice-format.log` |
| Whitespace/conflict-marker diff check | `git diff --check`, exit 0 | `target/specialist-voice-diff-check.log` |

The live provider fixture verifies reservation of real test-wallet funds,
provider closure following key revocation, and settlement of 18 completed
seconds from reported usage of 18.2 seconds. Organization platform audio selects
the acting person's wallet and fails revalidation after membership removal.
Insufficient funds, rollout disablement, denied platform ACLs and unsynced
duration prices remain enforced.

The automated gate scanner did not classify the Rust test paths. They were
reviewed manually: no tests, assertions, timeouts, checks or thresholds were
removed or weakened. The existing catalog-options fixture gained a real
endpoint and encrypted bearer credential to meet the corrected eligibility
contract; its pricing, model and provider-origin assertions remain intact.

## Test infrastructure recovery

The first concurrent compatibility runs exhausted the temporary MongoDB
process's inherited 8,192-file limit. Its log records `TooManyFilesOpen`,
WiredTiger error 24, fatal assertion 50853 and process termination. The failed
outputs are preserved under `target/specialist-voice-*-infrastructure-failure.log`.
They are not counted as passing checks.

Only the task-owned MongoDB process was restarted, with a process-local file
limit of 65,536. A fresh writable-primary check passed on the same replica URI.
The affected suites passed when rerun sequentially on the unchanged compiled binary;
no product source, assertions or timeouts were changed to address this failure.
After the final passing run, the task-owned database shut down gracefully with
exit code 0. Its log and all test evidence were retained.

The initial baseline build already emitted macOS compact-unwind and
`proc-macro-error2` future-compatibility warnings; the fix introduces neither.

## Evidence boundary

This verifies the local backend implementation with real database transactions,
wallet accounting and synthetic provider transport. It does not claim a paid
OpenAI session, browser microphone/playback validation or a production deployment.
Provider entitlement and the deployed platform credential, rollout, tariff and
funding configuration remain deployment inputs. At the initial implementation
handoff, no commit, push, deployment or production setting change had been made.

## Pull request preparation

After the user authorized a PR, the branch was fast-forwarded to
`897a89c2b092e86c6d1172c375234b01c9d88d21` from `origin/main` without conflicts.
The primary reviewer repeated the code, authorization/security, adversarial and
documentation reviews personally. No additional implementation changes or
unresolved findings resulted. The code and adversarial reviews covered the same
tree, `34271175c24dbd0c1af0cedcd267a67a17ce7f81`; subsequent edits only update
documentation with this publication context and evidence.

The integrated code was rebuilt with
`CARGO_TARGET_DIR=target cargo test -p nyxid --no-run` (exit 0, 4m21s). The test
binary `target/debug/deps/nyxid_server-4364b07c7739ce97` then ran the following
filters sequentially, with `--nocapture --test-threads=4` and the explicit
isolated replica-set URI above:

| Filter/check | Result | Evidence |
| --- | --- | --- |
| `voice` | 76 passed, 0 failed, 0 ignored; 34.92s, including all ten new regressions | `target/specialist-voice-pr-voice.log` |
| `operation_scope` | 26 passed, 0 failed, 0 ignored; 170.53s | `target/specialist-voice-pr-operation-scopes.log` |
| `services::org_agent_tests::` | 26 passed, 0 failed, 0 ignored; 27.26s | `target/specialist-voice-pr-org-agents.log` |
| `cargo fmt --all -- --check` | Exit 0 | `target/specialist-voice-pr-format.log` |
| `git diff --check` | Exit 0 | Final diff checked before commit |

These are overlapping filters, not a unique-test total. The integrated source
and executable hashes are retained in `target/specialist-voice-pr-snapshot.json`;
the build output is `target/specialist-voice-pr-build.log`. The same existing
macOS linker and dependency compatibility warnings remain.

The generic ship preflight could not infer this repository's Rust test lanes.
Its secret heuristic also reported 18 false positives: 17 Rust identifiers/call
expressions and one synthetic loopback bearer assertion. The reviewer mapped
every flagged line back to the exact scanned added content (SHA-256
`cba9294e16742c66eb330eab127069060f7877b3f226ce881875cf8718ffd79f`)
and verified no real credential was present. The helper's failed result was
retained; it is not represented as a passing check. Repository-native commands
and personal inspection provide the evidence above. The gate scan against
`origin/main` did not classify Rust test paths; all changed tests were reviewed
manually, with no weakened assertions, timeouts or checks.

The PR retains the existing rollout and provider readiness requirements:
billing enabled or paid status alone does not enable platform voice. The fix
removes the specialist tool-grant requirement for eligible platform audio. No
production configuration or deployment is part of this PR preparation.
