# Public and personal NyxBot: shared contracts v1

Status: contract candidate v1, 2026-10-15; all producer acknowledgements pending.
This slice contains service types and producer signatures, not implementations.
The new execution mode and all new HTTP routes remain disabled/unmounted.
The primary will reconcile Parts 1/3's domain recommendations and obtain Parts
2/4's engine/HTTP acknowledgement before calling this contract frozen. Subsequent
changes require affected-owner acknowledgement and one coordinated consumer update.

Authority: the [converged plan](2026-10-15-public-personal-nyxbot.md), its
[final review](2026-10-15-public-personal-nyxbot-opus-review.md), and
[file allocation](2026-10-15-public-personal-nyxbot-workstreams.md).
Base: `7ff461c92`; integration branch: `nyxbot-manager/integration`.

## Rust boundary and producer ownership

The executable signatures and field names are in
`backend/src/services/platform_manager_contract.rs`. These are non-serialized
service values, with no default implementations. They contain no keys, token
hashes, ciphertext or mutable payer override. They are internal proof snapshots,
not substitutes for live authorization. Every producer must validate the admitted
receipt and live state before claiming work or performing an effect.

Persisted serde structures belong in each owner's `models/` files. Models must
not import this service module. Owning services explicitly convert their models
to these values. HTTP DTOs belong in handlers and are distinct from persisted
models. Private text types intentionally implement neither Debug nor serde.

| Producer trait | Owner / implementation | Consumer and obligation |
| --- | --- | --- |
| `ManagerIdentityProducer` | Part 1, personal binding and runtime-control services | Parts 2/3/5; `runtime_policy`, `resolve_sender`, `fence_personal_origin`, `claim_binding_thread`, `clear_binding_thread_reference` |
| `PersonalTurnProducer` | Part 2, `platform_personal_turn_service.rs` | Parts 1/5; `start_personal_turn`, `stop_exact_platform_turn`, `decide_platform_reply`, `clear_binding_thread` |
| `NyxAgentExecutionProducer` | Part 2, extracted common execution service | Part 3; `execute_nyxagent_turn` with explicit `Owner` or `Public` context and bounded event channel |
| `PublicTurnProducer` | Part 3, public policy/agent services | Parts 1/2/5; `probe_public_readiness`, `prepare_public_context`, `fence_public_execution`, `run_public_turn`, `fence_public_reply` |
| `ManagerTransportProducer` | Part 5, manager ingress/delivery services | Parts 2/3; `admit_manager_update`, `deliver_platform_reply` |

Traits allow each owner to supply an adapter over their service functions and
allow transport tests to inject fake runners. Production integration must bind
real implementations; it must not install a successful fake or a fallback
runtime. Service dependencies/configuration can be held by the implementation;
database handles, string IDs and transaction sessions are explicit at the seam.

`VerifiedManagerUpdate` contains authenticated Telegram facts only: profile and
manager IDs, numeric bot/sender/chat/update/message IDs, observed webhook identity,
and chat kind. It contains no selected account, sponsor, personal grants or text.
Only native webhook verification constructs it. Unknown chat kinds are rejected
before construction. Group/supergroup updates choose `PrivateChatHandoff`, never
personal resolution, confirmation, public inference or a shared public session.

`resolve_sender(db, session, update)` has exactly these outcomes:

- `Unlinked`: deterministic absence of active personal consent; only this result
  permits public selection for a fresh private message.
- `Personal { origin, conversation_id }`: live consent for the exact verified
  private sender; `conversation_id = None` requests one-winner thread creation.
- `Unavailable(reason)`: disabled account, controls, authority or other known
  denial; return a bounded fixed status, without public fallback.
- `Err(AppError)`: database/infrastructure failure; abort the admission transaction.
  Never convert an error into `Unlinked` or admit either runner speculatively.

`PlatformTurnOrigin` captures stable manager identity and epoch, exact sender/chat,
binding ID/generation, personal control revision, user ID and NyxBot agent ID.
Personal fences exclude legacy guide/profile generation, `profile.enabled`,
public budgets, sponsor state and onboarding expiry. A personal pause/resume
advances its control revision so old admitted effects cannot revive; consent
itself remains durable. Public-only edits do not change that revision.

`TurnRef` always means exact conversation plus turn ID. `stop_exact_platform_turn`
adds exact persisted origin binding/generation/manager proof to its match; a
web-origin turn on the same conversation is not a match. `StopRecorded` means a
durable stop and cancellation-recovery record were written in the caller's
transaction. Its `cancellation_id` is metadata, not evidence of remote completion.
No broad node/desktop cancel or delegation cascade is allowed.

Concrete Part 2 extraction traps in the base: `assistant_nyxagent::request_stop`
also calls `async_service_operation::cancel` for the whole conversation, even
with no active turn. Do not reuse that function wholesale for transport unlink;
the new exact-stop helper must write only the matched Telegram turn's stop and
recovery record, preserving independently committed watches. Turn orchestration
currently lives in `handlers/assistant_nyxagent.rs::start_turn` and accepts
`AuthUser`. Extract service execution; never call it internally with fabricated
auth, and never let the public branch enter owner credential convergence.

`claim_binding_thread` returns the authoritative conversation ID: the proposed
new ID or the winner already stored under the same live binding. The losing
creation transaction must not commit a second thread/key. Clearing is a
compare-and-clear on origin and exact conversation ID; `false` means no matching
reference. Part 2's `clear_binding_thread` orchestrates that Part 1 helper in the
normal conversation-deletion transaction. Deleted history is never recreated.

`NyxAgentExecutionContext::Owner` preserves ordinary permissions and billing;
`platform_origin = None` is ordinary web/API owner work. `Public` is an explicit
branch with no ordinary orchestrator convergence. Its context pins sponsor,
public policy revision/digest, exact profile/model/service/operation, lifetime,
loop-wide token/model/tool bounds and isolated session/turn. No native tools,
arbitrary proxy, owner nodes, account inventory, resources/prompts, wrapped tools,
or unpinned model calls become available through the common adapter.

Part 3 owns validation of every model/tool hop and all public reservations.
`model_operations` must be nonempty, unique and fully priced or proven free;
missing bounds/pricing are unavailable. `PublicReadiness::Ready` requires actual
dated, expiring upstream build/profile containment, cleanup, credential propagation
and cost evidence. It binds the reviewed manager epoch, public revision/digest and
sponsor. Part 1 rechecks those and live admin authority in its enabling transaction.
No existing NyxAgent version or profile label alone constitutes readiness.

## Transactions, claims and effects

| Transaction owner | Operations that commit together | Work after commit |
| --- | --- | --- |
| Part 5 ingress | New UUID receipt with unique `(manager_bot_id, sender_id, update_id)`; classification; relevant live controls/binding/account write fences; immutable audience and claim | Runner dispatch only under that receipt; no model or Telegram request in transaction |
| Part 1 consent | Fresh proof consumption, prior binding revocation, new binding, intent terminal state, exact old-turn Stop through Part 2, public-session revocation/cleanup work through Part 3 | Exact cancellation and public session cleanup from durable recovery records |
| Part 1 revoke/replace/control | Live human/owner/admin authority; generation/revision CAS; pending intents/challenges cancellation; affected bindings/turns/receipts fenced; durable exact cancellation work | Bounded cleanup; no independent task cascade |
| Part 2 turn claim | Receipt claim validation; Part 1 origin/control/account fences; thread claim/creation; ordinary owner admission; user message and active turn carrying origin | Common stream/model execution; ordinary owner settlement and web events |
| Part 2 delete | Exact conversation/key deletion/stop and Part 1 compare-and-clear binding thread reference | Ordinary deletion cleanup; later message may create a new thread |
| Part 3 public call | Live public purpose/policy/session checks; both sender and total maximum-cost holds; exact billing reservation and claim | Dispatch, one existing billing settlement per usage event, idempotent cleanup |
| Part 5 delivery | Metadata-only delivery claim; destination generation/control/session fences; independent pacing reservation | Resolve current verified token/observation and send with `protect_content: true`; settle send outcome |

Every session parameter joins the existing transaction; helpers neither start a
nested transaction nor commit/abort their caller's session. Retry callbacks may
re-execute database work only. All participating authority fences perform a real
conditional write, not a snapshot read or no-op update. They conflict with
revocation/control/account lifecycle mutations. Mongo server time governs expiry
and windows; caller wall-clock timestamps cannot extend authority.

Receipts live in the new `platform_manager_ingress_receipts` collection; legacy
purpose-scoped receipts are never reinterpreted. Part 5 persists the chosen
audience and any downstream conversation/turn linkage under the same claim.
Personal payer is the admitted linked person under ordinary billing resolution;
authorized org BYOK keeps its existing resource-owner semantics. Public funding
is server-resolved sponsor funding only. A receipt is not a wallet debit.
Crash/retry may finish metadata/settlement, but never reclassify, re-run uncertain
work, switch audience, or post the same usage twice.

`ReplyDestination` never contains token material. All duplicated manager/sender/
chat/turn fields must exactly agree with the persisted receipt/origin/session.
Personal destinations require private chat, live binding generation and origin;
public destinations require the original public session/revision/expiry. A linked
event/watch/card continuation derives a fresh destination from the live binding
on `platform_thread`, not a historical receipt. After unlink it continues as
ordinary owner work in the web app with no Telegram destination. A delayed reply
from the stopped Telegram-origin turn retains its original proof and is refused.

`DeliveryRef::Reply` references an existing retained reply. `Fixed` references
only ingress metadata and permits only a fixed status/handoff, never private body
text. Delivery dedupe includes destination proof and reply/notice identity. The
transport validates `ReplyContent` against that reference and audience. Refusal
never returns `Sent`. `Sent` requires observed successful Telegram response and
nonempty returned message IDs; it is not proof a person read the message.
`Uncertain` follows any ambiguous post-send result and is never blindly replayed.
`Deferred` stores pacing/reference metadata only; bodies are reloaded only while
existing retention and authority allow. No new durable private-body replay queue.

Immediately before send, Part 5 loads the current manager token, verifies numeric
bot ID, current observation and webhook readiness. Same-identity token rotation
preserves consent; old tokens are not captured in destinations. A changed identity
advances epoch and revokes transport authority. Fences prevent new effects admitted
after revocation. Requests already forwarded before revocation cannot be undone;
neither Stop nor a database fence promises rollback of an external request.

Personal admission never uses `GUIDE_WORK`, public inference quotas or
`platform_native_replies`. Delivery has bounded separate public/personal queues,
fair per-user service and reserved personal capacity. Public yields first under
shared bot pressure. Tested Telegram `retry_after` governs delays; no fixed value
in this contract claims an external Telegram limit. Fixed error replies are paced.

## HTTP paths and authentication

All paths below include `/api/v1`. New paths are reserved contracts, not mounted
routes. Human means the verified first-party human session extractor, rejecting
API keys (including public-purpose keys), delegated/relay tokens and service
accounts. Admin additionally means a live platform admin, never merely operator
or sponsor-organization admin. Responses use `Cache-Control: no-store`.

| Method and path | Auth / owner | Purpose |
| --- | --- | --- |
| `POST /platform-personal-links/preview` | Public, intent possession; Part 1 | Bounded sanitized landing metadata, no account or binding inventory |
| `POST /platform-personal-links/review` | Human; Part 1 | Derive acting account and issue fresh 10-minute challenge |
| `POST /platform-personal-links/approve` | Human; Part 1 | Approve exact reviewed challenge/digest; does not activate binding |
| `POST /platform-personal-links/deny` | Human; Part 1 | Deny/cancel challenge and intent; no credentials |
| `POST /platform-personal-links/status` | Human; Part 1 | Challenge status for the same reviewing account |
| `GET /platform-personal-bindings` | Human; Part 1 | Acting person's links only, cursor-paged |
| `POST /platform-personal-bindings/{id}/revoke` | Human owning binding; Part 1 | Generation-fenced unlink of this personal purpose |
| `GET /admin/platform-bots/{id}/runtime` | Admin; Part 1 | Controls, public policy, support/readiness status |
| `PUT /admin/platform-bots/{id}/runtime` | Admin; Part 1 | Revision-fenced explicit control/config update; one atomic public Enable |
| `POST /admin/platform-bots/{id}/runtime/clear-emergency-stop` | Admin; Part 1 | Explicit audited clearing with both expected revisions |
| `POST /admin/platform-bots/{id}/public/profiles` | Admin; Part 1 handler / Part 3 service | Live public-safe profile discovery; sponsor-funded bounded calls |
| `POST /admin/platform-bots/{id}/public/readiness` | Admin; Part 1 handler / Part 3 service | Validate exact proposed selection/limits; no configuration write |

The only unauthenticated new endpoint is bounded intent preview. Raw intent
tokens travel in POST bodies; the first-party navigation URL carries them in its
fragment, which the linking page consumes without logs/analytics. Public signup
tools cannot select bot/sender/user/payer or manufacture an invitation.

Part 3 adds `/api/v1/platform-personal-bindings` and
`/api/v1/admin/platform-bots` (all descendants, including runtime) to
`delegated_read_denied_path` before mounting. This closes the new human-only GET
surfaces even for delegated `account:read`. No challenge/status GET is introduced.
Part 5 mounts only after these guards and real handler producers land. Existing
onboarding endpoints, creation webhook and child-installation semantics remain
owned by their existing flows. Broad native `/unlink` calls Part 1 to revoke
both purposes for the exact manager/sender; app revoke above is personal-only.

## DTO fixtures and defaults

These are handler DTO shapes for Part 4 fixtures. IDs shown as `<...>` stand for
server-generated UUID strings, not accepted actor/owner inputs. Telegram numeric
IDs are JSON decimal strings, preserving their exact values in JavaScript;
internal proofs use checked `i64`. Dates are RFC3339 UTC strings. Money is a
decimal credit string with at most 12 fractional digits, never a JSON number.
Request objects reject unknown fields. Nullable response fields are explicitly
`null`; omitted additive response fields remain readable by old clients.

Preview request: `{"intent_token":"<opaque single-use token>"}`.
Preview response:

```json
{"purpose":"personal_nyxbot","status":"pending","manager":{"id":"<manager-id>","username":"nyx_demo_bot","display_name":"NyxBot"},"sender":{"id":"123456789","display_name":"Alice"},"expires_at":"2026-10-16T08:00:00Z","invitation_required":true,"requires_same_chat_confirmation":true}
```

Review request uses the same `intent_token`. Review response:

```json
{"challenge_id":"<challenge-id>","status":"review","expires_at":"2026-10-15T08:10:00Z","review_digest":"<server digest>","account":{"id":"<acting-user-id>","display_name":"Alice"},"permission_mode":"nyxmode","billing":"personal","replaces_existing_binding":false,"disclosures":{"telegram_transport":true,"existing_nyxbot_access":true,"can_approve_sensitive_actions":true,"started_work_may_continue_after_unlink":true}}
```

Approve request: `{"challenge_id":"<challenge-id>","review_digest":"<server digest>"}`.
Response (also the status response before Telegram confirmation):

```json
{"challenge_id":"<challenge-id>","status":"awaiting_chat_confirmation","expires_at":"2026-10-15T08:10:00Z","binding":null,"confirmation_command":"/confirm <one-time-code>"}
```

Status request: `{"challenge_id":"<challenge-id>"}`. Completed status:

```json
{"challenge_id":"<challenge-id>","status":"confirmed","expires_at":"2026-10-15T08:10:00Z","binding":{"id":"<binding-id>","generation":1},"confirmation_command":null}
```

Deny request: `{"challenge_id":"<challenge-id>"}`; response
`{"status":"denied"}`. Challenge statuses are closed:
`review`, `awaiting_chat_confirmation`, `confirmed`, `denied`, `cancelled`,
`expired`. An intent lasts 24 hours; each post-authentication challenge lasts 10
minutes and is account/intent/review-bound. Only matching same-private-chat proof
completes consent. HMACs include distinct purpose, manager epoch, verified sender/
chat and random secret; no raw token is stored. Approval alone grants no access.
Public budget/model edits and signup exceeding ten minutes do not kill the intent.

Owner list response (`?before_id=<uuid>`, limit fixed at 50):

```json
{"bindings":[{"id":"<binding-id>","generation":1,"manager":{"id":"<manager-id>","username":"nyx_demo_bot","display_name":"NyxBot"},"sender":{"id":"123456789","display_name":"Alice"},"state":"active","conversation_id":null,"consented_at":"2026-10-15T08:05:00Z","revoked_at":null}],"next_before_id":null}
```

Revoke request: `{"generation":1}`. Response:
`{"revoked":true,"work_may_continue_in_app":true}`. Repeated same-generation
revoke is idempotent; a different owner is not-found-shaped. Several identities
per person are supported, each with its own thread and independent unlink.

Runtime GET for absent controls:

```json
{"contract_version":1,"supported":false,"runtime":null,"public_readiness":{"status":"unavailable","reason":"mode_disabled","checked_at":null,"expires_at":null}}
```

Configured runtime response:

```json
{"contract_version":1,"supported":true,"runtime":{"manager_authority_epoch":1,"personal_control_revision":2,"public_policy_revision":4,"personal_enabled":true,"public_enabled":false,"emergency_stopped":false,"public_policy":{"source":"nyxagent","profile":"nyxagent/chat","sponsor_owner_user_id":"<sponsor-org-user-id>","total_daily_credits":"1","sender_daily_credits":"0.1"}},"public_readiness":{"status":"unavailable","reason":"public_not_ready","checked_at":null,"expires_at":null}}
```

Runtime update (three-field public setup plus concurrency fences):

```json
{"expected_manager_authority_epoch":1,"expected_personal_control_revision":2,"expected_public_policy_revision":4,"public_enabled":true,"public_policy":{"source":"nyxagent","profile":"nyxagent/chat","total_daily_credits":"1","sender_daily_credits":"0.1"}}
```

Sponsor is retained from manager setup and displayed; an explicit Advanced
`sponsor_owner_user_id` change requires live authorized-org validation. It is
never defaulted to the acting admin. Omitted patch fields preserve existing
values. `null` is rejected for booleans, revisions, source, profile, sponsor and
money. `public_policy: null` is allowed only with `public_enabled: false` and
clears public configuration without touching personal controls. Missing controls
mean disabled; creation initializes both enabled flags false and an explicit
epoch/revisions. Server setup defaults apply only to a new draft: 1 total and 0.1
per sender; existing limits survive. Neither Enable nor legacy Resume clears an
emergency stop. Clearing requires its dedicated endpoint and explicit revisions.

Profile discovery request: `{"expected_public_policy_revision":4}`. Response:
`{"profiles":[],"default_profile":null,"readiness":{"status":"unavailable","reason":"public_not_ready"}}`
when the restricted upstream contract is unavailable. An empty list is not a
successful profile or a free-price inference. Readiness request uses the same
reviewed epoch/revisions and `public_policy` as the runtime update, with no enable
flag. Response uses `public_readiness` above. A successful fixture must include
`status: "ready"`, non-null checked/expiry dates and an opaque evidence digest;
only tests may simulate that outcome before real upstream proof exists.

Part 2 adds nullable `platform_thread` and `platform_origin` to existing owner
conversation and active-turn DTOs; all ordinary existing fields/events retain
their current names. Example additions:

```json
{"platform_thread":{"platform":"telegram","manager_id":"<manager-id>","binding_id":"<binding-id>","binding_generation":1},"active_turn":{"turn_id":"<turn-id>","platform_origin":{"binding_id":"<binding-id>","binding_generation":1,"manager_authority_epoch":1,"personal_control_revision":2}}}
```

This is a field fragment, not a replacement conversation response. Ordinary
threads/turns return null (older rows/clients may omit). No `NyxbotChannel`,
`owner_sender_ids`, sponsor access to history or home-thread adoption is introduced.
Web live events remain metadata-only and owner-scoped. Text and ordinary approval
codes are delivered with protected content; images/files use a truthful owner-app
notice. Consent explicitly states sensitive cards can be approved through Telegram,
Telegram transport is platform operated, and Telegram chats are not end-to-end
encrypted. Pending cards remain decidable in the app after unlink.

## Error fixtures

Use existing `AppError` variants and their existing response envelope; this
contract allocates no new numeric block and does not edit `errors/mod.rs`.
Messages below are fixed strings. Never place a token, upstream body or private
message in an error. Before mounting, feature-off routes are simply absent.

| Case | HTTP / variant | Response fixture |
| --- | --- | --- |
| Machine/delegated caller on human-only route | 403 `Forbidden` | `{"error":"forbidden","error_code":1002,"message":"Forbidden: First-party human session required"}` |
| Missing/expired/cancelled/replayed intent | 404 `NotFound` | `{"error":"not_found","error_code":1003,"message":"Not found: Personal linking request unavailable"}` |
| Changed reviewed account/digest or stale generation/revision | 409 `Conflict` | `{"error":"conflict","error_code":1004,"message":"Conflict: Personal linking review changed; review again"}` |
| Public Enable without verified readiness | 409 `Conflict` | `{"error":"conflict","error_code":1004,"message":"Conflict: Public runtime unavailable; keep public routing paused"}` |
| Invalid money or unknown input field | 400 `ValidationError` | `{"error":"validation_error","error_code":1008,"message":"Validation error: Invalid public runtime configuration"}` |

Infrastructure errors preserve the existing sanitized `AppError` response and
abort dependent work. Telegram ingress uses typed `UnavailableReason` for fixed
bounded replies, not raw exception text. Existing turn-active, funds, pool and
model failures preserve ordinary assistant semantics; none selects another payer
or audience. Token-bearing preview errors are deliberately indistinguishable.

## Additional exact file ownership and activation dependencies

Paths below are relative to `backend/src/`. They resolve the allocation's
previously unspecified paths; other exclusive assignments remain unchanged.

| Owner | Exact files / scope |
| --- | --- |
| Part 3 | `handlers/llm_gateway.rs`, `services/llm_gateway_service.rs`: provider proxy, OpenAI gateway and status/discovery public-purpose guards; `services/llm_usage_service.rs` only if public metering integration needs it |
| Part 3 | `services/api_key_mutation_service.rs`, `services/api_key_credential_service.rs`, `handlers/api_keys.rs`: public-purpose generic edit/rotate/delete refusal or purpose preservation where existing `key_service.rs` alone is insufficient |
| Part 5 | `services/admin_user_service.rs`: `set_user_active`, `delete_user_cascade_internal` wiring to Part 1/3 lifecycle services, preserving ordinary account cleanup and no-platform-state standalone-Mongo behavior |
| Part 5 | `handlers/users.rs`: account self-delete entry wiring only if the central cascade hook is insufficient |
| Part 5 | `services/telegram_new_admin.rs`, `services/telegram_new_service.rs`: same-identity token rotation/current observation and changed-identity/retirement lifecycle wiring to Part 1/3; business logic stays with producers |
| Part 5 | Shared registration in `services/mod.rs`, `models/mod.rs`, `handlers/mod.rs`, `routes.rs`, `db.rs`, `main.rs`, feature catalog registration; opening commit changes only service registration |

Part 1 must supply transaction-aware account/sender/manager revocation and
replacement entry points; Part 2 must supply exact-stop recovery and thread
helpers; Part 3 must supply public-session revoke/cleanup hooks. Their lifecycle
function signatures are finalized with those domain recommendations before
Part 5 edits the account/token entry points. No hook may be wired as best-effort
in-memory notification in place of durable revocation.

Acknowledgement needed: Part 1 identity/control/HTTP naming and lifecycle entry
points; Part 2 common adapter/event/thread/origin DTOs; Part 3 proof/readiness,
operation pricing and lifecycle hooks; Part 4 HTTP fixtures/null handling. Exact
new mode/feature registration and enablement remain a later Part 5 integration
slice. Personal enablement waits for all origin-enforcing replicas and producers;
public enablement also waits for real NyxAgent full-loop/cleanup evidence. Mixed
versions and flag-off paths fail closed, with no automatic manager conversion.

The primary-owned [acceptance record](2026-10-15-public-personal-nyxbot-acceptance.md)
tracks A1–A17 separately; no fixture or contract signature is a passing gate.
Validation for this opening slice: rustfmt parsing/formatting of the contract,
rustfmt check of module registration with child traversal disabled, staged
whitespace checking, and JSON parsing of the nine fenced DTO fixture blocks.
Cargo compilation, runtime/transaction tests, live Telegram, account
use, deployment, pushes and PR operations are outside this stage.
