# NyxID artifacts review ledger

This ledger is append-only for the #2037 workstream. Every plan or implementation finding, including low-severity issues and nits, must have a concrete fix or evidence closure. A changed plan or source commit invalidates the corresponding gate.

## Plan gate

| Revision | Plan SHA-256 | Opus reviewer | State | Findings |
| --- | --- | --- | --- | --- |
| v1 | `d2b3a6d6825483081a7aa30183c2f0e933a2897478d4a8a375ea97ae41ce067c` | not started | `SUPERSEDED` | Coordinator identified five material gaps before review |
| v2 | `98f00ee35c9be1fe2e5e630d99b185dc82fc62a57ca96d8c544a966cb0739d72` | not started | `SUPERSEDED` | Coordinator requested lifecycle provenance, flag-off drain, and one expiry contract |
| v3 | `439b081ccc9f637b7bb813d0b7d104464627dbf476d7e206b6d7669dde0bb6b5` | not created | `SUPERSEDED_BY_USER` | user explicitly superseded Opus plan/final gates |
| v4 | `7f5226f241b7a826c58be216f12dd4416471a48d91e67e4ed65871b811bd8493` | not required | `ASTRA_PLAN_APPROVED` | exact plan SHA personally assessed and approved under user-amended Astra→Sol workflow |

The user amendment on 2026-10-27 explicitly superseded both Opus gates. No Opus reviewer was created (`opus_reviewer_id=null`, `opus_session_id=null`); the earlier Claude authentication blocker remains historical evidence only. The active plan gate is Astra's personal `ASTRA_PLAN_APPROVED` bound to the v4 SHA.

### Plan findings

| ID | Severity | Reviewer | Finding | Resolution/evidence | State |
| --- | --- | --- | --- | --- | --- |
| P-001 | — | — | No Opus review was received before the user amendment. | Superseded by the user's explicit removal of Opus gates; no pass is claimed. | SUPERSEDED_BY_USER |
| P-002 | high | Coordinator | Optional fields alone did not prove old-worker/history compatibility; existing channel/scheduled/trigger/voice watchers needed an activation/drain/rollback fence and concrete gate. | v4 retains default-off `assistant:generated-artifacts`, original-Web provenance plus exact completion-event checks, legacy fallback, explicit all-replica upgrade and old-worker drain sequence, disable-before-rollback, and origin regression tests. Astra assessed this as resolved. | RESOLVED_BY_ASTRA |
| P-003 | high | Coordinator | Sandbox status `version` was not established as immutable result content identity; status/result races and modification semantics were underspecified. | v4 treats status version as diagnostics only, derives server-side `source_result_identity` from exact operation plus ordered verified bytes, rechecks expiry, fails closed on digest change, uses the single documented minimum expiry, and requires a new operation for a new revision. Astra assessed this as resolved. | RESOLVED_BY_ASTRA |
| P-004 | high | Coordinator | The shared 192 MiB attachment budget and MongoDB BSON limit were not proven across JSON/base64/decode/encryption/fetch paths. | v4 holds one shared byte-budget reservation across the complete fetch/decode/hash/encrypt/persist path, bounds artifact workers, and adds a serialized encrypted 8 MiB BSON-under-16-MiB test with oversized-row rejection. Astra assessed this as resolved. | RESOLVED_BY_ASTRA |
| P-005 | high | Coordinator | MP4 marker strings did not prove a visual track or browser support; playback fallback and HTTP range semantics were underspecified. | v4 requires bounded ISO-BMFF track parsing, real fixture/browser playback and decode-error fallback, download-only ambiguous files, and HTTP-compliant selected range behavior (`200` for valid unsupported ranges or correct `206`/`416` cases). Astra assessed this as resolved. | RESOLVED_BY_ASTRA |
| P-006 | high | Coordinator | Generic client-selected retry authority was unsafe and did not pin a source association. | v4 pins an empty-body owner/message `artifact-delivery` route to a server-written watch/operation/source/lineage association, forbids client IDs and new submissions, and makes duplicate recovery idempotent. Astra assessed this as resolved. | RESOLVED_BY_ASTRA |
| P-007 | high | Coordinator | The plan conflated the original Web user turn with the later async consuming event turn. | v4 records `delivery.origin == User` and all Web-only destination fields at watch admission, writes `artifact_mode`, and permits consumption only by the server-created completion `Event` whose event ID is the watch ID and whose watch `turn_id` is the original user turn. Astra assessed this as resolved. | RESOLVED_BY_ASTRA |
| P-008 | medium | Coordinator | Expiry and disabling semantics were inconsistent: materialization must not extend source availability, and disabling admission must still allow safe drain of already-admitted work. | v4 defines one stored effective expiry `min(source status expiry, watch expiry, normal retention deadline)`, uses source/watch expiry for pending cleanup, and makes the flag admission-only while allowing upgraded materializers to drain persisted `artifact_mode` watches. Astra assessed this as resolved. | RESOLVED_BY_ASTRA |

### User workflow amendment

| Date | Amendment | Effect | Evidence |
| --- | --- | --- | --- |
| 2026-10-27 | “lets skip opus authenthication entirely for now. lets just roll with a pure ASTRA - SOL approach” | Supersedes Opus plan and final implementation gates. No Opus reviewer is created, resumed, retried, or awaited. | Updated shared charter and user instruction |

### Astra plan sign-off

`ASTRA_PLAN_APPROVED` — 2026-10-27 — exact plan SHA `7f5226f241b7a826c58be216f12dd4416471a48d91e67e4ed65871b811bd8493`. Astra personally assessed the prepared plan, coordinator findings P-002–P-008, source lifecycle provenance, effective expiry, compatibility rollout, security, testability, and remaining live acceptance dependency. No remaining concrete plan defect blocks implementation; the live source-to-chat acceptance remains an explicit open evidence item.

## Implementation gate

| Commit | Astra review | Opus final review | State |
| --- | --- | --- | --- |
| uncommitted source candidate | `ASTRA_PLAN_APPROVED`; personal implementation review in progress | `SUPERSEDED_BY_USER` | Backend compile/tests and an exact implementation commit remain open |

### Astra implementation findings

| ID | Severity | File/area | Finding | Fix/evidence | State |
| --- | --- | --- | --- | --- | --- |
| A-001 | — | — | Implementation was not started at the initial gate. | The exact v4 Astra plan SHA was recorded, Sol's runtime settings were verified through Heca, and the source candidate is now present. This historical gate is superseded by the implementation review entries below; no final approval is inferred. | RESOLVED_AFTER_INSPECTION |

### Opus final findings

| ID | Severity | File/area | Finding | Fix/evidence | State |
| --- | --- | --- | --- | --- | --- |
| F-001 | — | — | Opus final implementation review was not requested. | Superseded by the user's explicit removal of the Opus final gate; Astra owns final implementation approval. | SUPERSEDED_BY_USER |

## Evidence ledger

| Check | Command or artifact | Result | Date |
| --- | --- | --- | --- |
| Workstream brief and charter | Read `docs/plans/2026-10-27-nyxid-workstreams/{README.md,artifacts.md}` | Passed | 2026-10-27 |
| Issue body/comments | `gh issue view 2037 --repo ChronoAIProject/NyxID --json number,title,body,comments,url` | Passed; issue has no comments | 2026-10-27 |
| Sandbox source contract | `https://sandbox.chrono-ai.fun/openapi.json` | Verified OpenAPI 0.3.0; documented in plan | 2026-10-27 |
| Source inspection | async operation, MCP response, assistant attachment, and frontend renderer files | Passed; findings documented in plan | 2026-10-27 |
| Focused backend tests | Pending implementation | — | — |
| Frontend tests/typecheck | Pending implementation | — | — |
| Authorized source-to-chat acceptance | Requires live authorized Sandbox/Web account | OPEN | — |

## Sol implementation update — 2026-10-27

The initial implementation-gate and A-001 rows above are historical. Sol has
implemented the feature in the authorized worktree without creating, calling,
or awaiting a reviewer or subagent. Implementation validation is still open:
the coordinator's backend resource hold has not been released, and the live
Sandbox-to-originating-Web acceptance has not run. No backend compilation or
test pass is claimed for the current source.

### A-002 — generated-result model-context privacy

| ID | Severity | Area | Finding | Resolution and inspection evidence | State |
| --- | --- | --- | --- | --- | --- |
| A-002 | medium/security | `generated_artifacts/sandbox.rs`, `async_service_operation::input_context` | Bounded stdout/stderr can contain file names, paths, file content, or secrets and must not enter generated-artifact model context. | The parser validates and discards both streams. The encrypted generated summary contains only typed success, optional numeric exit code, and verified file count. `input_context` reconstructs only that closed metadata schema, withholding older free-form generated summaries while retaining the separate server-owned delivery state. Unmarked legacy text handling and existing size limits remain in place. Personally inspected the parser, persistence encryption call, context reconstruction, and attachment-listing filter; generated attachments use the fixed `Sandbox result` label and are excluded from the user-upload model listing. | RESOLVED_AFTER_INSPECTION; regression execution pending resource release |

Focused regressions prepared:

- `generated_summary_selects_metadata_and_discards_process_streams` puts path,
  file-content, and secret markers in stdout/stderr and file metadata, checks
  exact primitive-only summary JSON, rejects old/extra/string-valued summaries,
  and retains wire-type and response-size checks.
- `generated_artifact_persisted_summary_and_input_context_exclude_process_streams`
  completes the real fixture-backed operation, decrypts the persisted summary,
  and checks both it and admitted Event model context for marker exclusion. It
  also replaces saved summary ciphertext with an older free-form summary and
  verifies that context withholds it while retaining delivery metadata.

Direct `rustfmt --check` and `git diff --check` passed after this change. The
Rust regressions are prepared and unrun under the explicit backend resource
hold. The approved v4 plan remains unchanged at SHA-256
`7f5226f241b7a826c58be216f12dd4416471a48d91e67e4ed65871b811bd8493`.

Previously completed frontend evidence remains: 141 focused unit tests across
four files, TypeScript typecheck, lint with zero errors and 33 existing warnings,
and two Chromium fixture tests covering actual MP4 playback, exact downloads,
refresh/revisit, image regression, and decoder-error fallback. These fixture
tests do not satisfy the open live Sandbox acceptance.

### Range-policy follow-up from personal source inspection

The range-policy source fix now also handles HTTP's arbitrary-length decimal
positions without integer overflow. A syntactically valid start larger than the
file produces `416`; a huge end clamps to the last byte, and a huge suffix
selects the whole file with `206`. Decimal endpoint comparison preserves
malformed reverse-order rejection even when both values exceed `usize`.
Parser and owner-response tests now cover these cases, including leading zeros.
This corrects the earlier prepared test that treated an overflowing decimal as
malformed. Test execution remains held; no backend pass is claimed.

### A-003 — status/result lifecycle consistency fence

| ID | Severity | Area | Finding | Resolution and inspection evidence | State |
| --- | --- | --- | --- | --- | --- |
| A-003 | medium/correctness/security | `handlers/async_service_operations.rs::artifact_result`, `generated_artifacts/sandbox.rs::status` | Before/after status validation did not compare the versioned lifecycle snapshots, so an operation changing during result retrieval could still publish bytes. | The worker now compares the validated optional lifecycle version tokens before pending persistence. Changed values and presence/absence transitions fail closed with `artifact_source_changed`. Present malformed tokens return `artifact_invalid` instead of collapsing to absence. Both snapshots still require the exact operation, successful/result-available state and valid expiry; their minimum expiry is retained. The lifecycle version remains diagnostic metadata and a fetch consistency fence. Canonical `source_result_identity` still derives only from the exact operation and ordered verified file metadata/bytes. Personally inspected the fence placement before `artifacts::persist` and the unchanged identity, privacy, budget and legacy branches. | RESOLVED_AFTER_INSPECTION; regression execution pending resource release |

`generated_artifact_worker_fences_lifecycle_version_across_result_fetch` changes
the fixture's status snapshot when the result is fetched. Cases `1 -> 2`,
`1 -> absent`, and `absent -> 1` require `artifact_source_changed`, no manifest or
encrypted summary, zero pending-artifact rows and zero attachment rows. Stable
`1 -> 1`, `2 -> 2`, and `absent -> absent` controls retain one verified artifact
with the same canonical byte-derived identity contract and diagnostic version.
Every case verifies one result fetch and two status reads scoped to the exact
stored operation. The status parser regression also rejects malformed present
tokens. The uploads and API-discovery docs now describe this consistency fence.

Direct `rustfmt --check`, `git diff --check` and status JSON validation passed
after the source change. No Cargo/rustc/build/backend-test command was started
or queued: the coordinator's explicit resource hold remains active. These Rust
regressions are prepared and unrun. Existing frontend evidence remains unchanged;
live Sandbox-to-originating-Web acceptance remains open. The approved plan file
is unchanged at SHA-256
`7f5226f241b7a826c58be216f12dd4416471a48d91e67e4ed65871b811bd8493`.

## Coordinator resource-policy amendment — artifacts remains HOLD

The coordinator's explicit resource update supersedes the earlier unchanged
profile instructions for future backend commands. Every subsequent backend
Cargo test/build/check/clippy invocation must use the shared target
`/Users/chronoai/Desktop/projects/nyxid/target`, `CARGO_BUILD_JOBS=2`, and
`cargo --config '/Users/chronoai/Library/Application Support/heca/worktrees/eafaaf47/arctic-eagle/docs/plans/2026-10-27-nyxid-workstreams/cargo-resource-profile.toml' <subcommand>`.
Personal read-only TOML inspection confirms that this config changes only
package `nyxid` dev/test `debug=0` and `incremental=false`; it changes no
optimization, assertions, overflow, dependencies or test coverage. No extra
flags, independent profile changes, lock removal or stream cleanup are allowed.
The approved plan file remains unchanged; this user/coordinator amendment is
recorded separately.

The reported recovery of 16.49 GiB (23.40 GiB free) does not release this stream:
Telegram alone owns the next retry. Artifacts has no Cargo/rustc command started
or queued and remains ready for an explicit backend-window handoff.

Personal source inspection during this hold traced the A-003 comparison before
persistence, recovery's session-scoped policy/availability fences, origin-scoped
legacy HTTP behavior, fixed recovery audit/action gates, and metadata-only model
context. Attachment egress retains its shared reservation in the actual byte
owner through HTTP slices/clones; the private read moves the decrypted buffer
out with `mem::take` rather than creating an extra plaintext copy. This pass
found no additional concrete defect in those paths. It establishes source
readiness only; compilation, focused Rust regressions and live source-to-Web
acceptance remain open.

## Shared automation overlap review — 2026-10-27

The read-only sibling automation candidate was inspected in
`nyxid-automation` at the seven coordinator-identified overlap files. The
artifact boundary is preserved by the source candidate's admission and
consumption checks: `reserve_with_arguments` writes `artifact_mode` only when
the original delivery is `TurnOrigin::User`, has no channel/group/voice/
invocation/trigger provenance, and is `WebOnly`; `bind_in_session` then
requires the later consuming turn to be a server event whose exact event ID is
the watch ID, whose watch turn is the original user turn, and whose event
queue contains exactly one async completion event. Recovery selects only the
server-written delivery association and the owner Web message lineage.

The sibling automation continuation path selects only events carrying the
matching trigger run and protocol version, while ordinary async completion
selection excludes trigger-tagged events. Its Stop path removes the exact
coordinator run/event state before invoking the shared async cancellation,
which discards only pending artifact rows and the matching async completion
event. Thus a trigger continuation cannot admit an artifact watch, consume a
Web artifact completion event, materialize its pending bytes, or recover it by
client-selected lineage. The generated-artifact path does not alter
`coordinator_run_ids`, trigger continuation rows, or trigger event routing.

The current artifacts worktree predates the sibling's additive
`AgentEvent.trigger_run_id` and `continuation_protocol_version` fields, so no
cross-worktree import or duplicate contract edit was made. Combined acceptance
must exercise a Web artifact completion coexisting with a protocol-v2 trigger
event, prove the trigger event remains queued while the exact artifact event
is consumed, and prove Stop cancels each corresponding state without
resurrection. This is an integration obligation for the coordinator's merge,
not an unresolved defect in the artifact candidate. The pending-row
`materializing` state is currently cleanup-only and unreachable by design:
attachment insertion and the `pending -> materialized` transition share one
transaction, so crash recovery cannot expose a partially committed state.
