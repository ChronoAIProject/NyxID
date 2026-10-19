# NyxBot owner CLI and API control

Status: implementation and personal review completed, including integration
with `main` on 2026-10-19. The validation history below distinguishes source
revisions. Current check results, required review, and merge status are tracked
in [PR #1941](https://github.com/ChronoAIProject/NyxID/pull/1941).

## Delivered behavior

`nyxid nyxbot` submits owner work, inspects durable invocation receipts, waits,
and requests exact cancellation. It also discovers agents, conversations,
history, and models. Prompt input supports text, files, and stdin. JSON reports
retain a recovery ID and distinguish execution failure, cancellation, owner
action, local timeout, uncertain transport, and invalid server responses.

The HTTP resource is mounted under `/api/v1/assistant/nyxagent`:

- `POST /invocations` admits work atomically or returns its existing receipt.
- `GET /invocations/{invocation_id}` reads a bounded snapshot.
- `POST /invocations/{invocation_id}/cancel` fences its associated work.

The [owner CLI/API reference](../chat/09-owner-cli-api.md) documents commands,
request and response fields, statuses, exit codes, and rollout requirements.

## Authorization and execution contract

Only the owner's first-party account session or access token can use these
endpoints. Developer OAuth clients, delegated applications, relay credentials,
service accounts, API keys, and Agent Key login are rejected. CMA integration
is outside the scope. The existing CLI transport requires HTTPS outside exact
loopback development hosts, binds credentials to their origin, and refuses
credential-bearing redirects and implicit credential switching.

First-party provenance describes how a credential was issued; it does not
prove that a person is present for every request. This API carries the owner's
existing authority. The shared NyxAgent engine continues to enforce specialist
grants, owner confirmations, billing, and engine availability. The CLI does not
change permission modes or approve action cards.

Admission stores an owner-scoped UUID-v4 identity and a keyed request
fingerprint in the same transaction as the conversation, turn claim, and
encrypted credential. A matching retry never starts a second worker. A changed
request conflicts. Receipts survive conversation deletion. Uncertain provider
effects are not replayed; there is no exactly-once downstream-effect claim.

Invocation lineage follows descendant turns, owner decisions, asynchronous
operations, setup watches, machine work, and publication continuations. Status
remains nonterminal while tracked work or required review remains. Producers
and terminal settlement serialize through the root receipt, preventing late
work from reopening a completed invocation.

Cancellation binds the exact invocation family and machine turn scopes. It
cannot stop a successor turn. Tool calls that wait for approval revalidate the
original turn and credential before dispatch. Credential generations retire
at invocation-family boundaries, and mandatory live-turn checks apply even
when the ordinary live-turn rollout flag is disabled.
API-key route checks use the original request path across nested routers, for
both Bearer and X-API-Key headers, preserving execution and management boundaries.

An uncertain external cancellation remains an explicit review outcome. The
API neither repeats an uncertain cancellation RPC nor claims to undo effects
that were already dispatched. Hidden group, voice, automation, channel, and
platform conversations are rejected. Invocation-driven `post_to_group` is
refused because shared group scheduling cannot preserve exact cancellation.

## Rollout

`assistant:owner-invocations` defaults off. Upgrade every replica before enabling
admission. Inspection and cancellation remain available with admission disabled
while the engine is enabled.

After the first invocation admission, supported recovery is forward-fix-only.
Disabling admission and draining or cancelling work does not make old binaries
safe: they ignore mandatory invocation credential fences. The public reference
explains the recovery procedure and why receipt cancellation alone is not a
credential-retirement migration.

## Initial verification

The primary reviewed the implementation personally and resolved 44 findings
with the same Heca implementer, running verified `gpt-6.1-sol`, `xhigh`, and
full access. No additional review agent was used. Review corrections
covered admission/replay, owner isolation, credential generations, exact
cancellation, delayed approvals, event producers, terminal status, publication,
setup flows, and safe CLI output.

Database validation uses MongoDB 8.0.34 with a real replica set. The setup probe
verified the server version, writable-primary role, transactional client bulk
writes, and absence of rows after abort. Transaction tests fail when this
infrastructure is unavailable instead of silently skipping.
After validation, the primary verified and stopped only the task's temporary
MongoDB process. The results below record those completed runs. On the later
checkout review, the temporary validation logs and file-hash snapshot were no
longer present; this document remains the durable result record. That review
rechecked route and feature-flag wiring, new-file whitespace, and documentation
links, and confirmed that the retained CLI executable is newer than every CLI
Rust source file. It did not rerun the backend or CLI test suites. These results
precede the integration with newer `main` described below; they are not a
validation claim for that integration.

| Check | Evidence |
| --- | --- |
| Surrounding backend regressions | 700 passed, 0 failed, 0 ignored on MongoDB 8; preceding compiled binary |
| Billing settlement and route inventory | 4 passed, 0 failed, 0 ignored on MongoDB 8; preceding compiled binary |
| Broad CLI suite | 1,579 unit and 80 integration tests passed; 6 existing ignored unit tests; before final CLI refinements |
| Final implementation invocation regressions | 27 passed, 0 failed, 0 ignored on MongoDB 8; includes the eight-case approval/cancellation matrix |
| Final implementation authentication regressions | 122 passed, 0 failed, 0 ignored; executed from the same backend test binary |
| Current-source focused CLI tests and build | 7 passed, 0 failed, 0 ignored; executable rebuilt after discovery-error and UUID variant corrections |
| Independent executable CLI smoke | 12 invocation scenarios and 4 discovery-error cases passed; malformed UUID variant rejected before authentication or network access |
| Scoped Clippy with warnings denied | `cargo clippy -p nyxid -p nyxid-cli --all-targets -- -D warnings` passed cleanly in 10m 05s |
| Formatting and diff whitespace | Current-source `cargo fmt --all -- --check`, `git diff --check`, and new-file whitespace checks passed |

The current-source approval regression covers MCP direct/meta calls, HTTP proxy
UUID/slug paths, and LLM provider/gateway paths. Both LLM paths exercise Bearer
and X-API-Key credentials. Each case waits for a persisted approval, cancels
the invocation, then approves and asserts refusal with zero downstream requests.
It also checks that invocation credentials cannot enter SSH or assistant
management routes.

After these runtime checks, Clippy identified a redundant closure in a test
fixture. Replacing `|| std::future::pending()` with `std::future::pending`
preserves its behavior and assertions; production code is unchanged. The final
clean lint run checked this fixture correction. Formatting and whitespace
checks also passed afterward. The final file-hash comparison found no source
changes during the clean lint run.

The first temporary database resolved through a Homebrew symlink to MongoDB 7.
That server rejected transactional client bulk writes and caused four billing
checks to fail. All four pass unchanged on the explicit MongoDB 8 binary; no
production billing logic or test timeout was changed for the environment error.

Builds use `CARGO_PROFILE_DEV_DEBUG=0`, `CARGO_PROFILE_TEST_DEBUG=0`,
`CARGO_INCREMENTAL=0`, and `CARGO_BUILD_JOBS=1` to bound storage and concurrent
compiler memory use. `NYXID_TEST_DATABASE_URL` identifies the isolated MongoDB
8 replica set. The same upstream Swagger UI archive was supplied locally after
its original download timed out; source and dependency versions are unchanged.

## Pull request integration (2026-10-19)

The original feature was committed as `4635735aa`, then rebased onto `main`
at `f96cf8778` as `82b44933a`. The primary reviewed the conflict resolutions
and range diff. New channel-adapter origin checks and boxed authority values
are retained alongside invocation identity and cancellation checks.

Integration work extends the newer shared service execution boundary to bind
the original invocation, checks root cancellation before asynchronous sends,
and retains only the bounded leased cancel/status cleanup capability. The
post-deadline status exception applies only to that tracked cleanup; ordinary
status and result reads retain their original deadlines. Machine operation
admission and job publication fence the root in their transaction, and service
egress checks the job's original invocation after its issuing turn settles.
The newer expired/cancelled channel-setup path settles its watch and queues
its event atomically. The primary reviewed these changes and the new regression
tests. The focused integration runtime gates passed on `09869814c`; the
first full CI run exposed one scheduler test stack overflow, described below.
The PR records subsequent verification of its correction.

The public CLI reference is now included in the docs navigation and linked
from the README, API reference, and MCP reference. It explains local client
use, owner login, prompt input, conversation follow-ups, recovery, and
cancellation. No delegated MCP invocation tool is added. The CLI uses the
existing hosted or self-hosted server; local file, shell, and desktop authority
still comes from separately configured machine capabilities.

Documentation validation passed: 18 existing tests across docs navigation,
Markdown rendering, and search; ESLint on the two changed frontend files;
local guide link and whitespace checks. New validation logs are stored in
task-specific Git worktree metadata rather than temporary storage.

## Integration validation and security review

The local integration checks used MongoDB 8.0.34 on a dedicated replica set.
Durable records include commands, exits, executable hashes, and a manifest of
1,294 Rust inputs. All selected backend cases ran without ignored tests.

| Check | Result |
| --- | --- |
| Invocation admission, cancellation, and authority regressions | 35 passed |
| Authentication and route boundaries | 124 passed |
| Shared service execution compatibility | 63 passed |
| Ordinary async operation and policy compatibility | 24 passed |
| Focused CLI regressions | 7 passed; CLI executable build passed |
| Independent CLI executable smoke | 20 loopback scenarios passed |
| Documentation navigation, rendering, and search | 18 passed |
| Feature-flag registry fixture correction | 6 passed; scoped ESLint passed |

Backend runtime results above apply to `09869814c`. Subsequent local Clippy
found two equivalent predicate simplifications in inherited platform reply
code: `!A && !(B && C)` to `!(A || B && C)`, and `!Option.is_some()` to
`Option.is_none()`. The primary reviewed the exact changes and their
short-circuit behavior. The original runtime manifest and the two-file delta
are retained separately; earlier runtime hashes are not claimed for changed
source. Formatting and whitespace checks pass after those changes. A clean
combined all-target Clippy retry ended without a recorded result and was not
counted as passing. The PR records the replacement run and final source checks.

The first full GitHub backend run passed 9,391 tests and failed one with a
stack overflow in
`schedule_specialist_authority_threads_and_guest_refusal`; 10 tests were
skipped by the existing suite configuration. Backend coverage aborted at the
same test. The frontend and frontend coverage each passed 6,144 tests and
failed the expected feature-flag registry literal, now corrected and verified
locally. CLI CI passed 1,887 tests with 9 existing skips. Workspace Clippy,
formatting, CLI coverage, all three KMS feature combinations, billing smoke,
image-input checks, bundle freshness, release integrity, and all CodeQL
languages passed on that first PR revision. These results do not constitute a
passing final CI run.

The integration review found and fixed concrete authorization gaps:

- Shared authority reload could lose the invocation identity. Reload now
  preserves and rechecks the original invocation and delivery binding.
- Async continuation and machine-job paths needed live root checks before
  execution, including after root deletion but before background cleanup.
  Those checks now cover service egress, job gateway access, transactional
  operation admission, and job publication.
- Cancellation cleanup needed a narrow exception for one leased cancellation
  attempt and its declared status operation. Result retrieval remains denied;
  ordinary watches retain their original deadlines.
- Owner channel-adapter conversations required rejection even when addressed
  directly by ID. Persisted channel provenance enforces that boundary.

The new regression cases exercise real database transactions and loopback
requests, including cancellation before background cleanup, changed lineage,
stale credentials, and a legitimate successor turn. These findings were
resolved in the reviewed implementation. Passing review and tests cannot prove
the absence of every vulnerability. Rollout prerequisites and the inability to
undo an already sent external effect remain part of the supported contract.

The scheduler test crash reproduced from the preserved runtime binary with
`RUST_MIN_STACK` unset. The symbol trace identified the large test future above
`run_job_once`, turn admission, and BSON serialization. Both production
scheduler call sites already heap-allocate `run_job_once`; the failing test
directly awaited it twice. The reviewed correction adds `Box::pin` at those
two test call sites to match production. No production path, stack setting,
assertion, or test selection changes. This reproduction establishes a test
stack allocation defect; it does not establish an exploitable production
crash. Default-stack runtime verification, scheduler compatibility, and a
fresh full CI run are required for the corrected revision; their results
are retained in the PR and durable validation records.

## Integration with channel queues and source images

Main revision `618c627b9` adds durable channel source questions and authorized
source-image retrieval. Integration preserves those paths alongside invocation
authority. Shared event writers reject invocation identity changes, preserve
accepted source questions and invocation continuations at capacity, and roll
back producer state on refusal. Event selection keeps those delivery contexts
out of unrelated owner turns.
Channel repeat coalescing cannot add destinations to an invocation or its
queued continuations. A repeated channel question survives invocation
cancellation and starts a separate turn under its own authority.

Source-image reads match the original invocation identity. Handle issuance,
confirmation binding, and the final attachment transaction also fence the live
invocation root. A delayed image cannot attach after cancellation, root deletion,
or terminal settlement while background cleanup is still pending. Ordinary
resource use does not query invocation storage.

Regressions cover mixed queue capacity and atomic rollback, duplicate delivery,
changed event lineage, isolated event selection, all inactive root states,
changed resource lineage, absence of stored bytes after refusal, and a live
invocation's successful attachment. Final results for the integrated revision
are recorded on PR #1941 and in durable task validation records.

## Source references

- [Assistant architecture](../chat/README.md),
  [NyxAgent engine](../chat/08-nyxagent-engine.md), and
  [agent isolation](../AGENT_ISOLATION.md).
- [Invocation handlers](../../backend/src/handlers/assistant_invocations.rs),
  [lifecycle service](../../backend/src/services/assistant_invocation_service.rs),
  and [receipt models](../../backend/src/models/assistant_invocation.rs).
- [CLI commands](../../cli/src/commands/nyxbot.rs),
  [HTTP/auth client](../../cli/src/api.rs), and
  [authentication middleware](../../backend/src/mw/auth.rs).
- [Shared admission and settlement](../../backend/src/services/assistant_nyxagent.rs),
  [runtime](../../backend/src/services/assistant_nyxagent_runtime.rs), and
  [conversation credentials](../../backend/src/services/assistant_agent_credential_service.rs).
