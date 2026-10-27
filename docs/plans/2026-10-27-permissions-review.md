# Permissions review ledger (#2004, #2007)

Worktree: 01a17a96-9c05-76b2-89c9-951191466647  
Branch: fix/assistant-permissions-2004-2007  
Base: b1dd6548ade04ec4f0051203e1353796331c7f87  
Plan: docs/plans/2026-10-27-permissions-implementation.md, revision 3
Previous plan SHA-256: 7e16c966d309bb77896edc300183d5820f1929bbdfb6cd317171773912fc4f7a

## Gate state

| Gate | State | Evidence |
| --- | --- | --- |
| Astra investigation/plan | ASTRA_PLAN_APPROVED | Revision 3 records the user amendment, retains the two-leg evidence, fixes the realistic 1,121-byte sanitized fixture requirement, and preserves the live acceptance gate. Exact SHA is recorded below and in status JSON. |
| Opus plan approval | SUPERSEDED_BY_USER | Historical reviewer 01a17aa6-4253-72c3-99c5-ba1913459dd3 / session 01a17aa6-4254-7f52-9e27-04e28932b1bd; prior authentication failures are retained below. No Opus prompt or approval is required under the amendment. |
| Sol implementation | current-source F-001/codex validation passed; remaining checks open | Sole Heca implementer: agent 01a17b00-1ad2-7cc0-ada4-61d925daac94; current provider session 01a17b00-1c46-7753-a330-f48a830666bd (historical session 01a17b00-1ad3-7093-a162-bc9708ed8394 retained); provider codex, model gpt-6.1-sol, effort xhigh, mode full-access, exact worktree 01a17a96-9c05-76b2-89c9-951191466647. The rebuilt binary passed the exact F-001 regression and focused `codex_` batch; #2007 backend checks, fmt and Clippy remain open. |
| Astra personal implementation review | in progress; no approval | Astra personally reviewed the complete current diff and surrounding code and recorded F-001. Current-source compile, exact F-001 and focused `codex_` evidence now pass; broader/#2007 checks, exact final commit review and final approval remain open. |
| Opus final implementation review | SUPERSEDED_BY_USER | Historical gate retained for traceability; no final Opus prompt or approval will be requested. |

## Findings and resolutions

| ID | Gate/source | Finding | Resolution/evidence | Status |
| --- | --- | --- | --- | --- |
| F-001 | Astra implementation review | The guest callback reached the provider under the original default Use grant; the mock's unmatched 404 was the provider response. `check_http_projected` excluded guests from raw model consent, but the later guest admission branch and generic approval allowed egress. | Valid finding. The narrow projected-boundary rejection preserves the original Use fixture and zero-provider-effect assertion and covers default Use, Read and All with metadata, metadata-free, stateful and provider-hosted callbacks, plus zero owner consent/cards. The current rebuilt binary passed the exact guest regression and the focused `codex_` batch; live callback/version and deployed ingress acceptance remain open. Finish notification requested (`notify_on_finish=true`). | valid; source fixed and current-source validation passed; live/final gates open |
| F-004 | Astra personal Clippy review | Required all-target Clippy rejected the new metadata validator's boolean expression as `clippy::nonminimal_bool` at `request_identity.rs:32` under `-D warnings`. | Valid low-risk finding. Same Sol was instructed to apply Clippy's equivalent simplification without changing the closed seven-key metadata contract, then rerun fmt, Clippy, and source-bound tests. | open; exact Clippy gate failed once |
| F-003 | Astra/Sol source review | F-001's guest rejection runs before the existing method-override branch. Compatibility had to be checked against the destructive exact-approval path and the guest ordinary-service contract. | Resolved with source evidence: raw proxy and LLM gateway reject guest method overrides before `check_http_projected`; guests cannot request or consume owner approval cards, even with `All`. Excluding overrides from F-001 would let a direct helper caller reach the guest admission branch without an owner card. Owner callbacks still retain the existing override-to-destructive classification. No source change required. Sol also found no `client_metadata` contract defect. | resolved; source-only review, no build/test run |
| F-002 | Astra personal source review | Initial checkpoint wording implied the F-001 guard preceded every credential/billing preparation step. Shared proxy preparation resolves the target, classifies billing and may inspect an agent credential override before `check_http_projected_inner`; the guard still precedes approval/card creation, approval wait, outbound credential/header injection, node dispatch and provider egress. | Corrected review/evidence wording; Sol asked to assess whether code movement is required. No authorization weakening or provider effect is present. | resolved; documentation corrected and Sol source assessment confirms no code movement required |
| P-001 | Astra investigation, superseded | Revision 1 incorrectly treated the stateful chrono-llm-public callback as if the NyxID-to-llm-nyx NyxAgentAdmission marker crossed the HTTP hop. | Source tracing proves the marker is installed only on the outer llm-nyx request; the model callback re-authenticates independently through api_key_auth_user and has no Axum extension. Revision 2 removes the marker fix and records the old claim as invalid. | invalidated; replaced by P-005/P-006 |
| P-002 | Coordinator C-P1 | The reported callback path/service and complete production call chain were not shown in the original issue evidence. | Revision 2 states the issue's chrono-llm-public/responses report, documents the two-leg chain, cites the local source paths, and keeps the deployed payload/version gate open. No production body or prompt is recorded. | resolved in plan; live evidence open |
| P-003 | Read-only upstream inspection | The upstream bridge normalizes the callback: model is profile-derived, store:false is forced, nested input tools are rejected, and only bounded NyxID tools plus optional web_search survive. | Revision 2 cites /tmp/nyxagent-permissions-inspect-20261027 at f6988c5a3955f69cfd895fcfa7c9e5c2a986541f and explicitly treats it as source evidence, not deployment proof. | resolved in plan; ASTRA approved |
| P-004 | NyxID classifier inspection | The stateless allowlist accepts tool_choice but rejects top-level client_metadata; a Codex 0.160.1 capture recorded that field. | Revision 3 fixes the shape to exactly seven scalar string keys (the four turn IDs, installation ID, metadata string and window ID), ASCII ID bounds of 1-128 bytes, metadata string max 1,024 UTF-8 bytes, whole-object sorted compact JSON max 2,048 bytes, and realistic sanitized 1,121-byte coverage. It keeps store:false, handle rejection and tool filtering and requires before/after sanitized classifier and operation tests. | resolved in plan; ASTRA approved |
| P-009 | Coordinator refinement | The original opening language could make live callback evidence appear to block plan review, while the later text treated it as optional. | Revision 3 allows ASTRA review of the narrower local defect independently, but keeps live callback body/version and Lark/web acceptance as mandatory final gates that local fixtures cannot waive. | resolved in plan; ASTRA approved |
| P-005 | Security boundary | A marker/header/body shortcut or blanket assistant-key POST read bypass would authorize stateful/provider-hosted effects. | Revision 3 forbids marker forwarding and blanket bypasses; negative tests cover handles, unknown fields, provider-hosted tools, overrides, wrong service/path and guests. | resolved in plan; ASTRA approved |
| P-006 | #2004 denial loop | Denial explanation callbacks must preserve acknowledgement ownership and avoid a second equivalent card or denied effect. | Revision 3 retains owner_denial_decided, existing CAS/live-turn fences and event identity, with channel/web, duplicate, concurrent, expiry and cancellation regressions. | resolved in plan; ASTRA approved |
| P-007 | #2007 readiness UX | Structured 503/12900 readiness failures are collapsed into generic temporary-unavailability copy. | Revision 3 specifies key-qualified mapping, trace preservation, and dirty draft retention with focused HTTP and operation-editor tests; activation/readiness behavior remains unchanged. | resolved in plan; ASTRA approved |
| P-008 | Automation overlap | Automation owns trigger-run fields, wake/drain behavior and continuation claims in shared assistant runtime. | Revision 2 names the exact overlap, leaves those fields/functions to automation, preserves run/thread identity and requires a combined Opus interaction review. | sent to coordinator and automation PM; combined review pending |
| P-010 | Coordinator fixture evidence | A small placeholder metadata fixture could pass while failing to exercise the observed 1,121-byte callback and boundary behavior. | Revision 3 requires deterministic sanitized values preserving 36/36/36/36/36/697/38-byte fields and 1,121-byte sorted compact JSON, plus explicit per-field and 2,048-byte object boundary tests. | resolved in plan; ASTRA approved |
| P-011 | User workflow amendment | User explicitly removed both Opus gates and authorized pure Astra-Sol execution. | Revision 3 records SUPERSEDED_BY_USER; historical Opus IDs and OAuth errors remain evidence, and no Opus session will be resumed or prompted. | resolved by user amendment |

## Evidence log

- Read-only NyxAgent inspection: /tmp/nyxagent-permissions-inspect-20261027,
  commit f6988c5a3955f69cfd895fcfa7c9e5c2a986541f; no files modified.
- Checked-in fixture measurement (keys/types/lengths only):
  docs/fixtures/codex-public-responses-0.160.1.json contains exactly seven
  scalar strings with lengths 36, 36, 36, 36, 36, 697, and 38 bytes; sorted
  compact UTF-8 JSON for the original metadata is 1,121 bytes. A deterministic
  sanitized replacement is 362 bytes. Both are below the planned 2,048-byte
  object limit; implementation tests must cover realistic 1,121-byte input and
  explicit per-field/object boundary failures.
- At the pre-implementation plan checkpoint no NyxID source files had been changed. The later implementation edits are listed in the evidence document; plan artifacts contain no prompts, credentials, raw callback bodies, context markers or ciphertext.
- Previous Opus prompts failed because Claude OAuth was expired. The local login
  process/session 23079 was cancelled as instructed; no second login flow was
  started. The later user amendment supersedes both Opus gates; no login,
  reviewer retry, or authentication recovery is required.
- Historical Opus errors are preserved verbatim: `Failed to authenticate: OAuth
  session expired and could not be refreshed` and `401 OAuth access token has
  expired`. These are superseded workflow evidence, never approvals.
- Astra personal plan assessment found one concrete defect and fixed it in
  revision 3: the implementation must preserve realistic sanitized fixture
  lengths and the measured 1,121-byte compact metadata size, not only use a
  small placeholder. The resulting plan SHA is recorded in the status file.
- ASTRA_PLAN_APPROVED is bound to revision 3 SHA c9a687732f634a997c193935e423e2aa423a8bbebbb658292a64b760cbba47da. The single Sol implementer was launched only after this sign-off and verified through Heca status.
- All command output added after implementation will be bounded and sanitized,
  with timestamps, exact commit IDs and test names only.


## Implementation checkpoint — 2026-10-27 01:51:48 UTC

- The approved revision 3 plan remains byte-identical at SHA-256
  `c9a687732f634a997c193935e423e2aa423a8bbebbb658292a64b760cbba47da`.
- Added bounded Responses-only `client_metadata` classification, a deterministic
  sanitized fixture preserving 36/36/36/36/36/697/38-byte values and 1,121-byte
  compact metadata, projected authorization and denial replay regressions, and
  key-qualified readiness copy with real HTTP operation-editor coverage.
- The standalone source comparison passed: the original classifier rejects the
  sanitized callback, the current classifier accepts it, and the original
  classifier accepts its metadata-free counterpart. This is local classifier
  evidence only; it does not replace the full backend test run.
- Frontend: 63 focused tests and 615 broader tests passed; TypeScript and focused
  ESLint passed. Changed Rust files pass standalone rustfmt checking; the final
  workspace Cargo formatting check remains pending after the last test edits.
- The queued normal-profile shared-target Cargo compile never acquired the lock.
  The coordinator terminated PID 431 during the resource stop; command exit was
  143 and the log contained only the build-directory lock wait. No backend
  compilation/test result is claimed. No subsequent Cargo/build/link/install
  command has been started.
- Source review found no production edits to proxy admission, callback ingress,
  automation runtime, event wake/drain, scheduling or continuation ownership.
  Local web/channel replay tests are added but await compilation/execution.
- No commit or Astra implementation approval exists yet. The production callback,
  deployed-version, live Lark/web and administrator-readiness acceptance gates
  remain open. Full details: `2026-10-27-permissions-implementation-evidence.md`.

## Coordinator hold — 2026-10-27 02:51 UTC

- Initial pre-F-001 compilation passed in 17m 44s. The corrected focused run
  subsequently passed 22 tests, failed the guest zero-egress assertion (valid
  F-001), and retained one pre-existing ignored coordinated-native test.
- F-001 production fix and strengthened original-fixture regression are present.
  The isolated rerun did not complete compilation or execute any test.
- Coordinator required immediate quiescence for Telegram's exclusive window.
  SIGTERM stopped only group 42211 (Cargo) and child 42278 (rustc). Cargo signal
  return: -15; runner exit: 143. A second full process scan verified no owned
  compiler/linker survivors at 02:51:19Z. No peer, target lock or cache changed.
- Free disk: 3.407 GiB at stop; 5.406 GiB after verified exit. Stop evidence:
  `/tmp/nyxid-permissions-r3-coordinator-hold.json`. Build log:
  `/tmp/nyxid-permissions-r3-guest-f001.log`.
- Source-only review confirms the new rejection precedes guest admission,
  model-consent persistence and provider forwarding; the original default Use
  fixture and zero-egress assertion are preserved. Current source is unvalidated.
- No Cargo/rustc/build/link/test/check/clippy command is queued. A coordinator
  handoff through Astra PM is required before resuming. No commit or Astra
  implementation approval is claimed.
- PM acknowledgement with `notify_on_finish=true` was attempted, but Heca
  rejected the caller's scope. It was not delivered; no agent was spawned.

## Astra personal source review — 2026-10-27 02:56 UTC

- The approved plan remains byte-identical at SHA-256 `c9a687732f634a997c193935e423e2aa423a8bbebbb658292a64b760cbba47da`; no material plan revision is required.
- I traced `check_http_projected_inner` and `execute_resolved_proxy_inner`: the F-001 rejection runs after shared target/operation resolution (which may classify billing and inspect an agent credential override) but before assistant admission, generic approval/card creation, approval wait, outbound credential/header injection, billing forwarding, node dispatch and provider egress. It rejects only assistant guest turns whose request matches the existing catalog inference + POST + supported model-generation path + non-empty `model` predicate. Ordinary guest service calls and legitimate owner callbacks remain on their existing paths.
- The stateless `client_metadata` contract remains closed to seven scalar strings with the stated ASCII/UTF-8 and sorted compact JSON bounds. Validated metadata is removed only from recursive handle scanning; the original body remains in the operation digest, so changing metadata changes the bound. Stateful handles, provider-hosted tools, unknown fields, method overrides, wrong paths/services, and guest callbacks remain fail-closed.
- At this checkpoint F-001 source fix and strengthened regression were present,
  but current-source backend compilation and the isolated regression were still
  interrupted by the coordinator's Telegram resource hold. The later
  05:28 UTC milestone below records the successful rebuilt-binary validation;
  no implementation approval or commit is claimed.
- Heca status verified the same implementer agent `01a17b00-1ad2-7cc0-ada4-61d925daac94` with provider `codex`, model `gpt-6.1-sol`, effort `x_high`, mode `full-access`, exact worktree `01a17a96-9c05-76b2-89c9-951191466647`, current session `01a17b00-1c46-7753-a330-f48a830666bd`; it is idle after source review.

## F-002 resolution — 2026-10-27 03:10 UTC

The same Sol implementer traced the source read-only and confirmed no production code movement is required under revision 3. Shared proxy preparation can refresh/decrypt an agent credential override and touch its last-used state before the projected assistant check; the F-001 guard is therefore not described as preceding every credential lifecycle operation. It does run before assistant admission, model-consent writes, approval-card creation/waiting, outbound credential/header injection, billing forwarding, node dispatch and provider egress. F-002 is resolved as a documentation/order-precision finding. No backend command was started during Telegram's hold.

## F-003 resolution — 2026-10-27 03:25 UTC

The same Sol implementer performed a source-only review of the F-001 guard's
ordering against effective method overrides. `legacy_model_generation` requires
the literal `POST`, so literal non-POST requests keep their existing paths.
Production raw proxy requests reject guest method overrides before
`check_http_projected`, and the LLM gateway rejects them in
`enforce_agent_llm_operations` (including scoped operation authorization).
`check_guest_approval` permits no owner approval path for guests, so adding a
`!method_override` exception to F-001 would leave a direct projected helper
caller able to reach the guest admission branch without an owner card. Owner
requests continue to classify overrides as destructive and require their
existing exact approval. The seven-field metadata contract was independently
rechecked with no defect. No source change or backend command was made.

## Resource-policy correction — 2026-10-27 04:20 UTC

The retained Sol session corrected the prepared disk-guarded runner at
`/tmp/nyxid-permissions-r3-run.py` to preserve the coordinator's Cargo
argument boundary: built-in test/build/check commands use
`cargo --config <path> <subcommand>`, while Clippy uses
`cargo clippy --config <path> --manifest-path backend/Cargo.toml --all-targets -- -D warnings`.
The runner normalizes either accepted input form without duplicating or
dropping the resource config. It was not invoked, and no backend command or
probe was started or queued. Argument propagation remains pending the required
post-handoff Clippy run and actual backend argv inspection. Telegram's
byte-identical baseline `nonminimal_bool` findings are outside this worktree;
no fixes were copied or imported.

## Prerequisite import and validation handoff — 2026-10-27 05:06 UTC

The coordinator released the exclusive backend window after Telegram's
verified prerequisite. Astra personally inspected source commit
`336405f8e667ea2eb2c6513effec3d15fd960b02`: exactly four boolean-only files,
8 insertions and 8 deletions, with no overlap with the Permissions feature
files. It was imported as local cherry-pick commit `21907f836` without
changing the pending feature work. The retained Sol session
`01a17b00-1ad2-7cc0-ada4-61d925daac94` was resumed with
`notify_on_finish=true` for current-source F-001/#2004/#2007 validation,
formatting and the exact Clippy command. Both ticket verdicts remain BLOCKED
until an exact implementation commit and all criteria are demonstrated.

## Current-source validation milestone — 2026-10-27 05:28 UTC

- The configured current-source compile passed with exit 0 in 576.86 seconds.
  It produced `target/debug/deps/nyxid_server-a7bf57b9c4cb166a` with SHA-256
  `b5e799506eeee8fddbba78e341daaf261a5e1e1ddf261d8a8efa6d3a7425c9c7`.
  Evidence: `/tmp/nyxid-permissions-r3-current-compile-1.json`.
- The exact rebuilt executable passed
  `services::assistant_service_execution::tests::permissions::codex_callback_guest_does_not_gain_owner_model_consent`:
  1 passed, 0 failed, 0 ignored, 10,457 filtered. Evidence:
  `/tmp/nyxid-permissions-r3-current-guest-direct.json`.
- The same executable passed the focused `codex_` batch: 23 passed, 0 failed,
  1 ignored, 10,434 filtered, in 146.91 seconds. The ignored case is the
  pre-existing coordinated A8/native test and is not counted as a pass.
  Evidence: `/tmp/nyxid-permissions-r3-current-codex-direct.json`.
- These results close local F-001/classifier/denial execution evidence, but do
  not close live callback/version, Lark/Web, #2007 readiness activation or
  allowed/excluded operation enforcement. No exact implementation commit or
  `ASTRA_IMPLEMENTATION_APPROVED` exists yet.


## Sol exact-commit handoff — 2026-10-27T06:35:20+00:00

Implementation `493b5c2dc012cc4de80c00f33f7cddf489cb316d` contains only the 11 Permissions source/test/fixture files. The equivalent metadata boolean lint finding was fixed; final-source compilation, focused and broader Rust regressions, formatting and exact all-target Clippy passed. Complete commands, argv, results, ignored cases and disk records are in `2026-10-27-permissions-implementation-evidence.md` and `/tmp/nyxid-permissions-r3-final-validation.json`.

This is a Sol handoff record, not Astra personal exact-commit approval. Both issues remain BLOCKED for the external/live gates and Astra review recorded in `2026-10-27-permissions-signoff.md`. No backend command is running or queued by Permissions; the retained Mongo8 fixture is unchanged.

## Astra exact-commit source review — 2026-10-27T07:38:15Z

Astra personally reread the complete exact implementation commit
`493b5c2dc012cc4de80c00f33f7cddf489cb316d`, all 11 feature/test/fixture files,
and the surrounding production authorization, classifier,
acknowledgement/continuation, error-envelope and operation-scope code. The
review found no unresolved production code finding. Sol removed only the
redundant uncommitted `request_identity.rs` refactor; that production file now
matches the committed source byte-for-byte.

The remaining working-tree source diff is the requested six-line test-only
negative regression. It removes `x-codex-turn-metadata`, inserts `unknown`,
asserts seven fields, and reuses the existing assertions that the request is
not stateless inference, remains an HTTP action, and requires exact change
approval. The committed predicate already enforces the intended closed set
through the seven-field cardinality, all six required valid identifier strings,
and the required scalar turn-metadata key. F-005 is classified as a negative
regression coverage gap, not a production hardening defect.

**SOURCE_REVIEW_COMPLETE** is supported for `493b5c2`. The new test-only
follow-up has passed `git diff --check`, but **LOCAL_VALIDATION_COMPLETE** is
pending the narrow rebuild/test and affected required fmt/Clippy checks.
`ASTRA_IMPLEMENTATION_APPROVED` remains pending both that local validation and
the applicable live/combined ticket gates. No backend command was started or
queued because Automation owns the window.

Candidate manifest including the uncommitted test-only follow-up:
`/tmp/nyxid-permissions-r3-review-source-manifest.sha256`, SHA-256
`d17bfd6a8c9fa611aec069828005328851c68c14842c731112c39bf8b4478c6c`.

## Astra finding F-005 — negative-regression coverage gap — 2026-10-27

The exact committed implementation at
`backend/src/services/assistant_service_execution/request_identity.rs:8-45`
already rejects the replacement shape because `object.len() == IDS.len() + 1`,
all six entries in `IDS` must be valid scalar strings, and
`x-codex-turn-metadata` must be present as a scalar string. The old production
predicate therefore satisfies the plan's closed-set contract; no concrete
behavioral counterexample was found.

The valid gap was test coverage. The retained regression removes the required
metadata key, inserts `unknown`, keeps seven fields, and exercises the existing
negative classifier and exact-approval assertions. Sol removed the redundant
production refactor, leaving only that test diff. Narrow execution remains
pending the next explicit Permissions window.


## Astra narrow local validation complete — 2026-10-27T12:41:02.142395+00:00

**SOURCE_REVIEW_COMPLETE; LOCAL_VALIDATION_COMPLETE** for source
`7185b5ef7eb56194b2e1cbf42f045536a9cb7847`. The complete prior source review is carried forward;
Astra personally inspected the fresh build, both exact nonzero test inventories,
actual test logs, formatting, required Clippy, compiler arguments, source/binary
identity and final process-quiescence records. This closes the F-005 test-coverage
gap. No production or test source changed during this batch.

Reviewer: GPT-6-Astra PM, Heca `01a17a9b-5082-7b83-8171-2e791e9fd01e`.
The retained Sol was verified as codex / gpt-6.1-sol / x_high / full-access,
agent `01a17b00-1ad2-7cc0-ada4-61d925daac94`, provider session
`01a17b00-1c46-7753-a330-f48a830666bd`. Execution HEAD was documentation-only
`dd83cc01fbb8bf1f6a70bab6c330f765ade90d59` above the source commit.

| Required check | Actual result | Archived evidence |
| --- | --- | --- |
| Fresh configured `cargo test -p nyxid codex_ --no-run` | Exit 0; 633.15 seconds | [Build log](evidence/permissions-2026-10-27/F005-narrow-compile.log.txt), [runner](evidence/permissions-2026-10-27/F005-narrow-compile.json) |
| Exact `codex_metadata_does_not_admit_stateful_provider_or_unknown_effects` | 1 passed, 0 failed, 0 ignored; inventory exactly 1 | [Test log](evidence/permissions-2026-10-27/F005-narrow-classifier.log.txt), [identity/command](evidence/permissions-2026-10-27/F005-narrow-classifier.json) |
| Exact `codex_callback_projected_read_path_keeps_exact_negative_approvals` | 1 passed, 0 failed, 0 ignored; inventory exactly 1 | [Test log](evidence/permissions-2026-10-27/F005-narrow-permission.log.txt), [identity/command](evidence/permissions-2026-10-27/F005-narrow-permission.json) |
| Configured `cargo fmt --all --check` | Exit 0; 13.65 seconds | [Runner](evidence/permissions-2026-10-27/F005-narrow-fmt.json) |
| `cargo clippy --config <shared-config> --manifest-path backend/Cargo.toml --all-targets -- -D warnings` | Exit 0; 353.86 seconds | [Clippy log](evidence/permissions-2026-10-27/F005-narrow-clippy.log.txt), [runner](evidence/permissions-2026-10-27/F005-narrow-clippy.json), [actual argv](evidence/permissions-2026-10-27/F005-narrow-clippy.argv.jsonl) |

Both exact test names have prefix
`services::assistant_service_execution::tests::permissions::` and ran from
`/Users/chronoai/Library/Application Support/heca/worktrees/eafaaf47/nyxid-permissions/backend`
with `--exact --test-threads=1`. The other commands ran from the owning worktree
root. The [complete validation record](evidence/permissions-2026-10-27/F005-narrow-validation.json)
contains the full command vectors, absolute config/runner paths, working
directories, exact inventories and before/after source/binary checks.

Fresh binary SHA-256: `1fbad9ed97638d877114c88593fb9db5e615eb27cbda66c9cd0eb613cf37d875`.
The 11-file source manifest SHA-256 remains
`d17bfd6a8c9fa611aec069828005328851c68c14842c731112c39bf8b4478c6c`.
Astra independently matched every entry to `7185b5ef7eb56194b2e1cbf42f045536a9cb7847` and to
the clean working tree, and separately verified the binary bytes. The historical
169 unique passes / 0 failures / 2 ignored remain bound to `493b5c2` and its
original tests. These two current executions overlap that inventory and are not
added to a new unique-test total. Frontend evidence is carried forward unchanged.

The isolated Mongo8 fixture on port 27471 passed strict writable-primary and
transaction commit/readback plus abort/readback checks. Retained Mongo PID 34439
matches its historical `nofile=65536` launch record; this is explicitly launch
evidence, not a live limit query. All new runner commands used nofile 65536.
The first three Mongo probe-script attempts failed due to shell helper/return
semantics; their logs are preserved with the corrected strict readback probe.
No application assertion or readiness condition was weakened.

The shared target/jobs=2/config were preserved. Actual package compiler argv
has no debuginfo/incremental or optimization/assertion/overflow override; actual
Clippy backend Cargo retains the config and all targets. Minimum free disk was
11.702 GiB; no guard fired. Astra independently confirmed zero heavy/backend-test
processes at release and 15.524 GiB free. The backend window was released before
this documentation work; no additional standalone command is queued.

**Both #2004 and #2007 remain BLOCKED. ASTRA_IMPLEMENTATION_APPROVED is not
granted.** Current-main reconciliation, combined I-05/I-06 and each ticket's
applicable live/deployed acceptance remain open. The coordinator accepted the
[bounded reconciliation direction](2026-10-27-permissions-main-reconciliation.md)
and has assigned source-only work in its isolated integration checkout to the
same Sol; original feature source stays frozen. Those forthcoming changes need
personal review and their own final-source validation.

Byte-preserving copies of the current records and carried-forward evidence are
in the [archive index](evidence/permissions-2026-10-27/index.json), with [file hashes](evidence/permissions-2026-10-27/SHA256SUMS).
Original `/tmp` paths inside copied records identify execution locations.


## Isolated reconciliation merge inspection — 2026-10-27T12:43:24.852898+00:00

Merge `26154051327a9b5692a63ad234eab7601853fb31` has parents
`5e8812b6f6b6c8137b23f116a910cc39c303a1cf` and
`dd83cc01fbb8bf1f6a70bab6c330f765ade90d59`. Astra verified the resolved
HTTP authorization blob is exactly main's `a1ba4959c1524d75b266691f6025f329a25dfe4a`.
The other imported feature files match pinned Permissions source byte-for-byte;
the shared test module contains only the expected additive two-line module
registration relative to the first parent. This carries forward the personally
read main route/identity/revalidation code and the prior Permissions review.

The original F-001 helper expects the old ApiKeyScopeForbidden variant. Main's
early route gate now correctly returns GuestModelRouteRequired for this unbound
persisted guest fixture. Astra assessed changing only that enum expectation as
valid; its real authenticated router HTTP403, default Use/generic-approval-disabled
and zero provider/consent/card assertions must remain. The existing Sol is
preparing the separate scoped I-05/I-06/reconciliation tests. They are unexecuted;
no combined source-review completion or implementation approval is claimed.
