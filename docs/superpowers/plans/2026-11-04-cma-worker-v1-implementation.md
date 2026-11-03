# CMA Worker v1 NyxID Implementation Checklist

**Goal:** Implement the NyxID side of the authoritative [source plan](../../../2026-11-03-cma-worker-v1-implementation.md): one personal-owner Sandbox slot, durable creation/Agent references, CMA operations and native bridges, first-party management, and the Agent execution view.

**Architecture:** Keep task submission/result delivery in `cma_operation` and assistant watches. Add the owner-authenticated catalog gateway, final-dispatch admission, durable reference/creation services, management handlers and frontend. The reconciled [design](../specs/2026-11-04-cma-worker-v1-design.md) describes the current contract.

**Status convention:** A checked implementation item means the code or regression source is present. It does not by itself claim that a test command ran or passed. The original plan's historical red-test steps are explicitly **not reconstructed** below. Final validation and local integration evidence are recorded under Task 7 and in the linked validation record. Original per-task commit recipes are superseded by a coordinated final review and local implementation commit.

## Task 1: Rollout flag and durable records

**Files:** `backend/src/models/nyxbot_cma_{sandbox,agent,creation}.rs`, `backend/src/models/mod.rs`, `backend/src/services/feature_flag_service.rs`, `frontend/src/lib/feature-flags.ts`.

- [x] Add the personal `assistant:cma-sandboxes` flag with default `false` and the matching frontend constant.
- [x] Add the Sandbox model with owner/service identity, occupied slot, generation, `agent_reference_version`, remote state/capacity and retirement facts.
- [x] Add the Agent model with exact remote/parent identities, parent generation, safe observation/profile fields and bounded source metadata.
- [x] Add a separate `nyxbot_cma_creations` model for allocation identity and encrypted recovery independent of chat/watch retention. Include deployment fingerprint, exact request hash/key/method/path, parent binding, receipt IDs, revision, submission generation, first-dispatch and recovery-check timestamps.
- [x] Use BSON helpers for required/optional dates and encrypted bytes; keep recovery material out of `Debug` and public DTOs. Add serialization/redaction regressions.

**Historical red-test steps — not reconstructed:** the earlier missing-model/missing-module failure sequence is not established by retained evidence. The regression sources are present; the final runs are recorded under Task 7.

## Task 2: Gateway, one-Sandbox admission and durable recovery

**Files:** `backend/src/services/nyxbot_cma_gateway.rs`, `nyxbot_cma_sandbox_service.rs`, `nyxbot_cma_admission.rs`, `backend/src/services/mod.rs`, plus final dispatch in proxy/MCP execution.

- [x] Define `CmaGateway` and production `CatalogCmaGateway` for Sandbox/Agent inventory and detail, Workspace-parent lookup, lifecycle reads/commands, transcript, events, Responses/controls, attachment writes and Desktop.
- [x] Hydrate optional configured CPU/memory from the exact Sandbox profile ID/revision. Reuse pins within each page and cap hydration at eight reads/two seconds; unavailable, denied or internal profiles remain unknown without blocking inventory.
- [x] Resolve the active canonical `cma` service and personal owner, enforce supported signed-identity configuration and live catalog access, pin the execution fingerprint, and bound destinations, bodies, pages and cursors. Preserve the distinction between explicit local owner mismatch and unavailable upstream authorization observations.
- [x] Reserve the one-owner slot transactionally with the erasure fence and partial unique index. Hash and encrypt the **exact original outbound request bytes**, preserving whitespace, object order and escapes. Parse/validate without normalizing or rewriting the approved request.
- [x] Record every allocation in the separate creation collection, including an Agent created under an existing parent. Fence dispatch and receipt updates with owner, deployment, original method/path/key/hash, parent generation, creation revision and submission generation.
- [x] Cover all supported final-dispatch allocation paths: permanent lifecycle `POST /api/v1/agents`, direct `POST /api/v1/sandboxes`, and `POST /api/v1/workspaces/{id}/agents`. Refuse unsupported allocation surfaces and connection-alias bypasses.
- [x] Distinguish lifecycle, direct Sandbox and direct Agent receipts. Validate exact operation/Agent/parent/Workspace identities before registration. Preserve saved receipts against late unknown/no-resource results.
- [x] Retain permanent same-key/body lifecycle replay. For generic allocation endpoints with CMA's 24-hour cache, persist first dispatch and reject uncertain replay at 23 hours, after clock rollback, or when legacy unknown records lack that timestamp. A durable direct resource receipt uses observation instead of another POST. Proven no-resource reacquisition advances submission generation.
- [x] Add bounded read-only recovery to Sandbox list/detail: at most eight known operation/resource identities, a three-second budget and persisted polling cooldown. Do not automatically resend uncertain creation, replace its key/body, or release an unknown/failed slot.
- [x] Drain complete remote Sandbox inventory; retain excess resources as visible cleanup references. Fold receipt/discovery duplicates into the original creation identity transactionally, including Agent generation bindings and retirement intent.
- [x] Implement safe list/detail/claim/label/command/event projections. Pause, revive, failure and missing reads keep the slot. Retirement releases it only after an exact owner-authenticated `retired` observation.
- [x] Add pure and optional replica-set regressions for raw-byte identity/encryption, concurrent slot/key races, replay deadlines, stale replies/generations, erasure, atomic no-resource release, receipt/discovery folding, bounded recovery and recovery without chat/watch rows.

**Historical red-test steps — not reconstructed:** the original missing-service failure run is not claimed. `--lib` was not a valid backend test target; all commands below use package `nyxid` and binary `nyxid-server`.

## Task 3: Agent references and first-party REST

**Files:** `backend/src/services/nyxbot_cma_agent_service.rs`, `backend/src/handlers/nyxbot_cma_{sandboxes,agents}.rs`, handler/service exports, `backend/src/routes.rs`, `backend/src/mw/auth.rs`.

- [x] Register live Agent observations using the owner/service/remote identity and an exact parent-generation write fence. Reject substituted parents and unsafe profile/source exposure.
- [x] Require a first-party human, active personal owner and feature flag for management handlers. Use dedicated DTOs and local reference IDs.
- [x] Mount the following routes under `/api/v1/assistant/nyxagent`:

| Methods | Path |
| --- | --- |
| GET | `/sandboxes` |
| GET | `/sandboxes/unregistered` |
| POST | `/sandboxes/claim` |
| GET, PATCH, DELETE | `/sandboxes/{id}` |
| POST | `/sandboxes/{id}/commands` |
| GET | `/sandboxes/{id}/events` |
| GET | `/cma-agents/{id}` |
| GET | `/cma-agents/{id}/transcript` |
| GET | `/cma-agents/{id}/events/snapshot` |
| GET | `/cma-agents/{id}/events` |
| GET | `/cma-agents/{id}/responses/{response_id}` |
| GET | `/cma-agents/{id}/responses/{response_id}/events` |
| GET | `/cma-agents/{id}/desktop` |
| GET | `/cma-agents/{id}/desktop/ws` |

- [x] Validate command generation/body and exactly one stable `Idempotency-Key`. Treat `202` as pending, not as retirement completion.
- [x] Keep snapshot `page_cursor`, Agent SSE `after_cursor` and Response SSE `starting_after` distinct. Preserve native event identities while bounding streams and periodically revalidating session, owner, flag, deployment, parent/generation and exact Response binding.
- [x] Relay CMA RFB binary traffic with fixed Agent identity, live authority checks, session duration/frame bounds and exact Origin policy. Leave shared-Sandbox viewer contention to CMA.
- [x] Add explicit coverage showing the existing assistant-family delegated GET denial covers every new read, stream and upgrade route.
- [x] Add handler/service/parser regressions for first-party/flag/owner gates, safe DTOs, immutable parents, exact cursors/Responses and Desktop capability/Origin states. Deduplicate identical durable transcript messages reused across failed/retried turns; reject conflicting reuse.

**Historical red-test steps — not reconstructed:** missing-handler/missing-route red runs are not asserted. Final binary-target handler and service execution remains an evidence item under Task 7.

## Task 4: Catalog overlay, native bridges, guidance and watch integration

**Files:** `backend/specs/catalog/cma.openapi.json`, `backend/src/services/catalog_spec_registry.rs`, `nyxbot_cma_tools.rs`, `assistant_account_tools.rs`, `assistant_nyxagent_runtime.rs`, `assistant_service_execution/cma.rs`, `cma_operation.rs`, and the existing acknowledgement/watch integration.

- [x] Embed and register the canonical `cma` OpenAPI overlay with its actual operation IDs and action classes:

| Area | Operation IDs |
| --- | --- |
| Agents | `list_agents`, `create_agent`, `get_agent`, `get_provisioning`, `stop_agent`, `delete_agent` |
| Sandboxes | `list_sandboxes`, `get_sandbox`, `pause_sandbox`, `revive_sandbox`, `retire_sandbox`, `list_sandbox_events` |
| Work/history | `run_task`, `get_task`, `list_messages`, `get_transcript`, `get_task_controls`, `control_task`, `interrupt_task` |
| Files/observations | `list_files`, `read_file`, `download_file`, `write_file`, `get_resources`, `get_usage` |

- [x] Target `cma__run_task` at `POST /api/v2/agents/{agent_id}/responses` with `stream=true`. Validate the exact Agent/Response/turn/revision and strict control union at execution when the hosted schema subset cannot express the tagged union.
- [x] Implement `nyxid__list_cma_resources`, `nyxid__cma_save_attachment`, and `nyxid__cma_request_owner` with schemas, descriptions and action classes. Resolve the selected Agent and live personal authority; exclude guest, shared/collaboration and organization contexts.
- [x] Apply native grant/key/access-mode/input checks to the actual outbound method, path, query and body, including inventory pages and attachment content. Copy only original attachments from the current conversation; return path/size/SHA256 and retain action-policy acknowledgement.
- [x] Create owner-action cards for the exact local Agent and resume on the owner's decision through existing acknowledgement/event machinery. Keep task results in the original watch/chat flow.
- [x] Inject stable CMA guidance through the existing guidance seam and a separate current snapshot of at most eight Agents. Exclude snapshots from the stable fingerprint and omit transcripts, credentials and other chats' task bodies.
- [x] Associate guarded watch observations with durable creation/Agent references. Read approval identities from bound Agent events rather than inventing them from controls responses. Preserve validated observed external callbacks without depending on them for internal navigation.
- [x] Add overlay, native-authority/input, control, callback, owner-action and observation regression sources.

**Historical red-test steps — not reconstructed:** the earlier missing-spec/tool/guidance failures are not claimed. Existing and new regression sources are checked by the coordinated final test runs.

## Task 5: Frontend schemas, hooks, routes and Sandbox management

**Files:** `frontend/src/schemas/nyxbot-cma-{sandbox,agent}.ts`, `frontend/src/hooks/use-nyxbot-cma-{sandboxes,agents}.ts`, `frontend/src/lib/assistant/cma-http.ts`, Sandbox list/detail pages, router, sidebar, search and assistant workspace integration.

- [x] Add bounded Zod parsing, typed owner-aware Query hooks, stable lifecycle keys and invalidation for local labels, claim, pause/revive and retirement.
- [x] Add `/assistant/sandboxes` and `/assistant/sandboxes/$sandboxId`, with feature-gated sidebar/search access alongside Machines.
- [x] Implement empty, pending/unknown, active, paused, retiring/retired and excess-inventory conflict states; nullable capacity/activity; safe billing attribution; local rename; claim and retirement confirmation.
- [x] Keep Agent management and Desktop controls off Sandbox pages and avoid resource creation on page open.
- [x] Add schema, HTTP helper, hook, page, navigation/flag and mutation-state regression sources.

**Historical red-test steps — not reconstructed:** missing-module Zod/hook test failures were proposed in the original plan but are not presented as observed history. Final frontend counts and checks remain in Task 7's evidence record.

## Task 6: Agent execution, task-card navigation and noVNC

**Files:** `frontend/src/pages/assistant-cma-agent.tsx`, `frontend/src/lib/assistant/cma-events.ts`, `frontend/src/components/assistant/cma-desktop-panel.tsx`, `cma-operation-card.tsx`, `nyxagent-acknowledgement-card.tsx`, Agent hooks/router, `frontend/src/types/novnc.d.ts`, frontend dependency manifests and relevant tests.

- [x] Add `/assistant/cma-agents/$agentId` with an optional exact `response` and `tab=desktop`. Task and owner cards point to the verified local reference; missing response binding does not select the latest task.
- [x] Separate current task output from clearly labelled shared Agent history. Keep Agent/Response cursor state distinct and deduplicate event identities while retaining reconnect/error state.
- [x] Render CMA's RFB protocol using pinned `@novnc/novnc` through the independent CMA WebSocket relay. Do not reuse the Machine Desktop JSON transport.
- [x] Handle no-capability, paused/stopped/deleted/failed, busy and unavailable-session reasons without switching Agents or creating resources. Close the viewer connection when leaving while preserving remote task/watch execution.
- [x] Preserve validated observed external callbacks as optional links; local execution navigation works without a CMA callback producer.
- [x] Add event, Agent page, task-card, owner-card and noVNC lifecycle/error regression sources. Add `frontend/e2e/cma-worker.spec.ts` for browser transport fixtures and desktop/mobile states.

**Historical red-test steps — not reconstructed:** the proposed pre-implementation event/page failure sequence is not reconstructed. Fixture browser tests and noVNC unit tests do not establish a real CMA RFB session.

## Task 7: Startup, erasure, boundaries and final validation

- [x] Register Sandbox, Agent and creation indexes in `backend/src/db.rs`, including the occupied-owner and permanent creation-key fences.
- [x] Add all three owned collections to personal erasure and use the persistent erasure barrier for reference/creation writes.
- [x] Record external CMA quota/callback work, the login-key contract discrepancy, Remote Machine removal and excluded Sandbox-level APIs in `docs/2026-11-04-cma-worker-v1-out-of-scope.md`.
- [x] Reconcile this generated checklist and design with exact raw bytes, separate creation durability, permanent-versus-generic retry rules, actual routes/tools and noVNC/RFB.
- [x] Record final backend compile and focused test evidence: the final binary compiled, 244 tests passed with one pre-existing live-CMA fixture ignored, and the separately selected account-parity regression passed. All ten new Mongo-backed transactional cases were explicitly rerun with `--nocapture` against the isolated replica set and passed without skip messages; see the [validation record](../../2026-11-04-cma-worker-v1-validation.md).
- [x] Record final frontend test counts, type/build results and scoped lint results after the final source edits: 140 related Vitest tests, current production build and scoped ESLint passed; see the [validation record](../../2026-11-04-cma-worker-v1-validation.md).
- [x] Record the final Playwright fixture run, viewport coverage and any limitations: 11 tests passed at desktop/mobile widths, with all CMA API/WS traffic intercepted; see the validation record for the live CMA boundary.
- [x] Review the final diff and preserve implementation, tests and validation in a local commit on `codex/cma-worker-v1`. Keep the worktree and the untracked source/preview references. No push, PR, merge or deployment is part of this delivery.

### Backend verification commands

Run from the repository root. The backend package has a binary target; there is no backend library target. Supply `NYXID_TEST_DATABASE_URL` for a disposable MongoDB replica set when running transactional tests. A skip is not a passing integration test. Use the CI stack size and low-memory build settings on this machine:

```bash
cargo check -p nyxid --bin nyxid-server --tests
RUST_MIN_STACK=8388608 CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 \
CARGO_PROFILE_TEST_DEBUG=0 CARGO_PROFILE_DEV_DEBUG=0 \
NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27019/?replicaSet=cmaTest' \
cargo test -p nyxid --bin nyxid-server -- \
  cma async_service_operation_tests assistant_authority_tests \
  assistant_invocations::tests feature_flag_service catalog_spec_registry \
  assistant_account_tools assistant_instruction_context \
  two_accounts_keep_real_credentials_memory_tools_payers_and_web_continuity \
  --test-threads=4
```

This combined reproduction command includes the corrected account-parity filter. The validation record preserves the actual main run and the separate account-parity/transactional follow-ups, including counts and log locations. `cargo fmt --all --check` and staged/unstaged `git diff --check` passed; no historical passing result is inferred from the commands alone.

### Frontend and browser verification commands

Run from `frontend/`. These paths correspond to current test files; Sandbox detail behavior is covered by the combined Sandbox page suite rather than a nonexistent separate detail test.

```bash
npm test -- src/schemas/nyxbot-cma-sandbox.test.ts src/hooks/use-nyxbot-cma-sandboxes.test.tsx src/hooks/use-nyxbot-cma-agents.test.tsx src/lib/assistant/cma-http.test.ts src/lib/assistant/cma-events.test.ts src/pages/assistant-cma-sandboxes.test.tsx src/pages/assistant-cma-agent.test.tsx src/components/assistant/cma-operation-card.test.tsx src/components/assistant/cma-desktop-panel.test.tsx src/components/assistant/nyxagent-acknowledgement-card.test.tsx
npx tsc -b
npm run build
npx eslint src/schemas/nyxbot-cma-sandbox.ts src/schemas/nyxbot-cma-agent.ts src/hooks/use-nyxbot-cma-sandboxes.ts src/hooks/use-nyxbot-cma-agents.ts src/lib/assistant/cma-http.ts src/lib/assistant/cma-events.ts src/pages/assistant-cma-sandboxes.tsx src/pages/assistant-cma-sandbox-detail.tsx src/pages/assistant-cma-agent.tsx src/components/assistant/cma-operation-card.tsx src/components/assistant/cma-desktop-panel.tsx src/components/assistant/nyxagent-acknowledgement-card.tsx
npx playwright test e2e/cma-worker.spec.ts --workers=1
npx vite build --config cma-preview.vite.config.ts
```

Record additional changed navigation/feature-flag suites and final-copy lint in the coordinated evidence. The standalone preview build is design-artifact validation; it does not replace the product build or browser transport checks.

### Acceptance boundaries

- [x] Fill in final local command results and required follow-up from the coordinated run; see the validation record.
- Live CMA acceptance was not performed: real identity/catalog configuration, effective owner quota, initial-login/key recovery, authenticated SSE, and a real RFB viewer remain target-deployment conditions. Fixture-only runs do not establish those results.

The login-key contract mismatch and target CMA owner quota remain external conditions, not permission to rotate keys or claim the one-environment guarantee for requests sent directly to CMA. Follow the user's repeated-blocker rule: after more than three unsuccessful attempts on the same issue, document the evidence/impact and move on to independent work.
