BLOCKED

Reviewed at `0016d657` on branch `feat/assistant-ephemeral-agent-poc`. Repository facts below are verified against this worktree. Facts from the live NyxID catalog are labeled live-only and are not treated as stable code facts.

## BLOCKER

### B1. The mutation allowlist does not bind the operation that will execute

**Claim attacked:** The plan calls `(service_slug, endpoint_name)` plus a method re-check, schema validation, deny-only evaluation, and a one-call budget the "smallest safe set" for one Aevatar write (`docs/chat/chrono-ephemeral-agent-poc-extension-plan.md:169-184`).

**Evidence:**

- MCP execution identity is explicitly source-qualified: `Platform` carries a `downstream_service_id`, while `UserManaged` carries a `user_service_id`, effective owner, node, and credential state (`backend/src/services/mcp_service.rs:37-51`). The proposed key contains none of that identity.
- An executable user service suppresses a platform service by both catalog ID and slug (`backend/src/services/mcp_service.rs:792-825`, `backend/src/services/mcp_service.rs:1066-1074`). A same-slug user/org service can therefore become the object behind `aevatar__<name>` without changing the proposed allowlist entry.
- The durable endpoint contract already has immutable `id`, `service_id`, `method`, `path`, explicit `risk`, and `supports_idempotency_key` fields (`backend/src/models/service_endpoint.rs:30-63`). MCP preserves the risk and idempotency metadata keyed by endpoint ID (`backend/src/services/mcp_service.rs:181-185`, `backend/src/services/mcp_service.rs:1120-1133`), but the plan ignores it.
- NyxID's existing exact-service binding records `user_service_id`, `endpoint_id`, catalog and endpoint-contract digests, operation generation, and an effect idempotency key (`backend/src/models/approval_request.rs:39-59`). That is direct repository evidence that names alone are not a stable effect boundary.

**Consequence for the POC:** An admin endpoint edit, instance-spec override, or same-slug user/org service can move an allowlisted name to a different base URL, path, body schema, or owner while the POC continues to advertise and execute it. Re-deriving `Write` only proves that the changed operation is still POST/PUT/PATCH; it does not prove that it is the reviewed effect.

**Exact minimal fix:** Resolve B1 to `POC_MUTATION_ALLOWLIST = &[]` for the first build. Do not pin a write until the entry is an exact record containing source kind, platform `downstream_service_id` (catalog-only safe default), `endpoint_id`, expected method and path, expected endpoint-contract digest, `risk == Write`, and `supports_idempotency_key == true`. Re-check every field against the freshly loaded catalog immediately before dispatch. A mismatch must be `operation_not_allowed`, not fallback-by-name. The reviewed record, not a one-line `(slug, name)` tuple, must be the PR-visible allowlist.

### B2. "Explicit user instruction" and exactly-once behavior are prompt claims, not enforced contracts

**Claim attacked:** The prompt requirement, one mutation per run, and hostile-skill tests are presented as sufficient to ensure that a mutation is selected only when the user explicitly requested it and is safely bounded (`docs/chat/chrono-ephemeral-agent-poc-extension-plan.md:180-184`, `docs/chat/chrono-ephemeral-agent-poc-extension-plan.md:203-210`, `docs/chat/chrono-ephemeral-agent-poc-extension-plan.md:226-232`).

**Evidence:**

- The POC calls `evaluate_deny_only`, which returns true only for `ApprovalEffect::Deny`; `RequireApproval` is not a veto (`backend/src/services/approval_service.rs:184-200`). `enforce_deny_only` therefore deliberately bypasses interactive approval semantics (`backend/src/services/assistant_direct_agent_poc.rs:1032-1055`).
- The Plan hop requests `tool_choice: "none"`, but if the upstream nevertheless returns tool calls, `PlanningTransition::ExecuteBatch` executes them (`backend/src/services/assistant_direct_agent_poc.rs:326-345`). The request still includes all tool declarations even in disabled mode (`backend/src/services/assistant_direct_agent_poc.rs:1088-1107`). This is not a structural Plan/Execute wall once a write tool exists.
- A timed-out tool is reported as `outcome_uncertain` (`backend/src/services/assistant_direct_agent_poc.rs:621-637`). The proposed budget prevents a second call in that run, but it supplies no downstream idempotency key and cannot prevent the first create from landing after timeout or a user retry from creating another object.
- The route's session gate is real (`backend/src/handlers/assistant_direct_agent_poc.rs:28-38`), but authentication and CSRF establish who submitted the request, not which exact downstream effect that person consented to let a model select.

**Consequence for the POC:** Untrusted skill content or a non-conforming/model-buggy Plan response can select the one write without machine-verifiable consent. Timeout, disconnect, or rerun can duplicate a create. A fixture in which a stub model behaves well cannot establish either property.

**Exact minimal fix:** Keep mutations disabled unless the request carries a server-validated, structured consent for the exact B1 operation identity. If the write remains in scope, add a request field/UI confirmation bound to that endpoint ID and contract digest, remove tools entirely from Plan requests, reject rather than execute any Plan-phase tool call, generate one NyxID-owned idempotency key per consent/run, inject it without exposing it as a model argument, and require the endpoint contract to support it. The same key must survive timeout/retry within the run. This necessarily invalidates the plan's "no FE schema change" claim. For the disposable demo, deleting the mutation beat is the smaller safe fix.

### B3. B2's platform path and fixed-descriptor fallback are different services with different authentication

**Claim attacked:** The plan says Aevatar works either from the canonical platform catalog or from fixed descriptors resolved like Ornn against an authentic connected `UserService`, with the choice deferred to live validation (`docs/chat/chrono-ephemeral-agent-poc-extension-plan.md:212-222`, `docs/chat/chrono-ephemeral-agent-poc-extension-plan.md:274-286`).

**Evidence:**

- Platform rows with `requires_user_credential:false` are auto-loaded (`backend/src/services/mcp_service.rs:720-782`) and emitted with `McpToolSource::Platform` (`backend/src/services/mcp_service.rs:1080-1095`). A fixed Ornn-style resolver accepts only one executable `McpToolSource::UserManaged` row (`backend/src/services/assistant_direct_agent_poc/tools.rs:271-287`). A platform-only Aevatar row cannot satisfy that fallback.
- The two sources execute through different resolver branches: exact `UserService` ID at `backend/src/services/mcp_service.rs:3081-3096`, versus platform `DownstreamService` ID at `backend/src/services/mcp_service.rs:3193-3241`. Substituting the fallback changes owner, credential, node, catalog precedence, and approval identity.
- `execute_tool` adds an identity token only when `identity_propagation_mode` is not `none` and is `jwt`/`both` (`backend/src/services/mcp_service.rs:3261-3306`). It does not forward the browser session bearer.
- PR #1443's master-credential gate is bypassed by design for `auth_method:"none"` (`backend/src/services/proxy_service.rs:736-748`) and does not govern the user-managed branch. It therefore cannot make an unauthenticated Aevatar request authenticated.
- Live-only readback on 2026-08-14 (`nyxid catalog show aevatar --output json`) found the platform row with `auth_method:"none"`, `requires_credential:false`, and an OpenAPI URL. The public response contract does not expose identity propagation fields (`backend/src/handlers/catalog.rs:24-145`), so successful typed authentication is still unverified.

**Consequence for the POC:** E5 can be unavailable when needed, or can silently execute a user/org service instead of the reviewed platform row. The live platform row can publish hundreds of operations yet have every authenticated Aevatar call fail because neither a bearer nor an identity JWT reaches it. The core demo path is not implementation-ready.

**Exact minimal fix:** Resolve B2 to canonical `Platform` source only and delete E5. The existing live OpenAPI URL means endpoint materialization/configuration is the smaller path than a second execution identity. Before enabling Aevatar tools, require an admin readback of the exact platform service ID, active stored `ServiceEndpoint` IDs, `identity_propagation_mode in {jwt,both}`, the exact audience accepted by Aevatar, visibility/category, and a successful read-only typed call through `execute_tool`. If any check fails, expose no Aevatar operation. Do not compensate with a `UserManaged` fallback.

## MAJOR

### M1. The account surface is unnecessary for the demo and sends avoidable identifiers to Chrono

**Claim attacked:** The proposed account module is a minimal, credential-safe reusable projection, and uniform `ModelToolResult` scrubbing is defense in depth (`docs/chat/chrono-ephemeral-agent-poc-extension-plan.md:186-199`).

**Evidence:**

- `key_service::list_api_keys` returns full `ApiKey` models (`backend/src/services/key_service.rs:538-549`), including `key_prefix` and `key_hash` (`backend/src/models/api_key.rs:16-25`). Projection is therefore the only safety wall; the backing result is not "metadata-only by construction."
- The codebase treats even `key_prefix` as redacted Debug data (`backend/src/services/key_service.rs:46-53`). The proposed model result nevertheless includes it, as well as email, user/org IDs, node IDs, owner display names, and exact heartbeat timestamps (`docs/chat/chrono-ephemeral-agent-poc-extension-plan.md:190-195`).
- `scrub_credentials` only removes a fixed set of canonicalized object keys; it does not include `keyprefix` or `keyhash`, does not remove email/node topology, and does nothing to secrets embedded in string values (`backend/src/services/assistant_direct_agent_poc/tools.rs:881-924`).
- A raw `Node` contains auth-token hash, encrypted signing secret, signing-secret hash, IP metadata, metrics, and last error (`backend/src/models/node.rs:27-95`). A future projection omission would pass several of those fields through the current scrubber.
- The target demo sequence does not use `nyx_account_call`; it uses `nyx_whoami` and connected services (`docs/chat/chrono-ephemeral-agent-poc-extension-plan.md:248-250`).

**Consequence for the POC:** The extension broadens model-visible PII, partial credential identifiers, and infrastructure topology for no demonstrated scenario value. A single projection regression can send hashes or encrypted fields to Chrono despite the plan's blanket "never sees credentials" claim.

**Exact minimal fix:** Remove `nyx_account_call` and E3 from this demo plan. Keep a minimal `nyx_whoami` only if the scenario needs it, projected to `display_name`, `auth:"session"`, and org display name/role; omit email and all stable IDs. If account contracts are retained for reuse, create explicit DTOs per operation, omit `key_prefix`, email, node IDs/owner names/exact timestamps by default, add `keyhash`/`keyprefix` to the scrubber as backup only, map service errors to stable synthetic codes, and test the serialized model envelope against an exact field allowlist. This is the main overengineering to remove from the disposable POC.

### M2. The frontend displays tool metadata, not tool results

**Claim attacked:** Existing frames and two metadata-only frontend fixtures are sufficient to demonstrate actual execution and grounded reporting without a schema change (`docs/chat/chrono-ephemeral-agent-poc-extension-plan.md:224-235`).

**Evidence:**

- `tool.completed` emits only outcome, HTTP status, duration, result byte count, and truncation; no result or preview is present (`backend/src/services/assistant_direct_agent_poc.rs:61-77`).
- The strict frontend schema accepts exactly those metadata fields (`frontend/src/lib/assistant/direct-agent-poc.ts:67-84`).
- Completion replaces the step metadata with only `tool name + HTTP status` or `outcome uncertain` (`frontend/src/lib/assistant/direct-agent-poc.ts:615-644`).

**Consequence for the POC:** A viewer cannot inspect the account/Aevatar evidence or downstream mutation identifier. The model's final paraphrase is the only visible claim, so the demo and reusable evals cannot distinguish a grounded answer from a fabricated one. Reload removes even that transient execution context.

**Exact minimal fix:** Add a bounded, explicitly projected `result_preview` (or ephemeral result artifact) to `tool.completed`, update the strict Zod schema, and render it in an expandable tool step. Account/skill tools should send their safe DTO; generic typed calls should send a separately capped scrubbed preview. Test that secrets and raw arguments are absent and that a mutation identifier shown by the final answer is also visible in its producing tool result. If results intentionally remain hidden, remove "visible actual results" from the demo/eval acceptance criteria.

### M3. The 16 KiB result cap is post-consumption; downstream request/response memory is not bounded

**Claim attacked:** Keeping the existing unbounded `execute_tool` drain is acceptable because tool results are capped at 16 KiB and calls time out at 60 seconds (`docs/chat/chrono-ephemeral-agent-poc-extension-plan.md:22-29`, `docs/chat/chrono-ephemeral-agent-poc-extension-plan.md:163-167`, `docs/chat/chrono-ephemeral-agent-poc-extension-plan.md:274-280`).

**Evidence:**

- Node streaming appends every chunk to an unbounded `Vec` before returning (`backend/src/services/mcp_service.rs:3509-3535`).
- Direct execution calls `response.text()` with no byte limit (`backend/src/services/mcp_service.rs:3554-3589`).
- Only after the entire string is returned does `ModelToolResult::from_response` scrub, serialize, and truncate it to 16 KiB (`backend/src/services/assistant_direct_agent_poc/tools.rs:815-859`).
- Tool-call arguments are parsed without a POC byte cap (`backend/src/services/assistant_direct_agent_poc.rs:585-592`); the outer hop/body cap is not a downstream request-body policy.

**Consequence for the POC:** A large Aevatar list/result, error document, or node stream can consume arbitrary memory and spend most of the run draining bytes that will be discarded. A 60-second timeout is not a memory cap and cannot undo a write already accepted downstream.

**Exact minimal fix:** Add explicit `MAX_TOOL_ARGUMENT_BYTES` and `MAX_TOOL_RESPONSE_BYTES` before dispatch/while reading. Direct and node paths must stop buffering at the limit and return a typed truncated/too-large outcome. Because the current `execute_tool` signature returns an already-buffered `String`, a real response cap requires ownership in `backend/src/services/mcp_service.rs` (and the node streaming path), which is missing from E1-E9 and contradicts the stated deletion boundary. Update file ownership and add direct/node oversized-response tests; otherwise restrict this extension to known bounded stubs and do not call the live catalog safe.

### M4. The context arithmetic is not a proof, and overflow does not force a final Report

**Claim attacked:** The estimated 430 KiB fits under 448 KiB and the forced-final path "absorbs overflow honestly" (`docs/chat/chrono-ephemeral-agent-poc-extension-plan.md:163-167`).

**Evidence:**

- Every hop rebuilds the system prompt/tool schemas and copies the complete accumulated message array (`backend/src/services/assistant_direct_agent_poc.rs:1088-1107`); each tool result is appended to that array (`backend/src/services/assistant_direct_agent_poc.rs:1127-1136`). The relevant limit is each serialized request's worst-case bytes, not `hop count * an assumed 1 KiB overhead`.
- The ingress allows 128 KiB of raw message content (`backend/src/handlers/assistant_direct_agent_poc.rs:58-69`) and an injected bundled skill can be 64 KiB (`backend/src/services/assistant_direct.rs:78-85`). JSON escaping, assistant tool-call arguments, schemas, envelopes, and phase text are not bounded by the plan's arithmetic.
- If serialization exceeds 448 KiB, `chrono_hop` immediately returns `ContextOverflow` (`backend/src/services/assistant_direct_agent_poc.rs:458-462`). The error path emits `context_overflow` and `[DONE]`, not a final model hop (`backend/src/services/assistant_direct_agent_poc.rs:971-988`). Forced-final is triggered only by call counts (`backend/src/services/assistant_direct_agent_poc.rs:433-436`).
- The 300-second value is only the in-process wall timeout (`backend/src/services/assistant_direct_agent_poc.rs:229-243`); repository code cannot prove an external ingress/load-balancer hard duration.

**Consequence for the POC:** A valid near-limit request plus the intended skill and 7-10 results can terminate with no Report. The plan promises graceful degradation that the code does not implement, and a five-minute local deadline may still be cut off by live infrastructure.

**Exact minimal fix:** Reserve bytes for one disabled-tool final hop before every tool dispatch. Add a serialized-size budget that accounts for the actual prompt, tool definitions, messages, maximum escaped arguments, and retained results; compact or replace old results with bounded summaries before the reserve is crossed. Add a worst-case test using escaped 128 KiB input, the largest bundled skill, ten maximum envelopes, and maximum tool-call arguments, asserting that a final Report still fits. Treat `300s` as live-only until the deployed ingress hard timeout is read back; keep the old deadline or set the app deadline at least 10 seconds below the verified external limit.

### M5. Skill provenance is not content provenance, and the observed-ID contract is underspecified

**Claim attacked:** `bundled@<CARGO_PKG_VERSION>` is build-pinned provenance, and any ID projected from an earlier Ornn search can safely enter a run-local observed set (`docs/chat/chrono-ephemeral-agent-poc-extension-plan.md:201-208`).

**Evidence:**

- Bundled skills are raw `include_str!` bodies with only size assertions; there is no content digest or per-skill version (`backend/src/services/assistant_direct.rs:53-85`). Package version does not change for every source edit or identify which bytes an eval consumed.
- Current Ornn projection accepts the first `id`/`guid`/`skillId` JSON value without type or UUID validation (`backend/src/services/assistant_direct_agent_poc/tools.rs:956-980`).
- Current fetch provenance hard-codes the one demo GUID in the fence (`backend/src/services/assistant_direct_agent_poc/tools.rs:926-943`). Generalizing the validator without also parameterizing and hashing provenance would misattribute fetched content.
- Path values are percent-encoded correctly (`backend/src/services/mcp_service.rs:2646-2648`), so segment injection is not the issue; source/content identity is.

**Consequence for the POC:** Two builds can report the same bundled version for different prompts, evaluations cannot reproduce the skill bytes, and malformed/search-controlled IDs can be treated as observed without a clear source/version binding. Coupling that content to a write-capable loop raises the cost of ambiguous provenance.

**Exact minimal fix:** Define one reusable provenance envelope containing `source`, normalized `id`, source version when available, and `content_sha256`. Compute the bundled digest from the included bytes. Accept only canonical UUID strings from Ornn search, store a source-qualified run-local map from ID to the search call that observed it, and fence/audit the requested ID plus digest of the fetched `SKILL.md`. Fetch failure or ID/version mismatch must not add or retain authority. Keep mutations disabled after untrusted skill fetch unless B2's independent structured consent exists.

### M6. Mutation audit/error/test coverage is keyed to display names, not the effect boundary

**Claim attacked:** Adding `"mutation":true` and the listed unit/fixture tests is sufficient audit and CI coverage (`docs/chat/chrono-ephemeral-agent-poc-extension-plan.md:224-245`).

**Evidence:**

- Current tool audit records public call ID, logical tool, service slug, endpoint name, outcome/status/size, and Ornn version only (`backend/src/services/assistant_direct_agent_poc.rs:1190-1208`). The plan adds a boolean but no source ID, service ID, endpoint ID, contract digest, idempotency key digest, or consent identity.
- Audit writes are fire-and-forget (`backend/src/services/audit_service.rs:46-73`). That is an existing project tradeoff, but it means the model response cannot be treated as a durable mutation receipt.
- The plan's tests cover name allowlisting and a second-call budget, but omit same-slug source shadowing, endpoint-contract drift, required-approval behavior, structured consent, idempotent timeout/retry, cancellation after dispatch, direct/node body caps, mounted cross-origin session POST, and UI result/evidence parity (`docs/chat/chrono-ephemeral-agent-poc-extension-plan.md:226-235`).
- Project rules require internal/database errors not to leak (`CLAUDE.md:31-34`). Existing downstream execution maps service errors to a stable `tool_execution_failed` code (`backend/src/services/assistant_direct_agent_poc.rs:823-852`), but the proposed account module does not specify an equivalent error mapping.

**Consequence for the POC:** A mutation can be audited as `aevatar + name` without proving which source/contract ran, while the test suite remains green under the source-drift and retry failures that matter most. New account errors can accidentally expose database or authorization detail to Chrono.

**Exact minimal fix:** Extend audit metadata with the immutable B1 identity, method, contract digest, consent ID, hashed idempotency key, and explicit `dispatch_started/outcome_uncertain/completed` state, never arguments or bodies. Define stable account error codes and log underlying `AppError` only server-side. Add the omitted boundary tests, including a mounted session+bad-Origin request that produces zero dispatches. Keep the existing billing coverage and Docker embed gates; they are necessary but do not cover effect safety.

## MINOR

### N1. "Understand" is a server preflight, not a model phase

**Claim attacked:** Prompt ordering is described as Understand -> Plan -> Execute -> Report (`docs/chat/chrono-ephemeral-agent-poc-extension-plan.md:59-66`, `docs/chat/chrono-ephemeral-agent-poc-extension-plan.md:248-250`).

**Evidence:** The understand stage only loads catalogs and emits counts (`backend/src/services/assistant_direct_agent_poc.rs:275-321`). The first model call is `AgentPhase::Plan` (`backend/src/services/assistant_direct_agent_poc.rs:323-333`), and `AgentPhase` has only Plan/Execute/Final (`backend/src/services/assistant_direct_agent_poc/prompt.rs:12-17`). The actual user messages are present in that Plan request (`backend/src/services/assistant_direct_agent_poc.rs:438-443`, `backend/src/services/assistant_direct_agent_poc.rs:1094-1104`).

**Consequence for the POC:** The model sees the user's goal, so there is no missing-goal bug, but it does not see the connected-service/operation counts shown by Understand. Acceptance language can wrongly imply a fourth model reasoning phase or a plan informed by those counts.

**Exact minimal fix:** Call it "Understand preflight" in the plan and evals. If the counts are intended to influence planning, append a server-owned, non-authoritative summary to the Plan request; otherwise explicitly state that discovery happens during Execute and keep Plan generic.

### N2. `list_user_nodes` cannot produce authoritative `is_connected`

**Claim attacked:** `node_service::list_user_nodes(db, user_id)` is the complete backing for a projection containing `is_connected` (`docs/chat/chrono-ephemeral-agent-poc-extension-plan.md:194`).

**Evidence:** The service returns `Vec<NodeWithOwner>` and only reads Mongo plus owner data (`backend/src/services/node_service.rs:430-456`). Authoritative connection state comes from `NodeWsManager::session_info` in the handler projection (`backend/src/handlers/node_admin.rs:390-403`). Persisted `Node.status` is a separate field (`backend/src/models/node.rs:67-87`).

**Consequence for the POC:** Deriving `is_connected` from persisted status can report a stale node as live, undermining the "actual account state" claim.

**Exact minimal fix:** If this overengineered account operation is retained, pass `&NodeWsManager` into the POC account function and use `is_connected/session_info`, or drop `is_connected` and label persisted `status` accurately. Add a test where status is online but no WS session exists.

### N3. Key and node account reads are unbounded before the 16 KiB envelope cap

**Claim attacked:** Exact projections plus `ModelToolResult` capping make the account calls bounded (`docs/chat/chrono-ephemeral-agent-poc-extension-plan.md:190-199`).

**Evidence:** `list_api_keys` collects every active key (`backend/src/services/key_service.rs:538-549`), and `list_user_nodes` collects every reachable active personal/org node (`backend/src/services/node_service.rs:430-456`). The result cap is applied only after all rows are loaded, projected, and serialized (`backend/src/services/assistant_direct_agent_poc/tools.rs:815-859`). Only approvals have explicit page/per-page arguments (`backend/src/services/approval_service.rs:1344-1387`).

**Consequence for the POC:** Large accounts spend unbounded DB/memory work and then return a syntactically truncated string that can lose row boundaries or totals.

**Exact minimal fix:** Prefer deleting these calls. If retained, add server-owned limits to all three operations, return `{items,total,returned,truncated}`, and truncate by complete projected rows before `ModelToolResult`. Do not rely on byte slicing as pagination.

## Confirmed invariants

- NyxID owns the loop and calls downstream services in-process; Aevatar is not a scheduler or workflow owner (`backend/src/services/assistant_direct_agent_poc.rs:701-854`).
- Generic proxy endpoints remain structurally excluded (`backend/src/services/assistant_direct_agent_poc/tools.rs:362-383`), and operation arguments are validated before execution (`backend/src/services/assistant_direct_agent_poc.rs:737-747`).
- The proposed account backings are reads: keys are filtered by user/active state (`backend/src/services/key_service.rs:538-549`), nodes use personal/org ACL membership (`backend/src/services/node_service.rs:430-456`), and `list_requests(..., &[], ...)` is personal-only (`backend/src/services/approval_service.rs:1329-1351`). No auto-provisioning call is hidden in those functions.
- The Plan model does receive the actual user goal, as shown in N1.
- The route is human/session-only in both router middleware and handler checks (`backend/src/routes.rs:1501-1564`, `backend/src/handlers/assistant_direct_agent_poc.rs:28-38`), and unsafe cookie-authenticated POSTs are covered by the private-router CSRF layer (`backend/src/main.rs:1186-1197`, `backend/src/mw/csrf.rs:80-130`).
- Billing classification is correctly mounted as `Metered(Proxy)` and fails closed before egress (`backend/src/routes.rs:124-145`, `backend/src/services/billing/route_inventory.rs:48-69`). Existing route smoke exercises the session-only agent and usage settlement (`backend/src/billing_integration_tests.rs:367-417`).
- PR #1443's platform master-credential claims survive attack: private rows require a valid app consent, public rows pass, invalid/unknown rows fail not-found-shaped (`backend/src/services/proxy_service.rs:150-213`); server-chosen Chrono credentials require a public valid row (`backend/src/services/proxy_service.rs:215-244`); user credentials are decrypted on their separate path (`backend/src/services/proxy_service.rs:862-884`). This gate is simply not Aevatar authentication when the live row is `auth_method:none`.

## Unresolved live facts

- **Verified live-only, read-only:** `nyxid catalog show aevatar --output json` on 2026-08-14 returned the platform Aevatar row with base URL `https://aevatar-console-backend-api.aevatar.ai`, `auth_method:"none"`, `requires_credential:false`, OpenAPI URL `https://aevatar-console-backend-api.aevatar.ai/api/openapi.json`, and `supports_proxy_read:false` / `supports_proxy_write:false`. `nyxid catalog endpoints aevatar` fetched 378 upstream operations. No write or tool execution was performed.
- **Still unresolved:** exact live `DownstreamService.id`, visibility/category, `identity_propagation_mode`, `identity_jwt_audience`, and whether Aevatar accepts that identity assertion. The normal catalog response cannot reveal those fields.
- **Still unresolved:** whether active stored `ServiceEndpoint` rows exist for the platform row, their endpoint IDs, response contracts, explicit risk values, idempotency support, and contract digests. Fetching 378 operations from the remote spec does not prove Mongo materialization or POC eligibility.
- **Still unresolved:** whether the demo user has a same-slug personal/org `UserService` that shadows the platform row, and the effective deny/require-approval policy owner for that service.
- **Still unresolved:** Ornn's unique executable `UserManaged` row and identity propagation configuration for the demo user.
- **Still unresolved:** deployed ingress/load-balancer maximum stream duration and whether it exceeds the proposed app deadline with margin. Heartbeats prove only that the application emits bytes; they do not prove absence of a hard connection-duration limit.

## Implementation-ready checklist

1. Ship the first build with an empty mutation allowlist; remove the mutation from the demo acceptance script until items 2-4 are complete.
2. Delete the fixed Aevatar descriptor fallback. Resolve and admit only the exact canonical platform service and stored endpoint rows.
3. Obtain admin readback for Aevatar service identity, endpoint materialization, identity JWT mode/audience, endpoint risk/idempotency metadata, capabilities, and approval policy; record a read-only typed-call result.
4. If a write remains mandatory, implement exact source/service/endpoint/contract binding, structured user consent, server-owned idempotency, fresh pre-dispatch revalidation, and immutable mutation audit metadata.
5. Omit tool declarations in Plan and make every Plan-phase tool call non-executable; add a zero-dispatch regression test.
6. Remove `nyx_account_call` from the disposable demo. If retained, use minimal DTOs, stable error codes, authoritative WS state, row limits, complete-row truncation, and exact serialized-field tests.
7. Add bounded downstream argument and response handling to the actual direct and node consumption paths, and update the task/file ownership and deleteability claims accordingly.
8. Reserve serialized context for the final Report and test the true worst-case body, rather than relying on approximate arithmetic.
9. Add bounded redacted tool-result previews to SSE/frontend so actual evidence and mutation identifiers are inspectable.
10. Replace package-version provenance with source-qualified content digests and strictly validated observed Ornn IDs.
11. Add source-shadow, contract-drift, require-approval, consent, timeout/retry/idempotency, cancellation, body-cap, CSRF zero-dispatch, org-owner, sanitized-error, audit-identity, and frontend evidence-parity tests.
12. Run the listed Rust/frontend/Docker gates, then perform only read-only live validation until the exact write has separate owner approval.

## Counts and verdict

`BLOCKER: 3` | `MAJOR: 6` | `MINOR: 3` | `TOTAL: 12`

**Verdict: BLOCKED.** The read-only extension can proceed after deleting the account expansion and Aevatar fallback, but the plan's required Aevatar mutation demo must not build or ship until B1-B3 are resolved with exact identity, authentication, consent, and idempotency rather than names and prompt instructions.
