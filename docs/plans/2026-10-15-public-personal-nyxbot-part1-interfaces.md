# Part 1 proposal for identity, consent, and runtime controls

Status: coordination proposal, 2026-10-15. Source investigation only, against
`7ff461c92` on `nyxbot-manager/identity`. The signatures and examples below are
proposed contracts, not implemented APIs. This commit contains no production
changes, module registration, build, test execution, or live-service evidence.
Part 5 owns the shared contract commit and resolves names with the consumers
before implementation begins. This proposal incorporates the candidate read from
its integration worktree at `docs/plans/2026-10-15-public-personal-nyxbot-contracts.md`
and `backend/src/services/platform_manager_contract.rs`. Those candidate files are
coordination inputs, not files added or changed by this commit.

The [implementation plan](2026-10-15-public-personal-nyxbot.md), the
[final Opus verdict](2026-10-15-public-personal-nyxbot-opus-review.md#convergence-verdict-2026-10-15-final-review-of-the-revised-plan),
and the [exclusive allocation](2026-10-15-public-personal-nyxbot-workstreams.md)
govern this proposal. Relevant existing contracts are
[Platform BotFather](../PLATFORM_BOTFATHER.md), the
[NyxAgent engine](../chat/08-nyxagent-engine.md), and the
[NyxBot orchestrator](../chat/09-nyxbot-orchestrator.md). The current access-mode
sections and code take precedence over older Full-only descriptions in the
orchestrator overview.

## Explicit delta against `1b5aea084`

Reviewed the committed contract candidate through `git show`; it is not merged
into this branch. Its shared types, producer signatures, HTTP paths, and DTO
names supersede earlier naming ideas. The signatures below either restate that
candidate or describe proposed producer-local additions, not a parallel API.
Part 1 accepts the candidate with these bounded corrections for the primary:

- Separate capability from configuration: capable runtime GET returns
  `supported:true, runtime:null` before setup. First runtime PUT uses
  `initialize:true`, current `expected_profile_generation`, and zero expected
  epoch/revisions as an absence precondition. Store positive authority starting
  at 1, retain the existing profile sponsor, and perform readiness plus atomic
  Enable behind the same user action. No controls are created on failure.
- Per Part 2's `338d92f5b`, resolve `PersonalBindingAuthority` without an agent.
  Part 2 ensures ordinary NyxBot before the turn transaction, validates its real
  ID during thread claim, and completes `PlatformTurnOrigin` afterward. Thread
  claim takes the real agent ID and returns both winning IDs. This supersedes
  this proposal's earlier consent-time transaction-aware agent ensure.
- Amend `StopRecorded` to `Captured(CapturedPlatformStop)`: Part 2 writes exact
  Stop and returns captured effects; Part 1 inserts its durable recovery record
  before committing. Clear helpers compare exact owner/binding/generation and
  old turn/thread only, and remain usable after revocation.
- Part 3 still needs local readiness recheck, sender-public-session revoke, and
  profile discovery seams. Public-session absence is a successful local no-op.
- HMAC-only manual proofs cannot be reconstructed for status. Return the
  purpose-prefixed code once on approval; status returns
  `confirmation_command:null`. A lost code requires fresh review.
- Part 5 must finish interrupted personal retirement before reactivation can
  restore `is_active:true`. Otherwise old consent could revive. The existing
  account-file allocation covers this hook.

The remaining model/index/test details below support implementation planning.
They establish no implemented or passing behavior. Further nonblocking details
remain for the implementation turn.

## Decisions for the shared contract

- `resolve_sender` returns `Unlinked`, `Personal`, or `Unavailable` inside the
  caller's MongoDB transaction, matching Part 5's producer proposal. Only
  `Unlinked` permits public selection. Disabled or missing
  account authority, paused personal routing, missing controls, and infrastructure
  errors cannot become `Unlinked`.
- Personal consent uses new bindings, intents, and challenge collections. It has
  no expiry and no dependency on onboarding `expires_at`, profile `generation`,
  `profile.enabled`, public readiness, sponsor funds, or public quota.
- A stable manager authority epoch identifies the manager's lifetime. Personal
  control revisions invalidate admitted personal work across pause/resume.
  Public configuration changes advance only the public policy revision.
- Every session-taking identity, thread, turn, lifecycle, and Stop helper joins
  an already-running transaction. It never starts, commits, aborts, or retains
  the caller's `ClientSession`.
- Part 2 returns `Captured(CapturedPlatformStop)` after writing exact Stop in
  the caller's transaction. Part 1 persists those effects before committing
  revocation. External cancellation happens after commit. Existing broad
  conversation Stop is unsuitable for transport unlink.
- Human HTTP decisions derive the account from `AuthUser`, use the existing
  first-party human guard, and never accept `user_id`, grants, or a mode override.
  Telegram reuses the owner's current NyxBot access and confirmation policy.
- Public readiness belongs to Part 3. Probe work happens before Part 1's config
  transaction, and enabling consumes a result bound to the exact candidate and
  live authority. A failed or absent probe cannot enable public execution.

## Source findings and compatibility traps

All source paths in this document are relative to `backend/src/`.

| Source anchor at the baseline | Observed implementation | Consequence |
| --- | --- | --- |
| `models/platform_binding.rs:4`, `services/platform_binding_service.rs:171` | Onboarding has its own collections, eight-character codes, a ten-minute request, and a 30-day binding. `begin` derives a repeatable code from verified update metadata. | Keep these semantics for child setup. Do not reuse this collection for personal proofs or prolong old requests into personal consent. |
| `services/platform_binding_service.rs:55` | Code HMAC uses `platform-onboarding-binding:v1`. | Reuse the HMAC primitive and normalization discipline, with different personal-purpose domains. Never let old `/confirm` consume a personal challenge. |
| `services/platform_binding_service.rs:159`, `:255`, `:313`, `:357`, `:378` | Begin/review/confirm/reply checks require guide generation and `enabled`; `live_for_sender` also returns `None` for an inactive user. | Personal routing requires a separate resolver and fences. `Option<Binding>` would erase a security-relevant distinction. |
| `services/platform_binding_service.rs:274`, `:313` | Web decision snapshots the existing generation, then confirmation transactionally replaces the onboarding row and invalidates setups. | Preserve explicit replacement and same-chat proof. Personal replacement cannot reuse setup invalidation as its sole revocation. |
| `services/platform_binding_service.rs:27`, `:802` | Human decision spacing is committed independently, including failed guesses. Native reply admission has public sender/profile windows. | Retain failed-attempt spacing, with personal proof namespaces. Personal execution/delivery cannot use public reply admission. |
| `services/platform_bot_config_service.rs:345`, `:378`, `:1184` | Pause and invalidation advance `generation`; guide save/enable also advances it. Guide readiness runs outside the final transaction, which rechecks catalog, admin, sponsor membership, bot, and delivery state. | Preserve legacy guide/child behavior, add independent runtime controls, and reuse the probe-then-recheck pattern. |
| `services/telegram_new_admin.rs:89`, `:96`, `:174`, `:258` | Token configuration invalidates the profile before `getMe`; a different numeric manager is refused. Same-manager configuration keeps the old observation ID, temporarily sets `webhook_ready=false`, and finalizes readiness afterward. Clear also calls `invalidate_manager`. | `invalidate_manager` cannot become synonymous with manager retirement. Same-identity rotation preserves consent/epoch and only suspends delivery while unverified. Clear needs an explicit retirement hook before credential deletion. |
| `services/platform_bot_authority.rs:19`, `:62` | Durable manager/installation markers refuse ordinary personal channel routing even when paused or tombstoned. | Keep this fence. Use the dedicated verified-sender runner, without synthetic `NyxbotChannel` rows or `owner_sender_ids` changes. |
| `handlers/platform_binding.rs:264` and `handlers/telegram_new.rs::webhook_with_service` | The old command handler parses, writes, throttles, and sends onboarding notices itself. | Part 5 owns new command transport. Part 1 exposes purpose-aware command services; the opted-in dispatcher must not call the old handler first and accidentally choose onboarding. |
| `handlers/login_client_context.rs:12`, `routes.rs:2482`, `mw/auth.rs:620` | A common first-party human guard rejects API keys, OAuth clients, service accounts, delegated, and relay tokens. Onboarding is already a delegated GET deny class. | Use both human router placement and the guard. Part 3 must deny the new owner-inventory GET prefix before Part 5 mounts it. |
| `handlers/auth.rs:361` | Registration consults the existing invitation flag and validates an invitation through normal registration. | The public signup tool issues a linking intent only. It cannot issue an invitation or bypass verification/MFA/login. |
| `services/assistant_team_service.rs:137`, `:612`, `:647`; `services/assistant_nyxagent.rs:2275` | NyxBot creation already has one-winner behavior; thread creation and both home selection paths need a new platform-thread exclusion. | Part 2 owns agent/thread creation. Part 1 claims references atomically and imports no transcript. |
| `services/assistant_nyxagent.rs:2516`, `:2897`; `handlers/assistant_nyxagent.rs:1055` | `request_stop` selects an exact active turn but also cancels conversation watches; the HTTP handler additionally cancels async operations and machine work. `force_stop` takes an exact turn but owns settlement transactions. | Neither is a session-aware unlink helper. Extract the precise flag/settlement/captured-job behavior without watch cancellation or broad desktop cancellation. |
| `services/assistant_nyxagent.rs:4059` | Conversation deletion owns a transaction and removes the key, encrypted credential, messages, and related state. | Part 2 calls Part 1's clear helper in that same transaction before deleting the thread. |
| `services/admin_user_service.rs:380`, `:506`; `services/platform_setup_service.rs:678`, `:755` | Account disable/delete writes inactive first, then revokes ordinary authority and retires onboarding state. The onboarding sweeper can recover unfinished retirement. | Part 5 wires personal lifecycle calls at these entry points and adds personal recovery discovery. Do not make unrelated accounts require a MongoDB transaction. |
| `services/api_key_mutation_service.rs:57`, `:70`; `test_utils.rs:257` | Transaction error conversion preserves application errors. Transaction tests require a writable replica set or mongos and fail on unsupported topology. | Use the established transaction wrapper. Do not hide infrastructure failures as unlinked or skip transaction tests when infrastructure is absent. |

## Exact Part 1 ownership and registration handoff

These are the only production files proposed for Part 1 implementation. Existing
in-file tests stay with their file owner.

| File | Part 1 responsibility |
| --- | --- |
| `models/platform_personal_binding.rs` (new) | Binding and reviewed-consent structs; collection constants; redacted debug. |
| `models/platform_link_intent.rs` (new) | Separate intent/challenge structs and collection constants; proof states; redacted debug. Both structs live here, avoiding another unallocated file. |
| `models/platform_bot_runtime_controls.rs` (new) | Runtime controls, public config persistence, and durable exact-stop effect records. |
| `services/platform_personal_binding_service.rs` (new) | Resolution, live fences, reference claim/clear, replacement/revoke/lifecycle helpers, indexes, focused transaction tests. |
| `services/platform_link_intent_service.rs` (new) | Intent issuance, authenticated review/challenges, approval/denial/status, private-chat confirmation, expiry and purpose-bound proof tests. |
| `services/platform_bot_runtime_control_service.rs` (new) | Control reads/writes, emergency stop, manager lifetime hooks, public enable orchestration, recovery and indexes. |
| `handlers/platform_personal_bindings.rs` (new) | Dedicated human DTOs/handlers for links and linking proofs. |
| `services/platform_binding_service.rs` | Session-aware onboarding unlink helper and purpose-aware command seam; preserve existing onboarding APIs/tests. |
| `services/platform_bot_config_service.rs` | Legacy pause/retire integration, stale guide edit rejection, delivery revalidation without personal epoch changes. |
| `models/platform_bot_config.rs` | Optional `manager_runtime_mode` marker with absent/null meaning legacy; retain the existing execution-profile enum. |
| `handlers/platform_binding.rs` | Expose command preparation through services; keep legacy onboarding HTTP behavior. Part 5 owns command selection and transport. |
| `handlers/platform_bots.rs` | Human admin runtime/settings/profile-readiness DTOs and handlers; retain legacy handlers. |

Part 5 registers the three new model modules, three new service modules, and
`handlers/platform_personal_bindings`; it owns `db.rs`, routes, feature flags,
startup/sweep scheduling, transport, and lifecycle entry-point wiring. Part 3
owns the delegated GET denials and readiness implementation. Part 2 owns all
assistant conversation, engine, credential, thread deletion, and exact-stop edits.
No Part 2–5 file is changed by this preparation commit.

Part 5 has specifically assigned itself `services/admin_user_service.rs` and
`handlers/users.rs` if needed for account lifecycle entry points. The personal
retirement business logic stays in Part 1 services. The existing onboarding
retirement sweeper is also outside Part 1's edit allocation; Part 5 owns adding
recovery wiring rather than Part 1 patching `platform_setup_service.rs`.

## Persisted records and indexes

Every `_id` is a UUID v4 string. Telegram numeric identities are validated as
checked positive `i64` values in service proofs and canonical decimal strings
in HTTP DTOs to avoid JavaScript number rounding. Part 5 verifies private chat,
`sender_id == chat_id`, and a non-bot sender. Forwarded-origin refusal applies
to proof/control commands; ordinary linked messages may contain forwarded text.
BSON dates use the project helpers,
including `bson_datetime::optional` for optional dates. Models have no business
logic and never become HTTP responses directly.

### Personal bindings

`platform_personal_bindings` has one document per consent lifetime, with
`platform_config_id`, `manager_bot_id`, `manager_authority_epoch`, sender/chat,
`user_id`, positive `generation`, `state` (`active` or `revoked`),
`revocation_reason`, `consent_version`, `reviewed_policy`, optional
`nyxbot_agent_id`, optional `conversation_id`, optional `active_turn_ref`,
`effect_revision`, and BSON created/updated/consented/revoked dates.

`reviewed_policy` records the disclosure version and the effective access mode
shown at approval. It is evidence of consent, not a frozen service grant or a
Telegram permission editor. Subsequent settings apply through normal NyxBot
policy. Consent and sender resolution do not require a NyxBot to exist.
Agent/thread references start null. Part 2 ensures the ordinary NyxBot before
its turn transaction, validates the actual agent in that transaction, and calls
Part 1's thread claim with that ID. It completes the origin afterward. Deletion
clears the conversation and active-turn references while retaining any agent ID.
Optional hints never authorize a turn or supply an invented placeholder ID.

Proposed replacement representation: retain A's revoked row, and insert B's new
UUID row with the next sender generation in the same transaction. Generation is
monotonic across this manager epoch and sender; revocation changes active state
without renumbering the historical row. New consent after explicit unlink also
uses the next generation. Same-account confirmation replay returns the already
completed result, preserving its binding and thread. There is no TTL or
`expires_at` on consent rows. B never receives A's thread/key references.

The highest-generation row supplies deterministic sender state even after
revocation. An explicit unlink or never-consented sender may resolve unlinked;
owner-disable/delete tombstones resolve unavailable. Do not query only `state=active`
and then infer that absence means unlinked. A deliberate new link can supersede
an unavailable tombstone only after fresh human approval and same-chat confirmation.

| Collection | Index keys and options | Purpose |
| --- | --- | --- |
| `platform_personal_bindings` | Unique `{platform_config_id:1, manager_bot_id:1, manager_authority_epoch:1, telegram_sender_id:1}`, partial `{state:"active"}` | One current owner per private sender. |
| `platform_personal_bindings` | Unique same identity keys plus `{generation:-1}` | Monotonic sender generation and latest-state lookup, including tombstones. |
| `platform_personal_bindings` | `{user_id:1, state:1, updated_at:-1, _id:-1}` | Owner inventory and bounded account retirement. No unique user/manager index; several Telegram identities per person are allowed. |
| `platform_personal_bindings` | Unique `{conversation_id:1}`, partial `{conversation_id:{$type:"string"}}` | A personal conversation belongs to one consent binding. |
| `platform_personal_bindings` | `{platform_config_id:1, manager_authority_epoch:1, state:1, _id:1}` | Bounded manager retirement/stop recovery. |

The compound generation index is also the serialization check for a proposed
successor. All resolver/link transactions write the common runtime-control row,
so a lookup that finds no binding still conflicts with a concurrent confirmation.
The partial unique index is a second guard, not the only absence-race defense.

### Intents and fresh challenges

Use `platform_personal_link_intents` and `platform_personal_link_challenges`,
both declared in `models/platform_link_intent.rs`. Old onboarding readers never
see them. Intent purpose is the closed versioned internal value `personal_nyxbot_v1`;
the HTTP purpose projection is `personal_nyxbot`.

An intent stores the verified profile/manager/epoch/sender/chat/update identity,
`token_lookup_hmac`, contextual `token_hmac`, state (`pending`, `consumed`,
`cancelled`, `expired`), optional authenticated claimant, creation/expiry/purge
dates, and completed binding ref.
It lasts 24 hours. The raw `nyx_pli_` token contains 32 OS-random bytes, lives in
`Zeroizing<String>`, and is returned only at issuance. Duplicate inbound
updates return `AlreadyIssued` without issuing a second token. A lost issuance
reply requires a fresh `/link`; no raw token is stored for replay.

A challenge stores intent ID, the same verified transport/epoch, fixed reviewed
user, `code_lookup_hmac`, contextual `code_hmac`, reviewed consent version/digest,
reviewed previous binding ID/generation/state, replacement acknowledgement in
the review digest, state (`pending_review`,
`awaiting_chat_confirmation`, `confirmed`, `denied`, `cancelled`, `expired`),
completion ref, and dates. Authentication creates a fresh ten-minute challenge.
The deadline is the earlier of issuance plus ten minutes and intent expiry.
Issuing a fresh challenge cancels prior live challenges for that intent without
extending the intent. A challenge cannot switch accounts after review.

Contextual HMAC domains are `platform-personal-intent:v1` and
`platform-personal-challenge:v1`; the approval digest uses
`platform-personal-consent:v1`. Bind purpose, record ID, manager/epoch, sender,
chat, and, for challenges, reviewed user and consent digest with an unambiguous
length-prefixed encoding. Lookup fields use separate
`platform-personal-intent-lookup:v1` and
`platform-personal-challenge-lookup:v1` domains over the raw token or normalized
code. Load by the lookup HMAC, then verify the contextual HMAC against the row
and caller's verified facts. This makes code-only `/confirm` lookup possible
without removing context binding. Never use the onboarding domain. The personal
manual code proposal is `P-XXXX-XXXX`, with eight Crockford characters after the purpose
prefix. Its strict normalizer requires that prefix, so an old onboarding code
cannot be selected accidentally. Use bounded attempts, independent failed-attempt
rate admission, and at least 750 ms decision spacing.
Raw token/code/HMAC/ciphertext never enters logs, audits, or Debug. Status never
returns a token or manual code. Only the explicit approval response returns the
fresh manual code once.

| Collection | Index keys and options |
| --- | --- |
| `platform_personal_link_intents` | Unique `{token_lookup_hmac:1}`; unique `{platform_config_id:1, manager_authority_epoch:1, telegram_sender_id:1, initiation_ref:1}`; `{platform_config_id:1, manager_authority_epoch:1, telegram_sender_id:1, state:1}`; `{claimant_user_id:1, state:1}`. |
| `platform_personal_link_challenges` | Unique `{code_lookup_hmac:1}`, partial `{code_lookup_hmac:{$type:"string"}}`; `{intent_id:1, state:1}`; `{reviewed_user_id:1, state:1}`; `{platform_config_id:1, manager_authority_epoch:1, telegram_sender_id:1, state:1}`. |
| Both proof collections | `{expires_at:1, state:1}` for bounded expiry; `{purge_after:1}` with `expireAfterSeconds:0` for terminal housekeeping. Live expiry is always checked before authority, independent of TTL timing. |
| `platform_personal_decision_spacing` | `_id` is the actor UUID; server-clock timestamp, same independent spacing pattern as onboarding. Fixed windows use existing coordination storage with personal-purpose namespaces. |

On first authenticated challenge issuance, atomically claim the intent for that
human. Another account cannot reuse it. Switching accounts requires cancelling
and starting a new intent in the initiating chat. Refresh/status cannot approve.
The intent stores its current challenge ID; stale denial/cancellation cannot
terminate a newer challenge. Terminal replay reports completion without
reactivating a subsequently revoked binding.
Forwarded codes, another chat, groups, old onboarding codes, and expired proofs
cannot complete personal consent. Intent possession alone conveys no account
authority. Signup/email verification may consume over ten minutes before any
challenge is issued; invitation handling remains ordinary registration.

### Runtime controls and durable cancellation

`platform_bot_runtime_controls` has UUID `_id`, stable `platform_config_id` and
`manager_bot_id`, `manager_authority_epoch`, `personal_control_revision`,
`public_policy_revision`, `personal_enabled`, `public_enabled`,
`emergency_stopped`, optional `public_configuration`, effect counter, retirement
progress, and created/updated dates. Missing row, unknown mode/version, or missing
required identity is closed. Explicit initialization starts with disabled lane
defaults and applies requested changes in one transaction. No read path silently
creates or enables controls.

Use unique `{platform_config_id:1, manager_bot_id:1}` and a bounded retirement
progress index. `public_configuration` stores the source/profile, explicit
sponsor, exact `Credits` budgets, and verified bounded runtime policy. It does
not change legacy child setup limits. Add optional
`PlatformBotConfig.manager_runtime_mode = "public_personal_v1"`; absent/null
retains legacy behavior. Keep `PlatformExecutionProfile::PlatformOnboarding`.

Declare `platform_personal_stop_effects` and its plain record in the same new
runtime-controls model file. Each row captures one exact turn/origin, reason,
Part 2's captured exact machine jobs/cancel identities, lease/retry state, and
dates. It contains no message body, key material, or tool arguments. Unique
`{conversation_id:1, turn_id:1, binding_id:1, binding_generation:1}` makes retries
idempotent; `{completed:1, retry_at:1, _id:1}` supports bounded recovery.
No TTL can remove unfinished effects. Part 5 schedules Part 1's recovery service;
Part 2 applies exact cancellation/settlement after commit.

## Proposed service signatures and transaction ownership

Use the shared types from Part 5's `1b5aea084`, amended by Part 2's
`338d92f5b`. `PersonalBindingAuthority` holds manager/sender/chat, binding and
personal control revisions, and owner, without requiring an agent. Sender
resolution carries that authority and optional existing agent/conversation hints.
`PlatformTurnOrigin` combines the authority with the validated real agent ID and
`PlatformOriginSource` (`Ingress` or fresh `ThreadEvent`). The origin is completed
at turn claim, not consent or sender resolution. These replace the candidate's
premature agent requirement; they do not add a second authority API.

The affected identity producer signatures follow Part 2's proposal:

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

`PersonalThreadClaim` returns `nyxbot_agent_id` and `conversation_id`.
`resolve_sender(db, session, update)` retains the candidate's session ownership
and `Unlinked/Personal/Unavailable` result distinction. The only change to its
personal payload is the pre-thread authority and optional hints.
`runtime_policy(db, profile_id)` still returns `Option<ManagerRuntimePolicy>`.
Shared values have no serde representation. Persistent models do not import the
service-contract module; owning services map persisted fields into those values.
The existing `async_trait` dependency supports the small producer traits.
Infrastructure, decoding, and transaction failures remain `Err(AppError)`.

Part 5 owns ingress receipt/classification transactions and passes the session
into `resolve_sender`. Private-chat and non-bot sender checks happen before this
API; forwarded content does not change sender authority. Groups receive the
deterministic private-chat handoff. Check common
manager/mode/emergency authority first, resolve consent state next, and apply
personal controls only to a personal-bound sender. A disabled public lane does
not block personal; a paused personal lane does not block public for an unlinked
sender. Read the latest consent state without discarding inactive owners.
`OwnerUnavailable` persists through account-retirement tombstones, personal pause
maps to `PersonalDisabled`, and changed binding authority maps to `OriginRevoked`.
No catch-all, `unwrap_or(None)`, or alternate-audience retry may choose public.

All successful resolver/classification paths write the common runtime-control
row, including a no-binding lookup, so confirmation and audience selection
serialize. Personal resolution also conditionally writes the active binding and
person rows. A committed receipt retains its audience after later consent changes.
The personal fence conditionally increments effect counters on controls, binding,
active person, and durable manager authority rows. It compares manager identity
and epoch, personal control revision, binding ID/generation/state, user,
and private sender/chat. Part 2 validates the actual agent and exact turn/source.
Public revision and guide generation/enabled are absent.
Part 3 validates the exact live turn through Part 2 before effect admission.
Ordinary web/API turns without a platform origin incur no platform lookup.

`claim_binding_thread` receives the actual validated ordinary NyxBot ID and
returns both authoritative IDs. Claim the pointer before inserting a new
conversation/key. If an existing winner differs from already inserted candidates,
Part 2 aborts and retries instead of committing an orphan. Existing binding agent
hints must agree with the active owner's ordinary NyxBot.

Turn claim writes the complete origin and exact turn reference in the same
transaction as active-turn admission. Clear helpers compare only exact owner,
binding ID/generation, and old turn/thread reference. They do not require live
consent, current personal control revision, current controls, or engine readiness.
Deleting or settling a revoked binding remains legal. An old clear cannot erase
a newer pointer. Part 2 performs live agent validation and any origin/source
comparison separately from those clear operations.

Part 2 uses existing `ensure_nyxbot(db, owner)` before opening its turn
transaction, then validates the actual agent within it. Losing admission may
leave an ordinary empty NyxBot, but no thread, credential, transcript, home
pointer, or executable turn. No transaction-aware agent-creation helper is
required during consent confirmation.

### Proof and lifecycle services

The public preview has no actor argument and returns only sanitized
intent-possession metadata. Every account-affecting HTTP operation derives
`actor` from the first-party human guard, never an input account ID.

```rust
// Own a transaction unless explicitly marked as joining the caller session.
pub(crate) async fn begin_personal_intent(
	db: &Database, hmac_key: &[u8], update: &VerifiedManagerUpdate,
) -> AppResult<IntentIssue>;

pub enum IntentIssue {
	Issued { intent_id: String, token: Zeroizing<String>, expires_at: DateTime<Utc> },
	AlreadyIssued { intent_id: String },
}

pub async fn preview_personal_intent(
	db: &Database, hmac_key: &[u8], token: &str,
) -> AppResult<PersonalIntentPreview>;

// HTTP POST /review: atomically claim intent and issue fresh review challenge.
pub async fn review_personal_intent(
	db: &Database, hmac_key: &[u8], actor: &str, token: &str,
) -> AppResult<PersonalConsentReview>;

// Manual code is generated here and returned once, after explicit approval.
pub async fn approve_personal_challenge(
	db: &Database, hmac_key: &[u8], actor: &str,
	challenge_id: &str, review_digest: &str,
) -> AppResult<PersonalApproval>;

pub async fn deny_personal_challenge(
	db: &Database, actor: &str, challenge_id: &str,
) -> AppResult<PersonalChallengeStatus>;

pub async fn personal_challenge_status(
	db: &Database, actor: &str, challenge_id: &str,
) -> AppResult<PersonalChallengeStatus>;

pub(crate) async fn confirm_personal_from_chat_in_session(
	db: &Database, session: &mut ClientSession, hmac_key: &[u8],
	update: &VerifiedManagerUpdate, code: &str,
) -> AppResult<PersonalLinkCompletion>;

pub async fn list_owned_personal_bindings(
	db: &Database, actor: &str, before_id: Option<&str>,
) -> AppResult<PersonalBindingPage>;

pub async fn revoke_owned_personal_binding(
	db: &Database, actor: &str, binding_id: &str, generation: i64,
) -> AppResult<RevocationResult>;

pub(crate) async fn revoke_personal_binding_in_session(
	db: &Database, session: &mut ClientSession,
	binding_id: &str, generation: i64, reason: PersonalRevocationReason,
) -> AppResult<RevocationResult>;

pub(crate) async fn unlink_sender_in_session(
	db: &Database, session: &mut ClientSession, update: &VerifiedManagerUpdate,
) -> AppResult<RevocationResult>;

pub(crate) async fn retire_personal_owner_in_session(
	db: &Database, session: &mut ClientSession, owner: &str,
	deleted: bool, after_binding_id: Option<&str>,
) -> AppResult<RetirementBatch>;

pub(crate) async fn retire_manager_in_session(
	db: &Database, session: &mut ClientSession,
	manager: &ManagerAuthority, reason: ManagerRetirementReason,
) -> AppResult<()>;
```

Preview and status do not mutate consent. Review, approval, denial, and owner
revoke own their transactions. Internal session variants let Part 5 compose
command admission with proof mutation without nesting transactions. Confirmation
above explicitly joins Part 5's command transaction. Intent issuance needs a
matching `_in_session` variant when composed with an ingress receipt; Part 3's
signup tool passes the server-verified public context through a Part 5-issued
admission reference, never caller-supplied bot/sender/payer identifiers.
Failed-attempt spacing/rate writes happen independently before the decision
transaction, so rejected decisions do not refund guesses.

The review digest covers disclosure version, owner, effective mode summary,
manager/epoch/sender, intent/challenge identity, and the prior binding ref/state.
For replacement, the UI must explicitly acknowledge replacement before enabling
Approve. The reviewed `replaces_existing_binding` flag and fixed disclosure
version are part of the digest. Approval of that digest is the acknowledgement;
no second client-controlled permission mode or grants enter the request.
Repeated approval returns awaiting-confirmation metadata with no raw code.
A lost approval response requires a fresh review/challenge; status never
reconstructs the code. Code issuance does not extend the original challenge
expiry. The code lookup/HMAC fields are null before approval, with the unique
lookup index partial on string values.

Approval snapshots the authenticated actor, exact consent/disclosure digest,
current access mode summary, replacement acknowledgement, and previous binding
ref/state. Confirmation derives the actor from the approved row, rechecks live
person and manager epoch/controls, compares the reviewed previous binding, and
consumes the challenge and intent atomically with consent. A repeated matching
confirmation returns its stored completion ref; it never generates another
binding/thread. Changed prior authority requires fresh review.

Replacement revokes A, persists A's exact stop effects, and inserts B with null
agent/thread/turn references. It cancels other pending personal
intents/challenges for the
sender, and asks Part 3 to revoke that sender's public session in the same
transaction. The Part 3 hook must only revoke/queue cleanup locally; external
session deletion occurs after commit. Confirmation cannot be declared complete
with a still-usable public session. No public transcript, memory, key, or pending
action is adopted.

`unlink_sender_in_session` is the broad reserved `/unlink`: it revokes personal
and onboarding purposes for this exact manager/sender, and cancels pending
proofs even if no active binding exists. Add
`platform_binding_service::unlink_sender_in_session(db, session, profile_id,
manager_bot_id, sender, chat)` to reuse existing onboarding setup
invalidation without requiring guide `enabled`. Human personal unlink affects
only the selected personal binding; the old onboarding inventory/revoke keeps
its existing scope. Manager pause must never prevent a person revoking consent.

For opted-in new mode, `/link` creates a personal intent. `/confirm P-...` selects
only personal confirmation. `/onboarding`, `/setups`, and legacy unprefixed
onboarding confirmation remain separate setup entry points under Part 5's
command classification. Existing managers retain the old `/link` meaning until
explicit mode opt-in. Purpose is a closed server command choice; it cannot name
an owner, grant, or generic handler. Ordinary account signup does not implicitly
create an onboarding binding or grant child setup authority.

Owner retirement uses bounded batches with live inactive/missing-user checks
and durable progress. The inactive person row is the immediate fence. Part 5
must also gate account reactivation: before changing `is_active` back to true,
finish cancellation/revocation of that account's old personal proofs/bindings
and exact Stop recording through Part 1's retirement batches. Refuse or leave
inactive if that work fails. Existing `set_user_active` otherwise permits
reactivation before retirement completes, and the old setup helper then returns
Conflict. Merely copying that sequence would allow old personal consent to
revive. No additional user-model field is required by this proposal. Personal
retirement is a no-op for accounts with no personal rows; ordinary accounts
retain standalone-compatible behavior. Fresh linking after reactivation requires
fresh proof and cannot restore a retired binding.
Manager retirement immediately advances epoch, disables both lanes, sets
emergency stop, and marks durable progress. Subsequent bounded batches revoke
bindings/proofs and collect exact stop effects. The epoch blocks every old origin
before that walk completes. Missing/retired manager markers stay closed.

### Required Part 2 exact-stop helper

Adopt Part 2's `338d92f5b` correction to the candidate. Part 1 invokes Part 2
inside its revoke/replace/control transaction, then persists the returned effects:

```rust
pub enum ExactStopOutcome {
	Captured(CapturedPlatformStop),
	NoMatchingActiveTurn,
}

async fn stop_exact_platform_turn(
	&self, db: &Database, session: &mut ClientSession,
	origin: &PlatformTurnOrigin, turn: &TurnRef,
) -> AppResult<ExactStopOutcome>;

// Part 1 calls this before committing the same transaction.
pub(crate) async fn enqueue_exact_stop_in_session(
	db: &Database, session: &mut ClientSession, effects: &CapturedPlatformStop,
) -> AppResult<String>;

// Part 2 applies effects after commit and during durable recovery.
pub async fn apply_exact_platform_stop(
	state: &AppState, effects: &CapturedPlatformStop,
) -> AppResult<ExactStopProgress>;
```

`TurnRef` remains conversation plus turn ID; the origin supplies owner identity.
Part 2 matches the exact persisted origin/source and active turn, writes Stop,
and captures exact current-turn runtime/node/job identities in the same session.
A mismatch returns `NoMatchingActiveTurn`. `Captured` does not claim that a durable
recovery record already exists. Part 1 maps the capture into its plain
`platform_personal_stop_effects` model and inserts it before committing revocation
or replacement. The exact-turn/binding unique key deduplicates recovery.
Part 1 leases pending records and reconstructs the capture for Part 2's runner.
No second queue is owned by Part 2. No provider or node request runs inside the
transaction. Independently committed tasks, watches, cards, and proxy work are
excluded from capture.

After commit, workers observe Stop and normal exact settlement remains valid.
Part 5 schedules Part 1's bounded queue recovery; Part 1 leases IDs and asks
Part 2 to apply exact cancellation and forced settlement using the established
grace period. Node cancellation requires exact-turn/job capability. No old-node
broad fallback or conversation-wide watch cancellation is allowed.
Pending cards and committed tasks remain available as the user's work in the app.

Part 2 distinguishes revoked Telegram-origin execution from a later normal
web/event turn on the same saved thread. The former cannot start new tool/model
effects. The latter remains usable. New card/watch/event continuations capture a
live destination at their own admission while linked, or run web-only after
unlink. Async results retain their original destination generation. Opening the
thread on the web does not attach a Telegram destination.

## Runtime policy, readiness, and configuration APIs

Adopt candidate `ManagerRuntimePolicy`, `ManagerAuthority`, and
`PublicPolicySelection` without a competing service type. Public policy has the
explicit sponsor owner, NyxAgent profile, and exact total/sender daily `Credits`.
The HTTP `source: "nyxagent"` is currently a closed discriminator, not an
arbitrary service or provider URL. Model allowlists and runtime bounds come from
Part 3's validated profile contract. They do not alter personal permissions.

Part 1's proposed admin service operations are:

```rust
pub struct ExpectedRuntimeRevisions {
	pub manager_authority_epoch: i64,
	pub personal_control_revision: i64,
	pub public_policy_revision: i64,
}

// A service patch, mapped from a strict handler DTO. Nested policy patches
// preserve omitted fields; null is explicit Clear only where documented.
pub struct RuntimePatch {
	pub initialize: bool,
	pub expected_profile_generation: Option<i64>,
	pub expected: ExpectedRuntimeRevisions,
	pub personal_enabled: Option<bool>,
	pub public_enabled: Option<bool>,
	pub emergency_stopped: Option<bool>,
	pub public_policy: PolicyPatch,
}

pub enum PolicyPatch {
	Unchanged,
	Clear,
	Update(PublicPolicyPatch),
}

pub async fn update_manager_runtime(
	state: &AppState, actor: &str, profile_id: &str, patch: &RuntimePatch,
) -> AppResult<ManagerRuntimeUpdate>;

pub async fn clear_emergency_stop(
	db: &Database, audit_key: &[u8], actor: &str, profile_id: &str,
	expected: &ExpectedRuntimeRevisions,
) -> AppResult<ManagerRuntimePolicy>;
```

`update_manager_runtime` owns the final transaction. The patch only changes
explicit fields. A public-only save/Enable cannot change personal controls or
clear emergency stop. `emergency_stopped:true` is a stronger whole-manager stop;
`false` is refused here and requires the dedicated clear endpoint. A no-op patch
preserves revisions. A changed public policy or enabled flag advances public
revision; a changed personal flag advances personal control revision. All three
expected values are compared atomically. The existing whole-profile Pause calls
a session-aware stop operation inside its config transaction.

Part 3's candidate probe is accepted:

```rust
// PublicTurnProducer, outside any MongoDB transaction.
async fn probe_public_readiness(
	&self, db: &Database, reviewed: &ManagerRuntimePolicy,
) -> AppResult<PublicReadiness>;

// Needed additions: local-only, no provider/network effects.
async fn fence_public_readiness(
	&self, db: &Database, session: &mut ClientSession,
	reviewed: &ManagerRuntimePolicy, evidence: &PublicReadinessEvidence,
) -> AppResult<FenceOutcome>;

async fn revoke_public_sender(
	&self, db: &Database, session: &mut ClientSession,
	manager: &ManagerAuthority, sender_id: i64, chat_id: i64,
) -> AppResult<()>;
```

Profile discovery also needs a Part 3 producer method behind the agreed admin
POST, for example `discover_public_profiles(&self, db: &Database,
reviewed: &ManagerRuntimePolicy) -> AppResult<PublicProfileDiscovery>`.
Its response is a bounded public-safe profile list and default selection. No
query can choose an arbitrary provider endpoint, payer, or model operation.
Part 3 owns these service result types; Part 1 maps them into dedicated DTOs.

`PublicReadiness::Ready` carries the candidate's exact manager/epoch, public
revision, canonical policy digest, sponsor, upstream build/profile, allowed model
operations, execution bounds, and checked/expiry times. Add or locally bind the
catalog/grant/pricing/cleanup evidence needed to recheck those claims. Although
the candidate's Rust fields are public, only trusted producer code constructs
successful evidence. No HTTP request accepts it or `ready:true`. Evidence is
short-lived and bound to the exact target policy revision, not reused after a
later save. `Unavailable` is a safe refusal; infrastructure errors remain `Err`.

Probe and sponsor reserve/settlement run before the config transaction, even if
final enable later conflicts. The transaction rechecks live admin/person,
sponsor authorization, manager identity/epoch/current observation/webhook
readiness, expected revisions, and Part 3's local readiness fence. A failed Enable
leaves existing policy and controls unchanged. Saving a paused draft validates
locally and makes no readiness claim. Readiness DTOs contain bounded reasons and
capability/version metadata, no credentials or model responses.

The sender-public-session hook revokes and queues cleanup in the caller's link
transaction. External cleanup runs after commit. No active public session means
success without waiting for upstream proof or public readiness. Database failures
abort linking. This hook ships with Part 3's early local security work so personal
linking does not depend on public runtime activation.

Personal enablement needs live manager authority and enforcing replicas. It does
not wait for public proof, sponsor readiness, or public budgets. Owner funds,
engine readiness, and owner-pool capacity belong to Part 2 admission, and their
failures never choose public. Omitted public fields preserve saved values;
`public_policy:null` clears only alongside explicit `public_enabled:false`.
Null booleans, revisions, profile, sponsor, and budgets are rejected. New-draft
budget defaults are `"1"` total and `"0.1"` per sender; existing limits survive.
Per-sender budget cannot exceed total. No turn request chooses its payer.

### Initial creation behind the first Enable

The candidate needs a concrete correction: `supported` describes server/rollout
capability, not whether a controls row exists. A capable, enforcing deployment
returns `supported:true, runtime:null` for an uninitialized manager. A feature
gate or unsupported deployment returns `supported:false, runtime:null`. Neither
case fabricates an epoch/revision or enabled default. The manager detail already
provides the current legacy profile generation for an explicit conversion fence.

Use the same `PUT /admin/platform-bots/{id}/runtime` for first initialization.
The request carries `initialize:true`, `expected_profile_generation` from the
manager detail, and zero for all three expected runtime values. Zero is an HTTP
creation precondition meaning that no controls row exists. It is never stored
or accepted in an origin. Normal updates require positive expected values and
omit `initialize` or set it false. Initial creation fails if a row already exists,
if legacy generation changed, or if manager/admin/sponsor authority changed.
A concurrent second initialization receives 409 and refreshes; it cannot reset
controls, overwrite emergency stop, or replace the sponsor.

The server builds a candidate with epoch 1 and initial positive revisions, using
`PlatformBotConfig.funding_owner_user_id` as the sponsor unless a separately
validated Advanced change explicitly replaces it. A fresh public Enable probes
that candidate before opening a transaction. The final transaction conditionally
updates the legacy profile generation/mode marker, inserts controls, validates
local evidence, and applies the requested lane changes atomically. Missing lane
fields start false. Enabling public does not silently enable personal. A failed
probe or transaction creates no controls and does not convert the profile.

This gives Part 4 one three-field Enable action for an existing manager. No
additional product setup screen or initial GET write is required. An explicit
Save paused can initialize disabled controls without a public probe. Existing
onboarding links, child setup limits, and legacy execution-profile enum remain
intact. All new-mode operations check the rollout gate.

Control changes, explicit emergency clear, consent replacement, and revocation
use `audit_service::log_actor_event_in_session` in their owning transaction, with
`AuditActor` derived from the verified human or server action. The existing
append path supplies the tamper-evident chain. Audits contain bounded reason,
record IDs, revisions, and safe policy summaries, with no raw proof or message.

| Operation | Runtime changes | Legacy/consent behavior |
| --- | --- | --- |
| Public source/budget save, enable, or guide-only pause | Advance public revision; affect public enabled/config only. | Personal consent and personal revision remain valid; preserve child profile semantics. |
| Personal pause | Disable personal; advance personal revision; queue exact Stops using durable bounded progress. | Keep consent and web history; old origins/destinations stay invalid after resume. |
| Personal enable | Explicit expected-revision write; advance personal revision. | Fresh admissions use the new revision; no old pending turn is replayed. |
| Existing whole-profile Pause | Set emergency stop and both lanes disabled; advance both revisions in its existing config transaction. | Keep old pause/generation behavior for setups; consent records survive pause. |
| Explicit clear emergency stop | Audited admin-only action, clear stop with both expected revisions/epoch. | Leaves both lanes disabled; enabling is a separate explicit action. Legacy Resume/guide Enable cannot clear stop. |
| Same-manager token rotation/reverification | Keep epoch, consent, pending intents/challenges, and personal revision. | Current delivery waits for verified credential observation. Existing profile invalidation may still advance legacy generation. |
| Clear, different identity, delete, or retire | Advance epoch and stop both immediately; bounded revocation/cleanup follows. | Retain tombstones; no un-conversion or old-manager fallback. |

For opted-in profiles, the legacy `PUT .../guide` payload receives an actionable
409 rather than modifying new controls. Existing pause/delete remain stronger
whole-profile controls. Public-only requests use the new runtime endpoint.
Targeted MongoDB updates preserve optional fields through legacy typed writers;
all replicas must still be upgraded before mode activation.

## Human HTTP paths and DTO fixtures

Adopt Part 5's HTTP paths. All below include `/api/v1`. The preview is the only
new unauthenticated endpoint. It requires intent possession and returns bounded
manager/sender landing metadata and invitation policy, with no account identity,
binding inventory, prior owner, grants, or authenticated state. It cannot create
a challenge or consent. Rate-limit it independently and return no-store.
All other linking routes use `require_first_party_human`, rejecting keys,
delegated/relay tokens, OAuth-client-attributed sessions, and service accounts.
Admin routes additionally require a live platform admin, not operator or
sponsor-org administrator alone.

| Method and path | Request and response |
| --- | --- |
| `POST /platform-personal-links/preview` | `{intent_token}`; sanitized possession-only metadata. |
| `POST /platform-personal-links/review` | `{intent_token}`; claim intent for current human and issue a fresh review challenge. |
| `POST /platform-personal-links/approve` | `{challenge_id, review_digest}`; explicit approval returns the one-time confirmation command. |
| `POST /platform-personal-links/deny` | `{challenge_id}`; deny only the human's current challenge and cancel its intent. |
| `POST /platform-personal-links/status` | `{challenge_id}`; same reviewing account only, no code/token. |
| `GET /platform-personal-bindings` | Owner inventory, `?before_id=<uuid>`, fixed limit 50. |
| `POST /platform-personal-bindings/{id}/revoke` | `{generation}`; idempotent personal-only revoke and exact Stop recording. |
| `GET /admin/platform-bots/{id}/runtime` | Capability, controls, saved policy, bounded readiness metadata. |
| `PUT /admin/platform-bots/{id}/runtime` | Unified revision-fenced patch; explicit first initialization as defined above; public Enable orchestrates probe and commit. |
| `POST /admin/platform-bots/{id}/runtime/clear-emergency-stop` | Expected epoch and both revisions; audited clear, lanes stay disabled. |
| `POST /admin/platform-bots/{id}/public/profiles` | Bounded discovery for the authorized saved/candidate sponsor and policy; may incur sponsor spend. |
| `POST /admin/platform-bots/{id}/public/readiness` | Exact candidate probe; safe readiness result, no config write. |

Part 3 adds the exact prefix `platform-personal-bindings` to
`delegated_read_denied_path` before Part 5 mounts the inventory GET. Runtime GET
and all descendants of `admin/platform-bots` are already covered by the `admin`
deny class; assert that exact path in auth tests. No new challenge/status GET is
introduced. Existing `onboarding` and `assistant` deny classes stay. There is no
state-changing GET or GET dispatching a readiness probe.

The browser landing path is `/link/telegram#intent=<opaque token>`. Part 4 keeps
only this intent through same-origin signup/login, using existing invitations,
email verification, and MFA. Public signup tools manufacture no invitation or
account. Token-bearing API requests use POST bodies; no raw token appears in
API paths, referrers, analytics, or logs. Linking responses send
`Cache-Control: no-store` and `Referrer-Policy: no-referrer`. Request DTOs reject
unknown fields and wrap raw proofs in `Zeroizing`.

The following JSON fixtures use Part 5's candidate field names. Angle-bracket
IDs are fixture placeholders for server-generated identifiers, not accepted
actor/owner input. Telegram IDs are decimal strings; money is an exact decimal
credit string with up to 12 fractional digits; nullable fields are explicit.

Preview response for `{ "intent_token": "<opaque token>" }`:

```json
{
	"purpose": "personal_nyxbot",
	"status": "pending",
	"manager": {"id": "<manager-id>", "username": "nyx_demo_bot", "display_name": "NyxBot"},
	"sender": {"id": "123456789", "display_name": "Alice"},
	"expires_at": "2026-10-16T08:00:00Z",
	"invitation_required": true,
	"requires_same_chat_confirmation": true
}
```

Review request uses the same intent token; response:

```json
{
	"challenge_id": "<challenge-id>",
	"status": "review",
	"expires_at": "2026-10-15T08:10:00Z",
	"review_digest": "<server digest>",
	"account": {"id": "<acting-user-id>", "display_name": "Alice"},
	"permission_mode": "nyxmode",
	"billing": "personal",
	"replaces_existing_binding": false,
	"disclosures": {
		"telegram_transport": true,
		"existing_nyxbot_access": true,
		"can_approve_sensitive_actions": true,
		"started_work_may_continue_after_unlink": true
	}
}
```

The disclosure version also explains shared NyxBot memory, personal billing,
platform-operated Telegram transport without end-to-end encryption, and that
Telegram codes can approve sensitive machine or saved-login actions. Part 4
presents these as fixed consent copy; no permission editor is added. The digest
covers that exact copy version and any replacement warning.

Approve request is `{ "challenge_id": "<challenge-id>", "review_digest":
"<server digest>" }`; only its first successful response returns the command:

```json
{
	"challenge_id": "<challenge-id>",
	"status": "awaiting_chat_confirmation",
	"expires_at": "2026-10-15T08:10:00Z",
	"binding": null,
	"confirmation_command": "/confirm P-6MX4-W2P9"
}
```

Status POST uses `{ "challenge_id": "<challenge-id>" }`. Before confirmation it
returns the same status with `confirmation_command:null`. Completed response:

```json
{
	"challenge_id": "<challenge-id>",
	"status": "confirmed",
	"expires_at": "2026-10-15T08:10:00Z",
	"binding": {"id": "<binding-id>", "generation": 1},
	"confirmation_command": null
}
```

Returning from login, preview, review, and status never approve. `expires_at`
above is the challenge deadline, not consent expiry. Terminal status never
reactivates a revoked binding. Deny request uses the challenge ID and returns
`{"status":"denied"}`. Closed HTTP statuses are `review`,
`awaiting_chat_confirmation`, `confirmed`, `denied`, `cancelled`, and `expired`.
Internal proof states map explicitly to these names.

Owner inventory and revoke fixtures:

```json
{
	"bindings": [{
		"id": "<binding-id>",
		"generation": 1,
		"manager": {"id": "<manager-id>", "username": "nyx_demo_bot", "display_name": "NyxBot"},
		"sender": {"id": "123456789", "display_name": "Alice"},
		"state": "active",
		"conversation_id": null,
		"consented_at": "2026-10-15T08:05:00Z",
		"revoked_at": null
	}],
	"next_before_id": null
}
```

Revoke `{ "generation": 1 }` returns
`{ "revoked": true, "work_may_continue_in_app": true }` after durable revocation
and exact Stop recording. Repeated same-generation revoke is idempotent;
foreign bindings are not-found-shaped. Several identities can belong to one
person, each with its own binding/thread. The list includes no replacement
account IDs, keys, or other people's history. Paused delivery does not revoke
consent or change its state to revoked. Durable binding models have no expiry;
HTTP omits an expiry rather than presenting the challenge expiry as consent TTL.

Corrected runtime GET on a capable server before initialization:

```json
{
	"contract_version": 1,
	"supported": true,
	"runtime": null,
	"public_readiness": {
		"status": "unavailable", "reason": "mode_disabled",
		"checked_at": null, "expires_at": null
	}
}
```

For an unsupported or gated deployment the same shape has `supported:false`.
Configured response adopts the candidate:

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
			"source": "nyxagent", "profile": "nyxagent/chat",
			"sponsor_owner_user_id": "<existing-sponsor-org-user-id>",
			"total_daily_credits": "1", "sender_daily_credits": "0.1"
		}
	},
	"public_readiness": {
		"status": "unavailable", "reason": "public_not_ready",
		"checked_at": null, "expires_at": null
	}
}
```

Initial three-field public Enable, with existing sponsor retained by omission:

```json
{
	"initialize": true,
	"expected_profile_generation": 7,
	"expected_manager_authority_epoch": 0,
	"expected_personal_control_revision": 0,
	"expected_public_policy_revision": 0,
	"public_enabled": true,
	"public_policy": {
		"source": "nyxagent", "profile": "nyxagent/chat",
		"total_daily_credits": "1", "sender_daily_credits": "0.1"
	}
}
```

Existing controls use the candidate's positive expected epoch/revisions and omit
initialization fields. A partial policy edit preserves saved sponsor and omitted
limits. Advanced sponsor changes recheck authorized organization membership and
active funding owner; they never default to the acting admin. First creation
uses positive epoch/revisions 1; the zero preconditions are not returned as live
runtime authority. `public_policy:null` requires explicit public disable and
never affects personal controls.

Error responses use existing `AppError`/`ErrorResponse`, without inventing a
second numeric registry. Invalid/unknown/foreign proof is 404; a known owned
expired challenge status can return `status:"expired"` while approval is refused.
Stale review, previous binding drift, revision conflict, emergency-stopped
enable, or opted-in legacy guide write is 409 with fixed recovery instructions.
Wrong credential class is the existing unsupported-credential 403; invalid
inputs are 400 and attempt limits 429. Infrastructure errors retain sanitized
5xx behavior. Public readiness failure is a safe unavailable result on the probe
endpoint and a refusal to enable on the write endpoint. These statuses never
alter an ingress audience already committed by Part 5.

Candidate stale review response, HTTP 409, using the existing error envelope:

```json
{
	"error": "conflict",
	"error_code": 1004,
	"message": "Conflict: Personal linking review changed; review again"
}
```

Unknown or foreign proof response, HTTP 404:

```json
{
	"error": "not_found",
	"error_code": 1003,
	"message": "Not found: Personal linking request unavailable"
}
```

## Focused implementation tests and integration obligations

These tests are proposed, not run. Part 1's new service tests use
`connect_transaction_test_database`, with `NYXID_TEST_DATABASE_URL` pointing to
a transaction-capable disposable database. Use barriers/command hooks for races
and stored BSON timestamps for expiry boundaries, avoiding ten-minute sleeps.
Part 5 owns any shared fixture changes and the single host build slot.

| Owner and focused test | Evidence required |
| --- | --- |
| Part 1, `personal_intent_survives_long_signup_then_fresh_challenge` | An intent older than ten minutes remains usable before 24h; challenge expiry starts after authentication and does not extend the intent. Exact deadline and expired/replayed token are refused. |
| Part 1, `personal_proof_requires_reviewed_account_and_same_private_chat` | Wrong actor, forwarded/wrong-chat/group proof, onboarding code, denial, cancellation, account switch, and digest mismatch cannot consent. Failed guesses consume spacing and rate limits. |
| Part 1, `personal_confirmation_has_one_winner_and_replay_result` | Concurrent confirms yield one consent/completion; no second binding or generation/thread. Stale prior-binding review requires new approval. |
| Part 1 with Part 2 fixture, `personal_replace_commits_revoke_exact_stop_and_new_binding` | Inject abort after each write and show no partial A/B replacement or stop record. Committed replacement leaves A's history/key owner unchanged and B's agent/thread/turn references null; first use resolves B's ordinary NyxBot. |
| Part 1, `resolve_sender_only_unlinked_can_select_public` | Never-consented/explicit unlink are unlinked; disabled/deleted owner, missing controls, paused personal-bound sender, emergency stop, unknown mode, manager retirement, malformed stored authority, and database failure are unavailable/error. |
| Part 1 with Part 5 fixture, `unlinked_receipt_conflicts_with_confirmation` | Race the no-binding lookup against confirmation. Common control-row writes force retry; one committed receipt classification survives later replay/linking. |
| Part 1, `public_changes_do_not_revoke_personal_consent_or_signup` | Change guide generation/enabled, public profile/budgets/revision, onboarding expiry, and same-ID token observation readiness. Consent and pending personal proofs survive; delivery waits while webhook is unverified. |
| Parts 1/5, `reactivation_waits_for_personal_retirement` | Crash after inactive write and before retirement, then request reactivation. Reactivation remains blocked until old bindings/proofs and exact Stop records are durable; old consent cannot revive. A fresh post-reactivation link works, and no-personal-state accounts need no new transaction. |
| Part 1, `personal_control_pause_resume_and_emergency_are_revision_fenced` | Old origins remain refused after personal pause/resume; whole Pause sets emergency; guide Enable cannot clear it; explicit clear leaves lanes disabled; manager retirement epoch closes effects before cleanup finishes. |
| Part 1 with Part 2 fixture, `thread_claim_delete_recreate_is_atomic` | One creator wins; loser leaves no conversation/key; deleting old pointer cannot clear a new one; deleting a revoked binding's thread remains allowed. No transcript is imported. |
| Part 1 with Parts 2/5, `revoke_refuses_late_turn_effect_card_and_send` | Race revocation with turn claim, card decision, effect admission, and delivery claim. The exact old turn stops; a web turn on the same thread and committed specialist/watch work survive. Durable stop recovery works after process loss. |
| Part 1, `initial_runtime_enable_uses_absence_preconditions_and_existing_sponsor` | Capable GET has `supported:true` and null runtime. Initial PUT requires explicit initialize, zeros, and matching profile generation; it persists positive authority and retains the existing sponsor. Failed readiness creates nothing. Concurrent initialization, stale generation, and recreation over an emergency-stopped row fail without resetting state. |
| Part 1, `runtime_enable_requires_matching_readiness_and_admin` | Probe failure, expired evidence, altered candidate, sponsor/admin revocation, manager rotation/retirement, and concurrent config save cannot half-enable. Save paused needs no fabricated readiness; failed enable leaves previous state. |
| Parts 1/3/5, human HTTP and invitation compatibility | Every new account/admin path rejects nonhuman credentials; possession-only preview reveals no account/binding state. POST status never reveals a code. The inventory GET deny class is installed before routing. Existing login/invitation/MFA policy remains enforced. First-party approval alone never activates before chat confirmation. |
| Part 1 existing regression modules | Existing `platform_onboarding_binding_*`, guide configuration, pause/retire, and child setup behavior remains intact. Part 5 retains native creation/token-rotation tests and checks ordinary no-platform accounts remain standalone-compatible. |

The candidate already settles non-serialized values, `async_trait` producers,
`Unlinked/Personal/Unavailable`, the personal control revision, session-taking
fences/thread helpers, and owner identity in `PlatformTurnOrigin`. Part 1 accepts
the unaffected choices with Part 2's `338d92f5b` amendments above, the POST proof
paths, fragment bearer, owner GET list plus POST revoke, and unified admin runtime
GET/PUT. Account lifecycle entry-point edits
remain with Part 5 as assigned. No alternate contract module is proposed.

The contract owner needs these concrete corrections or additions before freeze:

- Capability and configuration are separate. An enforcing server returns
  `supported:true, runtime:null` before initialization. First PUT explicitly uses
  `initialize:true`, current `expected_profile_generation`, and zero expected
  runtime values as an absence precondition. Store epoch/revisions 1 and keep the
  profile's existing sponsor. Probe and final atomic Enable require one product
  action. Normal updates use positive revisions. No stored zero or read-side
  initialization is permitted.
- Adopt Part 2's pre-thread `PersonalBindingAuthority`. Part 2 ensures ordinary
  NyxBot before the turn transaction, validates the real agent during thread
  claim, and completes `PlatformTurnOrigin` afterward. Thread claim accepts the
  actual agent ID and returns both winning IDs. Consent needs no existing agent.
- Add exact active-turn claim/clear helpers to the identity producer or agree
  their explicit module seam. Part 2 maintains the pointer in claim/settlement
  transactions. Its full origin revision remains available for later Stop.
- Replace candidate `StopRecorded` with Part 2's `Captured(CapturedPlatformStop)`.
  Part 2 writes exact Stop and returns effects in the caller's transaction.
  Part 1 stores its recovery row before commit. Part 2 applies cancellation
  after commit. Clear operations remain valid after revocation and compare only
  exact owner/binding/generation and old turn/thread. No duplicate queue exists.
- Add Part 3's local readiness revalidation and sender-public-session revoke
  hooks. The latter is a no-op without public state and ships independently of
  upstream proof. It durably closes active public authority before linking can
  complete. Profile discovery also needs a producer seam for its agreed POST.
- The manual code proposal is purpose-prefixed `P-XXXX-XXXX`. Generate/return it
  once on approval, not on status. Lost raw token/code responses require fresh
  intent/review rather than recovering raw secrets. This corrects the candidate
  wording that treats status as identical to the approval response.
- Part 5 gates reactivation on completing interrupted personal retirement before
  setting the person active. Its existing account entry-point ownership covers
  this change; no Part 1 edit to user/account handlers is proposed.
- Part 3 denies `GET /api/v1/platform-personal-bindings` and descendants before
  mounting. Runtime GET is already covered by the `admin` deny class. There is
  no new challenge/status GET. Expiry applies to proofs, never durable consent.

Public runtime containment, all-hop cost bounds, cleanup protocol, Telegram
delivery capacity, and real two-account acceptance remain with their assigned
owners. Nothing in this proposal establishes those gates or authorizes live
activation. Personal implementation can proceed after the shared contract
without waiting for public runtime proof.
