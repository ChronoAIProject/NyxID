# CMA Worker v1 NyxID Design

## Goal and scope

Implement one personal-owner Sandbox slot for resources created through this NyxID instance, durable creation and Agent references, model-facing CMA operations, first-party Sandbox management, and a cloud assistant Conversation/Desktop view. The existing `cma_operation` and assistant watch path remains responsible for returning task results to the originating chat.

Updated 2026-11-06 to include the approved creation-source restriction and conversation-view changes. This document describes the implementation contract; it does not itself establish that every target-deployment scenario has passed.

The work includes backend persistence/admission, the catalog gateway and OpenAPI overlay, native attachment and read-only guidance bridges, feature-gated frontend routes, history/events, and an RFB Desktop viewer. It does not change CMA's production quota or callback producer, remove Remote Machine, introduce Agent pooling, import externally created resources, add a Sandbox-level Desktop API, or introduce separate compute billing. CMA's own account limits remain independent; origin-aware upstream quota is not required.

## Durable records and creation identity

`nyxbot_cma_sandboxes` stores an owner/service-bound local reference, optional remote Sandbox ID, local label, admission and observed states, occupied-slot flag, generation, capability/capacity facts, and retirement timestamps. A partial unique index on `user_id` where `occupies_account_slot=true` protects the local account slot across chats, roles, keys and connection aliases. Only server-written creation provenance (`source=nyxbot`) establishes a NyxID-created resource. Legacy discovered/claimed/conflict references are excluded from management and parent resolution; stale external occupancy is released only when it has no creation/recovery identity. Direct CMA resources and another NyxID deployment do not consume this allocation. Raw catalog proxy reads retain CMA's response and may list external resources.

`nyxbot_cma_agents` stores the exact owner/service/remote-Agent identity, immutable parent Sandbox/Workspace/Worktree IDs, the parent generation, safe profile/status observations, and bounded chat/turn/watch metadata. Its partial unique remote-identity index prevents duplicate references. Source chat metadata is attribution, not an exclusive ownership binding. The Agent execution DTO does not expose that metadata or encrypted creation material.

`nyxbot_cma_creations` stores each allocation intent separately from chat and watch retention, including Agents created inside an existing parent. It binds the personal owner, catalog service and execution fingerprint, original HTTP method/path, stable idempotency key, exact request hash, local Sandbox/generation, optional selected Workspace, submission state, and any observed lifecycle/Agent/Sandbox IDs. It also stores a revision, submission generation, first-dispatch time, and recovery-check time. Required and optional dates use the repository's BSON datetime helpers; encrypted bytes use the BSON byte helper.

Creation hashing and envelope encryption preserve the **exact original outbound request bytes**, including whitespace, object order, and JSON escapes. JSON parsing validates structure, size and prohibited transport credential fields without rewriting those bytes. Recovery never normalizes a body or silently substitutes a new idempotency key. Each encrypted record uses its own `RecordContext` and does not appear in API DTOs, logs, audit metadata, or `Debug` output.

Creation writes commit with the persistent account-erasure fence and a write to the exact parent generation. `agent_reference_version` serializes parent/reference changes with receipt association. Creation-local `submission_generation` advances only when an explicit no-resource result allows the same intent to be reacquired; it rejects late replies even when an existing parent Sandbox keeps its generation. All three collections participate in personal erasure and startup index creation.

An exact creation receipt is associated with its original local reservation transactionally. Compatibility handling can fold a legacy metadata-only duplicate into that reservation while preserving its permanent key, label/retirement intent and Agent generation bindings; discovery never manufactures new creation provenance. Timeout, 404, missing inventory entries, failed provisioning, paused state, and permission loss do not prove that a resource was absent or retired. Only an authoritative no-resource refusal or an exact `retired` read can release the appropriate slot. Separate creation ciphertext is cleared after confirmed resource completion or confirmed no-resource; Sandbox recovery material is cleared on confirmed no-resource or retirement.

## Retry and observation rules

CMA has two different idempotency contracts:

| Creation endpoint | Remote identity | NyxID retry rule |
| --- | --- | --- |
| `POST /api/v1/agents` | Permanent lifecycle operation ledger | An explicit replay retains the original key/body and creation identity, subject to live owner, deployment, generation, and retirement checks. |
| `POST /api/v1/sandboxes` | Generic command cache with 24-hour retention | Uncertain replay is allowed only within a conservative 23-hour window from the persisted first dispatch. |
| `POST /api/v1/workspaces/{id}/agents` | Generic command cache with 24-hour retention | The same 23-hour fence applies; creating another Agent must not be confused with recovering an old request. |

A saved direct Sandbox/Agent receipt switches recovery to observation instead of another POST. Unknown legacy generic records without a dispatch timestamp fail closed. A clock rollback cannot extend the retry window. Expiry retains the slot and original recovery material; it does not authorize a new key or allocation. Proven no-resource reacquisition resets the dispatch timestamp while advancing the submission generation.

Sandbox list/detail refresh calls bounded, read-only recovery over saved lifecycle/resource IDs: at most eight records, a three-second observation budget, and a short persisted polling cooldown. This recovery survives chat deletion and watch expiration. It never resubmits a creation request. A lifecycle response must match the saved operation ID; its resulting Agent is read live and checked against the original parent and any explicitly selected Workspace.

The Agent lifecycle and older generic Sandbox command use different idempotency contracts. A generic-command refusal suggesting a new key does not prove a lifecycle same-key incompatibility. NyxID never silently rotates the approved key/body. First credential setup followed by exact request recovery remains deferred and lacks a never-delegated test account; successful lost-receipt recovery does not cover that prerequisite.

## Services and final dispatch

- `nyxbot_cma_gateway` defines the typed `CmaGateway` boundary and production `CatalogCmaGateway`. The production adapter resolves the canonical active `cma` catalog entry, validates the personal owner and live catalog access, requires its supported signed-identity/HTTP configuration, pins service identity and execution fingerprint, and sends bounded requests without redirects. Upstream authorization loss becomes an unavailable observation; an explicit local owner mismatch remains forbidden.
- `nyxbot_cma_sandbox_service` owns slot reservation, exact-byte Sandbox recovery storage, created-resource filtering and refresh, safe DTOs, local labels, lifecycle commands and confirmed retirement. It does not import or claim external resources.
- `service_dispatch_registry` selects protection once for REST/MCP from the strictly resolved `ProxyTarget.catalog_slug`. Only after selecting protection does it canonicalize the admission path and prepare its effective headers. Ordinary services retain their existing global path-safety checks and forwarding semantics, including trailing slashes and legal percent encoding. Catalog connections carry the authoritative catalog ID and slug; missing or unreadable catalog records fail resolution. A private neutral admission binds the policy, authenticated actor, resource owner, UserService, credential connection, destination and parsed base URL. Every common HTTP wrapper verifies the admission against its current trusted caller and target, including calls without admission. Protected targets refuse missing/mismatched admission, missing callers and streamed request bodies; ordinary targets accept no admission or explicit `Unprotected` without a guard.
- `nyxbot_cma_admission` implements `ServiceDispatchGuard` directly. Its `before_send` checks the final built request and persists dispatch state; `observe_response` records the receipt before later observation completes resource association. The registry owns policy selection and credential construction; the generic proxy knows neither CMA paths nor its permit type. Existing HTTP egress, billing, resource-proof and execution-permission fences remain independent. Local rejections, transport errors and failures after a received response retain their existing dispatch-stage classifications. Lifecycle creation, direct Sandbox creation and workspace Agent creation preserve their existing quota, same-key/body binding and recovery rules. Reads do not reserve capacity; writes require the supported personal context and NyxID-created parent when applicable.

Concrete and universal MCP calls retain the typed provider outcome through async submission tracking. `ProviderOutcomeUnknown` produces the existing `submission_uncertain` reason; a proven unreachable provider or pre-dispatch refusal retains `submission_failed`. The existing card renders uncertain submission as unconfirmed. This classification does not release a creation's slot, change its original key/body or authorize a new attempt.
- `nyxbot_cma_agent_service` registers owner-authenticated observations and resolves exact local Agent references. It validates immutable parents/generations and exposes safe detail and Desktop capability projections. It does not choose, create, or delete an Agent itself.
- `nyxbot_cma_tools` provides the native bridges. Its live grant/key/access-mode/input checks run at the actual gateway request boundary, including real paginated queries and attachment JSON.

Creating an Agent with an existing Sandbox or Workspace verifies a NyxID-created parent for the same owner. Existing Agents do not prevent additional Agents within that environment. Configured CPU/memory is read from the exact pinned profile or verified owner runtime allocation. The first complete positive pair and its source are persisted for that creation identity; later lifecycle reads skip capacity enrichment and preserve the saved values through pause, revive and unavailable observations. Initial enrichment has at most eight reads and a two-second budget per call, not an overall management-request deadline. Missing or unverifiable capacity remains unknown without making resource management unavailable; values are resource specifications, not utilization or charges.

A retirement intent closes new Agent/task admission. Known cleanup operations remain addressable, and CMA's state machine resolves accepted remote races.

## First-party HTTP surface

All paths below use the prefix `/api/v1/assistant/nyxagent`. Resource IDs in these management routes are NyxID's local reference IDs.

| Method | Path | Behavior |
| --- | --- | --- |
| GET | `/sandboxes` | NyxID-created resources and account allocation status |
| GET | `/sandboxes/{id}` | Live detail with durable unknown/pending facts |
| PATCH | `/sandboxes/{id}` | Local label change bound to generation |
| POST | `/sandboxes/{id}/commands` | `pause` or `revive`, bound to generation and one stable `Idempotency-Key` |
| DELETE | `/sandboxes/{id}` | Retirement intent; release only after exact retirement observation |
| GET | `/sandboxes/{id}/events` | Bounded Sandbox lifecycle events |
| GET | `/cma-agents/{id}` | Exact Agent detail and safe profile |
| GET | `/cma-agents/{id}/transcript` | Authoritative conversation turns and compatible flat messages |
| GET | `/cma-agents/{id}/events/snapshot` | Agent snapshot using CMA's `page_cursor` |
| GET | `/cma-agents/{id}/events` | Agent SSE using `after_cursor` |
| GET | `/cma-agents/{id}/responses/{response_id}` | Exact response bound to that Agent |
| GET | `/cma-agents/{id}/responses/{response_id}/events` | Response SSE using numeric `starting_after` |
| GET | `/cma-agents/{id}/desktop` | Availability/descriptor for the exact Agent |
| GET | `/cma-agents/{id}/desktop/ws` | Authorized RFB WebSocket relay |

Handlers require a first-party human, active personal owner, and `assistant:cma-sandboxes`; they return dedicated DTOs. The existing delegated-read denial for the assistant route family covers every new read/stream/upgrade path, with explicit regression coverage. Accepted lifecycle commands return `202` while pending; acknowledgement alone is not completion.

## CMA discovery, native tools, and assistant guidance

CMA no longer uses a NyxID-hosted OpenAPI overlay. Its operations come from the
current OpenAPI document discovered from the configured CMA connection. Operation
names may follow CMA's upstream names and can change between document revisions;
the CMA adapter therefore selects operations by HTTP method and path. In
particular, work uses the discovered
`POST /api/v2/agents/{agent_id}/responses` operation with `stream=true`, response and
controls reads use their discovered GET paths, and task controls use the
discovered `POST /api/v2/agents/{agent_id}/responses/{response_id}/controls`
operation. Agent and Sandbox deletion remains an explicit DELETE operation. The
local CMA risk markers are gone; existing generic HTTP method classification now
provides the default effects. The existing CMA adapter still binds the exact
Agent, response, turn sequence, revision, and approval identity where applicable.
The producer-owned execution boundary validates the full tagged control union.
The controls snapshot does not invent pending approval IDs; approval identities
come from bound Agent events.

The three native tools are `nyxid__list_cma_resources`, `nyxid__cma_save_attachment`, and `nyxid__cma_guide`. Listing returns only created resources, preserves pagination and fresh creation-key behavior, and does not reserve/select resources. Attachment copying reads an original file from the current conversation, applies action policy and verifies the selected Agent. Its path is relative to CMA's `/home/` API root; it returns path, absolute_path, size and SHA256 without raw bytes. An identical existing file is idempotent; different content is not overwritten. These bridges exclude guests, shared/collaboration and organization contexts and require the live personal feature/grant authority.

`nyxid__cma_guide` returns versioned local Markdown from `docs/assistant/CMA_WORKER_GUIDE.md` after the existing live-authority and eligibility checks. Its native discovery description names cloud environments, cloud assistants and shared desktops, including creating an assistant without running a task. It requires no Ornn installation/binding and performs no provider requests, resource queries/allocation or confirmation-card creation. Only CMA native tool descriptions point to the guide; general catalog operation descriptions do not name assistant-only tools. Detailed operation rules have one source in the guide. Models read instructions and current resources on demand; the generic turn runtime performs no CMA guidance or snapshot injection, and eligibility changes do not enter the stable instruction fingerprint.

Observed external callbacks retain their existing `cma_operation` validation.

## Frontend and Desktop transport

The frontend routes are `/assistant/sandboxes`, `/assistant/sandboxes/$sandboxId`, and `/assistant/cma-agents/$agentId`. Agent links may include an exact `response` and `tab=desktop`. The page presents the assistant's conversation; a supplied response is a precise navigation target, not a reason to substitute the latest task. Sandboxes appear in the feature-gated sidebar/search alongside the existing Machines navigation.

Sandbox list/detail cover created-resource states, allocation status, nullable capacity/activity, local rename, stable-key lifecycle commands and retirement confirmation. External linking/claim UI and routes are removed. The removed billing disclaimer and fee cards are not shown. Sandbox pages contain no Agent management or Desktop controls and do not allocate resources on open.

The Agent page has one Conversation view beside Desktop. The additive transcript `turns` preserve sequence, start time, per-turn status and messages; the compatible flat list still deduplicates identical messages. Reused input remains present in each authoritative turn, while conflicting identities and duplicate/non-increasing turn sequences are rejected. An exact response link must match the Agent and a consistent V2 turn binding; missing/conflicting bindings stay unresolved. Old links expand the matching request, View latest removes the response selection, and new activity does not move the reader. Execution details respect authoritative turn boundaries; terminal outcomes remain terminal. Agent snapshot pagination, Agent stream cursors and numeric Response cursors remain distinct. Closing the page does not cancel work or change original result delivery.

CMA Desktop uses **RFB binary WebSocket traffic rendered by a pinned `@novnc/novnc` client** in `cma-desktop-panel.tsx`. It does not use NyxID Machine Desktop's JSON protocol. The backend fixes the upstream Agent path, signs the owner's identity, forwards bounded binary/ping/pong traffic, and persists no frames. Cookie sessions require the exact frontend Origin; unrestricted first-party bearer sessions follow the explicit bearer Origin policy. Live session, feature, service, parent/generation and capability checks recur during relay. CMA remains authoritative for shared-Sandbox viewer contention. Stopped/deleted/failed/paused, no-desktop, busy and unavailable-auth states expose a reason without switching resources.

## Verification status

Validation evidence must distinguish:

- Rust compilation and pure model/parser/handler/state-machine tests;
- actual replica-set MongoDB execution of slot, receipt association, erasure and recovery regressions, with explicit reporting of any skips;
- frontend type/build/lint and focused Vitest results;
- Playwright transport-fixture browser checks, including desktop/mobile layouts and unavailable Desktop states;
- live CMA acceptance for login/key recovery, real SSE and real RFB, which fixture tests do not establish.
