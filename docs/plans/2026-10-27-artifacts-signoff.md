# NyxID Artifacts ticket sign-off — #2037

**Review time:** 2026-10-27 08:29:45 UTC
**Reviewer:** GPT-6-Astra PM, Heca agent `01a17a9b-50ea-7b43-a3d2-b5f39245cfc4`  
**Implementer:** Heca agent `01a17afe-64ad-7ed1-bd1f-8b75d719d9f9`, provider `codex`, model `gpt-6.1-sol`, effort `xhigh`, mode `full-access`, Heca session `01a17afe-64ad-7ed1-bd1f-8b98e1a1ec1a`, provider session `01a17afe-665a-7d11-aa64-ebac87a34a20`  
**Worktree:** `/Users/chronoai/Library/Application Support/heca/worktrees/eafaaf47/nyxid-artifacts`  
**Worktree ID:** `01a17a96-a542-7c40-ac74-8a2076acc0a3`

## Verdict

**BLOCKED for issue #2037.** The source candidate has passed Astra’s personal source review and the frontend fixture checks, but the issue is not approved because backend compilation/tests/clippy, combined Automation–Artifacts acceptance, and the authorized live Sandbox-to-originating-Web journey have not produced evidence on the final source tip. `ASTRA_IMPLEMENTATION_APPROVED` is intentionally not recorded.

This is a ticket sign-off verdict, not a merge, deploy, production-send, or issue-closure authorization.

## Ticket and acceptance contract

Issue [#2037 — “[Enhancement][NyxBot] 支持在聊天中预览和下载工具生成成果，并恢复失败的交付”](https://github.com/ChronoAIProject/NyxID/issues/2037) is open. GitHub reports **zero comments**. The issue body defines these acceptance requirements:

1. A Web chat Sandbox video request must produce an actual result card in the same conversation; a supported MP4 must play/pause and download, with the bytes obtained and opened before claiming success.
2. Existing image generation must regress with thumbnail, full preview, and download of the same actual image.
3. The card must show name, type, size, and applicable expiry; preview/download must remain owner-authorized.
4. The owner must not install a machine agent, grant unrelated command authority, handle base64, or use a public upload to bypass access control.
5. “Generated successfully but delivery failed” must recover the original result without a new execution; expired results must be reported clearly.
6. A supported modification must create and distinguish a new version; a failed modification must not relabel old bytes.
7. Refresh/revisit must preserve access while retained; expiry/unauthorized access must be clear; unsupported preview must retain a real download action.
8. The product must distinguish generated-but-undelivered from delivered-and-readable and must not substitute a machine setup entry for cloud-result delivery.

The approved implementation plan is [2026-10-27-artifacts-implementation.md](2026-10-27-artifacts-implementation.md), revision **v4**, SHA-256:

`7f5226f241b7a826c58be216f12dd4416471a48d91e67e4ed65871b811bd8493`

The exact implementation candidate is:

`9886d2620d69ec456f3c1fd814ec12aa972ee2cc`

Its common/base commit is:

`b1dd6548ade04ec4f0051203e1353796331c7f87`

The candidate contains the 70-file Artifacts implementation. Documentation-only commits `2ee6583894838da82d924e590c5dddc3a72b7740` and `08983a95a19536000f41223bc25614f55e6378ac` are kept distinct from that source identity.

The coordinator-authorized shared lint prerequisite was reviewed and imported separately:

- Original reviewed prerequisite: `336405f8e667ea2eb2c6513effec3d15fd960b02`, parent `b1dd6548ade04ec4f0051203e1353796331c7f87`.
- Applied cherry-pick in this worktree: `5b44175af5d69af272200a2fc7f374d4b10355f2`.
- The applied commit changes exactly four baseline files: `login_code_service.rs`, `service_pool_routing.rs`, `lark_base_writes/ledger.rs`, and `nyxbot_gateway_release_service.rs`, with one equivalent guard rewrite per file (four rewrites, 8 additions and 8 deletions). `git diff --check` passed, and the original and applied four-file patch bytes compare identical. No Telegram feature changes were imported.

The current source is therefore the composition of the Artifacts candidate plus the independently reviewed four-file prerequisite and the bounded correction commits described below. The prerequisite and corrections are recorded as separate identities; the candidate identity remains `9886d262…`.

**Effective reviewed source tip for this report:** `47a2f3f1845b1896e172e68ec077bb8270ee2e33`. It is the final test-correction tip after prerequisite `5b44175af5d69af272200a2fc7f374d4b10355f2` and source correction `91ef35238a0e81f46628fd377d3d7f6a74c9b08c`; later documentation-only commits must be recorded separately and must not replace this source identity. Since backend execution is still pending, this effective tip is not a final approved implementation commit.

The original Opus plan/final gates were **SUPERSEDED_BY_USER**, not passed. No Opus reviewer was created or awaited. This report uses the authorized pure Astra–Sol workflow.

## Astra’s personal review scope

Astra read the exact candidate diff and surrounding contracts, then separately read the complete authorized prerequisite diff before import. The candidate review covered the operation-scoped Sandbox status/result worker and lifecycle-version fence; canonical source identity; bounded JSON/base64/decode/hash/encryption allocation; encrypted pending rows and BSON sizing; materialization and completion-event binding; idempotent delivery and owner/message recovery; retention, cancellation, erasure and migration; private attachment HTTP/range policy; MCP/native acknowledgement behavior; frontend metadata mapping, MP4 playback/error fallback, download, and recovery UI; and the read-only Automation overlap.

The review checked the seven shared files identified by the coordinator for Automation overlap. The Artifacts boundary remains original Web user admission, exact server-created completion-event consumption, owner/conversation lineage, and no trigger/channel/voice/group/invocation admission. Automation’s trigger continuation remains run/protocol-specific. Combined execution is still required to prove those source boundaries after integration.

The follow-up review resolved the concrete malformed-status finding and verified the production-router, reservation-lifetime and legacy-route corrections at `91ef35238…`. The same Sol review then added exact composed response-header assertions in test-only commit `47a2f3f1845b1896e172e68ec077bb8270ee2e33`; Astra inspected the final four-file plus one-file correction diff and found no further source or test-quality finding. This source result is not being substituted for backend execution or live acceptance.

The malformed-status correction is precise: an admitted generated-artifact watch with successful but malformed initial status JSON settles terminal `artifact_invalid` before any result fetch; malformed successful post-result status also settles `artifact_invalid` after exactly one result fetch. Ordinary async watches preserve `ValidationError("Invalid async operation status")` and their existing retry/defer behavior. The prepared regression asserts no pending artifact persistence, no materialization, and no repeated result fetch after terminalization.

The production-router regression uses `build_router_with_state(state.clone())`, then `private.with_state(state.clone())`, and calls the actual mounted owner attachment and empty-body artifact-delivery routes. Its success path uses a signed first-party owner JWT on a completed/idle conversation and checks exact range bytes, `206`, `Content-Range`, MP4 content type and safe filename. It also checks anonymous, foreign-owner, expired, delegated, API-key, service-account and relay rejection, no body disclosure and no recovery effect. Recovery's success selects the server-written message association and makes no Sandbox request; a non-empty body is rejected. No `AuthUser` injection was used.

The shared 192 MiB budget regressions keep the weighted reservation through decode, encryption, persistence and response-body consumption, and cover stale-lease release. The BSON regression encrypts the maximum 8 MiB file plus metadata, asserts the document is below MongoDB's 16 MiB limit, and rejects an oversized row. The earlier attempted legacy `thread_image` broadening was reverted; `assistant_uploads.rs` remains SHA-256 `a0815a26c651918e9c5498bc8cce3ce1e8d66b6b50c8d90d25359e47206a2a83` and continues `uploads::chat_bytes` behavior.

## Evidence ledger

| Area | Exact command/filter or artifact | Result and identity | State for sign-off |
| --- | --- | --- | --- |
| Source candidate and correction tips | `git show 9886d2620d69ec456f3c1fd814ec12aa972ee2cc`; `git show 91ef35238a0e81f46628fd377d3d7f6a74c9b08c`; `git show 47a2f3f1845b1896e172e68ec077bb8270ee2e33`; 70-file diff against `b1dd6548a` plus correction commits | Candidate, prerequisite `5b44175a…`, both correction tips and ancestry verified; Astra source review complete | **PASS — source review only** |
| Shared prerequisite | `git show 336405f8e667ea2eb2c6513effec3d15fd960b02`; `git diff --check`; original/applied patch byte comparison | Four files only; applied as `5b44175af5d69af272200a2fc7f374d4b10355f2`; equivalent semantics reviewed by Astra/coordinator | **PASS — prerequisite only** |
| Plan gate | SHA-256 of v4 plan | `ASTRA_PLAN_APPROVED`, exact SHA `7f5226f…` | **PASS** |
| Sandbox contract | Read `https://sandbox.chrono-ai.fun/openapi.json` | Chrono Sandbox API version `0.3.0`; result files expose path/base64/error/size and no MIME field | **PASS — contract inspection** |
| Frontend focused tests | `npm test -- src/components/assistant/blocks/tool-image.test.tsx src/components/assistant/blocks/generated-attachment.test.tsx src/components/assistant/blocks/artifact-delivery.test.tsx src/lib/assistant/nyxagent-transport.test.ts > /tmp/nyxid-artifacts-2037-unit.log 2>&1`, cwd `frontend/` | 4 files, 141 tests passed | **PASS — fixture/unit evidence** |
| TypeScript | `npx tsc -b --pretty false > /tmp/nyxid-artifacts-2037-tsc.log 2>&1`, cwd `frontend/` | Exit 0 | **PASS** |
| ESLint | `npx eslint src/components/assistant/blocks/generated-attachment.tsx src/components/assistant/blocks/generated-attachment.test.tsx src/components/assistant/blocks/artifact-delivery.tsx src/components/assistant/blocks/artifact-delivery.test.tsx src/lib/assistant/attachment-errors.ts e2e/generated-artifacts.spec.ts playwright.artifacts.config.ts`, cwd `frontend/`, final output `/tmp/nyxid-artifacts-2037-lint.log` | 0 errors, 33 pre-existing warnings. An earlier run caught the valid `Date.now` render-purity error; the source was corrected and this final command passed. | **PASS — warnings recorded** |
| Browser fixture | `npx playwright test --config=playwright.artifacts.config.ts > /tmp/nyxid-artifacts-2037-browser.log 2>&1`, cwd `frontend/`; config uses port `49237` and one worker | 2 Chromium tests passed: real MP4 playback/time advance/pause, byte-identical video/image downloads, refresh/revisit, decoder-error download fallback | **PASS — fixture only** |
| Browser fixture correction | `/tmp/nyxid-artifacts-2037-browser-fixture-shape-failure.log` | Initial unrelated machine-list fixture shape failure corrected; final run passed without reducing checks | **RESOLVED** |
| Rust formatting | Direct `rustfmt --edition 2024 --config skip_children=true` over all 44 changed Rust files; no Cargo invocation | Passed; includes A-002/A-003/range changes | **PASS — formatting only** |
| Correction formatting and hygiene | `rustfmt --edition 2024 --config skip_children=true --check backend/src/handlers/async_service_operation_tests.rs backend/src/handlers/async_service_operations.rs backend/src/handlers/generated_artifact_delivery_tests.rs backend/src/services/generated_artifacts/tests.rs`; `git diff --check`, cwd `/Users/chronoai/Library/Application Support/heca/worktrees/eafaaf47/nyxid-artifacts` | Both passed on `91ef35238…`; no Cargo/rustc/backend test/build/check/Clippy command was launched or queued | **PASS — lightweight validation only** |
| Final mounted-header assertions | `rustfmt --edition 2024 --config skip_children=true --check backend/src/handlers/generated_artifact_delivery_tests.rs`; `git diff --check`, cwd `/Users/chronoai/Library/Application Support/heca/worktrees/eafaaf47/nyxid-artifacts` | `47a2f3f1845b1896e172e68ec077bb8270ee2e33` asserts exact `Content-Length`, `Accept-Ranges`, cache/security headers, CSP and generated `Content-Disposition`; passed lightweight checks only | **PASS — lightweight validation only** |
| Malformed-status regression | `generated_artifact_malformed_status_is_terminal_before_or_after_result_fetch` in `backend/src/handlers/generated_artifact_delivery_tests.rs` | Prepared raw malformed JSON fixture; execution pending backend hold | **OPEN** |
| Production-router regression | `generated_artifact_production_routes_enforce_owner_bytes_and_recovery_lineage` in `backend/src/handlers/generated_artifact_delivery_tests.rs` | Prepared real `build_router_with_state`/`private.with_state` mounted owner and recovery checks with signed first-party owner credential; execution pending backend hold | **OPEN** |
| Byte-budget/BSON regressions | `generated_artifact_worker_reservation_spans_persist_and_releases_on_error`; `generated_artifact_worker_reserves_shared_budget_before_result_body_and_releases_on_cancel`; `maximum_encrypted_pending_document_fits_bson_and_rejects_oversize` | Prepared shared 192 MiB lifetime and encrypted 8 MiB/16 MiB BSON assertions; execution pending backend hold | **OPEN** |
| Diff hygiene | `git diff --check` | Passed | **PASS** |
| Backend build | Required shared-target Cargo build/test commands | Not run after implementation changes because coordinator’s Artifacts resource hold remains active; no Cargo/rustc command was queued | **OPEN** |
| A-002 regression | `generated_summary_selects_metadata_and_discards_process_streams`; `generated_artifact_persisted_summary_and_input_context_exclude_process_streams` | Prepared in `generated_artifact_delivery_tests.rs`; execution pending backend window | **OPEN** |
| A-003 regression | `generated_artifact_worker_fences_lifecycle_version_across_result_fetch` | Prepared with `1→2`, `1→absent`, `absent→1` rejection and stable-version controls; execution pending backend window | **OPEN** |
| Generated-artifact service tests | Filters including `generated_artifacts_materialize_history_owner_http_ranges_recovery_and_retention`, `generated_artifacts_exact_event_binding_cancellation_capacity_and_recovery`, expiry/lease, admission rollback, identity and failure cases | Prepared; no compiled/test result after implementation changes | **OPEN** |
| Clippy | `cargo clippy --config '/Users/chronoai/Library/Application Support/heca/worktrees/eafaaf47/arctic-eagle/docs/plans/2026-10-27-nyxid-workstreams/cargo-resource-profile.toml' --manifest-path backend/Cargo.toml --all-targets -- -D warnings` | Artifacts run not started; coordinator requires the corrected argument order and shared target/jobs=2 | **OPEN** |
| Combined acceptance | Artifacts-applicable integration checklist rows **I-01–I-05 and I-07** in `integration-acceptance.md` (I-06 is Permissions/Automation-owned) | No integrated commit/test run; exact event selection, trigger isolation, Stop ordering, acknowledgement denial, and route coexistence remain unobserved | **OPEN** |
| Live Sandbox-to-Web journey | Authorized real Sandbox source/result/attachment bytes, playback/download/recovery | Not run; no verified feature build deployed and no live client journey evidence | **OPEN** |

The frontend test log confirms the fixture assertions but does not establish a live Sandbox result, deployed version, or production-originating Web journey. The planned backend test command and clippy command are intentionally not represented as passes.

## Criterion-by-criterion disposition

| Issue criterion | Disposition | Evidence / gap |
| --- | --- | --- |
| Web MP4 result card, playback, pause, download from actual bytes | **BLOCKED** | Fixture Chromium test passes, but no authorized live Sandbox-to-Web journey has obtained the actual result bytes on the implementation tip. |
| Existing image thumbnail, full preview and download regression | **PARTIAL** | Fixture test covers image regression and exact download; backend and live evidence are open. |
| Name/type/size/expiry and owner-only access | **PARTIAL** | Source review, metadata mapping and fixture behavior support the contract; backend retention/ACL execution is open. |
| No machine install, unrelated authority, base64 handling, or public bypass | **PARTIAL** | Recovery schema is empty and server-bound; backend acknowledgement/authorization tests and live behavior remain open. |
| Failed delivery recovers original result without a new execution | **PARTIAL** | Recovery source path and prepared tests enforce server-selected association; backend execution and live journey remain open. |
| Supported modification creates a distinct version and failed modification preserves old bytes | **BLOCKED** | Source identity and failure-preservation paths are reviewed; no backend execution or live modification result is available. |
| Refresh/revisit, expiry, unauthorized access, and unsupported-preview download fallback | **PARTIAL** | Browser fixture covers revisit, decoder failure and exact download; backend retention/HTTP tests and live evidence remain open. |
| Distinct generated/delivered states and no machine setup substitute | **PARTIAL** | State model and UI are reviewed; backend and live acceptance are open. |

## Resolved findings and proof status

| Finding | Resolution | Evidence status |
| --- | --- | --- |
| C-F1 mixed-writer compatibility and Web admission | Default-off `assistant:generated-artifacts` admission flag with readiness/drain/rollback contract; only original Web user provenance is admitted; already-admitted watches drain | Source and plan reviewed; mixed-worker and combined execution evidence open |
| C-F2 Sandbox identity/version race | Exact operation plus verified metadata/decoded-byte identity; lifecycle `version` is a status/result consistency fence, not content revision; before/after presence/value changes fail closed | Source reviewed; A-003 execution open |
| C-F3 shared 192 MiB budget and BSON limit | One reservation spans body, JSON/base64/decode/hash/encryption/persistence/egress; the prepared `maximum_encrypted_pending_document_fits_bson_and_rejects_oversize` test asserts that the encrypted maximum 8 MiB file plus bounded metadata serializes below MongoDB’s 16 MiB document limit and rejects an oversize serialized row | Source reviewed; backend execution open |
| C-F4 MP4 validity and HTTP behavior | Structured visual-track parser; AVC Baseline preview, unsupported/high-profile download fallback, decoder-error fallback, and correct single-range semantics | Source/frontend fixture reviewed; backend/live execution open |
| C-F5 retry authority | Empty-body `nyx__artifact_delivery` and owner/message-bound HTTP recovery select only the server association and never submit a new execution | Source reviewed; backend and live execution open |
| C-F6 completion provenance, expiry, flag semantics | Saved original Web provenance binds the exact server-created completion event; effective expiry is the minimum source/watch/normal-retention deadline; flag disable blocks new admission while admitted work drains | Source reviewed; combined execution open |
| A-002 process-stream disclosure | Parser discards stdout/stderr and persists/contextualizes only closed typed metadata; old free-form generated summaries are withheld | Source reviewed; two regressions prepared but unrun |
| A-003 lifecycle-version drift | Added, removed, or changed status versions fail before pending persistence; stable versions remain diagnostic only | Source reviewed; regression prepared but unrun |
| Range overflow/reverse-order handling | Arbitrary-length decimal positions use saturating arithmetic plus full decimal ordering; valid unsatisfiable ranges return `416` with `bytes */size`, unsupported/malformed ranges are ignored with `200` | Source reviewed; parser/HTTP tests prepared but unrun |
| Delivery lineage/idempotence | Exact saved watch/event/operation/source/original-turn/attachment lineage, unique association, `setOnInsert`, and typed readback | Source reviewed; backend execution open |
| Legacy HTTP behavior | Generated response policy is origin-scoped; existing tool documents and user images retain their stored MIME/disposition behavior | Source reviewed; regression execution open |
| Recovery audit/acknowledgement | Fixed audit metadata, owner Change acknowledgement, empty schema, and server-written continuation/card; no client-selected lineage | Source reviewed; backend/combined execution open |
| Automation overlap | Read-only sibling assessment found no Artifacts source defect; trigger events remain run/protocol-specific and Stop ordering is preserved | Combined I-01–I-04 execution open |

## Smallest remaining blockers and owners

1. **Coordinator-owned backend window:** after Automation releases the shared Cargo window, run the Artifacts candidate with `/Users/chronoai/Desktop/projects/nyxid/target`, `CARGO_BUILD_JOBS=2`, normal locking, and the required `cargo-resource-profile.toml`. Start with the generated-artifact-focused no-run/test filters, then run affected assistant/attachment regressions and all-target warnings-denied Clippy with `--config` after `clippy`. This is the smallest local action that can turn the prepared Rust evidence into execution evidence.
2. **Same-Sol repair loop:** every compiler/test/clippy finding must be sent to the existing Sol agent, personally reviewed by Astra, and rerun on a new exact source tip. No new agent is authorized.
3. **Combined integration:** on an integration commit containing the reviewed Automation and Artifacts sources, execute the Artifacts-applicable rows I-01–I-05 and I-07 from `integration-acceptance.md`, naming the exact commits, command, fixture database and assertions. I-06 remains a Permissions/Automation-owned row. Independent passes do not satisfy this gate.
4. **External acceptance:** obtain an authorized live Sandbox-to-originating-Web journey with real source/result bytes, MP4 playback/download, image regression, failed-delivery recovery without a new execution, retention and modification behavior. Fixture browser tests cannot substitute for this.
5. **Final approval record:** after all applicable criteria and findings pass on the exact final implementation commit, reread the complete final diff and relevant surrounding code, then record `ASTRA_IMPLEMENTATION_APPROVED` against that exact SHA and update this report. Until then the verdict remains **BLOCKED**.

The report itself is documentation; it does not release the backend hold or alter the source candidate. No merge, deployment, production send, or issue closure is authorized.
