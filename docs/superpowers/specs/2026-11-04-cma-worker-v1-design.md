# CMA Worker v1 NyxID Design

## Goal

Add the NyxID side of the CMA Worker v1 contract: one owner-scoped Sandbox allocation, durable CMA Agent references, model-facing CMA operations, first-party Sandbox management, and an Agent execution view that follows the existing CMA operation and desktop contracts.

The implementation follows the supplied `2026-11-03-cma-worker-v1-implementation.md` and the local CMA preview. It keeps the existing chat/watch delivery path as the source of task results and adds a read-oriented execution surface around it.

## Scope

This change covers the NyxID repository's M1 and M2 work:

- persistent Sandbox and Agent reference records with owner and service binding;
- an atomic personal-account Sandbox slot and recovery-safe creation identity;
- owner-scoped Sandbox management REST and Agent execution-view REST;
- CMA service discovery through the existing catalog and identity propagation path;
- the CMA OpenAPI overlay, NyxID-native resource/attachment/owner-request tools, and CMA guidance injection;
- feature-flagged frontend routes, query schemas/hooks, Sandbox management UI, task-card deep links, Agent history/event UI, and Desktop capability/error states;
- focused unit, handler, frontend component, and route tests.

The CMA service's own owner quota configuration, its production `callback_url` producer, removal of Remote Machine, and a new Sandbox-level Desktop API are recorded as out of scope. NyxID will surface unknown or unavailable downstream facts and will not infer billing completion or create a replacement resource.

## Architecture

### Durable records

`nyxbot_cma_sandboxes` stores one row per observed Sandbox reference. The row records the NyxID owner, exact CMA service, optional remote Sandbox ID, local label, admission state, slot occupancy, creation generation, observation facts, desktop capability, and retirement timestamps. A partial unique index on `{user_id: 1}` where `occupies_account_slot=true` is the local race fence. All timestamps use the BSON helpers required by the repository.

`nyxbot_cma_agents` stores an owner/service/remote-Agent reference plus parent Sandbox, Workspace and Worktree IDs, display/profile facts, observed status, source metadata, and the source chat/turn/watch identifiers. A partial unique index on owner/service/remote Agent ID prevents duplicate references while allowing historical tombstones.

Creation attempts keep a stable idempotency key, normalized request hash, operation ID, submission state, and an encrypted request body. The encrypted body is used only to recover the exact original request after a timeout or lost acknowledgement. It is removed after a terminal outcome that proves no remote resource was created or after retirement is confirmed. API DTOs, logs, `Debug`, and audit entries contain only identifiers and bounded status metadata.

### CMA gateway and services

`nyxbot_cma_service` owns the owner-slot state machine and persistence. It exposes an adapter boundary for CMA list/detail/lifecycle/event/file/response calls. The live adapter resolves the `cma` catalog row, validates the owner, builds the existing NyxID identity assertion, sends bounded requests with the user's identity, and returns typed safe projections. Unit tests use a deterministic fake adapter to exercise race, idempotency, and state transitions without a CMA deployment.

`nyxbot_cma_agent_service` resolves an internal Agent reference, enforces the same owner/service/generation fence, and translates CMA transcript, event snapshot/SSE, response, and Desktop descriptors into DTOs. It never creates or deletes Agents and never exposes another chat's private credentials. Shared history is explicitly labelled as shared across chats.

### HTTP surface

First-party human routes are mounted below `/api/v1/assistant/nyxagent`:

- `/sandboxes` and `/sandboxes/{id}` for list, detail, label, claim, pause/revive, retirement, and bounded Sandbox lifecycle events;
- `/cma-agents/{id}` for detail, transcript, event snapshot/SSE, response detail/events, and Desktop descriptor/WS relay.

Handlers call `require_first_party_human`, resolve the authenticated owner to a string, check `assistant:cma-sandboxes`, and return dedicated DTOs. Delegated read denial includes these stream/upgrade routes. A `202` response means accepted/in progress; it never means the remote operation completed.

The CMA overlay is embedded through `catalog_spec_registry`, exposes the v1 operation IDs from the plan, and marks create/approval/service-access/retire operations with the existing `x-aevatar-tool` action classes. The three NyxID-native tools remain separate because they need NyxID conversation attachments, owner cards, or the local registration book.

### Assistant guidance

When the flag is enabled and the caller is an owner or a specialist with the `cma` grant, the stable CMA guidance is appended through `Prepared::with_guidance`. A bounded, current resource snapshot is supplied beside memory notes and is excluded from the instruction fingerprint. Guests receive neither the guidance nor the snapshot. The guidance explains the one-Sandbox rule, parent-resource selection, stable idempotency recovery, Agent reuse, task/Agent/Sandbox lifecycle distinctions, and the fact that lists do not reserve resources.

### Frontend

The frontend adds typed schemas and TanStack Query hooks for the new DTOs, then routes `/assistant/sandboxes`, `/assistant/sandboxes/$sandboxId`, and `/assistant/cma-agents/$agentId`. The sidebar and assistant search expose Sandboxes; the existing Machine entry remains unchanged.

The Sandbox page mirrors the preview: allocation usage, one environment card, configured CPU/memory, last activity, authoritative billing attribution, rename, pause, resume, retirement confirmation, empty state, claim flow, conflict state, and unknown/pending status. It intentionally contains no Agent list or Desktop.

The Agent page is entered only from a CMA task card or owner-request card. It shows the exact Agent and optional response, current task versus shared history, bounded transcript/event data with cursor deduplication, and a Desktop tab only when the CMA descriptor says the parent Sandbox and Agent support it. Stopped/deleted/no-desktop/busy/expired-auth states explain the reason and never switch to another Agent or create a resource.

## Error and security behavior

- Owner, service, remote Sandbox/Agent, generation, and feature-flag checks happen before downstream calls.
- Remote 404/permission loss is represented as unknown or unavailable; it does not release a local slot.
- The original request body and idempotency key are reused for recovery; a new key is a new intent.
- Desktop frames are relayed without persistence or audit logging and are bounded by the existing Machine Desktop session/origin rules.
- The UI never displays credentials, encrypted request material, task secrets, or untrusted callback URLs.
- Billing text uses only CMA/catalog facts and explicitly preserves unknown status.

## Verification

The implementation is verified with:

- Rust unit tests for model BSON shape, slot admission/release, idempotency recovery, owner/generation checks, and fake-gateway lifecycle handling;
- handler tests for first-party/flag/delegated access and safe DTOs;
- frontend schema/hook/page tests for empty, pending, conflict, retirement, Agent history, desktop capability, and route deep links;
- frontend typecheck/lint/build and relevant Vitest suites;
- browser checks against the local production frontend for Sandboxes, task-card navigation, execution/history, and Desktop states.
