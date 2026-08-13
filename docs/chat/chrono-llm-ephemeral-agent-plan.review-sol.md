# Adversarial Review: Chrono-LLM Ephemeral Agent Plan

Reviewer: Codex Sol
Target: docs/chat/chrono-llm-ephemeral-agent-plan.md (Draft v1, 2026-08-13)
Lens: code-level feasibility, integration traps, and the Sol checklist in plan section 20.

Method: I checked the cited implementation on this branch. References to the live P1-P6 probes,
production catalog rows, and the Ornn registry are treated as live-only claims; they are not branch
facts. The findings below attack the proposed implementation, not the validity of an upstream probe.

## BLOCKER

### B1. The route is not actually session-only, and the proposed execution context drops caller scope

**Claim attacked.** Sections 5.3 and 7 say the human-only mount makes the endpoint session-only and
then load the catalog with unrestricted service/node scope, constructing
McpExecContext { api_key_id: None, allow_all_nodes: true, ... } (plan lines 112, 151, 157).

**Evidence.** The human-only router rejects delegated, API-key, service-account, and relay credentials at
backend/src/routes.rs:1492-1555, but ordinary OAuth access tokens are constructed as AuthMethod::AccessToken
and are not one of those rejection classes at backend/src/mw/auth.rs:804-851. Those tokens can carry
allow_all_services, allowed_service_ids, and resource restrictions. The existing MCP path derives
service scope at backend/src/handlers/mcp_transport.rs:579-610 and node scope at :1393-1414; the
context passed to execute_tool is explicitly the carrier for API-key identity and node restrictions at
backend/src/services/mcp_service.rs:138-152. The plan instead asks for ServiceScope::Unrestricted and
an unrestricted context.

**Consequence for the demo.** A normal OAuth bearer that reaches this route can discover and invoke
services outside its granted service resources. The proposed unrestricted node context also discards
node restrictions for any scoped caller type that reaches the handler (and makes a future scope-bearing
auth method fail open). The mount's comment says “human session surface,” but the code does not establish
that invariant. This is a direct violation of the claimed authorization boundary before prompt or approval
logic runs.

**Concrete minimal fix.** In the new handler, require auth_user.auth_method == AuthMethod::Session before
opening the stream and return the existing forbidden response otherwise. If OAuth access tokens are
intentionally supported, derive ServiceScope, NodeScope, resource_uris, and McpExecContext from
AuthUser exactly as MCP transport does, and add a restricted-OAuth test. For an internal demo, the
explicit session check is the smaller and safer choice.

### B2. “No raw secrets enter model/browser context” is not enforced

**Claim attacked.** Section 5.2 says Chrono receives only truncated results and the browser receives
summaries/byte counts; section 10 nevertheless specifies a browser-visible raw args_preview
(plan lines 106, 209-212).

**Evidence.** mcp_service::execute_tool returns a complete String and has no result-redaction or
response-limit parameter at backend/src/services/mcp_service.rs:3039-3058. Direct execution consumes
the entire downstream body with response.text() at backend/src/services/mcp_service.rs:3551-3583;
node streaming appends every chunk to an unbounded buffer at :3503-3529. Typed arguments are copied
into request/body fields at backend/src/services/mcp_service.rs:2638-2667, and the generic proxy accepts
arbitrary JSON body values at :3627-3639. The plan's args_preview is serialized user/model input,
not a redacted field (plan line 210).

**Consequence for the demo.** A downstream response can contain an access token, password, cookie,
private document, or other credential-shaped value and that complete value is available to the agent
layer before its proposed truncation. A model can also put credential-shaped data in an argument and
the browser frame would display it. Length limits and a provenance fence are not secret handling.
This contradicts a MUST requirement and makes a live demo unsafe with arbitrary connected services.

**Concrete minimal fix.** Narrow the first slice to an allowlist of typed, read-only operations with
declared safe response fields. Do not expose the generic proxy in the agent registry. Omit args_preview
or send only the tool name and parameter names; if arbitrary results remain, add deterministic redaction
before both the model message and every SSE frame, with tests containing tokens/passwords in arguments
and responses.

### B3. The advertised response/body bounds are post-hoc and can be bypassed by buffering

**Claim attacked.** Section 9 presents a 16 KiB tool-result cap, a 384 KiB upstream cap, and a 60-second
tool timeout as a boundedness floor (plan lines 181, 253-254).

**Evidence.** The reusable execution API returns only (u16, String) and accepts neither a byte limit
nor a cancellation token at backend/src/services/mcp_service.rs:3039-3058. Direct responses are
fully buffered by response.text() at :3569-3583; node streams grow body_buf for every chunk at
:3503-3529. The generic endpoint deliberately permits model-selected path, method, query, and body at
backend/src/services/mcp_service.rs:1582-1601 and :1618-1643. The proposed 16 KiB truncation therefore
occurs only after an arbitrary response has already been read into memory; the 384 KiB value is an
agent-message bound, not a downstream drain bound.

**Consequence for the demo.** A large response, package download, or never-ending stream can consume
memory and occupy the run until the outer timeout fires. Ornn's documented package response is itself
large, so the planned “extract SKILL.md then cap” does not cap the network read. The demo can become a
slow or memory-heavy process despite claiming hard limits.

**Concrete minimal fix.** Add a bounded, cancellation-aware response-drain API in the proxy/MCP/node
owners and settle billing based on bytes actually drained, or remove generic/streaming endpoints from
the first slice and permit only known bounded typed reads. Enforce request-body limits at the tool
boundary as well as the initial validate_direct_request body limit.

## MAJOR

### M1. Approval parity is underspecified and the named call uses the wrong identity path

**Claim attacked.** Section 5.3 calls a direct evaluate_deny_only invocation “the exact parity” of the
REST/MCP proxy (plan lines 113, 157; T4 AC3).

**Evidence.** evaluate_deny_only requires actor id, service-owner id, catalog service id, and an
operation descriptor at backend/src/services/approval_service.rs:184-200. The existing MCP resolver
explicitly documents that UserService.id must not be conflated with catalog_service_id and resolves
the owner/target through approval_target_for_tool at
backend/src/handlers/mcp_transport.rs:1424-1466; the resulting target is passed into approval at
:1497-1507. The metadata-only resolver also performs org owner access checks at
backend/src/services/proxy_service.rs:1666-1718. The plan supplies neither this target resolution nor
an org-owner rule and names only resolve_tool_call followed by evaluate_deny_only.

**Consequence for the demo.** A deny rule keyed to the catalog service can silently miss when the
resolved tool is a user-service row. Org-owned services can lose the owner/member/admin routing check.
The local deny-rule acceptance test can accidentally validate the wrong UUID and still pass.

**Concrete minimal fix.** Reuse or extract the existing approval-target resolver, then pass its
service_owner_user_id and catalog service_id to evaluate_deny_only. Add one personal-service and one
org-owned-service test with the deny row keyed to the catalog id.

### M2. The generic proxy makes the “delete refusal” and “no free-form paths” claims false

**Claim attacked.** Sections 5.3, 7, and the demo script say the model has no free-form path tool and
will decline a delete (plan lines 115, 163, 338).

**Evidence.** The catalog's generic endpoint is explicitly described as allowing arbitrary HTTP requests
at backend/src/services/mcp_service.rs:1582-1601; its schema exposes path, method, query, and body,
including POST, PUT, PATCH, and DELETE, at :1618-1643. Path validation at
backend/src/services/mcp_service.rs:3588-3639 and backend/src/services/proxy_service.rs:396-402
rejects traversal delimiters, but does not make the path non-arbitrary. DELETE is classified as
Destructive while other non-read methods are Write at backend/src/services/operation_descriptor.rs:144-149;
evaluate_deny_only blocks only an explicit deny rule, not all writes.

**Consequence for the demo.** If the connected service has a generic endpoint (or a typed delete), a
model-selected call can mutate or delete data. Prompt grounding does not implement the advertised
read-only policy, so the “Now delete one” honesty beat is nondeterministic and potentially destructive.

**Concrete minimal fix.** Exclude generic proxy and every non-read/destructive endpoint from the agent
tool definitions, or add an explicit default-deny read-only policy for this route before any downstream
call. Keep the generic proxy for a later, separately authorized surface.

### M3. Runtime auto-provisioning contradicts ephemerality and is slug/endpoint fragile

**Claim attacked.** Sections 2 and 9 promise no persistence, while section 8 assumes a run-start
auto_provision_no_auth_services call will materialize a usable ornn-api under that exact slug
(plan lines 170-173).

**Evidence.** Auto-provisioning first reconciles and deletes stale auto-provisioned rows at
backend/src/services/unified_key_service.rs:1464-1483, queries eligibility from catalog state at
:1485-1526, and creates persistent UserEndpoint and UserService records at :1618-1665. Slug collisions
are deliberately disambiguated through resolve_unique_slug(...AutoDisambiguate) at :1594-1608, so the
resulting slug can be ornn-api-2. The ordinary keys listing performs this mutation as a side effect at
backend/src/handlers/keys.rs:1043-1054. Endpoint availability still depends on live catalog/spec/template
rows; an absent row is not created by this code path.

**Consequence for the demo.** An “ephemeral” run changes account state and can delete stale rows. A
pre-existing slug collision breaks the hardcoded service_slug == "ornn-api" lookup, and missing typed
endpoint rows make both Ornn tools unavailable even when provisioning succeeds. These live-state failures
are discovered only after the implementation is built.

**Concrete minimal fix.** Make Ornn setup a Stage-0 go/no-go precondition: preconnect a known user and
resolve the actual UserService id/slug and two concrete typed endpoint rows. Remove auto-provisioning
from the run. If a generic fallback is later allowed, build its descriptor server-side and still gate
the service/endpoint explicitly.

### M4. Billing inventory and smoke-test ownership is incomplete

**Claim attacked.** T1 says adding the route and a matching inventory entry is sufficient for the billing
smoke (plan lines 238-245).

**Evidence.** The route macro installs policy extensions at backend/src/routes.rs:18-21 and the assistant
route declaration is currently the single tuple at :124-138. The smoke asserts that every mounted metered
route is crossed through its real HTTP boundary at backend/src/billing_integration_tests.rs:2177-2188;
CI runs that smoke at .github/workflows/ci.yml:238-250. T1's ownership list omits
backend/src/billing_integration_tests.rs and T8 only says “inventory additions,” not a mounted agent
invocation.

**Consequence for the demo.** Once the new route is mounted as metered, CI fails until a real agent run
crosses the route and is added to exercised_routes. A stub Chrono stream plus a stub downstream tool is
needed; this is not covered by merely adding a table row.

**Concrete minimal fix.** Add the billing integration test to T1/T8 ownership and budget a route-boundary
case that exercises one complete agent/tool run, records the new route, and verifies settlement. Do not
call the inventory-only unit green.

### M5. Billing classification is self-attested for the dominant egress

**Claim attacked.** T2 declares the route Metered(Mcp) and has each synthesized LLM request insert
Metered(Proxy) into its own extensions (plan lines 242-245).

**Evidence.** The fail-closed checker accepts only a policy already present and matching the expected
ingress at backend/src/services/billing/route_inventory.rs:48-70; the proxy then reads that extension
at backend/src/handlers/proxy.rs:1113-1123 and execute_proxy_inner enforces it at :1416-1418. The
route's router policy is installed by the macro, not by the handler, at backend/src/routes.rs:18-21.
Writing a fresh policy onto a synthesized request turns the route-level guarantee into handler
self-attestation for every expensive LLM hop.

**Consequence for the demo.** A missing extension or a future refactor produces an internal
classification failure after the run has started; a misplaced extension can make the route appear
correct to tests while bypassing the intended boundary. The plan calls this billing fidelity, but it is
an execution invariant.

**Concrete minimal fix.** Add a named AssistantAgent ingress (or another shared, explicitly tested
route-to-egress conversion) that can mint the Proxy permit for LLM hops and the MCP permit required by
`execute_tool` from one trusted route policy. Do not rely on a handler inserting a fresh policy into each
synthesized request; merely changing the route to Proxy would not by itself authorize MCP egress.

### M6. execute_tool has no cancellation path, so timeout/disconnect cannot meet T6's guarantee

**Claim attacked.** Section 9 says the run token stops the loop before the next hop/tool; T6 AC1 says a
disconnect while a tool sleeps reaches no later hop (plan lines 194-195, 261-263).

**Evidence.** Production request cancellation is obtained from request extensions at
backend/src/downstream_disconnect.rs:42-51 and consumed by execute_proxy_inner at
backend/src/handlers/proxy.rs:1416-1418. The public execute_tool signature has no token at
backend/src/services/mcp_service.rs:3039-3058; node collection waits on rx.recv() without a cancellation
branch at :3503-3519. A newly synthesized Request has no production connection token unless the
implementation explicitly copies it.

**Consequence for the demo.** The browser can receive a timeout/cancel frame while a direct request or
node-side effect continues, and the tool ledger has no reliable terminal outcome. A timeout around the
future only stops the agent task; it does not establish downstream cancellation or tell the model whether
the side effect happened.

**Concrete minimal fix.** Thread one run CancellationToken through a shared proxy/MCP execution API,
select it while awaiting node/direct response bodies, and emit tool.completed with an explicit
outcome_uncertain result before terminalizing the run. Add a test that observes the downstream stub
after disconnect.

### M7. Usage capture requires draining every LLM response, but the plan does not make that contract explicit

**Claim attacked.** T2 and section 13 promise existing Chrono usage capture is preserved while the agent
extracts tool-call fragments and advances the state machine (plan lines 244-245, 296-299).

**Evidence.** Proxy streaming usage settlement is performed while the response body is consumed in
backend/src/handlers/proxy.rs:3117-3306. The shipped frontend deliberately keeps reading after a
finish_reason so usage and [DONE] are consumed at frontend/src/lib/assistant/direct-transport.ts:751-755
and :680-691.

**Consequence for the demo.** If the new backend loop returns as soon as it has a tool-call finish reason,
or stops after final text before the usage frame/[DONE], usage rows and billing settlement can be missing
and an observer can remain blocked. This is especially likely because each hop is parsed for logical
content rather than forwarded as a body.

**Concrete minimal fix.** Separate logical parsing from transport draining: consume through the terminal
usage frame and [DONE] (or a bounded EOF/error), feed all bytes to the existing observer, and only then
advance/settle the hop. Add a fixture asserting usage is recorded when tool calls precede final text.

### M8. The shared backend SSE helper is not sufficient for the proposed fragment protocol

**Claim attacked.** T3 says to reuse services/sse_parser for streamed delta.tool_calls and treats P3
fragment reassembly as a bounded parser task (plan lines 77-80, 181, 248-249).

**Evidence.** The backend parser searches only for "\n\n" and stores the stream in a String at
backend/src/services/sse_parser.rs:15-69; it does not normalize CRLF/bare-CR boundaries or enforce an
event-size cap. The frontend parser explicitly handles all legal SSE line endings at
frontend/src/lib/assistant/sse.ts:12-40, so the two helpers do not have equivalent contracts.

**Consequence for the demo.** A CRLF upstream, a split UTF-8 sequence, or an oversized/incomplete event
can corrupt or retain tool-call argument fragments and turn a valid run into upstream_failed or an
unbounded buffer. The live P3 shape does not prove all transport chunking shapes.

**Concrete minimal fix.** Use a byte-oriented, bounded SSE reader for agent hops that normalizes all legal
line endings, preserves incremental UTF-8, rejects oversized events, and pins tests for split delimiters,
split multibyte characters, CRLF, empty continuation names, usage, and [DONE].

### M9. The existing RunCard cannot render the planned four stages without misleading output

**Claim attacked.** Section 10 says no renderer changes are needed and seeds Understand/Plan/Answer steps,
while promising Understand/Plan/Execute/Final (plan lines 223-234).

**Evidence.** RunCard hardcodes “waiting for approval” for every waiting step at
frontend/src/components/assistant/blocks/run-card.tsx:110-114 and renders active steps with a spinner at
:23-26. The run type only has generic step fields at frontend/src/types/assistant.ts:249-265; it has no
stage-kind or detail field that can distinguish a plan wait from an approval wait.

**Consequence for the demo.** The first frame visibly says “waiting for approval” even though approval
waits are a non-goal, and the initial mapping omits Execute/Final or labels Final as Answer. The required
four-stage story is false in the shipped UI.

**Concrete minimal fix.** Do not seed waiting stages. Append a stage only when it starts, or add a small
agent-specific renderer/step kind that distinguishes stage waiting from approval. Update the mapping and
fixture assertions to require all four labels and no approval text.

### M10. Error/cancel paths can leave active tool steps spinning forever

**Claim attacked.** T6/F2 says every started tool is terminalized and the run ledger remains honest
(plan lines 261-289).

**Evidence.** Active steps always render an animated icon at frontend/src/components/assistant/blocks/run-card.tsx:23-26.
toTerminalBlock can settle a run, but it is called by the existing cancelRun path only at
frontend/src/lib/assistant/direct-transport.ts:834-868; stream error/truncation paths instead call
finishUi at :680-708 without passing the run block through toTerminalBlock.

**Consequence for the demo.** A server deadline_exceeded, network failure, idle timeout, or malformed
terminal frame can produce a failed header with a tool step still spinning. This directly fails section
20's active-to-terminal check and makes a failed tool appear in progress.

**Concrete minimal fix.** Implement one settleRunBlock helper and invoke it on every error, timeout,
truncation, and cancel path; preserve failed tool status and mark only truly unknown active work as
failed/cancelled/outcome_uncertain. Add fixtures for network and client-side timeout, not only a server
error frame.

### M11. The browser idle timeout is reachable during a healthy tool loop

**Claim attacked.** F1 says the existing first-byte/idle timeouts can be reused unchanged (plan lines
275-277), while the backend allows a 180-second run and 60-second tool/hop waits (line 181).

**Evidence.** The frontend applies a 120-second timeout around every reader.read() at
frontend/src/lib/assistant/direct-transport.ts:37-38 and :645-654. A model hop that emits only tool-call
fragments followed by server-side tool execution may produce no agent SSE bytes for a long interval. The
existing direct transport's terminal handling confirms that reader progress, not merely logical
completion, controls the UI at :680-691.

**Consequence for the demo.** The browser can abort a healthy run while the server continues the hop,
showing a network/timeout failure and potentially leaving the server-side tool effect running.

**Concrete minimal fix.** Emit an SSE comment heartbeat (for example, : ping\n\n) at a cadence shorter
than the client idle timeout while the run task is waiting. Ensure the parser ignores comments and add
a long-tool integration test.

### M12. The call budget and terminal protocol are internally inconsistent

**Claim attacked.** Section 9 advertises MAX_LLM_CALLS = 6, calls planning “LLM call 1,” permits a separate
finalizing call, and then states total calls are 1 + MAX_LLM_CALLS; section 7 requires a tool reply for
every call id while section 9 says FINALIZING ignores tool calls (plan lines 161, 181, 189-197).

**Evidence.** The current direct request builder does not send an upstream output cap at
backend/src/services/assistant_direct.rs:250-262, so length/content-filter terminal reasons remain
possible. The existing client treats a stream without a finish reason as truncated at
frontend/src/lib/assistant/direct-transport.ts:680-689. The plan defines no transition for length,
content_filter, or a missing finish reason, and its “every tool_call_id gets a tool reply” rule conflicts
with “tool_calls ignored” in FINALIZING.

**Consequence for the demo.** The advertised limit can be off by one, the run can loop until the watchdog,
or the model can receive fewer tool-role messages than its preceding assistant message requires. A
budget edge can therefore produce a protocol-invalid continuation or an unexplained failure.

**Concrete minimal fix.** Define one total-call counter including planning and finalization; make the last
slot's behavior explicit; synthesize a tool result for every emitted id even when execution is skipped; and
give every terminal finish reason a deterministic DONE/error transition. Pin all combinations in T3 tests.

## MINOR

### m1. Ornn path interpolation is safe only if the implementation uses the declared-parameter builder

**Claim attacked.** Section 8 treats id_or_name as a pinned GET /api/v1/skills/{…}/json parameter and
concludes there is no model-controlled path risk (plan lines 171-173).

**Evidence.** The shared typed builder URL-encodes declared path parameters at
backend/src/services/mcp_service.rs:2638-2649, while generic path validation is a separate function at
backend/src/services/proxy_service.rs:396-402. The plan does not require the Ornn implementation to
represent the placeholder as an endpoint parameter entry or forbid direct string interpolation.

**Consequence for the demo.** A shortcut implementation can let ../, %2F, or double-encoded values
alter the target path, reopening the M11 risk the plan says it removed.

**Concrete minimal fix.** Require a concrete typed endpoint with a named path parameter, pass the value
only through build_proxy_args, and add traversal/double-encoding tests. Never concatenate the Ornn id
into endpoint.path or a URL string.

### m2. Fetched skill fencing is not an enforcement boundary

**Claim attacked.** Section 6.3 says a 32 KiB tool-result fence is sufficient for the M13 prompt-injection
risk, with authorship allowlisting deferred (plan lines 146-147, 175, 320).

**Evidence.** The branch has no agent prompt/tool implementation yet; the existing direct prompt is static
text assembled in backend/src/services/assistant_direct.rs:14-51. Tool results are ordinary model-context
content under the proposed protocol, not a parser-enforced capability boundary.

**Consequence for the demo.** A hostile SKILL.md can instruct the model to disclose data, fabricate
“grounded” claims, or choose a write-capable generic endpoint. Size and provenance labels do not prevent
instruction following. The risk is materially reduced only if the tool registry is read-only and
allowlisted.

**Concrete minimal fix.** Remove generic/write tools from the demo, state that Ornn content is reference
text rather than executable instructions, and add a hostile-skill fixture asserting no secret disclosure,
no undocumented call, and no destructive call. Defer arbitrary Ornn authors until an allowlist/version
policy exists.

### m3. B9 parser coverage does not validate malformed known fields

**Claim attacked.** F1 says unknown frame types are safely ignored and that this avoids the B9 trap
(plan lines 217, 275-281).

**Evidence.** The existing direct transport only knows the OpenAI chunk shape at
frontend/src/lib/assistant/direct-transport.ts:41-48; agent frame validation and stage handling are new
code. The reducer applies known events by a shallow merge at frontend/src/lib/assistant/stream.ts:155-168,
with no runtime validation of a stage enum.

**Consequence for the demo.** A malformed known frame such as {"type":"stage","stage":"executee"} can be
silently ignored or partially applied, leaving a missing visible stage while the transport later reports
success.

**Concrete minimal fix.** Validate every known frame and stage at the parser boundary; convert malformed
frames into a terminal upstream_failed/protocol_error state. Add fixtures for unknown types, malformed
stages, missing ids, duplicate terminal frames, and [DONE] without done/error.

### m4. The file-ownership list cannot deliver the stated acceptance criteria

**Claim attacked.** T1-T8 present a 9-10.5 day implementation with narrowly listed files
(plan lines 238-271, 275-289).

**Evidence.** The required fixes touch APIs whose ownership is outside those lists: approval target
resolution is private to backend/src/handlers/mcp_transport.rs:1424-1466; cancellation lives in
backend/src/downstream_disconnect.rs:42-73 and proxy execution at backend/src/handlers/proxy.rs:1416-1418;
bounded draining requires backend/src/services/mcp_service.rs:3039-3583 and node response code. Billing
coverage additionally requires backend/src/billing_integration_tests.rs:2177-2188.

**Consequence for the demo.** An engineer following the ownership table either cannot implement the
claimed security/cancellation/bounds behavior or silently duplicates shared logic. The estimate and
“exact reuse” claim understate cross-module work.

**Concrete minimal fix.** Expand ownership explicitly for shared approval, proxy/node drain/cancellation,
billing smoke, and FE terminal rendering, or narrow the demo to pre-existing typed read paths that fit
the listed files. Re-estimate after the scope decision.

## Explicitly Remove From This Demo

These items add state or attack surface without helping the first convincing vertical slice:

- Runtime auto-provision/reconciliation. Require a preconnected Ornn service and exact endpoint rows.
- Generic proxy exposure and all write/destructive operations. Start with a small typed read allowlist.
- Dynamic arbitrary skill fetching and the “two skills per run” policy. Keep one deterministic,
  preconfigured, bounded Ornn fetch until endpoint and prompt-injection tests exist.
- Deterministic oldest-exchange trimming as a feature. Use one hard context cap and fail closed until the
  tool result schema is stable.
- The billing-ingress compromise. Keep the existing permit check, but do not redesign classification in
  a way that requires per-hop self-attestation; billing fidelity can remain out of scope.
- Zero-renderer-change reuse of RunCard. A tiny agent-stage renderer is less risky than teaching a
  generic approval ledger about four new semantics.

## Verified Facts vs Live-Only Claims

**Verified on this branch:** human-only rejection layers exist; execute_admin_proxy is a server-chosen
platform-service path; execute_tool performs real credential/identity/node routing when given a valid
context; typed path parameters are URL-encoded; generic proxy paths are traversal-validated; billing
classification is fail-closed; direct request bodies are capped before validation; existing frontend SSE
parsing handles legal line endings.

**Live-only or not established locally:** P1-P6 Chrono compatibility results; the production
chrono-llm-public metric and credential row; the production ornn-api auth/identity configuration; whether
the registry currently publishes the two required typed endpoints; the exact Ornn skill inventory; and
proxy-path forwarding of tools bodies. These must remain Stage-0 go/no-go checks, not completed
dependencies in the verdict.

## Confirmed Invariants

- Reusing execute_admin_proxy is technically appropriate for a fixed, server-selected Chrono service;
  its documented AdminManaged mode is at backend/src/handlers/proxy.rs:1055-1087, and the shipped
  direct handler calls it at backend/src/handlers/assistant_direct.rs:167-176.
- execute_tool is genuine in-process execution, not a mock: credential resolution, identity headers,
  node routing, and billing settlement are present at backend/src/services/mcp_service.rs:3064-3070,
  :3190-3377, and :3481-3583.
- The existing direct limiter holds its permit until response-body drop at
  backend/src/mw/rate_limit.rs:133-219; adding a stream must preserve that lifetime.
- The rate-limit interaction is not an additional hidden per-tool throttle for a session run: the
  global/per-IP middleware checks the outer HTTP request at backend/src/mw/rate_limit.rs:669-706, while
  the proxy's per-agent limiter is reached from backend/src/handlers/proxy.rs:1421-1427 and only acts
  when an API-key identity is present; the proposed human-only route rejects that identity class.
- validate_direct_request's current client grammar is text-only (user|assistant), so client-supplied
  tool-role messages cannot be smuggled through the existing direct request validator.
- BillingEgressPermit is Copy and the classifier fails closed at
  backend/src/services/billing/route_inventory.rs:37-70; the issue is how the new route obtains and
  propagates it, not whether the primitive exists.
- The Docker/embed claim is locally supported for bundled prompts: backend/Dockerfile:45-58 stages the
  backend source/prompts, and CI runs the embed guard at .github/workflows/ci.yml:241-247. New prompt
  files still need to remain under the staged backend tree and be covered by that guard.

## Verdict

**BLOCKED** — 3 BLOCKER, 12 MAJOR, 4 MINOR findings.

The core vertical slice is feasible, but the current plan is not buildable as written because its
authorization boundary, secret-handling promise, and result-size bound are false on the existing APIs.
The following fixes are the priority order:

1. Enforce session-only authentication or preserve OAuth service/node/resource scope (B1).
2. Remove generic/write tools and add response/argument redaction (B2, M2, m2).
3. Add bounded, cancellation-aware downstream draining or restrict the registry to bounded typed reads
   (B3, M6, M7).
4. Reuse the source-aware approval target resolver and add personal/org deny tests (M1).
5. Make Ornn a preconnected, exact-endpoint Stage-0 prerequisite; remove runtime auto-provisioning
   (M3, m1).
6. Repair billing route ownership/classification and add the real route-boundary smoke case (M4, M5).
7. Define one call budget/terminal protocol and settle every FE ledger step on all terminal paths
   (M9, M10, M12, m3); add heartbeats for long hops (M11).
8. Expand file ownership and re-estimate after the narrowed scope (m4).
