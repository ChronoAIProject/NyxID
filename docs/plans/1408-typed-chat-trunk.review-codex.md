# Adversarial Review: NyxID #1408 typed chat trunk

Reviewed against NyxID `04602a74` and Aevatar `0a86713671fcf551dc19ad86b1b6aa8ae6cb980b`.

## BLOCKERS

### 1. PR 3 cannot land green: its deletion list omits compiling backend tests that directly call the deleted handlers.

**Evidence:** The plan promises that every PR lands green and deletes `workflow_chat`, `workflow_chat_ws`, and `get_create_recovery` in PR 3 ([plan](1408-typed-chat-trunk.md:141), [plan](1408-typed-chat-trunk.md:260)). `backend/src/handlers/proxy.rs:8985`, `:9144`, `:9483`, `:9582`, and `:9613` directly call those functions. These are not dead strings; removing the handlers leaves test compilation with unresolved symbols. The same integration test asserts the removed recovery route at `backend/src/handlers/proxy.rs:9204-9218`. `backend/src/mw/auth.rs:1485` also keeps the removed WebSocket route in the protocol-denial test.

**Why it matters:** PR 3 is explicitly claimed to be independently mergeable. It is not. More importantly, the existing test is the only concrete trace here proving `chatc-*` history and delete are still routed to scoped chat history (`backend/src/handlers/proxy.rs:9092-9141`); deleting it wholesale would erase the regression proof the plan relies on.

**Fix:** Make removal/rewrite of the proxy integration test and the delegated-route test an explicit PR 3 task. Retain and narrow tests for list, `chatc-*` GET, `chatc-*` DELETE, typed GET/DELETE/state, and assert removed facade routes return 404. Do not merely delete the whole test block.

### 2. The proposed `plan.resolve` conditional is insufficient and will strand pending-gate conversations.

**Evidence:** PR 2 is declared inert ([plan](1408-typed-chat-trunk.md:203-205)); PR 3 conditionally moves only the client-side `plan.resolve` command forward ([plan](1408-typed-chat-trunk.md:280-281)). The baseline has no decoder for `nyxid.task.snapshot`, `nyxid.task.step.changed`, or control frames ([plan](1408-typed-chat-trunk.md:68-70)). Yet the exact command needs actor-provided `taskId`, `planId`, `requestId`, `planRevision`, and `expectedStateVersion` (canon `docs/canon/nyxid-chat-api.md:420-436`). Those facts live in the task/current-state projection, not in `RUN_STARTED`.

**Why it matters:** A deployment answering `confirm/pending` does not merely need a POST helper. Before PR 4, the browser has no modeled gate, no values to send, and no UI to invoke it. The turn hangs exactly as the plan says it must not. Moving a command builder without its observed-fact pipeline is a broken split.

**Fix:** Gate PR 3 on an active, wired PR 2 projection, not just its files landing. If Q1 is pending, PR 3 must include decode/reduce/render/submit for the plan gate plus its reload path, or PR 4 must land before the flip. Add a fixture-driven test that starts with a pending gate, submits the exact body, observes the committed result, and proves that a missing or stale field cannot dispatch.

### 3. PR 2 does not specify a decoder/reducer for the canonical state it claims to converge.

**Evidence:** Its new schema is only `NyxIdChatTaskPlan`, step, and `step.changed`; its reducer lists snapshot, step, control, step-control, and continuation ([plan](1408-typed-chat-trunk.md:210-234)). The canon's `/state` snapshot also owns `activeTurn`, `latestTurn`, `pendingInput`, `pendingApproval`, `pendingActions`, resolution facts, control fences, action summaries, attention, `scopeId`, and `progressSequence` (canon `docs/canon/nyxid-chat-api.md:614-670). Live frames additionally include action, input, and approval request/change frames (canon `:159-170`). The working reference has a projection for all of these (`StudioAssistant/actor-state.js:24-50`, `:116-211`) and specifically reduces input, approval, and action events (`:88-112`).

**Why it matters:** The advertised one-model convergence is false if reload loses a pending input/approval/action, a continuation fact, or a control fence. It also leaves no safe source for the existing action journey after reload. This is not a cosmetic card gap; it changes whether the browser can submit actor-owned decisions correctly.

**Fix:** Define the full projection schema and reducer boundary: state envelope, task plan, input/approval/action/control/continuation facts, identities, state/progress sequence, and terminal state. State exactly which existing card reducer is retired or adapted. Add convergence fixtures for pending input, approval, action, plan gate, action continuation, retry/skip result, and each `/state` result status.

### 4. The planned typed-ID hardening still fails open at resource and stream identity boundaries.

**Evidence:** PR 1 adds a typed-family check only to the nine `/chat` parse arms ([plan](1408-typed-chat-trunk.md:188-196)). Resource requests still call `conversation_resource_family`, which accepts any syntactically valid `nyxid-chat-*` prefix (`backend/src/services/assistant_service.rs:63-108`), and `get_history`, `delete_conversation`, and `get_state` use that classifier (`backend/src/handlers/assistant.rs:836-843`, `:866-929`). PR 3 says only to adopt identities from `RUN_STARTED` and never change them ([plan](1408-typed-chat-trunk.md:242-249)); it does not require rejecting absent/conflicting outer `actorId`/`turnId` versus `runStarted.threadId`/`runStarted.runId`. The canon distinguishes every operation-key component and says mismatched evidence cannot advance state (canon `:91-97`).

**Why it matters:** A malformed typed resource ID still reaches the upstream canonical resource instead of being rejected locally, and a contradictory start frame can seed the wrong local identity. Both violate the migration's stated fail-closed identity boundary.

**Fix:** Use one strict typed-ID validator wherever a typed id selects an upstream canonical path, while retaining the separate `chatc-*` validator for legacy history. Specify and test the complete `RUN_STARTED` consistency rule before aliasing: required outer identity, matching nested thread/run identities when present, exactly one adoption, and terminal protocol error on mismatch. Add negative tests for GET/DELETE/state and for every conflicting/missing start identity combination.

### 5. PR 0 proposes committing raw production captures with a token-only redaction rule. That is not safe fixture handling.

**Evidence:** PR 0 calls for a real authenticated run and committed SSE and `/state` responses, with only "Redact tokens" as the safeguard ([plan](1408-typed-chat-trunk.md:152-158)). The contract excludes credentials from its intended state projection (canon `docs/canon/nyxid-chat-api.md:654-668`), but it does not make production prompts, assistant text, user identifiers, action resource identities, or a buggy upstream payload safe to commit. NyxID's own wire-log UI explicitly warns that raw captures may contain sensitive payloads verbatim (`frontend/src/components/assistant/assistant-wire-log-panel.tsx:534-540`).

**Why it matters:** This turns a source fixture into an unbounded data-exfiltration path. Redacting bearer-token-looking substrings does not remove private chat content, URLs, IDs, or secrets echoed by a model/tool.

**Fix:** Use a dedicated disposable test account and deterministic non-sensitive prompts only. Commit a minimized structural fixture, not a raw capture: replace all IDs/text/timestamps/URLs with synthetic values, delete unknown fields rather than retaining them, and add a fixture-sanitizer test or review checklist that rejects credentials, user data, and non-approved keys. Keep raw evidence local/ephemeral.

## GAPS

### 6. The `workflow` deletion inventory is materially incomplete outside `aevatar-transport.ts`.

**Evidence:** The plan calls out about 129 references and gives a short PR 3 frontend list ([plan](1408-typed-chat-trunk.md:73), [plan](1408-typed-chat-trunk.md:264-269)). Independent workflow protocol machinery remains in `frontend/src/lib/assistant/wire-replay.ts:35`, `:273-393`, and `:1251-1266`; its tests deliberately replay production workflow creation at `wire-replay.test.ts:437-482`. The mock scenario transport still creates `workflow-pending-*` at `scenario-intercept-transport.ts:35`, `:93-95`, and its test asserts that prefix at `use-assistant.mock-scenarios.test.tsx:129`. Workflow receipt/recovery state is separately persisted in `stores/assistant-receipt-store.ts:1-290`, with schema/tests still naming that prefix. The large transport test suite has workflow-create/recovery expectations through `aevatar-transport.test.ts:6693`, `:8280-:9188`, and `:10690-:10855`; hooks contain workflow alias/reconciliation fixtures at `use-assistant.test.tsx:715-716` and `use-assistant.aevatar.test.tsx:798-899`.

**Why it matters:** Leaving any of these paths unchanged makes the claimed regression "a fresh chat can never select workflow-pending-*" false in mock/scenario runs, or leaves stale workflow behavior in the diagnostic product. Blind deletion instead destroys valuable typed-versus-legacy coverage.

**Fix:** Inventory by behavior, not by a grep count. Convert all draft-capable mocks/tests to `draft-*`; delete the workflow create-receipt/recovery store only after proving no generic delete behavior depends on it; and make an explicit product decision for wire replay: either retain it as a clearly non-conversation historical diagnostic with fixtures, or remove its workflow protocol and update the inspector. Include every affected test file in the PR plan.

### 7. The plan overlooks the reference client's useful state-envelope rules and its own input normalization.

**Evidence:** The reference transport strips console-only `surface` and `attachment`, maps an attachment to typed `inputParts`, and sets `Idempotency-Key` from `clientRequestId` (`StudioAssistant/transport.js:394-423`). It forwards `/state` query parameters only for state resources (`:426-434`). Its state handler treats `not_modified`, `reload_required`, `not_found`, envelope/snapshot-version equality, `scopeId`, actor identity, and safe `progressSequence` as protocol checks (`actor-state.js:116-211`), not merely as card data. NyxID's current typed request DTO rejects unknown fields (`backend/src/services/assistant_service.rs:443-452`).

**Why it matters:** The plan's first-turn body happens to match the canon's text-only example, but it never declares whether attachments/inputParts are unsupported or must be carried. More seriously, it names only actor mismatch and generic monotonic state handling, missing scope/envelope/sequence checks that prevent a misleading reload from being treated as trusted state.

**Fix:** Explicitly choose attachment support: reject it at the NyxID UI boundary with a user-visible limitation, or add an audited typed `inputParts` schema/forwarder and tests. Lift the reference's envelope validation rules into the decoder specification, including `scopeId` and `progressSequence`; do not use an untyped `Record` after Zod decoding.

### 8. The stated test gates do not prove that the workflow path is absent rather than merely unused in one happy-path test.

**Evidence:** PR 3 tests check a fresh-chat body and a no-network-call legacy send ([plan](1408-typed-chat-trunk.md:271-279)), but do not require router-level absence, no fallback after a typed error, or a rejected unknown explicit discriminator. The canon requires unknown explicit types to return 400 and never fall through to Workflow (canon `docs/canon/nyxid-chat-api.md:687-693`). The existing typed facade already reconstructs strict bodies at `backend/src/handlers/assistant.rs:979-1035`; this is testable without a live deployment.

**Why it matters:** A client test can pass while an obsolete route remains mounted and an error-handling fallback quietly reintroduces Workflow Studio. That is the exact regression this issue exists to prevent.

**Fix:** Add router integration tests: `/workflow-chat`, `/workflow-chat/ws`, and facade create-recovery are 404 after PR 3; a typed first/follow-up reaches only upstream `/api/chat` with the discriminated body; an unknown/malformed typed request neither forwards nor reaches workflow; and a typed upstream 4xx/network error cannot retry through workflow. Assert the server debug echo/path and body, not only a mocked frontend URL.

## NITS

### 9. The baseline line-count claim is stale; the reference count is right but the evidence is not.

**Evidence:** The plan says the transport is a 6,117-line file ([plan](1408-typed-chat-trunk.md:73)). At the stated base it is 6,843 lines (`frontend/src/lib/assistant/aevatar-transport.ts`, `wc -l`), while `rg -i workflow` does produce 129 matching lines.

**Why it matters:** This does not change the migration, but a supposedly verified baseline should not contain a plainly false measurement.

**Fix:** Correct it to 6,843 or drop the line count entirely; keep the reproducible grep command in the implementation notes.

The plan is correct on one important legacy point: deleting `get_create_recovery` does **not** break existing `chatc-*` list/read/delete. `list_conversations` drains the shared history index (`backend/src/handlers/assistant.rs:715-817`), while `get_history` and `delete_conversation` independently dispatch `chatc-*` to the scoped history resource (`:819-897`); recovery is a create-only endpoint (`:931-954`). The safe-integer guard is also required by the canon (`docs/canon/nyxid-chat-api.md:219-227`), so placing it in the initial browser decoder is sound.

VERDICT: REJECT

The target architecture and the legacy read-only decision are defensible, but this is not an implementation-ready plan. Its PR sequencing makes a confirmed plan gate unusable, PR 2 omits substantial canonical state, and PR 3's deletion inventory cannot pass the claimed independent test gate. It also treats raw production data as commit-ready fixtures and leaves fail-closed identity work incomplete. Repair the boundaries and turn the omitted coupling into explicit, testable work before implementation begins.

## Revision 2 verification

Reviewed Revision 2 at `c24af2d5` against the same NyxID base and Aevatar revision. The revision is materially better; the original review was not merely acknowledged.

### Resolved: the PR 3 handler-deletion/test-compilation blocker is now planned correctly.

**Evidence:** The plan now names the eight `backend/src/handlers/proxy.rs` call sites, `mw/auth.rs:1485`, and explicitly preserves/narrows the legacy history/delete proof ([plan](1408-typed-chat-trunk.md:288-297), [plan](1408-typed-chat-trunk.md:382)). Those references match the baseline direct calls, including `workflow_chat` at `backend/src/handlers/proxy.rs:8985`, `get_create_recovery` at `:9144`, and `workflow_chat_ws` at `:9582`.

**Result:** Resolved. The router-absence tests at [plan](1408-typed-chat-trunk.md:312-320) also correct the earlier non-use-only test gap.

### Partially resolved: the plan-gate ordering is now sound in concept, but its backend dependency is unstated.

**Evidence:** Revision 2 correctly moves the full decode/render/submit/reload gate slice to PR 2 when Q1 is `confirm` ([plan](1408-typed-chat-trunk.md:177-181), [plan](1408-typed-chat-trunk.md:260-266)); this fixes the former builder-without-facts failure. However, `plan.resolve` does not exist until PR 1 ([plan](1408-typed-chat-trunk.md:195-207), baseline `backend/src/services/assistant_service.rs:257-266`). PR 1 is described as parallel with PR 0 ([plan](1408-typed-chat-trunk.md:191-193), but the plan never says that a Q1=`confirm` PR 2 must merge after PR 1. PR 2 could therefore present and submit a gate against a backend that returns `400 Unsupported assistant chat command`.

**Why it matters:** The new ordering rule prevents PR 3 from flipping too early, but does not establish an independently working PR 2 in the confirmed-gate case.

**Required change 1:** Add an explicit merge matrix: with Q1=`confirm`, PR 0 answers Q1, then PR 1 merges before PR 2's gate slice, then PR 2's gate E2E test passes before PR 3 may merge. With Q1=`auto`, PR 1 and PR 2 may merge independently after PR 0.

### Partially resolved: PR 2 is no longer TaskPlan-only, but its claimed full `/state` inventory still drops actor-owned fields.

**Evidence:** The new scope correctly covers input, approval, actions, controls, continuation, envelope checks, and both adapters ([plan](1408-typed-chat-trunk.md:223-266)); it is a real response to the prior finding. But its full-snapshot inventory omits `taskStatus` and `latestControlResult` ([plan](1408-typed-chat-trunk.md:131-140). `taskStatus` is explicitly present in the canonical current-state example (canon `docs/canon/nyxid-chat-api.md:646`) and the working reference preserves it as `snapshot.taskStatus || snapshot.activeTask?.status` (`StudioAssistant/actor-state.js:201`). The reference also preserves `latestControlResult` (`actor-state.js:45-48`, `:205-207`); it is distinct from `controlFence` and from `latestStepControlResult`.

**Why it matters:** Dropping `taskStatus` invites the browser to infer terminal task state from another field, contrary to the actor-owned-state rule. Dropping `latestControlResult` loses the committed outcome required to reconcile stop/steer control receipts after reload.

**Required change 2:** Add `taskStatus` and `latestControlResult` to §3's snapshot list and PR 2's projection schema/reducer. Add state-reload fixtures where `activeTask` is absent or differs from the actor-authored `taskStatus`, and where a control receipt is reconstructed from `latestControlResult`.

### Partially resolved: the plan identifies the old/new projection collision but still leaves the integration design to the implementer.

**Evidence:** The plan says to "state the retirement explicitly" ([plan](1408-typed-chat-trunk.md:257-258); that is an instruction to finish the plan, not the missing decision itself. The current typed path already reduces a separate `TurnEvent`/card model in `AevatarAssistantTransport.emit()` (`frontend/src/lib/assistant/aevatar-transport.ts:3744-3810`), translates input/approval custom frames through `applyInputChanged`/`applyApprovalChanged` (`:5137-5150`, `:5880-5940`), and reload reconciliation is history-oriented (`reconcileProjection`, `:2181-2254`). Those mechanisms overlap with the proposed actor projection and `preserveLocalStructuredMessages` (`:575`, `:3538-3610`).

**Why it matters:** Without an exact ownership map, PR 2 can silently run two reducers, let history overwrite actor state, or make a Q1=`confirm` gate visible in one projection but submit from stale card data. "Port from actor-state.js" is correct, but it does not identify where the port becomes authoritative in NyxID.

**Required change 3:** Before coding, name the exact destination and retirement boundary: which field on the transport/conversation owns `ActorProjection`; which SSE dispatcher sends each custom frame to it; which mounted-conversation/reload hook issues the conditional `/state` read; and which existing `TurnEvent` card reducers, history reconciliation writes, and `preserveLocalStructuredMessages` branches are bypassed for typed conversations. Add a test that reloads a typed pending input/approval/action while the legacy transcript is also present and proves history cannot overwrite the actor projection.

### Partially resolved: attachment and workflow-replay choices are surfaced but not decided.

**Evidence:** Revision 2 correctly discovers the reference transport's attachment normalization and requires an attachment choice ([plan](1408-typed-chat-trunk.md:278-280). It likewise requires a wire-replay decision ([plan](1408-typed-chat-trunk.md:306-308). Neither selects an outcome or names the decision owner. The current reference deliberately maps attachment to `inputParts` while stripping console-only fields (`StudioAssistant/transport.js:394-423`), and NyxID's typed DTO currently rejects unknown input fields (`backend/src/services/assistant_service.rs:443-452`).

**Why it matters:** These are functional and user-visible choices, not cleanup details. An implementing agent cannot decide whether to remove attachment capability or expand the backend contract, nor whether an existing diagnostic remains supported, without changing product scope.

**Required change 4:** Record one owner-approved choice before PR 3 begins: either (a) reject attachments at the Assistant UI boundary and remove/disable any related affordance, or (b) add `inputParts` end-to-end with its schema, secret review, and body-shape tests. Separately choose whether wire replay retains workflow captures as an explicitly historical diagnostic or removes its workflow parser/fixtures/UI copy, then list the exact files/tests for that choice.

### Resolved: fixture safety, resource-ID fail-closed work, and the deletion blast radius are now adequate.

**Evidence:** PR 0 now requires a disposable account, synthetic structural fixtures, removal of unknown fields, and a sanitizer test ([plan](1408-typed-chat-trunk.md:160-187). PR 1 applies validation to the typed resource selectors as well as command bodies and adds no-upstream-call tests ([plan](1408-typed-chat-trunk.md:199-207). PR 2 gives an explicit `RUN_STARTED` identity consistency matrix ([plan](1408-typed-chat-trunk.md:253-255), and PR 3 accounts for receipt-store, scenario, and replay coupling ([plan](1408-typed-chat-trunk.md:298-310). These address findings 4-9 substantively, subject only to Required change 4's two pending product choices.

VERDICT: APPROVE WITH CHANGES

Revision 2 has repaired the original structural failures: it no longer flips before a projection exists, it preserves the legacy read/delete proof while deleting sends, and it turns fixture capture into a controlled artifact. The four remaining changes above are narrow, unambiguous additions to make before implementation. Without them, a confirmed plan gate has an unspecified backend ordering, the projection is not actually complete, and two product-visible behavior choices are still delegated to the implementer.
