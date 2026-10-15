# Part 3 execution interfaces and security proposal

Status: implementation checkpoint, 2026-10-15, through `ee439179c` on
`nyxbot-manager/public`. The final checkpoint below supersedes historical
first-stage and next-wave status statements. The initial investigation used
`7ff461c9236bc87f65f8f89efd278a566d7ebce8` and candidate contract `1b5aea084`;
implementation follows frozen contract `ac78da2a7` and the explicitly approved
narrow ownership additions. Earlier proposed signatures remain investigation
history; actual consumer signatures and remaining proof gaps are recorded at
the end. Source implementation is not deployed runtime evidence.

Public activation remains disabled. Personal routing can proceed after the
shared contract and the local security slice, without NyxAgent public-runtime
proof. No upstream checkout, network probe, inference, account action, build,
test execution, or deployment forms part of this investigation.

The [design](2026-10-15-public-personal-nyxbot.md), the
[final Opus verdict](2026-10-15-public-personal-nyxbot-opus-review.md#convergence-verdict-2026-10-15-final-review-of-the-revised-plan),
and the [allocation](2026-10-15-public-personal-nyxbot-workstreams.md) govern
behavior and ownership. Project instructions require service-layer authority,
dedicated handler DTOs, BSON dates, envelope encryption, secret redaction, and
the existing exact-money ledger. This proposal retains those rules.

## Contract decisions for Part 5

1. Shared service types have no `Serialize` or `Deserialize` implementation.
   Persisted models remain with their owners. Handler DTOs expose a separate,
   bounded projection. Small async producer traits can depend on these types.
2. Public credentials have `ApiKeyPurpose::PublicNyxbot` and one immutable
   public-session ID. Neither scopes nor the `platform` label substitute for
   the purpose. Public keys never enter assistant orchestrator convergence.
3. Public authority comes from a live server-held session and active turn,
   including manager epoch, public-policy revision, sponsor, expiry, and pinned
   runtime contract. Input, headers, tool arguments, and the configured admin
   cannot select the payer or widen that authority.
4. Public discovery and execution use the same three closed FAQ/signup tools.
   Profile/model requests are separately pinned HTTP capabilities. Generic
   account inventory, LLM gateway fallback, owner connections, and nodes are
   denied before their ordinary resolution paths run.
5. Every billable HTTP dispatch reserves its bounded maximum against the total
   daily cap and the applicable sender cap together. Existing usage meters
   settle actual cost once to the sponsor. Cap reservations create no second
   wallet charge. Discovery and readiness share the total public budget.
6. Shared readiness is `Ready(evidence)` or `Unavailable(reason)`. Part 3 keeps
   denied and unverified diagnostics internally. Ready requires pinned,
   expiring cross-service evidence validated against live server state. A
   successful model-list request is insufficient.
7. Personal-origin validation is separate from public policy. It checks only
   the current admitted Telegram-origin turn, including inline continuations.
   Ordinary web turns and committed independent work retain ordinary authority.

## Acknowledgement of the shared contract candidate

Reviewed committed contract `1b5aea084` and the same repository's sibling
integration worktree files
`../NyxBot-manager-integration/docs/plans/2026-10-15-public-personal-nyxbot-contracts.md`
and `../NyxBot-manager-integration/backend/src/services/platform_manager_contract.rs`.
The commit records candidate v1, not a frozen contract. Part 3 acknowledges its
producer names, explicit owner/public context, unavailable/Err distinction, caller-owned
sessions, route names, DTO defaults, and assigned LLM/key files. No sibling file
is changed. The candidate has no runtime implementation or readiness success.

The following concrete refinements need affected-owner agreement before public
execution is enabled:

1. Bind `PublicModelOperation` to exact method/path/protocol, catalog execution
   authority and generation, and per-call bounds. The three existing strings
   may reference Part 3's authoritative proof through `policy_digest`; they
   cannot authorize arbitrary HTTP operations by themselves.
2. Define `PublicExecutionLimits.max_input_tokens/max_output_tokens` as totals
   for the whole loop. Add enforceable per-call, context/response byte, and
   continuation bounds in the pinned proof. Repeated input counts each time.
3. Treat constructible shared contexts/evidence as snapshots. Every consumer
   verifies them against authoritative state; only Part 3's private verified
   handles authorize dispatch. No caller-supplied `Ready` value enables public.
4. Part 2 supplies a key/conversation/current-turn origin callback, shown below,
   or equivalent extra evidence in `ChatAuthority`. The candidate's
   `fence_personal_origin` alone cannot discover which origin an MCP/proxy
   credential is currently executing. Both new boundaries apply with the
   ordinary live-turn flag disabled as well as enabled. The common adapter also
   needs the exact upstream-session lifecycle callback described below.
5. Amend the candidate's atomic public-call transaction description. Existing
   `BillingService::open_required` has no caller-session API. Either Part 3 adds
   a narrow, reviewed billing bridge or uses the durable two-stage admission
   below. No dispatch occurs until both billing and caps are durable.
6. Discovery/readiness needs a separate administrative probe context. There is
   no `PublicIngress` for an admin probe, and public can be disabled while being
   probed. Bind reviewed policy, sponsor, attempt, exact allowed operations and
   total cap without fabricating a sender, ingress receipt or Ready result.
7. Clarify the candidate's prohibition on wrapped tools: no wrapper receives
   independent authority. This proposal permits one compatibility wrapper only
   for the three fixed public tools. If the frozen contract chooses to reject
   all wrappers instead, the closed dispatcher and fixture change together.
8. Add transaction-aware public-session revoke/cleanup entry points, specified
   below, for Part 1 consent and Part 5 account/manager lifecycle transactions.
   A post-commit best-effort notification cannot revoke execution authority.

Internal proof-gap diagnostics project to shared
`Unavailable(UnavailableReason::PublicNotReady)` and the candidate HTTP
`status: "unavailable"`; infrastructure failures remain `Err(AppError)`.
Other known denials use the candidate's relevant unavailable reason. No expanded
public error DTO is required to retain detailed internal proof diagnostics.

### Delta acknowledging Part 2 commit `338d92f5b`

Reviewed Part 2's committed interface proposal. Acknowledge its service-owned
`NyxAgentRuntime` and manager-facing `NyxAgentExecutionProducer` on an already
durably claimed turn. Moving the loop alone does not remove its current
`execute_admin_proxy(&AuthUser, ...)` dependency. Part 3 must provide the typed
`NyxAgentEgress::request(authority, credential, request)` adapter through the
existing proxy/billing core, preserving ordinary actor/audit/payer semantics.
Neither runner fabricates owner/session `AuthUser` or calls an HTTP handler.

Acknowledge the proposed Part 3 `PublicNyxAgentRuntime::{prepare,
replace_credential, settle}` dependencies. Public preparation selects only
public prompt/context and restricted credentials before owner lookups; recovery
uses public replacement, and settlement returns a confirmed durable outcome.
Each response, capability/discovery request, retry and continuation requires
the same live public policy and exact-money admission. Private, non-serialized
`RuntimeCredential` and `NyxAgentRequest` stay inside the execution/egress seam;
credentials zeroize and redact, and requests cannot supply caller URLs/headers.
Cleanup remains a separate capability. Public work never reaches owner
credential convergence/load/replacement, memory/history, model fallback or hooks.

Use Part 2's concrete `fence_platform_execution_in_session(db, session,
user_id, api_key_id, turn, expected_origin)` as the implementation target for
the earlier origin-validator sketch. Obtain its expected origin from loaded
authenticated `ChatAuthority`, never request arguments. Platform-thread keys
require a live non-stopped turn even with the ordinary gate off; a live later
web turn has no platform origin and retains owner authority. The exact callback
joins the caller transaction and Part 1's live binding/control/account fences.

Part 2's requested `services/assistant_nyxagent_egress.rs` is a proposed Part 3
file allocation requiring Part 5 confirmation. Part 2 owns the single common
loop and narrow encrypted-key primitives; Part 3 owns public lifecycle/egress.
The exact upstream-session persistence callback remains required for cleanup.
These acknowledgements add no runtime proof; public readiness stays unavailable.

## Source trace and hook positions

Paths in source references are relative to the repository root. Line numbers
refer to the baseline above. Symbols remain the stable reference after edits.

| Boundary | Available source evidence | Proposed hook and ordering |
| --- | --- | --- |
| REST API-key auth | `backend/src/mw/auth.rs:505` `ensure_api_key_purpose_route`; `:809` `api_key_auth_user`; `:907` `AuthUser::from_request_parts`; `:950` and `:1392` cover bearer API keys and `x-api-key` | Recognize public purpose immediately after verified key lookup, before org/service/node expansion or assistant credential lookup. Enforce a closed route family, then load the live public binding. Both API-key encodings receive identical checks. |
| Key/child validation | `backend/src/services/key_service.rs:1667` `validate_api_key` resolves primary or child secret to its live parent | Preserve parent purpose. Reject public child credentials and malformed public bindings. Never turn a child or derived token into `General`. |
| Personal request fence | `backend/src/mw/auth.rs:151` `ensure_live_assistant_turn`; `backend/src/services/assistant_acknowledgement_service.rs:32` `AssistantTurnFence`, `:62` `enforce_turn_gate`, `:820` `ChatAuthority` | Part 2 exposes the persisted platform origin with the chat authority. Part 3 invokes the live-origin callback before execution and at the dispatch gate. The new origin requirement does not depend on the default-off ordinary live-turn flag. |
| Human/delegated GET auth | `backend/src/mw/auth.rs:620` `delegated_read_denied_path`; `:712` `delegated_request_allowed` | Add `platform-personal-bindings` before mounting its GET; reserve `platform-personal-links` for future GETs. The existing `admin` class already denies runtime GET. Preserve exact `account:read`, GET, and non-WebSocket conditions. Human handlers additionally require verified first-party human auth. |
| MCP authentication | `backend/src/handlers/mcp_transport.rs:615` `McpAuthContext`; `:810` `authenticate_mcp` | MCP authenticates manually. Add verified purpose and public authority here, independently of `AuthUser`. Public keys remain stateless and cannot fall back to a stored MCP session. Load public authority before ordinary org/assistant expansion. |
| MCP protocol | `backend/src/handlers/mcp_transport.rs:1304` `mcp_post`; `:1485` `mcp_get`; `:1573` `mcp_delete` | A public branch accepts only the chosen stateless protocol methods. Reject MCP GET/SSE and DELETE in the initial contract. No notification/session authority is minted. Unsupported resources, prompts, subscriptions, and extensions remain refused. |
| MCP tool list | `backend/src/handlers/mcp_transport.rs:1778` `handle_tools_list` loads the ordinary operation catalog and appends account/machine tools | Return the three fixed public definitions before catalog loading. Do not load the sponsor's services, connected inventory, account tools, memory, or machines. |
| MCP invocation | `backend/src/handlers/mcp_transport.rs:2012` `handle_tools_call`; `:2177` `dispatch_tools_call`; `:2417` `dispatch_meta_tool`; `:2540` `dispatch_service_tool` | Personal-origin validation precedes activity/card/effect handling. Public calls enter a closed dispatcher before `nyxid__`, machine, Oracle, SSH, connection, async-watch, or service dispatch. Unknown names return a fixed refusal. |
| Universal wrapper | `backend/src/handlers/mcp_transport.rs:3563` `handle_meta_call_tool` accepts `arguments_json`, nested `arguments`, or flat arguments, and dispatches account/machine tools directly | Intercept public wrappers before this implementation loads the operation catalog. Normalize exactly one wrapper and dispatch through the same public tool validator. Reject nested wrappers, mixed argument encodings, and unknown fields. A wrapper grants no additional capability. |
| MCP search/discovery | `backend/src/handlers/mcp_transport.rs:3983`, `:4064`, `:4119`, `:4156` search/load/list/discover ordinary services | Public policy denies these generic meta-tools. FAQ search is the dedicated public search tool. No private tool names, instance schemas, or service existence are disclosed. |
| Direct HTTP/WS proxy | `backend/src/handlers/proxy.rs:960` UUID entry, `:1280` slug entry, `:2743` and `:2761` root aliases; execution fences at `:1028` and `:2479` | Apply live personal origin to both paths. Public execution takes a pinned catalog-only branch before connection, pool, credential-override, or node resolution. Reject upgrades and caller-selected destinations/headers. |
| Internal proxy entry | `backend/src/handlers/proxy.rs:2784` `execute_proxy`; `:2838` `execute_admin_proxy`; `:3241` `execute_proxy_inner` | Put the final public policy check in the common inner path as well. `AdminManaged` skips ordinary service allowlists, so an internal call must carry a verified public dispatch permit rather than relying on the outer HTTP route guard. |
| Assistant inference exception | `backend/src/handlers/proxy.rs:3204` `assistant_model_call`; call sites `:3488` and `:3666` | Return false immediately for public purpose, and reject an inconsistent public key/conversation binding. A valid public model permit is evaluated separately. An active `inference` catalog row is never enough for public execution. |
| Payer and meter | `backend/src/handlers/proxy.rs:4158` payer resolution; `backend/src/services/billing/owner_resolver.rs:53` `resolve_for_execution`; `backend/src/services/billing/mod.rs:399`, `:405`, `:582`, `:596` | Keep ordinary resolver semantics. A validated public call supplies the sponsor context through a dedicated service-owned branch before `BillingRouteContext` construction. Prepare meter, reserve caps, commit dispatch admission, mark forwarded, then dispatch and settle through existing meters. |

The ordinary `assistant:live-turn-gate` flag is default-off and its current
enforcement is optional (`assistant_acknowledgement_service.rs:62`). Whenever
the new manager mode is enabled, platform-origin enforcement and public-purpose
containment are mandatory even if that ordinary flag is false. Neither hook may
return early through the flag-dependent ordinary `enforce_turn_gate` function.
New personal-origin checks must load live binding/control state at the effect
gate. A snapshot in an authentication struct cannot establish continued consent
by itself.

## Exact LLM proxy paths and additional entry points

Part 5 explicitly assigns both `backend/src/handlers/llm_gateway.rs` and
`backend/src/services/llm_gateway_service.rs` to Part 3. The following paths
are real routes, not proposed aliases.

| HTTP path | Source and behavior | Public policy |
| --- | --- | --- |
| `/api/v1/llm/{provider_slug}/v1/{*path}` | `routes.rs:51`; `handlers/llm_gateway.rs:340` `llm_proxy_request`. Live-turn check at `:348`, provider/catalog resolution, then UserService and legacy credential resolution around `:430` | Initially deny. A verified runtime that requires this path needs an explicit exact provider/service/operation mapping and a catalog-only public branch before ordinary resolution. Never authorize the family through `llm:proxy` alone. |
| `/api/v1/llm/gateway/v1/{*path}` | `routes.rs:42`; `handlers/llm_gateway.rs:833` `gateway_request`, `:842` `gateway_request_inner`, `:947` `gateway_provider_request`. Resolves provider from model and can fall back from OpenAI to Codex | Deny in the initial public contract. If upstream needs it, implement a public branch before model-prefix routing, provider fallback, pool selection, translation, or user credential resolution. It must resolve the single pinned service/model directly. |
| `/api/v1/llm/status` | `handlers/llm_gateway.rs:184`; `services/llm_gateway_service.rs:112` aggregates accessible personal/org/legacy credentials | Deny. Public readiness is a separate admin service result. |
| `/api/v1/llm/pools` | `handlers/llm_gateway.rs:220` enumerates actor-accessible pools | Deny. Public calls never use pools or fail over to another model/credential. |
| `/api/v1/proxy/s/{slug}/{*path}` and `/api/v1/proxy/{service_id}/{*path}`, including their root aliases | `routes.rs:98` through `:126`; direct provider/model access can use these routes without the LLM router | Permit only an exact approved catalog model operation after public validation. No slug collision, org instance, or arbitrary inference fallback. |
| Internal server-selected `llm-nyx` transport | `handlers/assistant_nyxagent.rs:1295` `proxy` calls `execute_admin_proxy`; paths are `v1/models`, `v1/responses`, and `v1/sessions/{id}` | The common adapter uses a typed public profile operation. The outer `llm-nyx` hop is separate from inner provider hops and must be accounted for independently when priced. |
| `/mcp` direct service tools and `nyx__call_tool` | `handlers/mcp_transport.rs:2540`, `:3563` | General service/model tools are denied. Pinned model execution is a transport capability, not an extra MCP grant. |

Provider-specific billing starts around `llm_gateway.rs:574` and forwarding at
`:720`/`:747`. Gateway billing starts around `:1208` and forwarding at
`:1389`/`:1402`. Both paths translate/normalize some requests. Any future public
support must validate the original body and the final operation/model after
translation, before approvals, decryption, billing dispatch, and forwarding.

Direct proxy pool execution also enters `proxy.rs:1686`
`proxy_request_through_pool` and `:2333` `proxy_request_by_selected_member`.
Public rejection must occur before pool selection, including gateway ingress
that reuses those helpers. Personal pool attempts retain the exact original
platform origin and revalidate before each attempt; failover is not a new
origin-free owner request.

`services/llm_gateway_service.rs:70` and `:85` choose catalog services from
providers. `get_llm_status` explicitly ranks reachable credentials. Those
functions do not constitute a public target resolver. A public resolver accepts
the pinned catalog ID and live platform ACL only.

The initial deny of the LLM router is deliberate contract scope. Upstream may
actually require that router; the repository does not establish its model-call
route. Until that mapping is proven or patched, readiness is unverified.

## Immutable purpose and live public execution context

The current `ApiKeyPurpose` has only `General`, `ScheduledInvocation`, and
`PermissionBound` (`models/api_key.rs:9`). The proposed additive unit variant
keeps the existing enum's simple shape. It does not add a
`PlatformExecutionProfile` variant.

```rust
// Proposed persisted fields, owned by Part 3 in models/api_key.rs.
enum ApiKeyPurpose {
	General,
	ScheduledInvocation,
	PermissionBound,
	PublicNyxbot,
}

// One immutable optional field on ApiKey; absent for all existing key classes.
// Required exactly when purpose == PublicNyxbot.
public_session_id: Option<String>
```

Public provisioning sets sponsor-org `user_id`, `purpose=public_nyxbot`, the
session ID, bounded expiry, `scopes="proxy"`, all three allow-all/auto flags
false, empty service/platform/node grants, and no assistant owner/group or
callback. The `platform` label is descriptive only. No ordinary request DTO
accepts purpose, public-session binding, sponsor, or public grants.

The public session stores its exact key ID, manager/profile IDs, sender proof,
manager epoch, public-policy revision, retention deadline, and encrypted
upstream context binding. A separate active public turn stores receipt ID,
claim/generation, deadline, call counts, runtime binding, and cancellation state.
Exactly one live public turn may use a session key. Public keys are not inserted
into `assistant_agent_credentials` or attached to the sponsor's NyxBot.

```rust
// Shared candidate v1 service snapshot; no serde or credential material.
pub struct PublicExecutionContext {
	pub ingress: PublicIngress,
	pub public_session_id: String,
	pub turn: TurnRef,
	pub sponsor_owner_user_id: String,
	pub policy_digest: String,
	pub nyxagent_profile: String,
	pub model_operations: Vec<PublicModelOperation>,
	pub limits: PublicExecutionLimits,
	pub expires_at: DateTime<Utc>,
}

// Proposed Part 3 internal value, created only after live policy validation.
// Private fields/constructors; no serde or secret-bearing Debug.
pub struct PublicDispatchAuthority {
	context: PublicExecutionContext,
	credential_api_key_id: String,
	runtime: VerifiedPublicRuntime,
}

// Proposed separate internal authority for a restricted administrative probe.
pub struct PublicProbeContext {
	manager: ManagerAuthority,
	public_policy_revision: i64,
	sponsor_owner_user_id: String,
	attempt_id: String,
	candidate_digest: String,
	allowed_operations: Vec<PinnedProbeOperation>,
	maximum_cost: Credits,
	expires_at: DateTime<Utc>,
}

pub struct PublicCleanupContext {
	cleanup_record_id: String,
	public_session_id: String,
	exact_upstream_session_id: String,
	manager_authority_epoch: i64,
}
```

`PublicIngress` carries the immutable receipt/claim, manager identity/epoch,
verified sender/chat and public-policy revision. Part 3
derives context from that receipt and live server policy. NyxAgent callbacks
derive context from the verified key's immutable session and current turn.
Neither path deserializes this struct from a request. Public context creation
must verify the receipt was admitted as public and is still claim-owned. A
probe has no sender/link capability and cannot call `public_signup_link`.
`PinnedProbeOperation` permits capability/profile inspection and explicitly
reviewed bounded validation calls only; it cannot create readiness by assuming
the runtime under inspection already satisfies the contract. Until a trusted
inspection/validation protocol exists, return unavailable without inference.

An authenticated context is revalidated before each model/tool dispatch and
inline continuation. Missing, expired, cancelled, linked-away, or mismatched
session/turn/configuration means denial. It never produces ordinary owner auth.
Metadata Debug may show opaque turn/status identifiers only. Raw sender IDs,
keys, ciphertext, prompts, and upstream context content do not enter logs.

## Runtime readiness, pinned operations, and denial types

```rust
// Shared candidate v1.
pub enum PublicReadiness {
	Ready(PublicReadinessEvidence),
	Unavailable(UnavailableReason),
}

// Proposed Part 3 internal diagnostic/result; not an HTTP or shared DTO.
pub enum PublicRuntimeAssessment {
	Verified(VerifiedPublicRuntime),
	Denied(PublicDeny),
	Unverified { gaps: Vec<PublicProofGap> },
}

pub enum PublicDeny {
	Disabled,
	EmergencyStopped,
	AuthorityExpired,
	AuthorityChanged,
	SponsorUnavailable,
	SponsorConsentLost,
	PlatformGrantDenied,
	ToolDenied,
	ModelDenied,
	OperationDenied,
	CostUnbounded,
	PriceUnavailable,
	BudgetExhausted,
	SenderQuotaExhausted,
	ConcurrencyLimited,
	RuntimeContractChanged,
	CleanupOnly,
}

pub enum PublicProofGap {
	RuntimeBuildUnknown,
	NativeToolsUncontained,
	AdditionalMcpServersUncontained,
	ModelCredentialPropagationUnknown,
	ModelRouteUnknown,
	ExecutionBoundsUnknown,
	SessionIsolationUnknown,
	SessionCleanupUnproven,
}

pub struct VerifiedPublicRuntime {
	build_digest: String,
	contract_version: String,
	profile_id: String,
	profile_digest: String,
	policy_digest: String,
	proof_record_id: String,
	candidate_digest: String,
	checked_at: DateTime<Utc>,
	valid_until: DateTime<Utc>,
	profile_operations: Vec<PinnedProfileOperation>,
	model_operations: Vec<PinnedModelOperation>,
	bounds: PublicExecutionBounds,
}

pub enum PinnedProfileOperation {
	ListProfiles,
	StartOrResumeResponse,
}

pub struct PinnedModelOperation {
	catalog_service_id: String,
	execution_authority_digest: String,
	endpoint_generation: Option<i64>,
	method: Method,
	canonical_path: String,
	protocol: PublicModelProtocol,
	model_id: String,
	max_input_bytes: u32,
	max_input_tokens: u32,
	max_output_tokens: u32,
	max_response_bytes: u32,
}

pub enum PublicModelProtocol {
	OpenAiResponses,
	OpenAiCompletions,
	AnthropicMessages,
}

pub struct PublicExecutionBounds {
	max_wall_seconds: u32,
	max_tool_calls: u32,
	max_model_calls: u32,
	max_continuations: u32,
	max_total_input_tokens: u32,
	max_total_output_tokens: u32,
	max_input_bytes: u32,
	max_context_bytes: u32,
	max_output_bytes: u32,
}
```

The model protocol is a closed set for the adapters actually supported by the
proof. Its requests map to exact catalog-relative operations such as GET
`models`, POST `responses`, POST `chat/completions`, or POST `messages`.
The outer NyxAgent profile maps separately to GET `v1/models` and POST
`v1/responses` on `llm-nyx`. A profile name and a provider model ID are distinct.
Cleanup uses a separate exact DELETE session capability, not this execute enum.

Each operation binds method, canonical path, catalog identity/authority, model,
and enforceable input/output limits. Reject query routing overrides,
`_nyxid_via`, WebSocket upgrades, redirects, caller URLs/auth headers, model
aliases, extra model-selection fields, and unsupported media/tool modes.
Reject malformed or duplicate model fields rather than depending on ambiguous
parser precedence. Validate supported token-limit fields for that protocol.
Live response/model evidence must match the pinned contract.

Unknown runtime builds/profiles or changed capability digests return unverified
or denied. Discovery failure never fabricates `nyxagent/chat` availability.
`handlers/assistant_nyxagent.rs:1272` `models` currently supplies that fallback for the
ordinary owner UI; the public probe cannot reuse its success projection.

```rust
// Shared candidate v1, implemented by Part 3.
pub trait PublicTurnProducer: Send + Sync {
	async fn probe_public_readiness(
		&self,
		db: &Database,
		reviewed: &ManagerRuntimePolicy,
	) -> AppResult<PublicReadiness>;

	async fn prepare_public_context(
		&self,
		db: &Database,
		admitted: &PublicIngress,
	) -> AppResult<PublicExecutionContext>;

	async fn fence_public_execution(
		&self,
		db: &Database,
		session: &mut ClientSession,
		context: &PublicExecutionContext,
	) -> AppResult<FenceOutcome>;

	async fn run_public_turn(
		&self,
		db: &Database,
		context: PublicExecutionContext,
		input: TurnInput,
	) -> AppResult<TurnRef>;

	async fn fence_public_reply(
		&self,
		db: &Database,
		session: &mut ClientSession,
		destination: &ReplyDestination,
	) -> AppResult<FenceOutcome>;
}
```

`ManagerRuntimePolicy` is Part 1's server-resolved reviewed configuration,
including manager epoch, public-policy revision, sponsor and exact-credit caps.
Part 3 resolves the proof and enforced limits for that selection. The probe
runs outside a MongoDB transaction. Its result binds a digest of that candidate
and the exact catalog/runtime evidence. `PublicReadinessEvidence` already binds
manager, public revision, policy digest, sponsor, build/profile, operations,
limits, checked time and expiry; the referenced proof also binds the profile
capability digest and exact operation details. Part 1 rechecks live authority,
proof expiry and revisions before committing enablement. The result is never
accepted from the browser. Infrastructure failures are `Err`, never Ready or
a claim that the sender is unlinked. A public fence returns shared
`FenceOutcome::Authorized | Unavailable(reason)`; a failure cannot fall through
to ordinary execution.

## Early personal security slice

This slice has no dependency on public model discovery or native-tool research.
It lands after Part 5 freezes routes and Parts 1/2 expose origin types. Public
purpose takes a deny-by-default branch until the verified runner exists.

Adopt the candidate's exact route families. Human collection read is GET
`/api/v1/platform-personal-bindings`; owner revocation is POST
`/api/v1/platform-personal-bindings/{id}/revoke`. Review, approve, deny and status
are POST children of `/api/v1/platform-personal-links`. Reserve both entire
first segments in `delegated_read_denied_path`, including future children,
before mounting. There is no preview/challenge/status GET. Admin runtime reads
stay under `/api/v1/admin/platform-bots`, already denied by the `admin` class.

All binding reads and link decisions require a live first-party human session or
first-party access token with no OAuth client identity. API keys, service
accounts, relay credentials, and delegated tokens are refused. A manager admin
or sponsor gains no participant-history access. Reuse the verified
`handlers/login_client_context.rs:12` first-party-human predicate, including its
API-key and acting-client checks. The sole unauthenticated new endpoint is POST
`/api/v1/platform-personal-links/preview`: intent possession permits sanitized
landing metadata only. Its token stays in the POST body; navigation consumes a
fragment token without analytics/logging. Preview grants no linking authority.

```rust
// Part 2 supplies this producer; Part 3 calls it from execution boundaries.
pub trait PersonalExecutionOriginValidator {
	async fn fence_personal_execution(
		&self,
		db: &Database,
		session: &mut ClientSession,
		key_id: &str,
		conversation_id: &str,
		turn_id: &str,
	) -> AppResult<PersonalOriginValidation>;
}

pub enum PersonalOriginValidation {
	OrdinaryTurn,
	PlatformTurn(PlatformTurnOrigin),
	Unavailable(UnavailableReason),
}
```

Part 2 loads the exact current turn by key, conversation, and turn ID inside
the caller's transaction. For a persisted `platform_origin`, it calls Part 1's
`fence_personal_origin` with the same `ClientSession`. The fence requires the
exact binding ID/generation, linked user, manager identity/epoch, active user,
`personal_enabled`, and no emergency stop or retirement. It writes the agreed
binding/control fence fields so revoke/disable transactions conflict.

The producer returns `OrdinaryTurn` only after positively loading a turn with
no platform origin. Missing turns, key drift, unsupported origin versions,
database failures, and expired/stopped turns are unavailable or errors.
Infrastructure failure specifically remains `Err(AppError)`. Authentication cannot
turn an error into an origin-free owner request. Part 2 owns origin propagation
through recovery, credential rotation, and inline continuations.

Part 3 invokes this hook in REST proxy and both LLM execution entries, MCP
authentication/discovery when an origin exists, and again at the common effect
gate before direct or wrapped tool dispatch. Part 2 invokes it for the outer
NyxAgent call, continuation, and owner card admission. No model/Telegram request
runs inside the fence transaction. An effect admitted before a concurrent
revocation has the existing remote cancellation limit; the next admission is
refused. Part 5 fences every send separately.

Checks follow the active turn's origin, never just the thread's
`platform_thread` marker. A later web/event turn after unlink can run without a
Telegram origin. Independently committed specialist/automation/watch work uses
its ordinary live ACL and reports to the web. A linked event's destination is
derived from the live thread/binding generation by Part 2, not a past ingress
receipt. Telegram revocation never creates a broad cancellation cascade.

Ordinary keys and origin-free web traffic do not acquire unrelated platform DB
reads. Part 2 adds origin information to its existing conversation-authority
lookup. Missing new runtime controls mean new manager routing is disabled.

## Small common NyxAgent adapter owned by Part 2

Part 3 does not copy `execute_turn_inner`, create a second assistant engine, or
edit assistant engine/credential files. The reusable seam is below audience
context preparation and above the existing proxy/SSE transport.

```rust
// Shared candidate v1, implementation/extraction owned by Part 2.
pub enum NyxAgentExecutionContext {
	Owner {
		user_id: String,
		turn: TurnRef,
		platform_origin: Option<PlatformTurnOrigin>,
	},
	Public(PublicExecutionContext),
}

pub trait NyxAgentExecutionProducer: Send + Sync {
	async fn execute_nyxagent_turn(
		&self,
		db: &Database,
		context: NyxAgentExecutionContext,
		input: TurnInput,
		events: mpsc::Sender<ExecutionEvent>,
	) -> AppResult<()>;
}
```

The adapter prepares a typed profile operation, bounded input/instructions,
server-held session handle and stable attempt identity from the authoritative
context. `TurnInput` intentionally has no Debug/serde and contains private text;
Part 3 bounds it before execution. Neither shared input nor context carries a
URL, arbitrary headers, mutable payer override, raw public key or assumed
default authority. Private credential leases resolve raw material only inside
the implementation and zeroize it after use.

The adapter reuses SSE decoding, reflected-secret redaction, cancellation,
idempotency/error classification, and transport billing integration. It returns
the observed upstream session/response identity only through a narrow internal
Part 3 session-lifecycle callback, never to a Telegram/user DTO. This callback
is a required adapter refinement: `ExecutionEvent::ReplyReady | Settled` carries
only reply/turn references and is insufficient to persist a cleanup handle.
The event channel is bounded and cannot become a private-body replay queue.
Personal transcript/active-turn settlement remains
Part 2's responsibility. Public session retention and cap bookkeeping remain
Part 3's responsibility. Both runners return reply/events to Part 5.

The owner branch retains current credential convergence, persona, memory,
recap, approval, image, recovery, and continuation behavior. The public branch
requires Part 3's validated operation/dispatch permit, public credential lease,
bounded public context, and cancellation signal. It never calls
`KeyAuthority::Orchestrator`, `load_or_provision_in_session`, owner model fallback,
owner memory/roster/skills, image continuations, or authority-widening cards.
Recovery/replacement stays with the same public policy or fails closed.

Relevant extraction locations are `handlers/assistant_nyxagent.rs:1295`
`proxy`, `:1612` `run_turn`, `:1886` `execute_turn`, `:1943`
`execute_turn_inner`, and `:2265` response dispatch. Body construction is
`services/assistant_nyxagent.rs:4814` `upstream_body`. Credential convergence is
`services/assistant_agent_credential_service.rs:27` `KeyAuthority`, `:50`
`key_fields`, and `:346` `load_or_provision_in_session`. Those files belong to
Part 2, including their existing tests.

## Per-call exact-money admission and sponsor settlement

The existing direct guide provides reusable machinery, not proof of a bounded
agent loop. `platform_inference_service.rs:356` reserves both cap scopes in a
transaction; `:389` release preserves forwarded maximums; `:465` records
dispatch admission. `:648` and `:983` sum exact `UsageMeterRow.reserved_credits`
before dispatch. Its current `scope` function at `:63` separates guide and
readiness. The new mode must combine those total-budget scopes.

```rust
pub struct PublicCallIdentity {
	subject: PublicCallSubject,
	logical_call_id: String,
	attempt: u32,
}

pub enum PublicCallSubject {
	Turn(TurnRef),
	Probe { attempt_id: String },
	Cleanup { cleanup_record_id: String },
}

pub struct PublicCallReservation {
	reservation_id: String,
	call: PublicCallIdentity,
	reserved_day: DateTime<Utc>,
	maximum: Credits,
	billing_request_id: String,
	meter_row_ids: Vec<String>,
	state: PublicCallState,
}

pub enum PublicCallState {
	Prepared,
	Funded,
	DispatchAdmitted,
	Settled,
	ReleasedBeforeDispatch,
}
```

Each chat callback is associated with the server's current public turn; probes
and cleanup use their separate bounded server-held authority. The upstream
contract must provide stable call/attempt correlation for retries; arbitrary
client idempotency strings cannot authorize another turn or replay a settled
call. If safe retry identity or outcome is unknown, stop or retain conservative
spend. Do not blindly retry an uncertain provider effect.

Admission follows this sequence:

1. Revalidate public authority, exact operation/model, live sponsor consent and
   platform grant before credential materialization. Resolve the full pricing
   lane, operation price, and configured components through existing billing.
2. Establish a hard usage bound for every charged metric. Repeated prompt
   context, reasoning output, response bytes, caches, retries, and orchestration
   charges must fit it. An estimate used by ordinary metering is insufficient
   when the runtime can exceed it. Unsupported metrics or missing price evidence
   refuse dispatch. An explicitly resolved zero-price contract is free and
   remains subject to non-money limits.
3. Persist a unique durable `Prepared` call record first, including its stable
   billing request ID. Prepare the normal sponsor `BillingRouteContext` and required meter holds
   without forwarding. Bind its exact frozen meter rows to one call reservation.
   The current `BillingService::open_required` does not expose a caller-session
   transaction, so use a durable preparation record and idempotent recovery,
   rather than pretending the wallet and public caps share a transaction.
4. In one MongoDB transaction, fence live public policy/turn, claim the unique
   call, and reserve `Credits::checked_sum` of all charged layers against both
   daily scopes. The scopes are manager/day and manager/sender/day, independent
   of policy revision, token rotation, and public-session recreation. Budget
   edits cannot reset spent capacity. Use Mongo server time for UTC day/leases.
5. After both meter and caps are durable, commit dispatch admission, revalidate
   prepared catalog/credential authority, mark the same meter forwarded, and
   perform the one dispatch. No provider call runs inside a DB transaction.
6. Persist actual usage through existing deferred settlement and the ledger.
   Mark the cap reservation settled idempotently. It remains consumed at the
   admitted maximum for the original day. Release only a hold proven not
   dispatch-admitted/forwarded. Crash recovery treats uncertain dispatch as spent.

Preparation failure releases unforwarded meter holds. Cap failure dispatches
nothing and durably schedules release of prepared holds. No in-memory `Drop`
callback is the sole recovery mechanism. A claim-fenced sweeper resumes partial
states and never refunds yesterday's reservation against today's budget.
The coordinator must idempotently recover a prepared billing request by its
stable ID, or durably compensate each partial meter reservation. This recovery
API and the transaction bridge are implementation obligations, not proven
capabilities of `open_required`. If neither can guarantee bounded holds and
at-most-once forwarding, public admission remains unavailable. A crash after
`DispatchAdmitted` spends caps conservatively even if forwarding is uncertain.

Discovery/readiness reserve the same manager/day total. They use a separate
bounded administrative attempt quota, not a fabricated Telegram sender. A
readiness probe that executes the agent cannot call signup or other user-effect
tools. Priced cleanup must also have a reviewed funded bound; revocation cannot
silently abandon retention because the public reply budget is exhausted.

`platform_inference_service.rs:212` validates the configured sponsor organization
and wallet-management consent. Its `:575` guide context intentionally sets the
explicit sponsor payer. Reuse that consent principle. The public key owner and
meter payer are always the same validated sponsor-org identity; configured admin,
Telegram sender, and linked person are never fallback payers.

Do not globally change `BillingOwnerResolver::resolve_for_execution`: platform
master credentials continue billing the acting person, and ordinary org BYOK
continues billing the authorized resource owner. Public execution uses a narrow
validated sponsor constructor. A public `NoAuth` outer hop still cannot fall
into ordinary org-key/resource resolution. When both `llm-nyx` orchestration and
an inner provider are priced, each real dispatch gets its own reservation and
usage meters. The ingress receipt and cap counters create no ledger postings.

Money is checked `Credits` in Rust, Decimal128 credits in BSON, and decimal
strings in DTOs. No floats, truncated micros, cap rounding, or second settlement
ledger is introduced. See `docs/BILLING_EXACT_ACCOUNTING.md:13` and `:43`.

## Generic org-key denial and rotation invariants

Org-owned ordinary keys act as the org, including direct org inventory
(`docs/AGENT_ISOLATION.md:227`). `AuthUser` currently expands org services/nodes,
so an empty public service allowlist alone cannot protect account APIs.
The public-purpose route guard precedes all ordinary expansion.

Generic API-key inventory excludes public-managed rows. Per-key metadata,
authorization, usage, operations, credential binding, edit, delete, rotate, and
login-delivery operations return not-found-shaped denial for a public key,
including when the human caller administers the sponsor org. Lifecycle revocation
uses the public service so it can stop the turn and queue exact cleanup.

The exposed handlers read keys directly in places:
`handlers/api_keys.rs:50` and `:78` owner resolvers, `:1176` list, `:1212` detail,
`:1632` update, `:1682` delete, `:1733` rotate, and `:1922`/`:1941` operations.
`handlers/agent_bindings.rs:41` also resolves the parent directly. Filtering only
`key_service::list_api_keys` would leave these paths open.

`services/key_service.rs:1062` already refuses generic rotation for other special
purposes. Add the public purpose to that refusal. The current successor copy at
`:1193` retains purpose, but the assistant adoption path at `:1215` can then
reapply orchestrator authority. Public rotation must never enter that branch.

Runtime-managed replacement uses one transaction to preserve public purpose,
session binding, sponsor, epoch, policy, expiry, and bounded grants; deactivate
the predecessor; store the encrypted successor; and reset upstream context.
Return metadata only. Because the exact key owns the upstream session, queue
cleanup with the predecessor's encrypted material before erasing it. Keep the
predecessor revoked for all execution. Rotation is not an excuse to lose its
cleanup record or import an old session under the successor.

`services/api_key_mutation_service.rs:14` currently protects only lineage,
version, and timestamp fields. The shared contract needs immutable purpose and
public-session field protection for `$set`, `$unset`, and `$rename`, including
dotted paths. Creation and narrowly scoped lifecycle operations get explicit
internal constructors; generic update helpers cannot change these fields.
Low-level revoke remains usable by authorized account deletion/lifecycle code.

`services/api_key_credential_service.rs:48` `issue` currently accepts any
parent, and `:154` `managed_key` checks owner access without a purpose check.
Reject public parents in issuance and managed reads/writes, so `:177` `list`
and `:188` `web_revoke` cannot expose a public row through org membership.
`revoke_children` at `:127` remains a transaction-aware internal lifecycle
primitive; refusing child issuance does not prevent removal of a forged or
legacy child. `key_service::validate_api_key` independently rejects such a
child before returning parent authority.

Agent Key login already requires `purpose == General`
(`services/auth_agent_key_login_service.rs:308`). Preserve and test that deny.
Parent validation must also reject a manually inserted child for a public key.
Relay/delegated/channel credentials must never be minted from a public key;
authentication refuses a derived credential whose live parent is public rather
than assigning it the non-API-key default `General` purpose.

## Denial matrix

| Attempt | Public credential | Personal Telegram-origin owner turn |
| --- | --- | --- |
| `public_faq_search`, `public_faq_read` | Allow only approved versioned corpus, bounded arguments/results, live turn | Ordinary owner's existing tool policy; no public authority inheritance |
| `public_signup_link` | Allow only current verified private sender; no user/payer/URL/invitation arguments; Part 1 creates the intent | Linking/decision services retain their own proof rules |
| Direct or wrapped `nyxid__*`, machine, memory, skill, SSH, Oracle, subagent, channel, connect-link tools | Deny before inventory lookup/card creation/effect | Apply live exact platform origin, then ordinary user ACL, permission mode, and confirmation policy |
| `nyx__call_tool` wrapper | One normalized wrapper around one allowed public tool; reject nested wrappers and alternate private names | Existing normalization and authorization, plus origin check before the final effect |
| Generic search/list/discover, resources, prompts, history, attachments | Deny; fixed public list only | Existing permitted discovery/read rules with origin validation where applicable |
| Pinned catalog model HTTP request | Allow only exact approved operation/model/bounds, live platform ACL, funded dispatch permit | Existing assistant inference exception and ordinary payer rules, plus current origin fence |
| Different model/service, model-less generation, expensive alias, runtime provider credential | Deny or public readiness remains unverified | Existing owner behavior, without sponsor fallback |
| LLM gateway, provider path, pool, org/personal instance, node, WS, redirects | Initial public contract denies; no resolution fallback | Existing authorized behavior, with exact origin at execution |
| Account/org/catalog/key/billing inventory or management | Deny regardless of sponsor/admin ownership | REST human-only gates remain; native tools use existing owner authority |
| Human binding list/revoke and link review/status/decision | Deny API keys, relay, delegated, service-account, and OAuth-client auth | Requires the human route's own first-party authority; conversation keys do not become human |
| Public POST intent preview | Public execution credential has no account API grant; unauthenticated intent possession is sufficient for sanitized metadata | Preview conveys no consent, account inventory or execution authority |
| Generic org-admin edit/delete/rotate/bind/login of public row | Deny or hide row | Ordinary conversation rotation retains Part 2's authority/origin invariants |
| Expired public key or stopped/revoked personal origin | Refuse new execution; exact cleanup only through cleanup service | Refuse the old turn's next tool/model/continuation and Telegram decisions/sends; preserve unrelated web work |
| Runtime/model/price evidence missing | Unverified/denied, no public activation | Personal readiness uses its ordinary engine contract independently |

## HTTP and MCP fixtures for the shared contract

These examples adopt contract `1b5aea084` DTO behavior. Public Ready/success cases
belong only in test fixtures until runtime proof exists. Numeric production
error codes remain owned by `backend/src/errors/mod.rs`; this proposal allocates
no new numeric block.

Adopt candidate GET `/api/v1/admin/platform-bots/{id}/runtime`. It is an
admin-only, non-spending read of persisted source/readiness and independent
controls, without probing or charging. All new HTTP responses use
`Cache-Control: no-store`; bodies reject unknown request fields. This configured
response matches candidate v1:

```json
{
	"contract_version": 1,
	"supported": true,
	"runtime": {
		"manager_authority_epoch": 1,
		"personal_control_revision": 2,
		"public_policy_revision": 4,
		"personal_enabled": true,
		"public_enabled": false,
		"emergency_stopped": false,
		"public_policy": {
			"source": "nyxagent",
			"profile": "nyxagent/chat",
			"sponsor_owner_user_id": "<sponsor-org-user-id>",
			"total_daily_credits": "1",
			"sender_daily_credits": "0.1"
		}
	},
	"public_readiness": {
		"status": "unavailable",
		"reason": "public_not_ready",
		"checked_at": null,
		"expires_at": null
	}
}
```

Candidate POST `/api/v1/admin/platform-bots/{id}/public/profiles` accepts
`{"expected_public_policy_revision":4}`. Its unavailable response is:

```json
{"profiles":[],"default_profile":null,"readiness":{"status":"unavailable","reason":"public_not_ready"}}
```

Candidate POST `/api/v1/admin/platform-bots/{id}/public/readiness` accepts the
reviewed selection and revision fences, without an enable flag:

```json
{"expected_manager_authority_epoch":1,"expected_personal_control_revision":2,"expected_public_policy_revision":4,"public_policy":{"source":"nyxagent","profile":"nyxagent/chat","total_daily_credits":"1","sender_daily_credits":"0.1"}}
```

Part 1 owns both admin handlers and passes server-resolved policy/sponsor to
Part 3. The sponsor is retained from manager setup, never defaulted to the
acting admin. An Advanced sponsor change requires live authorized-org consent.
Repeated clicks cannot avoid sponsor budget/rate admission. Unsupported runtime
evidence returns `public_readiness` with the unavailable fields above, without
changing controls; a stale revision is a conflict. Internal proof gaps remain
available to the bounded policy diagnostic record, not a fabricated Ready DTO.
Enablement is a subsequent PUT `/api/v1/admin/platform-bots/{id}/runtime` with
the same revisions/selection plus `public_enabled: true`. The enabling
transaction revalidates proof and live authority. Missing controls return
`supported: false`, `runtime: null`, reason `mode_disabled`; they never default
to enabled. Public edits preserve personal consent/control revision.

Candidate runtime PUT cannot implicitly clear emergency stop. Only POST
`/api/v1/admin/platform-bots/{id}/runtime/clear-emergency-stop` performs that
explicit audited operation with expected revisions. A failed public readiness
probe does not disable personal routing.

For delegated GET of the candidate binding family, the existing verified
extractor denies before the handler. Retain its actual `ForbiddenWithGuidance`
envelope (`mw/auth.rs:731`, `errors/access_denial.rs:32`):

```http
GET /api/v1/platform-personal-bindings
Authorization: Bearer <delegated-account-read-token>

HTTP/1.1 403 Forbidden
Content-Type: application/json

{"error":"forbidden","error_code":1002,"message":"Forbidden: Delegated tokens cannot access this endpoint","details":{"reason":"credential_type_unsupported","credential_type":"delegated","accepted":["user_session"],"hint":"Ask a human to run `nyxid login` and choose account access, or use an existing human account profile. Do not switch identity automatically."}}
```

Candidate handler-only denial uses fixed `Forbidden: First-party human session
required` as documented by Part 5. Existing auth middleware and
`require_first_party_human` can reject earlier with their fixed guidance/message;
tests must preserve those responses rather than assert a different handler
message was reached. No new numeric code or error variant is needed.

A public tool list has exactly these names and closed input schemas. Neither
generic service tools nor `nyx__call_tool` need to be advertised. Compatibility
wrapper invocation still receives the same public validator.

```json
{
	"jsonrpc": "2.0",
	"id": 1,
	"result": {"tools": [
		{"name":"public_faq_search","description":"Search approved public NyxID help.","inputSchema":{"type":"object","properties":{"query":{"type":"string","minLength":1,"maxLength":256}},"required":["query"],"additionalProperties":false}},
		{"name":"public_faq_read","description":"Read an approved public help article.","inputSchema":{"type":"object","properties":{"article_id":{"type":"string","minLength":1,"maxLength":80}},"required":["article_id"],"additionalProperties":false}},
		{"name":"public_signup_link","description":"Get this private chat's signup or linking URL.","inputSchema":{"type":"object","properties":{},"additionalProperties":false}}
	]}
}
```

Both direct and wrapped private calls return a fixed tool-result refusal, with
no private existence details or approval card:

```json
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"nyx__call_tool","arguments":{"tool_name":"nyxid__list_services","arguments_json":"{}"}}}
```

```json
{"jsonrpc":"2.0","id":2,"result":{"content":[{"type":"text","text":"This tool is unavailable in public chat."}],"isError":true}}
```

For a public model mismatch on an otherwise permitted proxy route, return
HTTP 403 before any provider request or meter forwarding. Use the existing
`forbidden` envelope with a fixed message such as
`"Forbidden: This operation is unavailable in public chat."`. Detailed `PublicDeny`
reasons remain typed internal metadata; they never include upstream error text,
credentials, private tool names, or raw request content.

## Available NyxAgent evidence and required upstream work

The repository records historical upstream source observations in
`docs/chat/08-nyxagent-engine.md:1154`, citing NyxAgent `58f647e4`. This
investigation reads that checked-in evidence only. It does not inspect the
referenced external checkout or assert that this build is deployed.

| Contract | Evidence available in this repo | What remains unproven for public activation |
| --- | --- | --- |
| MCP key propagation | Historical trace cites `src/nyxid.rs:78` key-derived owner HMAC and `:90` `x-api-key` forwarding. Local outer proxy supplies the conversation key in `Authorization` | All native/internal paths use the same restricted authority; additional MCP servers cannot bypass NyxID; derived credentials cannot lose purpose |
| Native tools | Local docs describe Codex developer instructions and NyxID tool filtering. `08-nyxagent-engine.md:1336` explicitly says `store:false` still runs an agent and the request has no `tools` switch | Shell, files, browser, web/network, plugins, skills, and other native abilities can be disabled by trusted runtime policy; a prompt is insufficient |
| Model propagation | Direct proxy permits ordinary assistant inference via `assistant_model_call`. This shows NyxID supports such requests | Every billable model dispatch, including reasoning, compaction, retries, and continuations, actually carries the public key through a named NyxID route; no provider key/runtime fallback bypasses admission |
| Profile identity | GET `/v1/models` and `nyxagent/{name}` are recorded; local default is `nyxagent/chat` | Profile/build digests, effective provider/model operation, pinned ceilings, and immutable runtime capabilities; a label does not establish a price bound |
| Request fields | Historical request is `deny_unknown_fields` with input, model, instructions, stream, store, conversation | The current request does not prove a public purpose/capability/budget contract. Sending invented JSON fields would fail the recorded protocol |
| Cancellation and limits | Historical stream-drop kills an ephemeral runtime. Local Stop drops SSE. Owner budget/time errors may auto-continue | Hard total turn/model/tool/time bounds across every retry, restart, and continuation; cancellation prevents further dispatch and uncertain calls remain charged conservatively |
| Session separation | Historical session owner derives from exact key; local `upstream_body` sets `store:true` | Actual deployed storage isolation and retention, including files/browser/native memories and recycled runtime state, for two public keys/senders |
| Deletion/expiry | Historical DELETE `/v1/sessions/{id}` clears a session by owner. Local human delete at `handlers/assistant_nyxagent.rs:1090` captures credentials and asynchronously attempts deletion after local delete | Authentication for exact cleanup after revocation, durable retry, confirmed deletion/expiry, and no remaining transcript/native state. Current local deletion is best effort and cannot prove the public retention guarantee |

The required upstream contract or dedicated public profile needs the following
properties, with exact names/version negotiated rather than guessed:

- A trusted public-purpose capability contract binds the runtime build, profile
  digest, fixed MCP source/tool policy, and disabled native/external abilities.
  Unknown or incompatible contracts fail before execution. Profile edits cannot
  restore prohibited capabilities.
- Every provider request authenticates with the scoped public key through the
  declared NyxID operation. The runtime refuses direct provider credentials and
  fallback routes. Trusted turn/call/attempt correlation makes each admission
  auditable without logging prompts.
- Hard per-call and full-loop bounds survive resumptions and cancellations.
  Token/context growth and reasoning limits have executable cost bounds, not
  advisory instructions.
- Sessions and any native state are separated by public key and sender. Cleanup
  is either authenticated exact-session deletion under a separate cleanup
  protocol, or enforced upstream expiry with verifiable completion. A revoked
  execution key cannot be assumed to authenticate an ordinary DELETE.

A proposed cleanup-only flow stores the old raw key under envelope encryption
after execution revocation. Only the internal cleanup service can read it and
send exact session DELETE. NyxID permits no MCP/tool/model execution or arbitrary
proxy use from that record. If upstream authenticates deletion by calling an
execution-capable NyxID auth endpoint, upstream must add a separate cleanup
capability or expiry mechanism. Re-enabling the old key even briefly is refused.
Erase the encrypted cleanup material only after confirmed delete/expiry.

No checked-in public proof currently satisfies these requirements. Source/model
discovery may be prepared as disabled code later, but `probe_public_readiness`
must return shared `Unavailable(PublicNotReady)` with internal unverified
diagnostics until external evidence is supplied. The historical
upstream hash is a research lead, not an approved public-runtime build.

## Transaction-aware public lifecycle hooks

Part 1's confirmed link/replacement/broad unlink and Part 5's sponsor-account
disable/delete or manager identity/retirement transactions need this additional
Part 3 producer surface. These are proposed service signatures, not implemented
helpers or handlers:

```rust
pub enum PublicRevocationScope {
	Sender { manager: ManagerAuthority, sender_id: i64 },
	Manager { manager: ManagerAuthority },
	Sponsor { sponsor_owner_user_id: String },
	Session { public_session_id: String },
}

pub enum PublicRevocationReason {
	PersonalLinkConfirmed,
	SenderUnlinked,
	ManagerRetired,
	ManagerIdentityChanged,
	SponsorUnavailable,
	SessionExpired,
}

pub struct PublicRevocationRecorded {
	pub recovery_record_id: String,
}

pub async fn revoke_public_sessions_in_session(
	db: &Database,
	session: &mut ClientSession,
	scope: &PublicRevocationScope,
	reason: PublicRevocationReason,
) -> AppResult<PublicRevocationRecorded>;

pub async fn process_public_cleanup_record(
	db: &Database,
	recovery_record_id: &str,
) -> AppResult<()>;
```

The transactional helper fences all affected execution/reply authority and
records durable exact-session cleanup work before returning. It never starts,
commits or aborts its caller's transaction. Large scopes use a durable scope
revocation marker and bounded cleanup cursor; the marker makes old sessions
unusable immediately, without pretending every key was scanned in one
transaction. All public dispatch/reply fences conflict with the corresponding
scope/control/account mutation and consult that authoritative generation.
They perform real conditional writes, not read-only or no-op fences.

The post-commit worker performs remote effects under a claim, retries metadata
and proven-idempotent exact deletion only, and erases encrypted cleanup material
after confirmed deletion/expiry. The recovery reference is not a deletion
receipt. A failed remote deletion cannot restore execution authority. Scope
selectors derive from verified server state, never tool/HTTP owner arguments.
Part 5 owns worker scheduling/index registration; Part 3 owns its business
logic. Part 1's personal-only app revoke does not acquire a broad unrelated
sender/session cancellation scope. Public-only configuration changes do not
revoke personal consent or stop independent owner work.

## File ownership and registration manifest

The following allocation is concrete for the next stage. Only this document is
edited now. Requested additions need Part 5's ownership acknowledgement before
their first production edit.

| Owner | Exact paths and scope |
| --- | --- |
| Part 3, assigned existing files | `backend/src/mw/auth.rs`, `backend/src/handlers/mcp_transport.rs`, `backend/src/handlers/proxy.rs`, `backend/src/handlers/llm_gateway.rs`, `backend/src/services/llm_gateway_service.rs`, `backend/src/models/api_key.rs`, `backend/src/services/key_service.rs`, `backend/src/services/platform_inference_service.rs`, `backend/src/services/platform_guide_service.rs`, and their existing in-file tests |
| Part 3, additional ownership confirmed in candidate v1 | `backend/src/handlers/api_keys.rs`, `backend/src/services/api_key_mutation_service.rs`, `backend/src/services/api_key_credential_service.rs` for generic purpose confinement; `backend/src/services/llm_usage_service.rs` only if public metering integration requires it |
| Part 3, proposed new files | `backend/src/services/platform_public_policy.rs`, `platform_public_agent_service.rs`, `platform_public_credential_service.rs`, `platform_public_budget_service.rs`; `backend/src/models/platform_public_session.rs`, `platform_public_credential.rs`, `platform_public_turn.rs`, `platform_public_call_reservation.rs`, `platform_public_runtime_proof.rs`; reviewed corpus `backend/content/platform-public-faq.json` |
| Additional ownership requested for key confinement | `backend/src/handlers/agent_bindings.rs`, `backend/src/services/agent_binding_service.rs`. These parent-resolution/binding paths need explicit public-purpose denial even if ordinary owner access succeeds. They are not assigned by candidate v1; Part 5 must assign them before editing. Existing login files need owner-supplied regression tests only unless the frozen contract finds another bypass. |
| Part 3, billing-core scope if required | `backend/src/services/billing/mod.rs`, `backend/src/services/billing/route_context.rs`, and `backend/src/services/billing/reservation.rs` for a narrow public admission bridge. Keep ledger/funding behavior unchanged; any further file is assigned before editing. `owner_resolver.rs` retains ordinary behavior and needs no global sponsor override. |
| Part 2, exclusive | Existing assistant conversation/credential models and assistant engine, credential, team, acknowledgement, continuation, and assistant handler files listed in the workstream allocation. Part 2 also owns the common adapter, personal turn service, and origin producer. Part 3 supplies policy inputs and adversarial cases only. |
| Part 1, exclusive | Manager configuration/runtime control, personal bindings/intents/challenges, and human handlers. Supplies live sender/origin fencing and sponsor/configuration policy to Parts 2/3/5. |
| Part 5, exclusive | `backend/src/services/platform_manager_contract.rs`, dispatcher/delivery/ingress, `routes.rs`, `db.rs`, `main.rs`, module registration, sweeper scheduling, feature registration, account lifecycle entry points, and combined fixtures |
| Part 4, exclusive | All frontend schemas, hooks, UI, and browser fixtures |

Part 3 supplies Part 5 module/index/sweeper entry signatures without editing
registration files. Public session keys and encrypted cleanup records must not
have destructive TTL deletion before required cleanup. A bounded durable worker
revokes live authority first, queues cleanup, erases encrypted material after
confirmation, and then permits metadata expiry. Part 1 linking and Part 5
lifecycle wiring call that service; they do not issue ad hoc public-model writes.

## Focused verification plan

These are proposed tests for implementation owners. None was run during this
documentation stage. Heavyweight Rust checks use the shared host slot later.

| Test group | Required observation | Owner/location |
| --- | --- | --- |
| Human GET denials | Candidate binding GET, reserved link-family GET and admin runtime GET, including trailing/encoded/OriginalUri forms, refuse delegated `account:read`; human routes also refuse OAuth-client, API-key, service-account and relay auth. Public POST preview stays sanitized and unauthenticated. No challenge/status GET is mounted; valid humans and ordinary permitted inventory reads retain their behavior | Part 3 `mw/auth.rs`, Part 1 handlers, Part 5 route fixtures |
| Live personal origin | Run every revoke/replace/disable/emergency-stop race with `assistant:live-turn-gate` both false and true. Between auth and dispatch, refuse direct proxy, both LLM entries/pool attempts, direct/wrapped MCP and inline continuation. Missing callback evidence is unavailable/Err. Ordinary web turns and independent tasks remain usable, without new platform reads | Parts 2/3 transaction fixtures; Part 5 combined races |
| Public purpose | With the ordinary live-turn flag both false and true, bearer/X-API-Key and stateless MCP enforce identical mandatory purpose containment. Malformed/missing binding, inserted child, derived relay token, forged general-purpose update and unknown purpose refuse. No orchestrator convergence or sponsor inventory query occurs | Part 3 auth/key/MCP tests; Part 2 convergence regression |
| Tool allowlist | Exact list contains three names. All wrapper encodings, nested wrappers, `nyxid__`, native machine/Oracle/SSH, search/discovery, resources/prompts, unknown aliases, and signup payer/user/URL/invitation injection fail before effect/card creation | Part 3 policy and MCP tests |
| Model confinement | Test both direct proxy scope branches, internal admin proxy, UUID/slug/root routes, LLM provider/gateway/pool routes, model aliases/duplicates, path/query/headers/URL overrides, WS, redirects, instance shadowing, and node choices. Only exact approved catalog operation/model dispatches | Part 3 proxy/LLM tests with provider request counter |
| Generic org-key lifecycle | Sponsor org admin list/detail/authorization/usage/edit/delete/rotate/operation/binding/login and child-credential list/issue/web-revoke cannot access a public row. Mutation attempts with `$set`, `$unset`, `$rename` source/target, dotted paths and replacement documents cannot alter purpose/session. Runtime replacement preserves purpose and encrypted ownership atomically and exposes no key. Cleanup predecessor never regains execution | Part 3 key/handler tests; binding owner after assignment; Part 2 credential regression |
| Sponsor accounting | Distinct admin, sponsor, sender, linked user, and org resource owner. Every outer/inner/readiness dispatch has sponsor-owned meters; each settles once; no cap/receipt debit. Personal platform inference bills user and ordinary org BYOK remains org-funded | Part 3 billing tests; Part 5 integration |
| Full-loop caps | Parallel callers/replicas, sub-micro exact prices, component/operation prices, explicit free operation, missing/retired price, context growth, retry/continuation, cap reduction, sponsor switch, midnight rollover, and crash at each preparation/dispatch/settlement boundary. Maximums never exceed both caps; refunds require proof of no dispatch | Part 3 transaction-capable Mongo fixtures |
| Readiness/controls | Discovery fallback cannot return available; build/profile/model/ACL/config drift invalidates proof; repeated probe clicks share the total budget. Personal consent/capacity survives public pause, budget exhaustion, and same-identity token rotation | Parts 1/3/5 |
| Retention | Two senders have separate encrypted keys/sessions. Link/expiry/account/manager revocation joins the caller transaction, immediately refuses new effects and persists durable bounded cleanup work. Cleanup retries after process restart; no expired execution before physical deletion; public context never enters personal transcript or memory | Part 3 local tests; Parts 1/5 lifecycle integration; upstream proof required for remote deletion |
| Upstream contract proof | Pinned build disables native/extra-MCP bypass, every model attempt reaches named NyxID admission with scoped key, hard bounds hold, session deletion/expiry completes after revoke, and profile edits fail closed | External evidence gap; public activation blocked |

Local mocks can prove NyxID rejection, reservation races, and lifecycle wiring.
They cannot prove NyxAgent native-tool isolation, provider credential propagation,
or remote deletion. A13 real account/Telegram acceptance remains with Part 5
under separate authorization. Personal implementation does not wait for these
public proof gaps.

## Implementation slice: typed personal transport

`services/assistant_nyxagent_egress.rs::producer(state, identity)` implements
the frozen `NyxAgentEgress` interface for an already admitted personal turn.
It requires `Owner.platform_origin = Some`, validates the exact raw primary
credential and live turn, and invokes Part 2’s transaction-owned
`platform_personal_turn_service::fence_execution_in_session` before dispatch.
Only the known NyxAgent paths are accepted; queries, absolute URLs, path
traversal, body/header overrides, and redirects are unavailable. Public
requests return `503 {"error":"public_not_ready"}` before any owner, key,
credential, target, or billing lookup.

Ordinary owner HTTP execution retains its original proxy wrapper and real
`AuthUser` approval provenance. An HTTP-layer implementation of the same
egress trait can capture that verified caller for the common loop. The typed
personal service does not approximate ordinary nonhuman approval behavior;
`Owner.platform_origin = None` is rejected. Personal outer transport retains
explicit deny policy, configured identity propagation, per-person service
concurrency, ordinary NoAuth outer-hop billing classification, existing exact
usage settlement and metadata-only audit. Inner model/tool calls retain their
own live gates and meters.

`services/proxy_http_response.rs` extracts existing HTTP response streaming,
cancellation, usage capture and deferred settlement unchanged except that
the resale metric is supplied directly. The existing proxy calls this same
core. Its old response/header tests remain in `handlers/proxy.rs`. New pure
operation tests cover the typed transport’s closed operation set. Targeted
Rust formatting and `git diff --check` passed; Cargo checks/tests were not
run because the primary owns the host build slot.

Registration manifest for Part 5: `services/mod.rs` must register
`assistant_nyxagent_egress` and `proxy_http_response`; the adapter also
requires Part 2’s `platform_personal_turn_service` module. No route, model,
index, or background-task registration is added by this slice.

## Implementation checkpoint: public confinement and unavailable runtime

The public key representation is now the immutable unit purpose
`ApiKeyPurpose::PublicNyxbot` plus a separate immutable session-to-key binding
in `platform_public_sessions`. It adds no session fields to `ApiKey` and
requires no other owners to change key literals. The mutation service rejects
purpose changes, dotted authority fields, replacement documents and `$rename`
targets. Generic key, credential and agent-binding APIs hide/refuse this
purpose. Generic issuance cannot create it. Ordinary REST authentication,
stateless MCP, proxy and LLM ingress reject it before owner convergence or
sponsor inventory resolution, independently of `assistant:live-turn-gate`.
The existing ordinary assistant model-call exception excludes this purpose.

The LLM confinement covers exactly these mounted route families:

- `/api/v1/llm/{provider_slug}/v1/{*path}`
- `/api/v1/llm/gateway/v1/{*path}`
- `/api/v1/llm/status`
- `/api/v1/llm/pools`

`platform_public_policy` records metadata-only Unavailable assessments for
the exact selected sponsor/profile/caps/manager/public revision and pinned
source build `fe9745799cc1d46f1290e84effb366ff12600577`. Its candidate digest
is not runtime proof. Repeated discovery returns no profiles or default;
it does not fall back to the bundled chat profile. All readiness, execution
and reply fences return `Unavailable(PublicNotReady)`, including when the
caller supplies a structurally valid Ready evidence value. Context preparation,
turn execution, credential issuance/replacement, observed binding adoption and
reply settlement return an error. None calls owner credential convergence,
opens a meter, performs a provider request, or manufactures a saved reply.

`platform_public_agent_service::revoke_in_session` implements local lifecycle
work independently of public readiness and joins its caller's transaction.
For each matching session it validates the exact sponsor/public-purpose key,
deactivates that key through the shared mutation service, revokes child
credentials, requests the exact active public turn stop, clears encrypted
credential material, and records durable exact-session cleanup work. An empty
scope returns `recovery_record_id: None`. The separate session model retains
encrypted upstream handles, exact ingress receipt/claim/turn linkage, the
absolute expiry and revocation metadata. There is no destructive TTL index.

This bounded slice permits at most 256 matching live sessions in a single
caller transaction. A larger scope aborts the whole mutation before any
cleanup intent or key update; it never returns partial success. The larger
scope generation marker and paged drain described above remain unimplemented.
This limitation must be removed before public activation or any issuer is
enabled. Manager identity matching includes the exact profile/bot/Telegram
identity and only epochs at or before the revoked epoch, preserving a future
replacement manager's authority.

The cleanup worker uses durable claims and retries. It records
`upstream_cleanup_contract_unproven` when an upstream session may exist, keeps
the encrypted upstream binding, and leaves cleanup incomplete. It never
restores a revoked key, calls an unproven deletion protocol or reports expiry
as verified erasure. Only a session durably known never to have dispatched,
with no upstream binding and no active key, can complete local cleanup. Missing
legacy upstream evidence defaults conservatively to `upstream_may_exist: true`.
This is local recovery support, not proof of upstream session deletion.

The personal egress transaction uses MongoDB's explicit `and_run` context and
boxed callback to avoid the driver's documented reborrowed async-closure Send
issue. Its context owns authority metadata and an identity-producer reference;
raw key material is not captured. Known failures after opening a meter and
before dispatch use the existing billing failure/release path. Ambiguous
forwarded attempts retain ordinary durable reconciliation behavior.

Registration manifest for Part 5, without new routes:

- `models::platform_public_session` (sessions, cleanup records, readiness records).
- `services::{platform_public_policy, platform_public_agent_service,
  platform_public_credential_service}`.
- `platform_public_agent_service::ensure_indexes(&Database)` at index setup.
- `platform_public_agent_service::producer(state, execution, ingress)` supplies
  the frozen `PublicTurnProducer`.
- `platform_public_credential_service::producer(state)` supplies the frozen
  `PublicNyxAgentRuntime`.
- `platform_public_agent_service::sweep_public_lifecycle(&AppState, limit)`
  returns `RecoveryBatch`; each pass revokes at most 100 expired sessions and
  processes at most 100 due cleanup records, with at most 256 sessions in each
  cleanup record. Part 5 owns scheduling and lifecycle entry-point wiring.

Added focused tests cover forged readiness/discovery refusal, idempotent local
assessment, transactional rollback of revocation, bounded-scope rejection,
unknown upstream cleanup retention, completion of proven never-dispatched
sessions, retry timing, BSON date/binary storage and redacted Debug output.
Existing confinement tests cover ordinary auth/MCP route denial, LLM guard,
operation allowlist and protected key mutations. Targeted rustfmt and
`git diff --check` passed. No Cargo/build/test command ran on this worker; the
primary owns central compile and transaction-test execution.

Per the primary's worker rotation, exact-money cap admission, sponsor-meter
binding/settlement, FAQ/signup tool execution, scope revocation markers and
runtime acceptance remain next-wave work. There is no public budget producer
or public tool runtime in this slice. Source-reviewed native-tool flags and
NyxID responses/compaction propagation still do not prove deployed build,
immutable profile/purpose binding, hard model/token/whole-loop cost bounds,
disabled provider web search, strict request-field denial, absolute upstream
retention, revoked-key cleanup, or enforcement on every replica. Public
activation remains blocked; personal execution does not wait for those gaps.

## Resumed implementation: paged scope fences and exact sponsor bridge

The previous 256-session scope limitation is superseded by `1ff2fc756` and
its follow-up. Revocation writes a persistent DB-time scope cutoff in the
caller's transaction, drains at most 256 keys/turns into exact cleanup records,
and leaves the remaining pages pending. Admission writes the same manager,
sender, sponsor and session scope documents; undrained old sessions fail the
local fence immediately. The recovery worker drains pages transactionally,
without restoring keys or claiming remote deletion. Scope digests are unique
lookup keys; persisted model IDs remain UUID v4. New scope/cleanup indexes are
owned by the existing public lifecycle `ensure_indexes` entry point.

`models::platform_public_budget` and `services::platform_public_budget_service`
add exact whole-loop holds and per-dispatch reservations. A loop reserves its
maximum in both total and sender UTC-day buckets in one caller transaction.
Bucket identity is profile/day and profile/sender/day; policy revisions,
sponsor switches and manager epochs do not reset spent capacity. MongoDB
`$$NOW` sets the day and bounds the loop deadline to that day's end. Money is
checked `Credits` and stored as Decimal128; no micro truncation or cap ledger
posting exists. Each prepared call consumes loop call/token limits and an
allocation within the already reserved whole-loop maximum.

The durable call states are `Prepared -> Funded -> DispatchAdmitted`, with
`Cancelling/Released`, `Conservative` and `Settled` recovery states. A stable
billing request ID, exact sponsor/key, pinned model, complete billing route
digest and frozen meter row IDs bind each call. The existing pool funding
transaction was generalized behind its unchanged wrapper. Public admission
uses a separate typed deadline, commits allowance/grant/wallet holds and all
meter rows atomically with `Prepared -> Funded`, and retains existing funding
precedence and charge identities. No fabricated pool identity or second wallet
debit is introduced. The public branch requires enabled, chargeable billing;
a disabled rollout cannot silently turn sponsor work into metering-only work.

Final dispatch admission fences the exact session/turn/receipt/claim, live
controls/sponsor/key and revocation scopes, and atomically marks every frozen
meter row forwarded with the once-only call transition. A repeated call ID
cannot dispatch again. Expired unadmitted calls first lose dispatch authority,
then release only existing rows proven unforwarded. Ambiguous admitted calls
consume their full bound conservatively; only settled sponsor meter gross
cost can refund the exact difference against the original buckets. Existing
billing owns all real settlements and ledger postings, including allowance
and grant funding. Unused loop holds release only after every call is terminal
or conservatively accounted. Recovery uses bounded scans and per-call retry
times so blocked old evidence does not monopolize one page.

Production access to quoting and loop creation requires an opaque
`VerifiedPublicContract`. Its fields are private, and the production verifier
currently returns unavailable for every supplied evidence value: the pinned
upstream has no accepted deployed public contract. Only transaction fixtures
can construct a local test capability. This support does not create Ready
state, issue a production key, or make live model calls. Remaining public
engine/model dispatch wiring must consume these helpers after actual proof.

Written tests cover two competing senders against a sub-micro total cap,
caller rollback of both buckets, idempotent preparation, dispatch rollback and
once-only admission, conservative crash accounting and exact late refunds.
The real funding test forces meter insertion failure after benefit/wallet
mutations and verifies rollback of every hold and `Funded`, then verifies
successful once-only funding with unchanged allowance/grant/wallet precedence.
Existing pool admission tests exercise the unchanged ordinary wrapper. No
Cargo or runtime test ran on this worker; targeted rustfmt and whitespace
checks passed.

Additional registration manifest: register `models::platform_public_budget`
and `services::platform_public_budget_service`; call its
`ensure_indexes(&Database)` at setup and bounded
`sweep(&Database, limit) -> AppResult<RecoveryBatch>` from the recovery runner.
No route or shared producer signature changes are required. Closed tools,
immutable public issuance and final common-loop wiring remain the following
slices; deployed capability/retention proof remains an external blocker.


## Public consumer checkpoint: exact callbacks and pre-dispatch evidence

The authorized implementation stage adds these reviewable commits after the
budget work: `042bbb1bc` (restricted issuance, disposable encrypted turns and
settled reader), `63a5f3823` (ephemeral signup with status-only model output),
`64ddaa4d5` (preparation/expired-turn recovery), `593db0e99` (current bot
observation for settled delivery and MCP enum boxing), `85f07ae4c` (public
execution plus sponsored callbacks), `e61d7af44` (quote/catalog pin and admission
fixture correction), and `c32ef6cb1` (scheduled cleanup and absolute reply
expiry). Primary integrates module/route/index/worker registrations and the
small paired shared-contract cleanup. Ordinary owner execution, its real
incoming AuthUser approval adapter and OwnerTurnSettledHook are unchanged by
the public dispatch bridge.

Actual public execution enters
`services/assistant_nyxagent_runtime.rs::ExecutionProducer`'s Public arm and
calls `platform_public_execution::execute` (`:15`). It uses the existing
Responses decoder and the frozen runtime/egress traits; it does not converge
through owner credentials or read owner history. Preparation encrypts bounded
input and reserves the complete loop before network activity. A saved reply
and exact ingress settlement commit together before ReplyReady is emitted.
The public turn collection has an absolute session expiry of at most 24 hours;
non-TTL session, meter and cleanup records retain authority and crash evidence.
Idle session reuse and credential replacement preserve that absolute expiry.

`platform_public_proxy_service::dispatch_nyxagent` (`:629`) permits one exact
POST `v1/responses` for the prepared turn. The current outer catalog contract
is NoAuth. This adapter accepts it only when its billing configuration is
absent; a priced outer service is refused. Preparation pins the server-selected
outer service ID and full catalog digest. Final admission rechecks it and
commits both `outer_dispatch_admitted=true` and the non-TTL session's
`upstream_may_exist=true` before the outbound effect (`:603`). A lost response
cannot authorize a replay. Ambiguous failed execution shortens and revokes the
session and leaves cleanup pending. Expired-turn recovery also revokes any
potential upstream session even when no handle returned. No cleanup path
restores the revoked bearer or reports unsupported remote deletion complete.

The dedicated callback handler is
`handlers/platform_public_model.rs::dispatch` (`:22`). It accepts only a
public-purpose primary key and derives the active authority from storage. It
requires the exact session, turn, receipt and claim headers; reused credentials
do not let old callbacks attach to a new turn. A UUID v4 call ID is the once-only
billing/dispatch identity. Ordinary AuthUser middleware must not wrap this
handler. The dedicated public authentication and final transactional fence
replace it. Query, upgrade, child credentials, arbitrary payer/destination
selection and caller header forwarding are refused.

A symbolic callback fixture, requiring actual verified runtime support, is:

```http
POST /api/v1/public-nyxbot/models/MODEL_SERVICE_ID/v1/responses
Authorization: Bearer PUBLIC_SESSION_KEY_REDACTED
Content-Type: application/json
x-nyx-public-session-id: EXACT_SESSION_UUID
x-nyx-public-turn-id: EXACT_TURN_UUID
x-nyx-public-receipt-id: EXACT_RECEIPT_UUID
x-nyx-public-claim-id: EXACT_CLAIM_UUID
x-nyx-public-call-id: UNIQUE_CALL_UUID_V4

{"model":"PINNED_MODEL","input":"public question","max_output_tokens":256,"stream":false}
```

The body model and explicit positive output bound must match the verified
operation. Input bytes, conservative input-token admission, response bytes and
absolute turn duration are bounded. Closed schemas accept text and fixed
FAQ/signup function call/result round trips for OpenAI Responses, OpenAI Chat
Completions and Anthropic Messages. Unknown fields, provider web search,
computer/shell/native tools, images/audio, remote content references and custom
tool declarations are denied. Only the server's exact three tool declarations
are admitted. Public MCP uses the same four authority headers and checks the
committed outer attempt before spending a tool call; wrapper normalization
continues to resolve exclusively to those three public tools.

Every paid callback uses `dispatch_model` (`:382`), not an ordinary LLM wrapper.
It validates the pinned catalog projection before quote and after awaited
funding, then fences the published endpoint generation, exact model, live
public context, callback turn and sponsor. Whole-loop total/sender cap holds
and per-call Prepared/Funded/DispatchAdmitted transitions remain the sole
public admission system. Funded uses existing sponsor allowance/grant/wallet
holds and real usage-meter IDs. Final admission marks those meters forwarded
in the same transaction as dispatch permission. Known no-dispatch failures
cancel only Prepared/Funded; ambiguous admitted work retains its reservation.
Billing posts actual cost, including overrun, before cap reconciliation. There
is no second wallet debit or public cap ledger.

| Path | Public behavior in this checkpoint |
| --- | --- |
| `/api/v1/public-nyxbot/models/{service_id}/v1/responses` | Exact pinned OpenAI Responses callback through public sponsor admission |
| `/api/v1/public-nyxbot/models/{service_id}/v1/chat/completions` | Exact pinned OpenAI Chat Completions callback through the same admission |
| `/api/v1/public-nyxbot/models/{service_id}/v1/messages` | Exact pinned Anthropic Messages callback through the same admission |
| Any dedicated callback ending in `v1/responses/compact` | Denied: current compaction contract has no accepted explicit output bound |
| `/api/v1/llm/{provider_slug}/v1/{*path}` | Denied before ordinary provider/credential resolution |
| `/api/v1/llm/gateway/v1/{*path}` | Denied before fallback, translation, pool or owner resolution |
| `/api/v1/llm/status`, `/api/v1/llm/pools`, ordinary direct proxy | Denied; no ordinary inventory is exposed |

The required upstream `public_execution` wire extension is built at
`platform_public_execution.rs:153`. It binds purpose, profile/model selection,
policy, exact session/turn/receipt/claim, callback contract and paths, deadline,
and hard call/tool/continuation/token/byte bounds. **Pinned NyxAgent source
`fe9745799cc1d46f1290e84effb366ff12600577` does not establish acceptance or
enforcement of this extension.** It also does not prove callback headers or
that every model call, retry and compaction uses the dedicated admission route.
Public compaction must be disabled under a verified profile or receive an
explicit bounded supported upstream contract. No authorization for upstream
repository edits has arrived, and no upstream edit or checkout was performed.

`verify_contract` still refuses all production evidence. Source flags for
native tools and NyxID responses/compact credential propagation are useful
source evidence, but deployment/build/effective-profile binding, hard upstream
limits, no fallback/provider web search, every-replica purpose enforcement,
absolute remote expiry and cleanup using revoked credentials remain unresolved.
No Ready value, successful capability probe, live bill or deployed acceptance
is claimed. Personal execution remains independent of this boundary.

`public_reply_expiry(db, context)` (`platform_public_credential_service.rs:763`)
returns the exact session's absolute delivery deadline separately from the
execution lease. The settled fence (`:783`) uses the current profile observation
while retaining the original numeric bot, manager epoch, sender, session and
claim. Same-bot token rotation therefore does not bind a queued reply to the
old transport token. The primary owns the tiny transport expiry call-site
pairing. The obsolete cleanup producer forwarding method and returned recovery
record ID are removed as a paired shared-contract cleanup; actual lifecycle
sweeps, paged drains and durable cleanup records remain authoritative.

Written regressions cover three protocol schemas and fixed function round
trips, native/multimodal/unknown-field denial, exact callback headers after key
reuse, outer admission rollback and once-only ambiguous evidence, Prepared
cancellation without a meter or duplicate allocation release, and refusal to
cancel an already admitted call. Existing transaction sources cover competing
sender/day caps, funding rollback, conservative/late/overrun cost correction,
mixed recovery batches, scope migration/drain, preparation failure before a
turn row exists and active-lease expiry. Targeted rustfmt and `git diff --check`
passed. No Cargo/build/runtime test ran on this worker. Central CI, backend
transaction execution and the independent bridge review remain primary-owned;
this checkpoint does not claim they passed.

## Final bounded settlement correction (`ee439179c`)

The existing failed-turn settlement already revoked sessions with ambiguous
upstream state. The final correction binds that action to the transaction's
successful compare-clear of its own active conversation/turn/receipt/claim,
retained credential ID, sponsor and policy. `settle_reply_in_session`
(`platform_public_credential_service.rs:724`) returns whether that exact
transition occurred. Only that transition may call
`expire_ambiguous_session_in_session` (`:623`). A duplicate old settlement can
still observe its saved reply and terminal receipt, but cannot revoke an idle
successor, clear a newer active turn or revoke a replacement key.

The transaction regression at `:1459` commits the actual outer-attempt marker,
then calls the real runtime settlement producer with an error and no upstream
handle. It checks saved encrypted safe text, terminal ingress, key revocation,
pending cleanup, unchanged admitted call/cap evidence, and fresh-session
admission for the same sender under the live-session unique index. It also
aborts the settlement/revocation transaction to check rollback. The regression
at `:1678` covers idle/active successors and retained-key mismatch. These local
fixtures exercise durable primitives without constructing public readiness.

`validate_reported_usage` (`platform_public_proxy_service.rs:559`) now rejects
inconsistent normalized token totals and bounds input/output/total usage after
truthful sponsor settlement and cap reconciliation. Separate Anthropic cache
classes count toward the input bound; its shared extractor's synthesized total
remains uncached prompt plus completion. The focused test at `:913` includes
the actual Anthropic usage extractor, total inconsistency and output overrun.
Targeted rustfmt and `git diff --check` passed. These new tests have not been
executed by this worker. The primary reported a production Cargo check pass
before this correction; that is not validation of this new commit. No module
registration or shared-contract change is required by this correction.

Public readiness still refuses activation. The upstream wire/profile,
callback/compaction confinement, deployed hard bounds and exact cleanup proof
gaps described above remain unresolved; this settlement correction supplies no
new upstream evidence. Part 3 releases its implementation slot at this
checkpoint; central integration, registration and CI remain with the primary.
