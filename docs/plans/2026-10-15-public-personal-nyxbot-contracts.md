# Public and personal NyxBot: shared contracts v1

Status: frozen integration contract v1, 2026-10-15. This commits concrete service
signatures and HTTP schemas, not producer implementations or passing acceptance
claims. The new mode and all new routes remain disabled/unmounted.

Authority: the [plan](2026-10-15-public-personal-nyxbot.md),
[converged review](2026-10-15-public-personal-nyxbot-opus-review.md), and
[allocation](2026-10-15-public-personal-nyxbot-workstreams.md). This freeze adopts
[Part 1](2026-10-15-public-personal-nyxbot-part1-interfaces.md),
[Part 2](2026-10-15-public-personal-nyxbot-part2-interfaces.md),
[Part 3](2026-10-15-public-personal-nyxbot-part3-interfaces.md), and compatible
C1–C4 from [Part 4](2026-10-15-public-personal-nyxbot-part4-interfaces.md), with
PM's explicit resolutions below. It supersedes incompatible candidate/proposal
signatures and fixtures. There is no remaining producer proposal cycle before
implementation; changes to this boundary require a coordinated consumer update.
Base: `7ff461c92`; owner: Part 5, `nyxbot-manager/integration`.

## Rust boundary

The actual Rust types and `async_trait` signatures are in
`backend/src/services/platform_manager_contract.rs`. They are non-serialized
service values. Durable serde structures belong in each owner's `models/` files;
models must not import service types. Handlers project dedicated HTTP DTOs.
Private body types have no Debug/serde; runtime credentials use Zeroizing and
redact both key IDs and material. Snapshots are not authority without live checks.
There is no default producer, successful stub, fabricated AuthUser, synthetic
NyxbotChannel, owner_sender_ids addition, or fallback runtime.

| Producer | Owner | Concrete seam |
| --- | --- | --- |
| `ManagerIdentityProducer` | 1 | Runtime read; typed sender resolution; caller-transaction personal fence; binding thread/turn claim and exact clear; thread-event authority resolver |
| `ManagerLifecycleProducer` | 1 | Durable exact-stop record insertion; sender unlink; bounded owner retirement; manager retirement; leased lifecycle recovery |
| `PersonalTurnProducer` | 2 | Normal owner start; exact Stop capture/apply; mandatory live execution fence; provenance-bound card decision; thread deletion/clear |
| `NyxAgentExecutionProducer` | 2 | Common loop with explicit `Owner` or `Public` context and bounded event sender |
| `NyxAgentEgress` | 3 | Server-built typed request through existing metered proxy service core |
| `PublicNyxAgentRuntime` | 3 | Public preparation, purpose-preserving credential replacement, encrypted upstream-binding observation and settlement |
| `PublicTurnProducer` | 3 | Profiles/probe/local readiness fence; public context/start/effect/reply fences; sender/scope revocation and exact cleanup |
| `ManagerIngressTurnStore` | 5 | Caller-transaction execution/decision claim and settlement for the persisted audience |
| `ManagerTransportProducer` | 5 | Authenticated ingress classification and protected, fenced reply delivery |

`resolve_sender` returns `Unlinked`, `Personal { authority, optional real-agent
hint, optional conversation hint }`, `Unavailable(reason)`, or `Err(AppError)`.
Only `Unlinked` can select public. Bound-but-paused, disabled owner, engine/funds/
capacity failure and infrastructure failure never fall through to public.

`PersonalBindingAuthority` contains stable manager identity/epoch, sender/chat,
binding ID/generation, personal control revision and linked user. Consent and
resolution do not require an existing agent. Part 2 runs ordinary
`ensure_nyxbot(db, owner)` before its turn transaction and validates the real
agent inside it. `PlatformTurnOrigin` then adds that agent and exact source:
`Ingress(IngressClaim)` or `ThreadEvent { admission_id }`. Personal proofs exclude
public policy revision, guide generation and onboarding enabled state.

`claim_binding_thread` returns the authoritative real-agent/conversation pair.
Claim before inserting candidate thread/key; abort/retry if a different winner
exists. A losing admission may leave an ordinary empty NyxBot, never an orphan
platform thread/key/transcript/home pointer. Turn claim records the complete
origin and binding active-turn pointer atomically. Clear helpers compare exact
owner/binding/generation/old pointer and remain legal after revocation or engine
failure; they cannot erase a newer pointer.

`resolve_thread_authority` returns `Personal(authority)`, `WebOnly`, or Err.
A committed watch/card/specialist event gets a fresh admission and live origin
while linked, or runs as normal web-only owner work after unlink. Async result
delivery retains its original binding generation/destination separately; a fresh
web event must not promote that stale result to a new Telegram link. Opening a
saved thread on the web does not attach a Telegram destination.

### Authenticated command provenance

`VerifiedManagerUpdate` comes only from authenticated native ingress. Its
`command_eligibility` is `OriginalPrivateMessage` only for a direct, original,
non-bot private message meeting the native control rules; forwarded/imported,
channel/anonymous, edited or otherwise untrusted control contexts are Ineligible.
The actual sender/chat, never forwarded-origin identity or user text, determines
routing. Intentionally forwarded ordinary text remains usable as `TurnInput` in
the linked person's own thread.

`OriginalManagerCommand::from_verified(update, input)` returns None unless the
verified update is private and command-eligible. It carries private text without
Debug/serde and is required by `decide_platform_reply` and sender unlink.
Producers compare its update/provenance with the persisted ingress claim and
live binding; constructing a snapshot is not consent. Personal intent issuance,
same-chat confirmation, yes/no/code decisions and controls require this same
eligibility. Forwarded text must never decide a card, issue consent or unlink.

The opted-in dispatcher intercepts new personal `/link`, purpose-prefixed
`/confirm P-...`, and broad `/unlink` before the old
`platform_binding::handle_command`. Preserve native creation classification,
legacy child installation, `/onboarding`, `/setups` and legacy unprefixed
onboarding confirmations. No generic user/owner/payer argument comes from text.

### Turn events, receipts, decisions and exact Stop

The transport creates a bounded `mpsc::Sender<ExecutionEvent>` before calling a
start method; the producer installs it before spawn. Immediate completion must
be observable. Part 2 claims the personal receipt/turn inside owner admission;
Part 3 claims the public receipt before execution. Claims compare exact audience,
identity, binding/session and claim ID and refuse a duplicate start. Decision
claim and acknowledgement/continuation scheduling share the card transaction;
a consumed decision receipt cannot also start a free-form turn.
`claim_personal_decision` also takes `&OriginalManagerCommand` and matches its
authenticated update/message/observation against that receipt, including the
persisted original-private eligibility. Same-sender proof alone is insufficient.

Settlement persists reply linkage through `ManagerIngressTurnStore` in the
owning settlement transaction. `ReplyReady(ReplyRef)` references a retained saved
reply. `Settled` is emitted only after confirmed durable settlement, never for
`PersistenceUnavailable`. Dropped notifications recover from durable receipt/
reply linkage, without running the turn again or queuing private bodies.

Card decisions reuse `AssistantAcknowledgement.requested_turn_id`; missing or
untrustworthy issuing-turn provenance makes the card app-only. Match a unique
explicit code or one eligible recent yes/no anchored to the previous
non-steering, non-pending owner message in this same thread, including web input.
Wrong owner/thread, ambiguity, expiry and authored-skill app-only review refuse.
The issuing turn need not still be active. `PlatformCardDecision` returns the
saved acknowledgement ID, exact requested turn, allowed result and existing
voice/continuation/trigger references. Queue wakeup atomically; resolve its new
origin when admitted. Existing access modes and sensitive/machine policy persist.

`stop_exact_platform_turn` joins the revoker's session, matches the complete
persisted origin/source and exact active turn, writes Stop, and returns
`Captured(CapturedPlatformStop)` or `NoMatchingActiveTurn`. **Part 2 captures;
Part 1 inserts the durable recovery record in that same transaction.** Capture
alone is not durable recovery. Part 1's bounded leased worker calls Part 2's
`apply_exact_platform_stop` after commit and stores exact progress.

Capture includes only jobs atomically issued with this turn's provenance and
exact node/runtime/job IDs. Signed cancellation targets those IDs; unsupported
nodes have no broad fallback. Runtime changes cannot cancel a successor job.
No idle/web/different-origin turn, earlier job, pending card or independently
committed watch/task is captured. Existing
`assistant_nyxagent::request_stop` unconditionally cancels conversation-wide
async watches, even idle; it cannot be reused wholesale for transport unlink.

## Transactions, billing and delivery

| Transaction owner | Atomic local work | After commit / outside transaction |
| --- | --- | --- |
| 5 ingress | New-purpose receipt claim, manager/consent/account fences and one audience decision | Dispatch only that audience |
| 1 consent/revoke/control | Live human/admin authority, revision CAS, proof cancellation, binding changes, exact Stop capture + durable store, public revocation marker/cleanup record | Exact cancellation/settlement and public cleanup from durable records |
| 2 personal turn/delete | Receipt/owner/origin fences, one-winner thread, normal owner admission, input/active origin and binding pointer; exact deletion/clear | Common execution and ordinary owner billing/settlement/live metadata |
| 1 public Enable | Recheck candidate identity/revisions/admin/sponsor and Part 3's local readiness fence; atomic desired controls/config | Billed probe happens before transaction; no rollback of its real cost |
| 3 public admission | Durable public call state and both sender/total cap scopes coordinated with existing exact meter holds | Forward only after both cap and meter admission are durable |
| 5 delivery | Metadata-only send claim, current destination fences and independent pacing admission | Load current verified token; protected send; settle observed/uncertain outcome |

Session-taking helpers join the caller's transaction; no nested transaction or
helper commit/abort. Retry callbacks perform database work only. Authority
fences make real conditional writes that conflict with revocation/control/account
mutations, including no-binding ingress resolution. No model, Telegram, node or
provider network call runs in the transaction. Use trusted backend clock
conventions (`Utc::now()` at authoritative boundaries); untrusted timestamps
cannot extend authority. Persist original admission time/day for retries.

Public caps are not a second wallet debit. Existing `BillingService::open_required`
does not take a caller session, so do not claim one transaction includes its
meter holds and cap records. Part 3 coordinates durable
`Prepared -> Funded -> DispatchAdmitted -> Settled` state, or
`ReleasedBeforeDispatch` only when no forwarding is proven. Both cap scopes and
both outer/inner existing meter admissions must be durable before forwarding.
Recovery/compensation reuses exact identities and existing settlement, never
posts twice. Unknown dispatch outcome consumes its conservative maximum; retry
and midnight settlement retain the original day. Price/operation/component,
wallet/grant/allowance and audit/ledger behavior remain with existing billing.

Personal execution retains normal permissions, pool and person billing;
authorized org BYOK retains existing resource-owner semantics. It never uses
GUIDE_WORK, public quotas or platform_native_replies. Public funding is the
server-resolved sponsor only, with no global owner-resolver override.

Receipts use `platform_manager_ingress_receipts`, never reinterpret legacy
purpose receipts, and cannot switch audience after link/retry. Destinations bind
exact manager/sender/chat/receipt/turn and binding generation or public session/
revision/expiry. Fixed references permit only bounded status/private-chat handoff,
never private text. Delivery validates content against its saved reference.

Sent requires an observed successful Telegram response with nonempty message IDs;
refusal never returns Sent. Ambiguous post-send outcomes are Uncertain and never
blindly replayed. Deferred queues store references/pacing metadata, reload only
under current retention/authority and do not persist a private-body replay queue.
Text/cards use `protect_content: true`; unsupported images/files use a truthful
owner-app notice. Public/personal delivery queues and inference capacities remain
independent, with fair per-user delivery and reserved personal capacity. Public
yields first under shared bot pressure. Real Telegram retry_after controls pacing;
this contract invents no external rate limit.

Immediately before send resolve the current token, numeric bot identity,
observation and webhook readiness. Same-identity token rotation preserves consent;
changed identity advances epoch and revokes transport authority. Already forwarded
external requests cannot be undone. Stop/revocation fences new effects and sends,
without promising external rollback.

## Public engine and readiness

The common loop selects the public adapter **before** any owner agent creation,
history, memory, credential convergence, model fallback, title or team preparation.
`NyxAgentEgress` accepts an explicit authority, zeroizing credential and server-built
relative operation; reuse service-layer proxy/metering, never call an HTTP handler
or fabricate an owner session. `PublicNyxAgentRuntime` prepares restricted context,
replaces credentials without widening purpose, records encrypted upstream session/
response handles under exact key/turn fences, and settles public persistence.
Cleanup uses those exact handles and never enters owner lifecycle callbacks.

Public model operations pin service, method, canonical path, protocol, model,
execution-authority digest, endpoint generation where applicable, and per-call
input/output/response bounds. Whole-loop limits include duration, model/tool
calls, continuations, total input/output tokens and bytes/context bounds. Repeated
input/retries/continuations count again; no budget reset. No caller URL, arbitrary
model, header override, node selection or unknown protocol is authorized.
Public purpose denies owner/machine/Oracle/SSH/native/discovery/wrapper bypasses;
only the reviewed FAQ/signup tool allowlist and exact model operations are usable.

Profiles/probes take a reviewed candidate `ManagerRuntimePolicy`; they do not
create controls or public sender receipts. Absent-control initialization builds
an internal candidate with positive epoch/revisions 1, bound to legacy profile
generation and exact policy. No fake sender/receipt is invented for an admin
probe. Part 3 uses a separate bounded public-probe authority; all billable probes
consume the sponsor's total cap. Final Enable revalidates local evidence against
the exact candidate in Part 1's transaction, with no network call there.

Ready evidence binds proof-record ID, candidate/policy/evidence digests, manager/
public revision, sponsor, pinned upstream build/profile/model operations, bounds,
checked time and expiry, with live catalog/grant/price/cleanup fences. Until real
full-loop isolation, routing, hard bounds and deletion evidence exists, return
Unavailable(PublicNotReady); historical hashes/discovery/mocks do not prove ready.
A local unavailable result differs from Err infrastructure/decoding failure.

Sender revocation joins link/unlink transactions; scope revocation fences
manager/sponsor/session authority and queues bounded exact cleanup. No affected
session is a successful local no-op (`recovery_record_id: None`) independent of
public readiness. Remote failure cannot restore authority. Do not TTL-delete
keys/encrypted handles before confirmed cleanup/expiry. Personal-only app revoke
must not broaden to unrelated public/owner work.

## HTTP paths and authentication

All paths below are complete. They are reserved, unmounted routes. Human means
verified first-party human session, rejecting API keys, public-purpose credentials,
delegated/relay tokens, OAuth clients and service accounts. Admin additionally
requires live platform admin, not operator or sponsor-org admin. Linking responses
use `Cache-Control: no-store` and `Referrer-Policy: no-referrer`; runtime responses
also use no-store.

| Method/path | Auth | Producer |
| --- | --- | --- |
| POST `/api/v1/platform-personal-links/preview` | Intent possession only, sanitized bounded output | 1 |
| POST `/api/v1/platform-personal-links/review` | Human | 1 |
| POST `/api/v1/platform-personal-links/approve` | Human, exact reviewed challenge | 1 |
| POST `/api/v1/platform-personal-links/deny` | Human, same reviewing account | 1 |
| POST `/api/v1/platform-personal-links/status` | Human, same reviewing account | 1 |
| GET `/api/v1/platform-personal-bindings` | Human, own inventory | 1 |
| POST `/api/v1/platform-personal-bindings/{id}/revoke` | Human owning binding | 1 |
| GET `/api/v1/admin/platform-bots/{id}/runtime` | Admin | 1 |
| PUT `/api/v1/admin/platform-bots/{id}/runtime` | Admin | 1 |
| POST `/api/v1/admin/platform-bots/{id}/runtime/clear-emergency-stop` | Admin | 1 |
| POST `/api/v1/admin/platform-bots/{id}/public/profiles` | Admin | 1 handler / 3 service |
| POST `/api/v1/admin/platform-bots/{id}/public/readiness` | Admin | 1 handler / 3 service |

Admin `{id}` and owner `manager.id` are the stable platform profile ID.
Assistant thread `platform_thread.manager_id` uses the same profile ID.
Part 3 adds the personal-bindings GET family and reserves the personal-links GET
family in delegated_read_denied_path before mounting. Existing admin deny class
already covers admin routes. There is no proof/status GET. All actor/payer/user
identities derive from verified session or admitted server state.

The browser URL is `/nyxbot/link#intent_token=<opaque>`, superseding `/link/telegram`.
Capture and scrub even malformed fragments synchronously before router/auth/
telemetry construction. Keep the token only in memory and dedicated tab-scoped
sessionStorage, at most 24 hours shortened by server expiry. Clean same-origin
return_to; no secret query/path, OAuth parameter, query key, persisted Zustand,
analytics, toast or diagnostic. Suppress linking telemetry. Clear terminal intent
state; blocked storage keeps the URL clean and explains reopening the original
link. Invitation/auth methods remain authoritative; signup/verification/MFA never
approve consent. A 10-minute challenge expiry permits fresh review of a valid
24-hour intent, including after lengthy signup or account/policy change.

## Concrete DTO schemas and fixtures

Requests reject unknown fields. IDs are server UUID strings; Telegram IDs are
decimal strings, internally checked i64. Dates are UTC RFC3339. Money is an exact
decimal credit string with at most 12 fractional digits, never a JSON number.
Nullable response fields are explicit null; legacy clients can omit additive
assistant metadata. No response exposes raw proof, ciphertext or key identifiers.

Preview/review request: `{"intent_token":"<opaque>"}`. Preview:

```json
{"purpose":"personal_nyxbot","status":"pending","manager":{"id":"<manager-id>","username":"nyx_demo_bot","display_name":"NyxBot"},"sender":{"id":"123456789","display_name":"Alice"},"expires_at":"2026-10-16T08:00:00Z","invitation_required":true,"requires_same_chat_confirmation":true}
```

Review:

```json
{"challenge_id":"<challenge-id>","status":"review","expires_at":"2026-10-15T08:10:00Z","review_digest":"<digest>","account":{"id":"<acting-user-id>","display_name":"Alice"},"permission_mode":"nyxmode","billing":"personal","replaces_existing_binding":false,"disclosures":{"telegram_transport":true,"existing_nyxbot_access":true,"can_approve_sensitive_actions":true,"started_work_may_continue_after_unlink":true}}
```

permission_mode is exactly nyxmode, ask_every_time or autopilot. Review text
explicitly discloses memory/persona and account/service access, personal billing,
sensitive approvals, platform-operated Telegram without end-to-end encryption,
and continued work after unlink. Previous owner's identity/history is never shown.

Approve request: `{"challenge_id":"<id>","review_digest":"<digest>","replace_existing":false}`.
replace_existing defaults false if omitted; true is required after explicit
replacement acknowledgement when review says replacement. Digest binds these
semantics. Only the first successful approval returns the purpose-prefixed code:

```json
{"challenge_id":"<id>","status":"awaiting_chat_confirmation","expires_at":"2026-10-15T08:10:00Z","binding":null,"confirmation_command":"/confirm P-6MX4-W2P9"}
```

Status request: `{"challenge_id":"<id>"}`. All status and replayed approval
responses have confirmation_command null, including while awaiting confirmation.
Lost code requires fresh review. Confirmed status:

```json
{"challenge_id":"<id>","status":"confirmed","expires_at":"2026-10-15T08:10:00Z","binding":{"id":"<binding-id>","generation":1},"confirmation_command":null}
```

Deny request uses challenge_id; response `{"status":"denied"}`. Closed challenge
states: review, awaiting_chat_confirmation, confirmed, denied, cancelled, expired.
HMAC domains bind distinct purpose, manager epoch, actual private sender/chat and
random secret. Store no raw token/code. Only eligible same-chat proof completes
consent; browser approval grants no execution. Public edits never expire consent.

Inventory query uses optional before_id, fixed page size 50. Binding state is
exactly active or revoked; nullable conversation means no first thread yet:

```json
{"bindings":[{"id":"<binding-id>","generation":1,"manager":{"id":"<manager-id>","username":"nyx_demo_bot","display_name":"NyxBot"},"sender":{"id":"123456789","display_name":"Alice"},"state":"active","conversation_id":null,"consented_at":"2026-10-15T08:05:00Z","revoked_at":null}],"next_before_id":null}
```

Revoke request `{"generation":1}`; response
`{"revoked":true,"work_may_continue_in_app":true}`. Repeated same-generation revoke
is idempotent; other owner is not-found-shaped. Multiple bindings per person are
independent; never adopt home/old public transcript.

One readiness DTO is used by runtime, profiles and readiness endpoints:
status ready|unavailable; reason nullable closed snake_case; checked_at,
expires_at and evidence_digest each nullable string. Ready requires real dates/
digest and reason null. Unavailable has no usable evidence (digest null); unknown
or unverified dates are null. Closed reasons map exactly to UnavailableReason:
mode_disabled, manager_unavailable, emergency_stopped, personal_disabled,
public_disabled, owner_unavailable, transport_unavailable, origin_revoked,
engine_unavailable, insufficient_funds, capacity_limited, public_not_ready,
public_quota_limited, runtime_uninitialized, sponsor_unavailable,
profile_unavailable, proof_expired. Infrastructure remains sanitized HTTP Err.

An enforcing supported server with absent controls returns:

```json
{"contract_version":1,"supported":true,"runtime":null,"public_readiness":{"status":"unavailable","reason":"runtime_uninitialized","checked_at":null,"expires_at":null,"evidence_digest":null}}
```

Unsupported/flag-off means absent routes or supported false with null runtime and
mode_disabled. GET never creates controls or fabricates versions. Configured:

```json
{"contract_version":1,"supported":true,"runtime":{"manager_authority_epoch":1,"personal_control_revision":2,"public_policy_revision":4,"personal_enabled":true,"public_enabled":false,"emergency_stopped":false,"public_policy":{"source":"nyxagent","profile":"nyxagent/chat","sponsor_owner_user_id":"<sponsor-org-user-id>","total_daily_credits":"1","sender_daily_credits":"0.1"}},"public_readiness":{"status":"unavailable","reason":"public_not_ready","checked_at":null,"expires_at":null,"evidence_digest":null}}
```

First Enable bot uses initialize true, current legacy profile generation, and all
three expected runtime values zero. Zero means row absent, never stored authority.
It explicitly requests BOTH lanes and commits readiness/config/controls atomically:

```json
{"initialize":true,"expected_profile_generation":7,"expected_manager_authority_epoch":0,"expected_personal_control_revision":0,"expected_public_policy_revision":0,"personal_enabled":true,"public_enabled":true,"public_policy":{"source":"nyxagent","profile":"nyxagent/chat","total_daily_credits":"1","sender_daily_credits":"0.1"}}
```

PM chooses this Part 1 creation CAS, not Part 4's alternate expected_runtime field.
Concurrent creation/stale legacy generation conflicts; failure leaves no controls
or conversion. Candidate/stored initial versions are positive 1. Missing desired
lane fields initialize false; no GET/write implicitly enables another lane.
Explicit personal-only initial opt-in does not require public proof/funding:

```json
{"initialize":true,"expected_profile_generation":7,"expected_manager_authority_epoch":0,"expected_personal_control_revision":0,"expected_public_policy_revision":0,"personal_enabled":true,"public_enabled":false}
```

Never downgrade a failed combined Enable silently. Offer personal-only explicitly.
Later public-only updates omit personal changes and compare positive versions:

```json
{"expected_manager_authority_epoch":1,"expected_personal_control_revision":2,"expected_public_policy_revision":4,"public_enabled":true,"public_policy":{"source":"nyxagent","profile":"nyxagent/chat","total_daily_credits":"2","sender_daily_credits":"0.2"}}
```

Public pause sends these expected fields and public_enabled false, omitting
personal_enabled. Omitted patches preserve values; public_policy null clears only
with explicit public_enabled false. Nested missing fields preserve; null source/
profile/sponsor/money/booleans/revisions is invalid. initialize defaults false;
expected_profile_generation is required on initialization, absent otherwise.
No-op preserves revisions. Public changes advance only public revision; personal
changes advance only personal revision. Emergency true is whole-manager stop;
false is refused here. Dedicated clear endpoint takes all three positive expected
fields and never enables a lane; neither Enable nor legacy Resume clears Stop.

Map saved guide.limits.daily_credits to total and guide.anonymous_daily_credits to
sender quota. Default 1/0.1 only when amounts are absent. Retain setup's
funding_owner_user_id as sponsor; explicit Advanced replacement requires live
org authorization and never defaults to acting admin. Quota <= total. No new
rate/concurrency/execution-limit editor; validated effective bounds are read-only.

Profiles request uses the same expected revisions, optional initialization
precondition and proposed public_policy, without enable flags. Existing-policy
requests can omit the policy. Entries are exactly {id,label}; default_profile is
an advertised ID or null. Requested nyxagent/chat is only a draft selection.
The complete initial discovery/readiness request schema is illustrated below;
existing-runtime requests omit initialize/expected_profile_generation and use
the three positive expected values. All three expected values are required.
public_policy is optional only when a saved policy exists; its source is exactly
nyxagent, profile is a nonempty advertised candidate ID, sponsor is optional to
retain setup ownership, and both budget fields are exact decimal strings. No
enable/emergency flag or caller evidence is accepted by these two endpoints.

```json
{"initialize":true,"expected_profile_generation":7,"expected_manager_authority_epoch":0,"expected_personal_control_revision":0,"expected_public_policy_revision":0,"public_policy":{"source":"nyxagent","profile":"nyxagent/chat","total_daily_credits":"1","sender_daily_credits":"0.1"}}
```

Unavailable discovery response:

```json
{"profiles":[],"default_profile":null,"readiness":{"status":"unavailable","reason":"public_not_ready","checked_at":null,"expires_at":null,"evidence_digest":null}}
```

Readiness request uses the same reviewed preconditions/policy; response is
`{"public_readiness":{"status":"unavailable","reason":"public_not_ready","checked_at":null,"expires_at":null,"evidence_digest":null}}`.
Paid probe is explicit and never automatically retried by the UI. Successful
fixtures require real evidence in production; tests alone may simulate ready.

Part 2 projects optional-nullable platform_thread on index/history conversation
DTOs and platform_origin on active turns AND settled messages. Field fragment:

```json
{"platform_thread":{"platform":"telegram","manager_id":"<manager-id>","binding_id":"<binding-id>","binding_generation":1},"active_turn":{"turn_id":"<turn-id>","platform_origin":{"binding_id":"<binding-id>","binding_generation":1,"manager_authority_epoch":1,"personal_control_revision":2}},"messages":[{"platform_origin":{"binding_id":"<binding-id>","binding_generation":1,"manager_authority_epoch":1,"personal_control_revision":2}}]}
```

manager_id matches owner inventory manager.id and the admin platform profile ID. Ordinary web
turns have null origin even on a platform thread. No full proof, numeric private
sender/chat, key ID or body-bearing live event. Existing owner-scoped live
invalidation/polling preserves field names. A thread badge survives settlement/
reload; web history/cards remain after unlink. Sponsor cannot read personal history.

## Error fixtures

Existing AppError variants/envelope remain authoritative; no new numeric block.
Never include secret, upstream body or private text. Feature-off routes are absent.

| Case | HTTP / variant | Fixed response fixture |
| --- | --- | --- |
| Human guard in handler | 403 Forbidden | `{"error":"forbidden","error_code":1002,"message":"Forbidden: First-party human session required"}` |
| Missing/expired/cancelled intent | 404 NotFound | `{"error":"not_found","error_code":1003,"message":"Not found: Personal linking request unavailable"}` |
| Changed review/digest, stale versions, unacknowledged replacement | 409 Conflict | `{"error":"conflict","error_code":1004,"message":"Conflict: Personal linking review changed; review again"}` |
| Public Enable lacks evidence | 409 Conflict | `{"error":"conflict","error_code":1004,"message":"Conflict: Public runtime unavailable; keep public routing paused"}` |
| Invalid configuration/unknown field | 400 ValidationError | `{"error":"validation_error","error_code":1008,"message":"Validation error: Invalid public runtime configuration"}` |

Auth middleware may instead return existing ForbiddenWithGuidance with additive
guidance; the handler fixture is not a universal middleware message. Database/
infrastructure errors use existing sanitized responses and abort dependent work.
Telegram fixed replies use typed reasons. Turn-active/model/funds/pool failures
retain ordinary assistant semantics without alternate audience or payer.

## Exact ownership amendments

All original exclusive allocations remain. Paths below are relative to
backend/src and resolve the previously unspecified/PM-approved narrow additions.

| Owner | Files / exact scope |
| --- | --- |
| 1 | models/platform_bot_runtime_controls.rs also owns durable personal Stop effects; no second Part 2 cancellation model/queue |
| 2 | services/assistant_nyxagent_runtime.rs common runtime; models/assistant_message.rs settled origin; handlers/nyxbot.rs owner_thread exclusion plus existing reply-decision extraction |
| 2 | handlers/assistant_nyxagent_images.rs execution helpers; handlers/assistant_nyxagent_steering.rs upstream_capabilities only; handlers/assistant_titles.rs owner-ID execution hook only |
| 2 | models/machine_job.rs issuing-turn provenance; services/machine_service.rs issue_job; exact cancellation extraction from handlers/machine_cancel.rs and handlers/machine_tools.rs dispatch |
| 2 | models/async_service_operation.rs DeliveryBinding and services/async_service_operation.rs registration/delivery binding only; independent polling/proxy authorization stays with 3 |
| 3 | New services/assistant_nyxagent_egress.rs, typed existing proxy-core adapter; public runtime/credential implementation in services/platform_public_credential_service.rs |
| 3 | handlers/llm_gateway.rs, services/llm_gateway_service.rs guards; services/llm_usage_service.rs if public metering requires it |
| 3 | handlers/api_keys.rs, services/api_key_mutation_service.rs, services/api_key_credential_service.rs immutable purpose/session confinement |
| 3 | handlers/agent_bindings.rs and services/agent_binding_service.rs public-purpose guards, exclusively |
| 3 | services/billing/mod.rs, route_context.rs, reservation.rs narrow public admission bridge only; no global owner_resolver sponsor override |
| 5 | services/admin_user_service.rs set_user_active/delete_user_cascade_internal; handlers/users.rs self-delete entry only if central cascade is insufficient |
| 5 | services/telegram_new_admin.rs and services/telegram_new_service.rs current-token observation, changed identity/retirement lifecycle wiring |
| 5 | Shared services/models/handlers module registration; routes.rs, db.rs, main.rs, feature catalog, bounded worker scheduling; transport/ingress/delivery and combined fixtures |

For ApiKey constructor additions, Part 2 updates its literals, Part 3 its literals,
Part 5 other existing tests/fixtures/registration; coordinate mechanical edits.
A separate authoritative public binding collection is allowed if immutable
purpose/session linkage is enforced. No broad scaffold of unrelated files.

Account disable/delete invokes durable Part 1 retirement and Part 3 sponsor
revocation. Before reactivation sets is_active true, finish interrupted personal
retirement/cancellation; refuse or stay inactive on failure. Old consent cannot
revive. No-platform accounts retain standalone-Mongo compatible no-op behavior.
Manager retirement/identity change advances epoch and records durable progress
before bounded binding cleanup; same-identity token rotation preserves epoch.

## Producer registration and bounded-worker signatures

These are required producer entry signatures for later implementation, not
installed placeholders. Types come from the shared contract unless qualified.
Each factory owns real dependencies and returns a real adapter; it performs no
network readiness claim. Composition uses the existing crate::AppState dependency
bundle, not an auth assertion. All adapters use that same database.

```rust
// Part 1: services/platform_personal_binding_service.rs
pub fn producer(state: crate::AppState) -> Arc<dyn ManagerIdentityProducer>;
// Part 1: services/platform_bot_runtime_control_service.rs
pub fn producer(
    state: crate::AppState,
    personal: Arc<dyn PersonalTurnProducer>,
    public: Arc<dyn PublicTurnProducer>,
) -> Arc<dyn ManagerLifecycleProducer>;

// Part 5: services/platform_manager_ingress_service.rs
pub fn producer(state: crate::AppState) -> Arc<dyn ManagerIngressTurnStore>;
// Part 3: services/assistant_nyxagent_egress.rs
pub fn producer(
    state: crate::AppState,
    identity: Arc<dyn ManagerIdentityProducer>,
) -> Arc<dyn NyxAgentEgress>;
// Part 3: services/platform_public_credential_service.rs
pub fn producer(state: crate::AppState) -> Arc<dyn PublicNyxAgentRuntime>;

// Part 2: services/assistant_nyxagent_runtime.rs
pub fn new(
    state: crate::AppState,
    identity: Arc<dyn ManagerIdentityProducer>,
    ingress: Arc<dyn ManagerIngressTurnStore>,
    egress: Arc<dyn NyxAgentEgress>,
    public_runtime: Arc<dyn PublicNyxAgentRuntime>,
) -> Arc<NyxAgentRuntime>; // Implements NyxAgentExecutionProducer.
// Part 2: services/platform_personal_turn_service.rs
pub fn producer(
    runtime: Arc<super::assistant_nyxagent_runtime::NyxAgentRuntime>,
) -> Arc<dyn PersonalTurnProducer>;
// Part 3: services/platform_public_agent_service.rs
pub fn producer(
    state: crate::AppState,
    execution: Arc<dyn NyxAgentExecutionProducer>,
    ingress: Arc<dyn ManagerIngressTurnStore>,
) -> Arc<dyn PublicTurnProducer>;
// Part 5: services/platform_manager_dispatch.rs
pub fn producer(
    state: crate::AppState,
    identity: Arc<dyn ManagerIdentityProducer>,
    lifecycle: Arc<dyn ManagerLifecycleProducer>,
    personal: Arc<dyn PersonalTurnProducer>,
    public: Arc<dyn PublicTurnProducer>,
    ingress: Arc<dyn ManagerIngressTurnStore>,
) -> Arc<dyn ManagerTransportProducer>;
```

Construction order is identity/ingress/egress/public-runtime, common runtime,
personal/public, lifecycle, transport. Existing ordinary owner hooks are internal
to the Part 2 runtime constructor. Part 3 egress calls module-level policy/live
fences, not a cyclic personal producer dependency. PlatformManagerProducers holds
identity, personal, public, lifecycle and transport adapters, with no Default.
Execution, egress, public-runtime and ingress dependencies live in their actual
consumers; the aggregate does not retain redundant copies. An absent required
producer means unmounted mode.

Each of these service modules supplies the exact index entry below:
platform_personal_binding_service, platform_link_intent_service,
platform_bot_runtime_control_service, platform_public_policy,
platform_public_agent_service, platform_public_credential_service,
platform_public_budget_service, platform_manager_ingress_service,
platform_manager_delivery. A domain with no new indexes explicitly returns Ok
only after confirming no indexes are required; this is not readiness evidence.

```rust
pub async fn ensure_indexes(db: &Database) -> Result<(), mongodb::error::Error>;
// Part 1: platform_link_intent_service, logical expiry before optional TTL cleanup.
pub async fn expire_personal_proofs(db: &Database, limit: u32) -> AppResult<RecoveryBatch>;
// Part 3: platform_public_agent_service, revoke first, then leased exact cleanup.
pub async fn sweep_public_lifecycle(
    state: &crate::AppState, limit: u32,
) -> AppResult<RecoveryBatch>;
// Part 5 calls ManagerLifecycleProducer::recover_personal_lifecycle(db, personal, limit).
```

Part 5 registers modules/indexes/workers only after those files and adapters land.
No nonexistent producer module is registered by this commit. Part 1 owns leases,
expiry/retirement and Stop-record retry logic; Part 3 owns public revocation/
cleanup/admission reconciliation. Part 5 schedules bounded passes, never writes
another domain's collections. No destructive TTL before required public cleanup.

The [acceptance record](2026-10-15-public-personal-nyxbot-acceptance.md) tracks
A1–A17. Contracts and fixtures prove no gate. Next dependencies are the real
Part 1 models/fences/lifecycle/handlers, Part 2 runtime/thread/Stop/event producers,
Part 3 local purpose guards/egress/revocation and later real public evidence, and
Part 4 DTO/browser implementation. Part 5 transport resumes only when scheduled;
personal activation needs enforcing replicas, public activation additionally
needs real full-loop/cleanup evidence. Mixed versions stay closed.

Validation for this contract slice is rustfmt parsing/checking, whitespace and
JSON fixture parsing. No cargo compilation, runtime/transaction tests, live
Telegram/account use, deployment, push or PR operation is claimed.


### Reserved command receipt clarification

Personal `/link`, purpose-prefixed `/confirm P-...`, and broad `/unlink` use a
`Control` audience in the same ingress collection. Receipt and command effects
commit together; a control receipt can never claim execution or a card decision.
One-time intent output stays ephemeral and must use a dedicated typed command
send path. It is not a `DeliveryRef::Fixed` text bypass, is never queued as a body,
and is never reconstructed on a Telegram replay. A failed link delivery requires
a new `/link` update. Delivery still checks original-private provenance, live
manager identity/current observation, and a durable single-send claim.


### Personal transport checkpoint and recovery

The mounted personal path uses the real identity, ingress, personal execution,
NyxAgent egress and lifecycle producers. Public execution remains unavailable;
no public runtime proof or successful public reply is manufactured. Expected
personal admission failures yield a bounded status response with their existing
personal receipt; infrastructure failures remain errors. Forwarded ordinary
text retains authenticated sender/chat identity, while forwarded proof/control
cannot approve or decide. Saved replies link to the exact web conversation at
`/assistant?c=<conversation-id>` for reviewing action cards. Telegram card
summaries/codes and fresh event/after-turn delivery remain follow-up work owned
with Part 2, not implemented claims in this checkpoint.

The delivery transaction validates exact ingress, owner, binding generation,
manager epoch, personal revision, saved reply origin/body and thread. It reads
the current profile observation, fences that observation, and captures the
matching current webhook-ready credential. A same-bot rotation can defer a
retained reply while observation readiness changes; a different numeric bot,
epoch, retired binding or stopped reply refuses delivery. The provider operation
runs after commit with `protect_content`; only one durable dispatch claim can
send. An ambiguous response or a crash after dispatch stays uncertain and is
never automatically replayed. A proven Telegram 429 may defer a retained reply
until `retry_after`; command bearer output is ephemeral and never recovered.

Ingress `delivery_recovery_queued` defaults false on legacy rows. It becomes true
only after delivery metadata is durable, or after deleted/altered proof is
terminally refused. Recovery drains oldest settled unqueued replies in bounded
batches; delivery then owns pending retries. Account retirement recovery runs
before exact-stop recovery. Each sweep bounds selected records and uses state
CAS; no unbounded update-many cleanup is used in transport recovery. Ingress
and terminal delivery tombstones currently have no TTL: retaining metadata
prevents replaying old Telegram update IDs or uncertain/sent attempts. Bodies
and one-time tokens are absent from these collections. Safe metadata compaction
requires a separate durable dedupe horizon and is not claimed here.

`platform_manager_transport::journey_tests` uses local transaction-capable MongoDB,
a local mock Telegram endpoint and a test NyxAgent egress. These are regression
sources, not upstream/public proof or real Telegram acceptance evidence. Their
first checkpoint was formatted and whitespace-checked; compilation and execution
belong to the primary's single build slot.

### Production consumer cleanup checkpoint

Personal card decisions resolve thread authority inside the existing decision
transaction and require full equality with the admitted binding proof, including
the personal control revision. Web-only or changed authority refuses the
decision. Original-private provenance, exact ingress claims, requested-turn
checks and the previous-message selection boundary still apply. Unlink reports
actual revocation and queued exact-stop counts only after its transaction commits;
the durable lifecycle recovery path owns those effects.

The unused personal execution/thread forwarding methods and lifecycle
enqueue/owner-retirement/manager-retirement wrappers were removed with their
matching implementation bodies. The free execution fence, deletion/thread clear,
durable stop helpers and account/manager lifecycle entrypoints remain. Recovery
consumes processed/backlog results and yields between bounded passes; delivery
reports confirmed, deferred, refused and uncertain outcomes without logging
bodies or bearer tokens. Public runtime and fresh-event obligations remain in
the contract for their owners to connect. This source cleanup was rustfmt- and
whitespace-checked only; production compilation remains primary/CI validation.

### Ephemeral public signup and saved-reply hook pairing

`platform_manager_delivery::deliver_public_signup(state, context, token)` accepts
only an already committed mint and returns `PublicSignupDeliveryOutcome::{Sent,
Unavailable, Uncertain}`. `PublicSignup` is a separate durable metadata reference,
deduped by the original public receipt and intent ID. Its claim transaction
rechecks the exact executing receipt/session/turn, current public authority,
pending intent HMAC/initiation/sender/chat and manager observation/credential.
The bearer exists only in `Zeroizing` memory for one protected send with previews
disabled. It is absent from delivery records, upstream requests and tool results.
Provider 429, ambiguity and process loss never authorize bearer replay. Recovery
refuses unattempted ephemeral rows and terminalizes lost dispatch claims.

Composition now receives Part 2's required `OwnerTurnSettledHook` as the fifth
personal producer dependency. Main and the HTTP router construct the handler
adapter; services depend only on its trait. The callback receives a durable
`ReplyRef` and reloads that saved reply before ordinary after-turn orchestration.
The local journey fixture uses the same adapter. Fresh-event/original-destination
transport pairing and the separate public encrypted-reply reader remain the next
required consumers; this checkpoint claims neither complete public readiness
nor passing runtime tests. The new signup regression sources use local injected
public state solely to exercise the send boundary.


## Saved reply transport completion checkpoint

Personal Telegram delivery now derives its destination only from the actual
Saved message's `platform_reply: Captured` value. Fresh `platform_origin` remains
execution provenance. A final send-claim transaction validates the original
input's route and source turn, the original settled receipt for ingress sources,
or the retained event input for genuine ThreadEvent sources, then the current
exact binding/person/manager fences and canonical rendered body. A resumed
reply may differ from its original receipt's reply ID; it cannot alter that
receipt or adopt a newer link. Absent routing and explicit WebOnly never acquire
a Telegram destination. Original expiry (currently the turn's 2100-second
ceiling) prevents sending while preserving the durable web reply.

`platform_manager_delivery::saved_personal_reply(state, &conversation, &ReplyRef)`
is called by the handler-owned Saved hook and ordinary event wrapper before
existing after-turn queue draining. Services do not import handlers. Callback,
event receiver, and recovery converge on the same metadata-only delivery key.
Delivery rows have nullable ingress IDs for genuine events and retain the
original source turn ID. The serde-default message recovery marker and partial
index permit bounded oldest-first event recovery: enqueue first, source marker
second; malformed/revoked/expired proofs are terminal, infrastructure failures
remain pending. No event fabricates an ingress receipt. Captured origin is boxed
in Rust to bound enum size; serde/BSON shape is unchanged.

Pending action rendering reads a bounded exact-owner/conversation pending set.
Only exact Saved-turn/current-key cards with retained valid CardRouting and
unique confirmation codes expose bounded summaries and `yes CODE` / `no CODE`.
Legacy routing, code collisions, different turns/keys, and unsupported review
cards direct to the exact web conversation. The final claim re-renders the same
canonical content. The original authenticated-command decision gate remains
required; forwarding ordinary text never grants control consent.

The public dispatcher now uses concrete preparation, transactional execution
fencing and execution producers, establishing its bounded event receiver before
spawn. Public reply delivery branches before any owner history lookup, reads the
separate encrypted public-turn collection and invokes the producer's settled
reply fence within the send-claim transaction. Its metadata reference and rate
lane are separate from personal replies and one-time signup proof delivery.
Unsupported upstream readiness remains an actual refusal; no ready evidence or
successful public execution is fabricated.

Validation for this checkpoint is targeted rustfmt and whitespace checking.
New journey/card/event/expiry regression sources have been written, not executed
or compiler-verified in the worker. Primary owns combined compilation, test
execution, public-loop producer integration and runtime acceptance evidence.
