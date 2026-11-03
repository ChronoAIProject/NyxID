# CMA Worker v1 NyxID Design

## Goal and scope

Implement the NyxID portion of the authoritative [CMA Worker v1 source plan](../../../2026-11-03-cma-worker-v1-implementation.md): one personal-owner Sandbox slot, durable creation and Agent references, model-facing CMA operations, first-party Sandbox management, and an Agent execution view. The existing `cma_operation` and assistant watch path remains responsible for returning task results to the originating chat.

This document describes the implementation in this checkout. It does not replace the source plan or claim that outstanding validation has passed. The [implementation checklist](../plans/2026-11-04-cma-worker-v1-implementation.md) separates completed implementation from final verification.

The work includes backend persistence/admission, the catalog gateway and OpenAPI overlay, native attachment and owner-action bridges, feature-gated frontend routes, history/events, and an RFB Desktop viewer. It does not change CMA's production quota or callback producer, remove Remote Machine, introduce Agent pooling, add a Sandbox-level Desktop API, or introduce separate compute billing. The [external-dependency record](../../2026-11-04-cma-worker-v1-out-of-scope.md) tracks those boundaries and the login/idempotency contract mismatch.

## Durable records and creation identity

`nyxbot_cma_sandboxes` stores an owner/service-bound local reference, optional remote Sandbox ID, local label, admission and observed states, occupied-slot flag, generation, capability/capacity facts, and retirement timestamps. A partial unique index on `user_id` where `occupies_account_slot=true` protects the local account slot. Verified excess remote resources remain visible as conflict references and can be retired; a local slot alone cannot enforce CMA's quota for requests sent directly to CMA.

`nyxbot_cma_agents` stores the exact owner/service/remote-Agent identity, immutable parent Sandbox/Workspace/Worktree IDs, the parent generation, safe profile/status observations, and bounded chat/turn/watch metadata. Its partial unique remote-identity index prevents duplicate references. Source chat metadata is attribution, not an exclusive ownership binding. The Agent execution DTO does not expose that metadata or encrypted creation material.

`nyxbot_cma_creations` stores each allocation intent separately from chat and watch retention, including Agents created inside an existing parent. It binds the personal owner, catalog service and execution fingerprint, original HTTP method/path, stable idempotency key, exact request hash, local Sandbox/generation, optional selected Workspace, submission state, and any observed lifecycle/Agent/Sandbox IDs. It also stores a revision, submission generation, first-dispatch time, and recovery-check time. Required and optional dates use the repository's BSON datetime helpers; encrypted bytes use the BSON byte helper.

Creation hashing and envelope encryption preserve the **exact original outbound request bytes**, including whitespace, object order, and JSON escapes. JSON parsing validates structure, size and prohibited transport credential fields without rewriting those bytes. Recovery never normalizes a body or silently substitutes a new idempotency key. Each encrypted record uses its own `RecordContext` and does not appear in API DTOs, logs, audit metadata, or `Debug` output.

Creation writes commit with the persistent account-erasure fence and a write to the exact parent generation. `agent_reference_version` serializes parent/reference changes with receipt association. Creation-local `submission_generation` advances only when an explicit no-resource result allows the same intent to be reacquired; it rejects late replies even when an existing parent Sandbox keeps its generation. All three collections participate in personal erasure and startup index creation.

A receipt discovered after inventory has already registered the resource is folded into the original creation reference transactionally, preserving its permanent key, label/retirement intent, and Agent generation bindings. Timeout, 404, missing inventory entries, failed provisioning, paused state, and permission loss do not prove that a resource was absent or retired. Only an authoritative no-resource refusal or an exact `retired` read can release the appropriate slot. Separate creation ciphertext is cleared after confirmed resource completion or confirmed no-resource; Sandbox recovery material is cleared on confirmed no-resource or retirement.

## Retry and observation rules

CMA has two different idempotency contracts:

| Creation endpoint | Remote identity | NyxID retry rule |
| --- | --- | --- |
| `POST /api/v1/agents` | Permanent lifecycle operation ledger | An explicit replay retains the original key/body and creation identity, subject to live owner, deployment, generation, and retirement checks. |
| `POST /api/v1/sandboxes` | Generic command cache with 24-hour retention | Uncertain replay is allowed only within a conservative 23-hour window from the persisted first dispatch. |
| `POST /api/v1/workspaces/{id}/agents` | Generic command cache with 24-hour retention | The same 23-hour fence applies; creating another Agent must not be confused with recovering an old request. |

A saved direct Sandbox/Agent receipt switches recovery to observation instead of another POST. Unknown legacy generic records without a dispatch timestamp fail closed. A clock rollback cannot extend the retry window. Expiry retains the slot and original recovery material; it does not authorize a new key or allocation. Proven no-resource reacquisition resets the dispatch timestamp while advancing the submission generation.

Sandbox list/detail refresh calls bounded, read-only recovery over saved lifecycle/resource IDs: at most eight records, a three-second observation budget, and a short persisted polling cooldown. This recovery survives chat deletion and watch expiration. It never resubmits a creation request. A lifecycle response must match the saved operation ID; its resulting Agent is read live and checked against the original parent and any explicitly selected Workspace.

The source plan requests same-key retry after initial CMA login. The inspected CMA generic-command refusal instead says to use a new key after login. NyxID does not silently rotate the key/body, and this end-to-end scenario remains an external contract-alignment requirement.

## Services and final dispatch

- `nyxbot_cma_gateway` defines the typed `CmaGateway` boundary and production `CatalogCmaGateway`. The production adapter resolves the canonical active `cma` catalog entry, validates the personal owner and live catalog access, requires its supported signed-identity/HTTP configuration, pins service identity and execution fingerprint, and sends bounded requests without redirects. Upstream authorization loss becomes an unavailable observation; an explicit local owner mismatch remains forbidden.
- `nyxbot_cma_sandbox_service` owns slot reservation, exact-byte Sandbox recovery storage, complete-inventory reconciliation, claim, safe DTOs, local labels, lifecycle commands and confirmed retirement.
- `nyxbot_cma_admission` binds the fully resolved request immediately before generic proxy/MCP dispatch. Connection aliases do not select a different contract or bypass the account slot. It covers lifecycle creation, direct Sandbox creation, and workspace Agent creation; unsupported allocation surfaces fail closed. It records the correct kind of receipt and owns durable creation recovery.
- `nyxbot_cma_agent_service` registers owner-authenticated observations and resolves exact local Agent references. It validates immutable parents/generations and exposes safe detail and Desktop capability projections. It does not choose, create, or delete an Agent itself.
- `nyxbot_cma_tools` provides the native bridges. Its live grant/key/access-mode/input checks run at the actual gateway request boundary, including real paginated queries and attachment JSON.

Creating an Agent with an existing Sandbox or Workspace verifies that it belongs to the owner's sole live environment. Existing Agents do not prevent additional Agents within that environment. Configured CPU/memory is hydrated from `GET /api/v1/sandbox-profiles/{id}/revisions/{revision}` using the Sandbox's exact pinned profile. Each page reuses identical pins and has an eight-read/two-second total budget. Only numeric capacity is projected; denied, unavailable or CMA-internal profiles remain unknown, and no latest-profile substitution or scope expansion occurs. This optional display read never fails safety inventory.

A retirement intent closes new Agent/task admission. Known cleanup operations remain addressable, and CMA's state machine resolves accepted remote races.

## First-party HTTP surface

All paths below use the prefix `/api/v1/assistant/nyxagent`. Resource IDs in these management routes are NyxID's local reference IDs.

| Method | Path | Behavior |
| --- | --- | --- |
| GET | `/sandboxes` | Reconciled owner inventory and allocation status |
| GET | `/sandboxes/unregistered` | Verified remote Sandboxes available to claim |
| POST | `/sandboxes/claim` | Claim the specified remote Sandbox after owner verification |
| GET | `/sandboxes/{id}` | Live detail with durable unknown/pending facts |
| PATCH | `/sandboxes/{id}` | Local label change bound to generation |
| POST | `/sandboxes/{id}/commands` | `pause` or `revive`, bound to generation and one stable `Idempotency-Key` |
| DELETE | `/sandboxes/{id}` | Retirement intent; release only after exact retirement observation |
| GET | `/sandboxes/{id}/events` | Bounded Sandbox lifecycle events |
| GET | `/cma-agents/{id}` | Exact Agent detail and safe profile |
| GET | `/cma-agents/{id}/transcript` | Durable shared Agent history |
| GET | `/cma-agents/{id}/events/snapshot` | Agent snapshot using CMA's `page_cursor` |
| GET | `/cma-agents/{id}/events` | Agent SSE using `after_cursor` |
| GET | `/cma-agents/{id}/responses/{response_id}` | Exact response bound to that Agent |
| GET | `/cma-agents/{id}/responses/{response_id}/events` | Response SSE using numeric `starting_after` |
| GET | `/cma-agents/{id}/desktop` | Availability/descriptor for the exact Agent |
| GET | `/cma-agents/{id}/desktop/ws` | Authorized RFB WebSocket relay |

Handlers require a first-party human, active personal owner, and `assistant:cma-sandboxes`; they return dedicated DTOs. The existing delegated-read denial for the assistant route family covers every new read/stream/upgrade path, with explicit regression coverage. Accepted lifecycle commands return `202` while pending; acknowledgement alone is not completion.

## Overlay, native tools, and assistant guidance

The embedded `backend/specs/catalog/cma.openapi.json` is registered for the canonical `cma` service. Its current operation IDs are:

- Agent discovery/lifecycle: `list_agents`, `create_agent`, `get_agent`, `get_provisioning`, `stop_agent`, `delete_agent`.
- Sandbox management: `list_sandboxes`, `get_sandbox`, `pause_sandbox`, `revive_sandbox`, `retire_sandbox`, `list_sandbox_events`.
- Work and history: `run_task`, `get_task`, `list_messages`, `get_transcript`, `get_task_controls`, `control_task`, `interrupt_task`.
- Files and observations: `list_files`, `read_file`, `download_file`, `write_file`, `get_resources`, `get_usage`.

The overlay supplies `x-aevatar-tool` action classes. `cma__run_task` targets `POST /api/v2/agents/{agent_id}/responses` with `stream=true`. Control calls bind the exact Agent, response, turn sequence, revision, and approval identity where applicable. The hosted schema subset cannot express the full tagged control union, so the producer-owned execution boundary validates it. The controls snapshot does not invent pending approval IDs; approval identities come from bound Agent events.

The three native tools are `nyxid__list_cma_resources`, `nyxid__cma_save_attachment`, and `nyxid__cma_request_owner`. Listing does not reserve/select resources. Attachment copying reads an original file from the current conversation, applies action policy, verifies the selected Agent, and returns path/size/SHA256 without raw bytes. Owner requests open the exact internal Agent; the owner decision resumes through the existing acknowledgement/event path. These bridges exclude guests, shared/collaboration and organization contexts and require the live personal feature/grant authority.

Stable CMA guidance is added through the existing `Prepared::with_guidance` path. A current snapshot of at most eight Agents is supplied beside dynamic memory notes and stays outside the instruction fingerprint. It contains bounded resource facts, not transcript text, credentials or another chat's task body. Observed external callbacks retain the existing validation in `cma_operation`; internal execution links do not depend on a callback URL.

## Frontend and Desktop transport

The frontend routes are `/assistant/sandboxes`, `/assistant/sandboxes/$sandboxId`, and `/assistant/cma-agents/$agentId`. Agent links may include an exact `response` and `tab=desktop`; absence of a response never selects the latest task. Sandboxes appear in the feature-gated sidebar/search alongside the existing Machines navigation.

Sandbox list/detail cover empty, pending/unknown, active, paused, retiring/retired and excess-inventory conflict states, nullable CPU/memory/activity, local rename, stable-key lifecycle commands, retirement confirmation and claim. Billing copy preserves unknown cost rather than inventing usage charges. Sandbox pages contain no Agent management or Desktop controls and do not allocate resources on open.

The Agent page separates the selected task from explicitly shared history. Transcript projection preserves durable message order and deduplicates identical admitted messages repeated across failed/retried turns, rejecting conflicting reuse of one identity. Agent snapshot pagination, Agent stream cursors and numeric Response cursors remain distinct. Repeated event identities do not duplicate UI output. Closing the page does not cancel work or change the original watch/result delivery.

CMA Desktop uses **RFB binary WebSocket traffic rendered by a pinned `@novnc/novnc` client** in `cma-desktop-panel.tsx`. It does not use NyxID Machine Desktop's JSON protocol. The backend fixes the upstream Agent path, signs the owner's identity, forwards bounded binary/ping/pong traffic, and persists no frames. Cookie sessions require the exact frontend Origin; unrestricted first-party bearer sessions follow the explicit bearer Origin policy. Live session, feature, service, parent/generation and capability checks recur during relay. CMA remains authoritative for shared-Sandbox viewer contention. Stopped/deleted/failed/paused, no-desktop, busy and unavailable-auth states expose a reason without switching resources.

## Verification status

Implementation and test-source presence are tracked in the generated plan. Final command results are owned by the coordinating agent and are not inferred from this document. The required evidence distinguishes:

- Rust compilation and pure model/parser/handler/state-machine tests;
- actual replica-set MongoDB execution of slot, receipt association, erasure and recovery regressions, with explicit reporting of any skips;
- frontend type/build/lint and focused Vitest results;
- Playwright transport-fixture browser checks, including desktop/mobile layouts and unavailable Desktop states;
- live CMA acceptance for login/key recovery, real SSE and real RFB, which fixture tests do not establish.
