# Part 2 interface decision: personal turns and the shared NyxAgent engine

Status: producer review and implementation proposal, 2026-10-15. Baseline
`7ff461c9236bc87f65f8f89efd278a566d7ebce8`, branch
`nyxbot-manager/personal`. Reviewed Part 5's candidate contract reported committed
as `1b5aea084`, including its Rust interface module and contract document, and
Part 1's sibling interface proposal. This document does not freeze either
producer's contract. Only this proposal is changed in this stage.

Authority: the [implementation plan](2026-10-15-public-personal-nyxbot.md),
[final Opus verdict](2026-10-15-public-personal-nyxbot-opus-review.md#convergence-verdict-2026-10-15-final-review-of-the-revised-plan),
and [workstream allocation](2026-10-15-public-personal-nyxbot-workstreams.md).
The current access-mode rules in [the engine contract](../chat/08-nyxagent-engine.md)
take precedence over older Full-only comments in code.

## Decisions PM can freeze

| Candidate boundary | Part 2 decision / required correction |
| --- | --- |
| Separate personal/public admission, server-derived payer, metadata-only replies | Acknowledge. Personal uses ordinary owner authority and billing; public never enters owner credential convergence. Neither runner sends Telegram. |
| `PlatformTurnOrigin.nyxbot_agent_id: String` at sender resolution | Correct. Admit binding authority without a required agent ID; resolve the actual NyxBot before forming a complete turn origin. Include resolved agent ID in thread claim. No invented placeholder ID. |
| Session-taking identity fences and thread claim | Acknowledge transaction ownership; add exact turn-pointer claim/clear and a clear operation that works after revocation. Claim returns both authoritative agent and conversation IDs. |
| `stop_exact_platform_turn -> StopRecorded { cancellation_id }` | Correct ownership mismatch with Part 1: Part 2 captures Stop/effects in the caller's transaction; Part 1 inserts its durable stop-effect record before committing. The capture result cannot claim a recovery record already exists. |
| Owner/public execution enum | Acknowledge intent; enum tags alone do not enforce it. Public preparation, credential replacement, request dispatch and settlement require their own Part 3 implementation. Move the existing loop into services once. |
| `execute_nyxagent_turn(db, context, text, events)` | Keep as the manager-facing adapter, with explicit dependencies below. It operates on a durably claimed turn, never claims it again. Ordinary HTTP turns keep their richer `TurnStart` input. |
| Start/settlement event handoff | Correct. `start_personal_turn` must accept a bounded event sender, as the execution adapter does; otherwise Part 5 cannot subscribe before an immediate completion. Durable reply linkage backs lost notifications. |
| Receipt and effect fencing | Add Part 5 transaction-aware receipt claim/bind/settle methods. A claim pair without those operations is insufficient to atomically prevent duplicate start/decision work. |
| Event destinations | Add an origin source distinguishing ingress from a fresh thread-event admission. A later event is not an old receipt replay. Preserve a delayed result's original destination generation. |
| Conversation/turn DTO additions | Acknowledge the nullable candidate fragments. Also retain origin on the settled reply; this needs a small additional model allocation. |
| Live execution checks | Platform-thread execution is gated even with `assistant:live-turn-gate=false`. Apply the platform-origin check before that flag's existing early return. |

`TurnRef` remains Part 5's `{ conversation_id, turn_id }`; owner identity comes
from the checked origin or explicit owner argument. Part 1 should not independently
add an owner field under the same type name. Service snapshots never become model
imports or HTTP authority inputs. Infrastructure errors abort; they are not
`Unlinked`, a successful cancellation, or a public retry.

## Actual paths and extraction boundaries

Paths and line anchors below refer to `backend/src/` at the baseline.

| Path | Current behavior | Implementation slice |
| --- | --- | --- |
| `handlers/assistant_nyxagent.rs:1377`, `:1424`, `:1437` | `turns` parses/authorizes, acquires the owner permit and reads billing policy. `start_turn_with_voice` calls `begin_turn`, applies async delivery, loads the credential, subscribes before spawn, and owns spawn. | Keep request parsing, real `AuthUser` extraction and SSE in the handler. Move claim orchestration and detached execution into services, accepting an owner ID and verified server context. |
| `services/assistant_nyxagent.rs:1725`, `:1734` | `begin_turn_with_voice` ensures NyxBot before its transaction; inside it resolves agent authority, reclaims expired turns, provisions/converges the key, drains events, binds uploads, writes input and `ActiveTurn`, and may adopt home. | Extract the transaction body as an in-session primitive. Ordinary wrapper preserves its transaction/audit behavior; the personal wrapper adds receipt/binding fences and thread claim in that same transaction. |
| `handlers/assistant_nyxagent.rs:1612`, `:1763` | Detached `run_turn` owns heartbeat/permit/Stop polling. `complete_turn` retries settlement within five minutes; deleted conversations are quiet, forced outcomes are recognized, terminal stream events follow persistence. | Move this code, preserving bounded retries, exact-turn fences, partial text, redaction and permit release. Publish a manager reply reference only when persistence is confirmed. |
| `handlers/assistant_nyxagent.rs:1943`, `:2399`, `:2652` | Execution prepares private memory/history/checkpoints and attachments, calls the metered proxy, decodes SSE, rebinds/replaces credentials, and continues budget/time/image rounds. | Preserve owner preparation. Select public preparation before any owner history/agent/key lookup; public recovery calls Part 3, not `credentials::replace`. Revalidate origin before every egress/retry/inline continuation. |
| `services/assistant_nyxagent.rs:2650`, `:2897`, `:2933` | Settlement writes the reply/checkpoint and clears active state transactionally; force-stop/orphan recovery uses the same path. | Copy persisted origin onto the reply, clear only the matching binding turn pointer, and settle receipt metadata in the same transaction. A revocation cannot prevent saving the owner's late reply. |
| `services/assistant_agent_credential_service.rs:27`, `:79`, `:110`, `:346`, `:473`, `:572`, `:618` | Ordinary keys regain orchestrator/specialist authority at each start; replacement reloads conversation authority; load verifies encrypted raw-key hash. Rotation adopts the encrypted successor and clears sessions. | Keep personal parity. Add explicit ordinary-purpose guards to convergence/load/replacement/adoption. Share only envelope/raw-key primitives with Part 3's separate public credential store and lifecycle. |
| `services/assistant_team_service.rs:532`, `:637`; `services/assistant_nyxagent.rs:901`, `:2275` | Thread creation, existing-home validation, newest-thread fallback, startup repair and begin-turn adoption know channel/automation exclusions. | All must exclude `platform_thread`, including ordinary web turns later run on that thread. Platform creation has no learning enrollment/home side effect. |
| `handlers/nyxbot.rs:2724` | `owner_thread` accepts an existing home using only group/channel checks. | Needs the narrowly requested additional exclusion below before personal activation. |
| `handlers/assistant_nyxagent.rs:1068`; `services/assistant_nyxagent.rs:4059` | Delete first stops/cancels, then a service transaction revokes the current/rotated key and removes transcript, credentials, acknowledgements, attachments and watches. | Add Part 1 compare-and-clear to the service deletion transaction. Deletion remains stronger than unlink; preserve voice guards and existing upstream deletion behavior. |
| `handlers/assistant_team.rs:198`, `:362`, `:3218`; `services/assistant_continuation.rs:54`, `:101` | Event wake-ups use team limits; settlement queues/wakes work and delivers ordinary channels. Inline progress writes already fence turn and continuation. | Move reusable wake/notes/settlement orchestration into a service; retain HTTP/tool facades. Add fresh platform event admission and reference notification, with no sticky web destination. |
| `services/async_service_operation.rs:157`, `:362`, `:427`; `models/async_service_operation.rs:24` | Watch registration saves its originating turn and channel/group delivery; binding applies that saved delivery when the result resumes. There is no platform destination field yet. | Preserve a platform result's original delivery proof as metadata, separately from the fresh event turn's execution origin. Do not recover a delayed result's destination from the newest binding. |
| `handlers/nyxbot.rs:2675`; `services/assistant_acknowledgement_service.rs:1982`, `:2011`, `:2251` | Word decisions use the previous same-thread user message, parse yes/no/code, select a pending action and open their own decision transaction. Four-digit code selection currently uses first match. | Extract the shared decision service, reject code collisions, and provide an in-session decision body so live platform authority and decision commit together. |
| `services/assistant_nyxagent.rs:2516`; `handlers/machine_cancel.rs:20`; worker `run_turn` cancelled branch | Ordinary Stop cancels conversation async watches; machine cancellation selects all running jobs of the conversation, including older work. Worker cancellation invokes that broad helper too. | Transport unlink uses none of these broad paths. Branch the worker's platform cancellation into the durable exact runner; ordinary explicit app Stop/delete retain existing behavior. |

`proxy` at `handlers/assistant_nyxagent.rs:1295` currently rebuilds a request and
calls `handlers/proxy.rs::execute_admin_proxy(state, &AuthUser, ...)`. This is a
real dependency, not resolved by relocating `run_turn`. Image capability probes,
session cleanup and model listing use it too. The engine service must depend on
the typed egress adapter supplied by Part 3 below; it must not fabricate
`AuthUser`, call `turns`, or reimplement metering with a direct HTTP client.

## Binding authority, thread claim and origin

Proposed amendments to Part 5's service types (plain non-serde values):

```rust
pub struct PersonalBindingAuthority {
    pub manager: ManagerAuthority,
    pub sender_id: i64,
    pub chat_id: i64,
    pub binding_id: String,
    pub binding_generation: i64,
    pub personal_control_revision: i64,
    pub user_id: String,
}
pub struct AdmittedPersonalUpdate {
    pub ingress: IngressClaim,
    pub authority: PersonalBindingAuthority,
}
pub enum PlatformOriginSource {
    Ingress(IngressClaim),
    ThreadEvent { admission_id: String },
}
pub struct PlatformTurnOrigin {
    pub authority: PersonalBindingAuthority,
    pub nyxbot_agent_id: String,
    pub source: PlatformOriginSource,
}
pub struct PersonalThreadClaim {
    pub nyxbot_agent_id: String,
    pub conversation_id: String,
}
```

`SenderResolution::Personal` carries `authority` and optional existing agent and
conversation hints. The admitted receipt needs only binding authority. Part 2
uses `ensure_nyxbot(db, user_id)` (existing unique per-person NyxBot index and
duplicate-winner reload) before its turn transaction; this may leave an ordinary
empty NyxBot if admission subsequently loses, but cannot leave a credential,
thread, transcript, home pointer or usable turn. It does not need a new
transaction-safe NyxBot creation API. Inside the claim transaction, reload/check
the resolved agent as this active person's NyxBot and join live identity fences.
Any existing binding agent ID must agree. Never treat an optional hint as authority.

Part 1 producer methods, using the candidate's argument order:

```rust
async fn fence_personal_origin(
    &self, db: &Database, session: &mut ClientSession,
    authority: &PersonalBindingAuthority,
) -> AppResult<FenceOutcome>;

async fn claim_binding_thread(
    &self, db: &Database, session: &mut ClientSession,
    authority: &PersonalBindingAuthority, nyxbot_agent_id: &str,
    candidate_conversation_id: &str,
) -> AppResult<PersonalThreadClaim>;

async fn claim_binding_turn(
    &self, db: &Database, session: &mut ClientSession,
    origin: &PlatformTurnOrigin, turn: &TurnRef,
) -> AppResult<()>;

async fn clear_binding_turn(
    &self, db: &Database, session: &mut ClientSession,
    user_id: &str, binding_id: &str, binding_generation: i64, turn: &TurnRef,
) -> AppResult<bool>;

async fn clear_binding_thread_reference(
    &self, db: &Database, session: &mut ClientSession,
    user_id: &str, binding_id: &str, binding_generation: i64,
    conversation_id: &str,
) -> AppResult<bool>;
```

Claim the binding pointer before inserting a new thread/key. If a winner already
exists, validate and use that thread; if candidate inserts already occurred,
abort and retry rather than commit an orphan. Clear methods deliberately do not
require live consent, current controls, or engine readiness. Deleting a revoked
thread is legal. Old settlement/delete cannot clear a newer turn/thread pointer.

Persistence in Part 2 models: nullable `AssistantConversation.platform_thread`
contains platform, manager/profile identity, binding ID/generation and owner
binding; nullable `ActiveTurn.platform_origin` contains complete proof/source.
Use a dedicated persisted type, not the service contract type. An additive
`AssistantMessage.platform_origin` retains the settled proof even after the
active turn is removed. Both conversation and reply DTOs explicitly project
owner-visible metadata; no keys, token hashes, numeric private destination or
full internal proof needs to appear in those DTOs. Ordinary fields/events and
null/absent legacy deserialization remain compatible.

## Concrete runtime and integration seams

New Part 2 module: `services/assistant_nyxagent_runtime.rs`. Move the existing
execution functions into it; do not fork them for Telegram. HTTP wrappers convert
their actual authenticated context and serialize/subscribe only.

```rust
pub struct NyxAgentRuntime {
    state: AppState,
    identity: Arc<dyn ManagerIdentityProducer>,
    ingress: Arc<dyn ManagerIngressTurnStore>,
    egress: Arc<dyn NyxAgentEgress>,
    public_runtime: Arc<dyn PublicNyxAgentRuntime>,
    hooks: Arc<dyn AssistantTurnHooks>,
}

pub async fn start_owner_turn(
    runtime: Arc<NyxAgentRuntime>, owner_user_id: &str,
    start: TurnStart, policy: Option<BillingRoutePolicy>,
    permit: DirectChatPermit, voice_request_id: Option<String>,
) -> AppResult<StartedOwnerTurn>;

pub struct StartedOwnerTurn {
    pub conversation: AssistantConversation,
    pub receiver: broadcast::Receiver<Value>,
}

pub(crate) async fn begin_owner_turn_in_session(
    db: &Database, session: &mut ClientSession, keys: &EncryptionKeys,
    owner_user_id: &str, start: &TurnStart, ids: &OwnerTurnIds,
    agent: &AssistantAgent, platform: Option<&PlatformTurnOrigin>,
    voice_request_id: Option<&str>,
) -> AppResult<OwnerTurnClaim>;

pub(crate) async fn run_claimed_turn(
    runtime: Arc<NyxAgentRuntime>, prepared: PreparedExecution,
    events: RuntimeEvents,
) -> AppResult<SettledExecution>;
```

`OwnerTurnIds` holds the preallocated conversation/turn/input-message IDs reused
by transaction retries. `OwnerTurnClaim` holds the row and zeroizing credential
returned by the database callback; it is not executable before commit.
`PreparedOwnerTurn` privately combines that committed claim with rendered input,
billing policy and the owned `DirectChatPermit` outside the transaction.
`PreparedExecution` is an explicit
owner/public enum, with no `Default`/`From<Public...>` conversion to owner.
`RuntimeEvents` preserves the existing broadcast cursor grammar and adds an
optional bounded manager-event sender. `SettledExecution` distinguishes a
confirmed saved reply, deleted conversation and settlement deadline failure;
a terminal SSE failure is not proof of a durable reply.

`AppState` is retained as the existing runtime dependency bundle: database,
encryption keys, audit HMAC, base URL/config, owner limiter, HTTP client and node
dispatcher. It is not an authentication assertion. Dependencies held by `self`
explain why the manager-facing candidate can take just `db`; adapters must use
the same database as their runtime, not cross databases under one proof.

Part 3 supplies typed egress and public lifecycle adapters; Part 2 supplies the
runtime hooks for ordinary preparation/settlement. Proposed signatures:

```rust
#[async_trait]
pub trait NyxAgentEgress: Send + Sync {
    async fn request(
        &self, authority: &NyxAgentExecutionContext,
        credential: &RuntimeCredential, request: NyxAgentRequest,
    ) -> AppResult<axum::response::Response>;
}

#[async_trait]
pub trait PublicNyxAgentRuntime: Send + Sync {
    async fn prepare(
        &self, db: &Database, context: &PublicExecutionContext, input: TurnInput,
    ) -> AppResult<PreparedPublicTurn>;
    async fn replace_credential(
        &self, db: &Database, context: &PublicExecutionContext,
        expected_key_id: &str,
    ) -> AppResult<RuntimeCredential>;
    async fn settle(
        &self, db: &Database, context: &PublicExecutionContext,
        message_id: &str, result: &TurnResult,
    ) -> AppResult<SettledExecution>;
}

async fn start_personal_turn(
    &self, db: &Database, admitted: &AdmittedPersonalUpdate, input: TurnInput,
    events: mpsc::Sender<ExecutionEvent>,
) -> AppResult<PersonalTurnStart>;
```

`NyxAgentRequest` is a private-body, non-Debug server-built request containing
method, relative path, JSON body, idempotency key and billing route policy; no
caller URL/header overrides. `RuntimeCredential` contains a key ID and
`Zeroizing<String>`, has redacted Debug, no serde, and is never in a manager
response/event. `PreparedPublicTurn` contains only the public context/session,
restricted credential and public prompt/context, supplied by Part 3. The shared
loop can decode, redact, handle bounded retry/Stop and settle either preparation;
owner history, uploads, memory, settings, title generation and team hooks are
entered only in the owner branch. Public limits intersect the loop's existing
bounds and are never reset by a retry or continuation.

Encrypted-key primitives exported to Part 3 are deliberately narrower than
ordinary credential provisioning:

```rust
pub(crate) async fn encrypt_runtime_key(
    keys: &EncryptionKeys, raw: &Zeroizing<String>,
) -> AppResult<Vec<u8>>;
pub(crate) async fn decrypt_runtime_key(
    keys: &EncryptionKeys, ciphertext: &[u8],
) -> AppResult<Zeroizing<String>>;
```

They retain the existing envelope limits and zeroizing intermediate bytes.
Ordinary authority/load/rotation/replacement checks accept only General purpose
(including legacy absent purpose). A purpose mismatch must refuse before any
convergence, deletion or reprovisioning; treating it as an invalid key and
minting a General successor would still widen public authority. Part 3's public
credential rows stay in their separate collection and never become ordinary
`AssistantAgentCredential` rows.

Part 3 must implement egress through the existing proxy/billing service core,
preserving actor/audit information for real HTTP owners and server-resolved
personal owners. Moving or exposing that core requires Part 3's proxy scope;
Part 2 cannot solve it by making a Session-shaped `AuthUser`. Public egress
performs Part 3's session/policy/price/maximum-cost admission for every response,
discovery/capability request, retry and continuation. Cleanup is its dedicated
cleanup-only path. No owner credential/model-list fallback is permitted.

`AssistantTurnHooks` is the service-owned home of the existing `turn_notes`,
`after_turn_boxed`, event wake and optional title hooks, preserving their boxed
future boundary. It accepts owner ID, conversation snapshot, saved `ReplyRef`
and outcome, not `AuthUser`. Existing channel/group/trigger behavior stays behind
the ordinary hooks. Part 5 receives platform references and handles delivery;
the new runner has no Telegram adapter/token dependency.

Legacy effects that still live in handler modules are injected at the composition
boundary while their service extraction lands; the runtime service never imports
or invokes a route handler. A callback carrying a verified owner ID is not an
invitation to construct `owner_auth` inside the new execution path.

Its minimum typed operations are:

```rust
async fn turn_notes(
    &self, db: &Database, row: &AssistantConversation,
    agent: Option<&AssistantAgent>, previous_owner_message: Option<DateTime<Utc>>,
) -> String;
fn after_turn(
    self: Arc<Self>, row: AssistantConversation, reply: ReplyRef,
    text: String, error: Option<TurnError>,
) -> Pin<Box<dyn Future<Output = ()> + Send>>;
```

Use ordinary body/owner checks before limiter admission. The personal runner
checks the linked person's engine flag and existing catalog readiness, acquires
that person's ordinary owner permit and uses `SERVER_TURN_POLICY`; it never
acquires the manager's or sponsor's permit. Busy, pool-full, engine/readiness and
funding failures return ordinary bounded outcomes without another audience or
payer. Event turns retain their existing owner team pool and event rate limits.

Part 5 must expose a `ManagerIngressTurnStore` with these transaction operations:

```rust
async fn claim_personal_execution(
    &self, db: &Database, session: &mut ClientSession,
    admitted: &AdmittedPersonalUpdate, turn: &TurnRef,
) -> AppResult<()>;
async fn settle_personal_execution(
    &self, db: &Database, session: &mut ClientSession,
    ingress: &IngressClaim, reply: &ReplyRef,
) -> AppResult<()>;
```

Claim validates persisted audience and every duplicated identity field and writes
receipt-to-turn linkage atomically with the input/active turn. A consumed claim
does not execute again. Decision admissions need the analogous exact-card
decision claim under that ingress. Part 5's receipt settlement/reply index is the
recovery source for a dropped bounded event channel; event loss cannot authorize
another model run. `ReplyReady` always points to retained data, and `Settled` is
emitted only for a confirmed durable outcome. No model/Telegram call or spawn
runs inside a retryable MongoDB callback.

## Exact Stop and live-origin callback

Prefer Part 1's durable-record ownership and amend the candidate outcome:

```rust
pub enum ExactStopOutcome {
    Captured(CapturedPlatformStop),
    NoMatchingActiveTurn,
}
pub async fn stop_exact_platform_turn(
    db: &Database, session: &mut ClientSession,
    origin: &PlatformTurnOrigin, turn: &TurnRef,
) -> AppResult<ExactStopOutcome>;
pub async fn apply_exact_platform_stop(
    state: &AppState, effects: &CapturedPlatformStop,
) -> AppResult<ExactStopProgress>;

pub async fn fence_platform_execution_in_session(
    db: &Database, session: &mut ClientSession, user_id: &str, api_key_id: &str,
    turn: &TurnRef, expected_origin: &PlatformTurnOrigin,
) -> AppResult<FenceOutcome>;
```

Stop conditionally writes the conversation with owner, exact turn, complete
persisted origin/source, binding ID/generation and epoch predicates. It captures
node/runtime IDs and exact job IDs in the same transaction; no matching origin
means no cancellation side effects. Part 1 inserts `platform_personal_stop_effects`
before committing revocation/replacement and maps the capture into its own plain
model. The unique exact-turn/binding key deduplicates recovery. Part 1 leases
pending records; Part 2 applies exact cancellation and forced settlement; Part 5
schedules recovery. A crash after Stop but before notification remains recoverable.

The live callback verifies the credential-to-conversation binding, live exact
turn, non-stopped state and persisted proof, writes the conversation fence and
joins Part 1's binding/control/account write fences in the caller's effect
transaction. A read-only preflight is not this callback. Origin is extracted
from authenticated conversation state, not tool arguments. Inline continuation
retains the exact origin/source. Recovery must not substitute a new generation.

Add platform metadata to the already-loaded `ChatAuthority`/`AssistantTurnFence`:
a platform-thread marker plus active origin. On a platform thread, require a
live non-stopped turn independent of the ordinary rollout flag; when that turn
has origin, call the live platform fence before execution. A later web turn has
no platform origin and uses normal owner authority. Other conversation keys
retain their rollout behavior and ordinary keys retain their no-extra-read path.
Trusted committed async readers retain their narrowly scoped background path,
not a fabricated live turn or a stale platform-turn execution exception.

An exact node request uses `Operation::CancelExact` with captured conversation,
turn and job IDs, checked against the node runtime identity and `exact_cancel`
support. Unsupported/offline nodes leave truthful recovery status, with no broad
fallback. `MachineJob` currently has no turn provenance and `issue_job` inserts
outside a conversation transaction. Required small extension: nullable issuing
`turn_id` and atomic exact-turn admission plus job insertion. New platform turns
then capture only their own jobs; missing legacy provenance is never a license
to select every conversation job. No committed async watch, specialist task,
automation, key/grant or unrelated web turn is revoked by unlink. The worker's
cancelled branch must also use this exact path, or it defeats the unlink helper.

## Events, cards and deletion

| Entry | Admission and destination behavior |
| --- | --- |
| Fresh private Telegram message | Validate receipt and current binding, claim dedicated thread and exact turn, `guest=false`, ordinary owner limiter/funds/readiness, `TurnOrigin::User` with separate platform proof. No legacy channel/home routing or public quota. |
| Ordinary web send on the saved thread | Existing ownership checks, no platform origin/destination. It remains usable after unlink. It still cannot adopt this thread as home. |
| Inline model/time/image continuation | Same logical turn and origin; re-fence before each upstream call, replacement and lease renewal. Never look up a newer binding to repair revoked work. |
| Newly admitted event/card continuation while linked | Resolve the live binding from `platform_thread` in the new event-turn transaction; create a fresh `ThreadEvent` admission/proof/destination and binding turn pointer. Do not reuse a historical ingress receipt. |
| Event/watch/card continuation after unlink/pause | Run ordinary authorized owner work for the web, without platform origin or destination. Infrastructure failure is deferred, not treated as unlink. Delayed results with a previously captured destination keep that generation and cannot adopt a replacement link. |
| Late old turn settlement | Save to the owner's web history using ordinary exact settlement; delivery rechecks the old proof and is refused after revocation. |
| Delete/recreate | Stop/delete follows explicit owner deletion semantics. Clear matching thread/turn reference in the deletion transaction, even if revoked. Next eligible personal input creates a fresh dedicated thread under the same consent, with one winner and no deleted history recovery. |

Part 1 needs an in-session thread-origin resolver, returning linked authority,
deterministically web-only, or infrastructure error. Part 2 generates the new
event admission ID inside turn claim; Part 5 derives delivery from that persisted
admission. Merely opening history never creates a destination. Settlement can
signal Part 5 with `ReplyRef`; text/cards are rendered from retained owner data,
and tool images/files produce an accurate app link/notice, not a send claim.

For async results, add optional original platform delivery metadata to
`async_service_operation::DeliveryBinding` and populate it at registration.
`bind_in_session`/`apply_delivery` preserve that destination ceiling separately
from the new event origin. Polling and committed work continue under ordinary
live owner authority after unlink. Their old destination is refused, and a fresh
link or control revision cannot make it deliverable. Generic newly raised events
without a captured async destination use the live binding at admission.

Shared decision service entry points:

```rust
pub async fn decide_owner_reply(
    db: &Database, owner_user_id: &str, conversation_id: &str, text: &str,
    audit_actor: &AuditActor,
) -> AppResult<Option<OwnerReplyDecision>>;
pub async fn decide_platform_reply(
    runtime: &NyxAgentRuntime, admitted: &AdmittedPersonalUpdate,
    conversation_id: &str, input: TurnInput,
) -> AppResult<PlatformDecisionOutcome>;
pub(crate) async fn decide_in_session(
    db: &Database, session: &mut ClientSession, decision: &CardDecision,
) -> AppResult<AssistantAcknowledgement>;
```

`CardDecision` carries exact owner/conversation/card/key/issuing-turn IDs,
allow/deny and the existing decider/voice fields. The platform transaction checks
its ingress, live binding and dedicated thread, reads the previous non-steering,
non-pending owner message in this same thread (including web input), selects one
eligible card and writes the decision using the extracted existing body.
Explicit code matches must be unique as well as plain recent yes/no matches.
Expired cards, another owner's/thread's code, authored-skill app review and
ambiguity refuse a decision. Do not require the card's issuing turn still to be
active: pending cards intentionally survive settlement and remain web-decidable
after unlink. Its exact issuing-turn association must instead agree with the
stored card provenance. General Auto/Ask/Full policy is unchanged.

The existing provenance field is `AssistantAcknowledgement.requested_turn_id`;
reuse it rather than adding another card-turn model. A platform decision whose
card lacks trustworthy issuing-turn provenance is app-only.

Decision outcome needs the saved acknowledgement ID and allowed/denied result,
plus existing automation/voice continuation metadata; the candidate's bare
`Decided` cannot tell the caller which continuation to schedule. Resume/wakeup
must be durably queued with the decision, with origin/destination resolved at
its later admission. No second free-form turn is launched by the same ingress
after a decision is consumed. Ordinary channel wrapper retains its existing
allow/deny narration behavior through `OwnerReplyDecision`.

## Scope additions to allocate before production edits

| Proposed owner | Exact expansion | Reason |
| --- | --- | --- |
| Part 2 | `handlers/nyxbot.rs::owner_thread` guard only, in addition to approved `reply_decision` extraction | Existing home pointer must refuse `platform_thread`; current allocation otherwise blocks this necessary A5 fence. No dispatcher/ordinary channel rewrite. |
| Part 2 | `models/assistant_message.rs`: nullable platform origin and constructor/test adjustments | Plan requires origin on settled replies; active-turn-only storage loses it at settlement. |
| Part 2 | `handlers/assistant_nyxagent_images.rs` execution helpers; `handlers/assistant_nyxagent_steering.rs::upstream_capabilities` only; `handlers/assistant_titles.rs` owner-ID execution hook only | These are direct engine dependencies outside the named parent file. Move shared execution into services; keep HTTP steering routes and ordinary behavior. |
| Part 2 | `models/machine_job.rs` issuing turn field; `services/machine_service.rs::issue_job` and exact cancellation service extraction from `handlers/machine_cancel.rs` / `handlers/machine_tools.rs::dispatch` | Exact current-turn job capture and signed cancellation cannot use conversation-wide selection. Preserve ordinary Stop and machine protocol. Coordinate these unallocated files with PM. |
| Part 2 | `models/async_service_operation.rs::DeliveryBinding`; `services/async_service_operation.rs` registration and delivery binding only | Retain an async result's original platform destination independently of its fresh owner event turn; unlink must not cancel the watch or promote a stale send. Poll/proxy authorization stays with Part 3. |
| Part 3 | New typed `services/assistant_nyxagent_egress.rs` plus existing owned proxy core refactor | Reuse real metering/ACL/owner resolution without a service calling HTTP handlers or fabricating Session auth. |
| Part 1 | Add binding turn claim/clear and thread-event resolver to its identity producer; keep stop-effect model/store | Same transaction for revoke, admission, settlement and deletion; no cross-owner collection writes. |
| Part 5 | Contract amendments, ingress claim/settle/decision methods, recovery scheduling and module/index registration | Durable notifications/cancellation and a usable subscription handoff. |

No production edit to these additions is made by this proposal. Existing
`assistant_live` metadata projection already carries owner/conversation/turn/message
identifiers; use its refresh behavior. No body-bearing live-event expansion is
needed. Part 5 owns registration of the new Part 2 runtime, personal-turn and
reply-decision services and the new indexes; Part 2 supplies their manifest.

## Implementation slices and verification handoff

1. Extract ordinary runtime and dependency adapters; keep existing wrappers and
   stream grammar. Move existing loop/recovery/settlement tests with the code.
   Add purpose guards before enabling either new runner.
2. Add persisted thread/origin/reply fields and in-session owner claim. Integrate
   Part 1/5 fences, one-winner dedicated-thread creation and all home exclusions.
3. Add exact Stop capture, Part 1 durable recovery, issuing job provenance and
   exact worker cancellation; add the early platform effect gate with Part 3.
4. Extract and transact shared decisions; add fresh event-origin admission,
   post-unlink web continuation, reply references and atomic deletion clear.
5. Integrate Part 3's public preparation/egress/replacement/settlement through the
   same loop. Keep public unavailable until its real runtime evidence passes.

Focused tests to run in the separately scheduled Rust build slot:

| Target | Evidence required |
| --- | --- |
| Existing handler/runtime/credential/team suites | SSE unsubscribe/lag, heartbeat/orphan reclaim, settlement deadline, secret-prefix redaction, session-lost/busy/key recovery, retry digests, uploads/images, voice, group/trigger and ordinary access modes retain behavior. |
| Personal transaction tests | First-use no-agent consent, two simultaneous creators, isolated Alice/Bob and multiple identities, same-user existing NyxBot reuse, private thread visible in list/history, existing home and newest fallback/repair never choose platform thread. |
| Stop/revocation races | Exact matching source/turn stops; idle thread, wrong generation, web turn and other threads do not. Commit crash recovers cancellation. Prior jobs and committed watches/tasks survive; old nodes never receive broad cancellation. Worker cancellation does not invoke the broad helper. |
| Effect gate tests with Part 3 | `assistant:live-turn-gate` both off/on; revoked/paused/expired/stopped origins fail outer model, MCP direct/wrapped, proxy and inline continuation admission. Ordinary keys incur no platform lookups. |
| Decision/event tests | Plain recent yes/no includes web anchors; duplicate codes, wrong thread/account/generation, expiry and concurrent unlink cannot decide. Full bypass receipts and sensitive/machine policy persist. Linked event gets fresh origin; post-unlink card/watch/specialist result is web-only; delayed old delivery cannot attach to a new link. |
| Delete/recreate/settlement races | Key successor revoked, reference clear commits with deletion, old clear cannot erase new thread, late settlement is quiet for deleted rows, revoked binding's web history remains deletable, concurrent recreation leaves one thread/key. |
| Public adapter adversarial tests with Part 3 | Test hooks fail on any owner `ensure_nyxbot`, history/memory, credential converge/load/replace, owner model fallback, title or team call. Recovery/rotation retains public purpose; every hop calls bounded public admission exactly once. |
| Part 5 receipt/delivery integration | Immediate completion observable; dropped event recovers from saved reply linkage; duplicate ingress never executes twice; no reclassification after link; no send from Part 2 and no private-body replay queue. |

Stage validation is document-only: source traces and signature review,
`git diff --check`, and link-target checks. No cargo compilation, runtime tests,
upstream proof, pushes, deployment or live messaging were performed. PM can use
this commit to reconcile the named producer changes and allocate the precise
scope additions before scheduling implementation.

## Personal implementation checkpoint

The shared owner loop, personal admission/settlement, exact Stop, reply decision,
credential recovery and deletion slices are committed. Composition now uses:

```rust
assistant_nyxagent_runtime::producer(
    state: AppState,
    egress: Arc<dyn NyxAgentEgress>,
    public_runtime: Arc<dyn PublicNyxAgentRuntime>,
) -> Arc<dyn NyxAgentExecutionProducer>;
```

The personal factory retains its four arguments `(state, identity, ingress,
egress)`. No additional registration is needed for the execution factory; it is
in the existing runtime module. Part 5 constructs both producers. The execution
factory loads the exact admitted input and existing bound key, without a new
admission or credential convergence. The shared loop claims
`active_turn.execution_started` only for platform-origin turns, transactionally
with the exact turn/origin/key and live consent. A lost worker is settled by
existing orphan recovery; its claim never reopens. Ordinary HTTP execution keeps
its real-auth adapter and does not use this marker. Public preparation occurs
before any owner work and remains unavailable, including if a future runtime
preparer is supplied before the bounded public loop is implemented.

New regressions cover competing execution workers producing one upstream call,
altered input/origin refusal, legacy rows without the marker, and public/ordinary
context rejection before owner I/O. The earlier personal regressions cover
durable reply references, revoked admission, exact Stop with independent watches,
and atomic credential successor adoption. Targeted rustfmt and whitespace checks
passed; no Rust compilation or tests ran in this worker.

The personal journey still needs PM's compiler/test pass and Part 5's real
webhook/composition/delivery/lifecycle wiring. Queued Part 2 acceptance work is
fresh event admission, existing after-turn reporting/event draining for personal
turns, and delayed async-result delivery using the original captured destination
independently of a fresh execution origin. Public shared-loop execution and
runtime proof remain queued with Part 3. These gaps are not covered by the
personal checkpoint.
