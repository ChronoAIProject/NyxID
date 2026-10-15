# Public guide and personal NyxBot through one Telegram bot

Status: implementation plan converged with Claude Opus 5.5 on 2026-10-15;
no remaining architectural blockers. Public activation remains conditional on
the phase 0 runtime evidence. No product implementation or production change
is included in this document.

Source baseline: `origin/main` at `b6f11c1d3`, fetched on 2026-10-15. The
previous UI branch adds the simplification from PR #1896 (`2dd4e0297`). This proposal
replaces that PR's direct-model selection direction for the manager. It does not
declare the shipped BotFather design incorrect or remove its customer bots.

## 1. Product contract

One Telegram bot serves two audiences through NyxAgent:

| Audience | Experience | Authority | Payer | History |
| --- | --- | --- | --- | --- |
| Public, including someone signed up but not linked | FAQ, explain NyxID and its use cases, offer signup/account linking | Fixed public tool policy; no person's account | Configured sponsor, subject to total and per-person daily quotas | Separate, short-lived public context |
| Verified private sender who has consented to personal access | Their own NyxBot, including their services and account-management tools | The sender's existing NyxBot permissions and confirmation policy | That user under existing execution billing rules | A new Telegram thread under their own NyxBot, visible and usable in the web app |
| Group or supergroup | Public information or a private-chat link | Public policy only, even for a linked sender | Sponsor if an AI reply is admitted | Never a personal thread |

Signup alone is not Telegram authentication. Personal access begins only after
first-party web approval and confirmation from the same private Telegram sender.
An existing onboarding-only link never silently becomes personal-account access.

The public guide runs on NyxAgent. Its source is a NyxAgent profile/configuration,
defaulting to `nyxagent/chat`, not a separate Chrono LLM model selector. The profile
chooses model behavior; NyxID independently enforces tool authority and funding.

Every verified user reuses their existing persistent NyxBot, or has the ordinary
one created on first use. A separate copy of its credentials, agent, or grants is
not created per transport. The Telegram conversation has its own conversation
credential, as ordinary NyxBot threads already do.

Personal access means parity with the user's NyxBot. The consent page explains
that scope and their existing permission mode. It does not silently downgrade
them to read-only or upgrade them to Full access. Auto, Ask every time and Full
access retain their existing semantics, including deliberate Full-access bypass
of confirmations. Explicit denies and resource ACLs continue to apply.

This first implementation does not add a second, independent service-permission
editor just for Telegram. If a narrower Telegram scope is wanted later, that is
a separate capability-ceiling feature: keys, native tools, memory, delegated
work and continuations would all need to honor it. Filtering a tool list or
editing a prompt is not an implementation of that restriction.

Existing child-bot/restaurant provisioning remains an optional workflow reached
through a setup link or reserved setup command. It is not required to sign up,
link an account, or use personal NyxBot. Its zero-capability publication contract
and independent customer test remain intact.

## 2. Source findings that constrain implementation

| Existing behavior | Evidence | Required change or reuse |
| --- | --- | --- |
| Converted managers cannot enter ordinary personal routes | `services/platform_bot_authority.rs::{permits_personal_routes,require_personal_routes}`; `handlers/nyxbot.rs::start_chat_turn` | Preserve this fence. Add a dedicated, verified-sender route instead of allowing the manager's ordinary owner route |
| Native manager webhook handles control commands, then guide, then legacy relay | `handlers/telegram_new.rs::webhook_with_service` | Insert one authoritative audience dispatcher after native/control classification; a claimed platform manager must never fall through |
| Current links authorize onboarding only | `models/platform_binding.rs`; `services/platform_binding_service.rs`; `docs/PLATFORM_BOTFATHER.md` | Reuse proof mechanics, require separate personal-access consent |
| A normal channel belongs to one user and has a list of owner sender IDs | `models/nyxbot_channel.rs::NyxbotChannel` | Do not add every customer to the manager's `owner_sender_ids` or pretend each owns the shared channel |
| Owner private chats normally reuse the agent's home thread | `handlers/nyxbot.rs::{owner_thread,start_chat_turn}` | Use a new sender-owned Telegram thread for this shared-manager surface; do not import pre-link public history or silently redirect an existing home |
| Ordinary orchestrator keys regain broad service/account authority at each turn | `services/assistant_agent_credential_service.rs::KeyAuthority` | Appropriate for explicitly consented personal parity; inappropriate for public keys or a claimed restricted personal mode |
| Native account-management tools already exist | `services/assistant_account_tools.rs`; `docs/chat/08-nyxagent-engine.md` | Reuse their ownership, secret exclusion and confirmation behavior |
| Assistant model execution can reach any active inference catalog row outside the key's service grants | `handlers/proxy.rs::assistant_model_call` | Public execution needs its own bounded model policy; an empty ordinary assistant key is insufficient |
| Public guide currently uses a purpose-confined, sponsor-paid direct inference path | `services/platform_inference_service.rs` | Reuse exact-credit admission/lease principles; extend them to every billable NyxAgent hop |
| Ordinary platform-key usage bills the acting person; other credential classes can bill the authorized resource owner | `services/billing/owner_resolver.rs::resolve_for_execution` | Preserve ordinary billing. Sponsor funding is a narrowly authorized public execution context |
| NyxAgent profiles already have a live listing and default | `services/assistant_nyxagent.rs::DEFAULT_MODEL`; `handlers/assistant_nyxagent.rs` | Reuse discovery/configuration contracts, not raw-provider model heuristics |
| Role-to-profile settings are currently inactive | `services/assistant_profile_routing.rs::ROUTING_ACTIVE` | Do not claim those admin settings already control execution or enable unrelated role routing as a side effect |
| Public receipt deduplication is purpose-scoped today | `services/platform_native_receipt_service.rs::admit` | A single ingress receipt must cover classification before choosing public versus personal, including retries across linking |
| Guide configuration writes increment the existing profile generation | `services/platform_bot_config_service.rs::configure_guide` | Personal consent must not expire when an admin changes a public budget/model; separate runtime policy revisions from durable manager identity |
| The current linked guide sender cap equals the platform total | `services/platform_inference_service.rs::reserve_budget` | In the new mode, all public senders use the same per-person quota, including onboarding-linked people without personal consent |

The local repository does not prove the deployed NyxAgent runtime has no native
tools outside NyxID MCP. That is a contract-verification item, not a fact inferred
from a tool list. Likewise, a NyxAgent profile name is not itself a model-cost
bound or an immutable tool policy.

## 3. Simple setup and user journey

The admin's primary form has three fields and one action:

| Field | Default / behavior |
| --- | --- |
| Agent source | NyxAgent, `nyxagent/chat`; use live NyxAgent profile labels |
| Total public budget per day | Preserve an existing configured budget; new setup defaults to 1 credit/day |
| Free quota per person per day | Preserve an existing configured quota; new setup defaults to 0.1 credit/day, capped by total budget |

One **Enable bot** action validates the profile, public runtime capabilities,
sponsor funding and selected limits, then saves/enables atomically with a revision
check. A failed readiness call must not leave a half-enabled profile. Costs and
limits are decimal credit strings and use the existing exact-money type.

The sponsor chosen during manager setup is shown beside the budget. If absent,
an admin selects an authorized organization once; never guess a sponsor or charge
the admin personally. **Advanced** contains profile overrides, rate/concurrency
limits, bounded execution limits and sponsor changes. A fixed public-tool summary
is visible without requiring an admin to configure individual tools.

Default hidden public-sender limits retain 10 starts/hour and one concurrent turn.
These limits do not apply to user-funded personal turns.
Other safe existing limits are retained where compatible with NyxAgent. Deadline,
tool-call and token bounds must come from the verified runtime contract; do not
assume the current direct inference timeout bounds a multi-step agent turn.

The public quota applies before personal consent. After linking, personal turns
use the user's own billing. There is no implicit free-credit grant, separate
"bound sender" sponsor tier, or fallback to sponsor money when the user runs out.
Welcome-credit campaigns remain ordinary grants/allowances configured elsewhere.

User journey:

1. A person messages the manager and gets FAQ/help plus a sign-up or sign-in link.
2. The link preserves only a server-issued, expiring linking request through
   signup/sign-in. It never accepts a client-supplied destination user or bot.
3. Web consent identifies the Telegram bot/sender, selected NyxID account,
   personal billing, and use of that account's existing NyxBot access/mode.
4. The same private Telegram chat confirms. Denial, expiry and replacement remain
   explicit; pending approval is still public-only.
5. The next newly admitted private message runs on the user's NyxBot. The app
   shows a Telegram thread, its messages, tool activity and ordinary approvals.
6. The user can disconnect in the app or with a reserved `/unlink` command. A
   disconnected user can still read their own saved personal history on the web.

Registration respects the existing global `auth:invitation-code` policy
(`handlers/auth.rs::register`, default required). The public tool never issues
invitations or bypasses that policy. The landing page explains the invitation
requirement when enabled and accepts a legitimate invitation through the normal
registration form. Existing users can sign in directly. General-availability
signup requires an explicit operational choice to allow open registration, or a
working approved invitation distribution process; this plan makes no live policy
change and does not claim unrestricted signup while the gate is enabled.

Use a separate personal-link intent, valid for 24 hours, with a random,
high-entropy single-use token stored only as an HMAC, bound to the verified
manager/sender/private chat and manager authority epoch. It preserves navigation
through registration and email verification without granting personal authority.
After authentication, issue a fresh ten-minute review/confirmation challenge.
Consume the intent on completed confirmation; unlink/replacement cancels pending
intents. Expired intent pages offer a clear instruction to send `/link` again.
Test signup taking more than ten minutes, both permitted and invitation-required
registration, refresh, replay, wrong-account review and abandoned attempts.

For an existing account, the new thread does not replay its old home transcript.
It is still the same NyxBot: existing personal memory/persona and authorized
account data follow normal NyxBot behavior, disclosed during consent. "History
starts at linking" is a transcript boundary, not a promise that the user's own
agent forgets its existing memory. An existing agent's home is never replaced.

Groups use a deterministic private-chat handoff in the first release, as today.
This fulfills the public-only group boundary without introducing shared public
agent sessions or extra sponsor usage. Group FAQ turns can be added later under
the same public policy; personal operations remain private-only.

## 4. Authority, persistence and routing

### Identity proof and consent

Keep the existing onboarding binding and its expiry/generations for existing
setups. Add a separate `platform_personal_bindings` record instead of changing the
meaning of every existing `PlatformBinding` row. Reuse common proof/decision
helpers, with a closed purpose bound into the link request/HMAC and approval
digest. Store personal intents/challenges in separate collections that old
onboarding handlers cannot read. Old requests have onboarding purpose only.

Expose typed human-only handlers for intent preview, web approval/denial, the
owner's active links and unlink. Derive the user from the verified first-party
session, never a request body. Add any new authentication/provisioning GET path
to `mw/auth.rs::delegated_read_denied_path` before mounting it. DTOs expose no
credential/token hashes, raw keys or ciphertext. Use a dedicated linking page
that preserves a server-owned intent through sign-in; keep `/onboarding` as the
existing optional BotFather setup surface. Broad `/unlink` revokes both personal
and onboarding links for this exact manager/sender; the app can label each
purpose explicitly rather than conflating their authority.

A personal binding contains server-derived profile/manager identity, numeric
sender/chat IDs, user ID, target NyxBot ID, conversation ID, binding generation,
consent version, reviewed NyxBot access-policy summary, active/revoked state,
and BSON timestamps. It records consent to the user's existing NyxBot authority,
not a frozen copy of service grants. Updates in the user's NyxBot settings apply
normally. Unique indexes prevent two active owners for one manager/private
sender and make concurrent confirmation idempotent. Account replacement revokes
the former binding and never reassigns its conversation or credentials. One
person may link several Telegram identities to the same manager, each with its
own binding and dedicated thread; unlinking one does not disconnect the others.

Personal consent is durable until unlink, replacement, account disable/delete
or manager retirement. It does not inherit the onboarding binding's 30-day
expiry. Pending linking requests still expire after ten minutes, as today.
Guide model, budget and content edits do not require personal re-linking. Use a
stable manager authority epoch for identity/retirement and separate public-policy
revisions for public configuration; never treat the old profile generation as
the lifetime of a personal identity link. The same independence applies to
personal-link intents and pending confirmation, so a budget edit cannot interrupt
signup.

Persist new runtime controls separately as `platform_bot_runtime_controls`, keyed
by the stable profile ID and manager ID: `personal_enabled`, `public_enabled`,
`emergency_stopped`, `manager_authority_epoch`, `personal_control_revision` and
`public_policy_revision`. Missing controls mean the new mode is off. Admission
and effect fences write the relevant control row in the same transaction as
the personal binding, forcing conflicts with pause/revocation. Personal fences
require active binding generation, live user, live manager identity/epoch,
`personal_enabled`, no emergency stop, and no manager retirement. They never
depend on guide `generation`, `profile.enabled`, sponsor funding or public quotas.

Before delivery, resolve the live manager token and verify its numeric bot ID,
current profile `observation_id` and webhook readiness. Token rotation preserving
bot identity preserves personal consent; sends wait for a verified current
observation rather than using captured token material. A different bot identity,
retirement or deletion advances the manager authority epoch and revokes personal
bindings. Removal of the platform marker is not a supported un-conversion path.

Existing whole-profile Pause/Retire operations also set the emergency stop for
new runtime controls when present. New guide-only pause/model/budget endpoints
change public controls only and retain legacy child-installation semantics.
Reject stale old guide-edit payloads against an opted-in new configuration with
an actionable conflict instead of silently overwriting runtime controls. Clearing
the emergency stop requires an explicit audited admin action; an ordinary
legacy Resume or guide Enable must not silently clear it.

### One dispatcher, one receipt

Create a service-owned dispatcher behind the verified native Telegram ingress.
It receives verified transport facts, not a client-provided owner or payer.

1. Preserve Telegram creation events and reserved control commands first.
2. Load the durable manager marker and runtime configuration. Missing, retired,
   unsupported or paused authority is closed, with a bounded status response.
3. Admit one metadata-only receipt in a new `platform_manager_ingress_receipts`
   collection, keyed by stable manager identity, sender and
   Telegram update ID before selecting an execution audience. Record audience,
   binding/config generation, conversation, payer and claim in that admission.
4. If a live personal binding and private sender proof match, resolve that
   user's NyxBot. A deterministically unlinked sender selects public policy.
   Database errors, disabled owners, paused personal routing and failed personal
   authorization produce a bounded status response, never silent public fallback.
5. Execute under the admitted audience. A replay never reclassifies the same
   message across linking/unlinking or charges both paths.

Only replicas implementing this receipt and dispatcher may enable the new mode;
legacy purpose-scoped receipts are not reinterpreted as new audience decisions.
Messages admitted before personal consent stay public or are cancelled. Linking
does not replay them into the personal agent. A fresh post-consent message starts
personal work. Personal work is an owner turn (`guest = false`) using the
ordinary user's engine flag, assistant readiness and billing admission. Missing
engine/readiness, insufficient funds and a full owner turn pool produce a fixed
status/app link or normal bounded busy behavior. Do not retry through another
identity or the public guide.

### Shared execution services, explicit authority

Extract the reusable NyxAgent turn start/stream/settlement logic from HTTP
orchestration as needed, preserving `handlers -> services -> models`. Do not
duplicate the whole engine or call HTTP handlers internally with a fabricated
`AuthUser`. Introduce typed, server-resolved authority for public and linked
personal turns. The existing owner-turn path remains the default for ordinary
NyxBot callers.

Public execution gets a distinct immutable credential purpose and persisted
turn/session binding. Reuse envelope-encrypted credential lifecycle machinery,
but do not create the sponsor organization's full NyxBot or reuse a human key.
The key/credential is owned by the sponsor organization's existing user identity,
with public purpose overriding ordinary org-key authority on every entry point.
Generic org inventory/account APIs must therefore reject it too. The credential
refers to sponsor funding and public policy only. Its expiry,
rotation, deletion and recovery must never converge to `Orchestrator` authority.
Model discovery/readiness uses the same purpose restriction. Generic org-admin
key edit/rotation must refuse these keys; runtime-managed replacement preserves
public purpose and never returns raw material. Revocation/removal uses the
public runtime lifecycle so session cleanup and active-turn cancellation remain
consistent.

Personal execution gets an ordinary user-owned conversation credential plus a
persisted origin binding on the admitted turn. New tool/proxy calls and inline
continuations of that turn, Telegram approval replies and outbound delivery
recheck that origin. Independently committed owner work has the lifecycle
specified below. Scope this check to those turns so ordinary
web/API traffic does not gain unrelated platform lookups.

Introduce an additive `platform_thread` marker on the personal conversation and
a `platform_origin` proof on its active turn/reply, rather than forcing the new
route into the existing `ChannelOrigin.nyxbot_channel_id` shape. Explicitly exclude
these threads from home adoption and recovery in both `assistant_nyxagent` and
`assistant_team_service`, including the "find newest thread" home fallback.
Assistant list/read APIs expose them to their owner in the ordinary web UI.
Deleting a personal thread atomically clears the matching binding's conversation
reference, while normal deletion revokes its key and stops that exact thread.
The next personal message may create a new dedicated thread under the same live
consent. Concurrent recreation has one winner; deleted history is not restored.

Never use `owner_sender_ids` or a fake `NyxbotChannel` to represent customers of
the shared manager. Keep a distinct reply destination with manager ID, verified
chat/sender, binding ID/generation, origin turn and expiry. Delayed results carry
the same destination proof. The initial transport supports text and approval
cards; tool images/files remain accessible in the user's app with an accurate
link/note. Outbound image delivery can be added later using this same proof.
Normal web turns do not acquire a
sticky Telegram reply destination merely by opening this conversation. New
event/card continuations that are entitled to reply while linked capture the
thread's live binding generation when admitted. Async results retain their
original destination generation; neither may adopt a newer link to make a stale
result deliverable. After revocation, those continuations are web-only.

Extract the existing word-decision logic (`handlers/nyxbot.rs::reply_decision`)
into a shared service. Telegram renders pending action summaries with their
existing codes, accepts `yes 4821`/`no 4821` only from the linked private sender,
and binds the decision to the exact pending action, user, turn and consent
generation. Expired, ambiguous or mismatched replies do not decide cards.
Full access follows the current automatic-receipt behavior; ordinary web cards
remain usable. This is part of the first personal-routing release.
The "since previous owner message" ambiguity anchor includes web-origin owner
messages in this same thread. Manager operators/sponsors gain no personal-history
read endpoint or UI by operating the transport. Manager logs/audit contain only
bounded metadata, never prompts, replies, tool arguments or personal results.
Consent explains the platform-operated Telegram transport and shared NyxBot
context; private Telegram bot chats are not end-to-end encrypted. Consent also
states that Telegram confirmation codes can approve the same sensitive actions
(including machine and saved-login actions) as the user's existing NyxBot. Preserve
`protect_content` on replies as in the current guide.

### Revocation and concurrent effects

Link, replace, unlink, account disable/delete and manager retirement must write
the same authority rows that effect admission fences. Read-then-act checks alone
are insufficient. Transport revocation prevents new Telegram turns, queued
messages, Telegram card decisions and new delivery claims. Request Stop only
where conversation ID, `active_turn.turn_id`, persisted origin binding ID and
generation all match the ingress receipt. Stop blocks further tool/model calls
and inline continuations of that active turn. A web-origin turn on the same
thread, or any turn on another conversation, is not matched by this filter.

Use the existing exact conversation-turn cancellation path and captured job IDs;
never broad agent/desktop cancellation. Node work gets exact cancellation only
when supported, with no broad-cancel fallback for old nodes. Already-committed
independent specialist tasks, automations, connect-link watches and proxy
operations remain the user's own authorized work. Unlink does not cascade-cancel
them or revoke that user's NyxBot grants. They retain normal live account/ACL
checks and report to the web. This is an intentional boundary, not a claim that
the existing code records a complete delegation provenance graph.

After unlink, the saved thread remains usable in the web app. Pending action
cards remain decidable there. Event/watch/card continuations can run as normal
owner work without a Telegram destination; every outbound send still fences its
original destination generation. Late settlement is saved in the user's web
history. The unlink UI states: "Work already started may keep running in NyxID;
stop it from the app." Account disable/delete is stronger: ordinary account-wide
credential revocation, task and data cleanup continue to apply. Replacement
revokes account A's binding and creates B's in one transaction, with exact Stop
for A's old Telegram turn; it never transfers A's thread or key to B.

Public work uses public-purpose cancellation and expiry regardless of personal
consent. No stale public turn can be promoted into personal authority.

An external request already forwarded before revocation cannot be undone.
Report that boundary accurately: cancellation is best effort for already-running
remote work, with no new effects admitted after revocation. An uncertain Telegram
send or external mutation is not blindly replayed. Persist metadata-only send
claims; reuse existing reply-retention policy and never make a durable queue of
private message bodies solely to replay them.

### Capacity separation on the shared bot

Personal work never enters `GUIDE_WORK`, public inference quota scopes or
`platform_native_replies` admission. It uses the user's ordinary turn pool and a
separate per-chat outbound pacing policy. Add a manager send scheduler with
bounded public/personal queues, fair per-user service and reserved personal
capacity; public replies yield first under shared Telegram pressure. Persist
only metadata claims/references, not a replay queue of message bodies.

The existing public reply throttle (100/profile/minute, 10/sender/minute) and
32-permit guide semaphore are not personal service limits. Overall bot pacing
must respect tested Telegram rate-limit responses/`retry_after`; exact Telegram
limits are an external contract to verify, not values inferred here. Even fixed
quota/error replies are throttled. Public exhaustion must leave personal
admission and its reserved delivery capacity usable, subject to Telegram's
overall availability and normal personal limits.

## 5. Public NyxAgent tool and model policy

Start with three closed capabilities, using proposed names until implementation:

| Tool | Allowed data/effect |
| --- | --- |
| `public_faq_search` | Search a versioned, approved public FAQ/use-case corpus; no account inventory, arbitrary URLs or private documents |
| `public_faq_read` | Read an allowlisted public article by stable ID; bounded output |
| `public_signup_link` | Return/initiate the server-bound signup/linking URL for this verified inbound sender; cannot choose user, grants, manager or payer |

These are a fixed server policy, not a dynamic list of tools an admin/model can
widen. Adding future capabilities requires schema, authority and budget review.
The FAQ corpus needs an authoritative publication owner and version; initial
content can be reviewed repository content with a later publishing UI.

Enforce the same policy at discovery and invocation: direct MCP names,
`nyx__call_tool` wrappers, search/discovery, resources/prompts and transport
variants. Deny unknown tools by default. Public keys cannot call native account,
memory, subagent, machine, Oracle, skill, channel-management or general proxy
tools, create connect links for accounts, request expanded grants, or raise
personal approval cards. Public keys cannot use delegated management GETs or
mint credentials that lose their public purpose.

Profile/model calls are a separate capability, not a public tool grant. Pin the
allowed NyxAgent profile and supported model-service/operation bounds in trusted
execution policy. Deny arbitrary catalog inference fallback, personal connection
resolution, owner nodes and any user-controlled URL/headers/model override.
Enforce the public purpose in auth, MCP, direct proxy and LLM proxy paths before
the ordinary assistant inference exception can widen it.

Upstream contract gate, before public activation:

- Record the exact NyxAgent build/profile contract and integration evidence.
- Prove native shell/files/browser/web tools and additional MCP servers cannot
  bypass the public allowlist. If the runtime lacks this control, add an
  upstream purpose/profile capability contract; a prompt is insufficient.
- Prove a bounded lifetime/tool-call count and that every billable model request
  carries the same scoped credential through NyxID admission. Runtime-held
  provider credentials must not create unmetered sponsor work.
- Prove upstream sessions are isolated by credential and that public-session
  deletion/expiry can be enforced; record NyxAgent's `store: true` behavior.
- Profile changes must not silently expand authority or cost limits. Unsupported
  contracts make readiness fail and keep the public guide paused.

If `nyxagent/chat` cannot satisfy the restricted runtime contract, implement and
verify a dedicated public profile before public activation. Keep the same
NyxAgent configuration surface; never silently substitute the broad personal
runtime. A public-purpose override enforced by the runtime is also acceptable
if the cross-service tests prove it cannot be weakened by profile changes.

Public context is keyed by manager plus sender, with 24-hour encrypted retention
and its own upstream session. Never share one session across strangers or reuse
a public session, key, checkpoint, memory or action request after personal link.
TTL is housekeeping; live expiry checks enforce authority and revoke keys even
before physical deletion. A bounded sweeper deletes expired upstream sessions
and local encrypted context. If upstream deletion requires the original session
key, retain its envelope-encrypted material only in a cleanup-only record: the
key is already revoked for all execution, and the dedicated cleanup path permits
only deletion of that exact session. Phase 0 must prove this cleanup protocol or
an upstream-enforced expiry; do not promise deletion with a key that can no
longer authenticate it. Erase the cleanup key after confirmed deletion/expiry.
Link confirmation ends the public session and queues cleanup without importing it.

## 6. Billing and quota accounting

Public funding is an explicit server-issued context linking profile, sponsor,
sender, turn, immutable policy revision and expiry. Neither Telegram input nor
NyxAgent tool arguments may select the payer. Validate sponsor readiness and
catalog platform grants with the existing live ACL rules.

Count all billable public work: NyxAgent invocation if priced, inner model calls,
model discovery/readiness and any future paid public tool. Reuse normal metering
and the exact `Credits`/Decimal128 ledger. The cap is an admission limit, not a
second wallet charge: each usage event settles exactly once through existing
billing. Distinct outer orchestration and inner provider charges both count when
the catalog prices both; the same usage must not be posted twice.

Before each billable dispatch, reserve its bounded maximum against both the
sender's remaining daily quota and the platform's remaining daily budget in one
transaction. Already-admitted concurrent requests hold capacity. Missing price
or an unbounded cost contract refuses the call, rather than treating it as free.
Legitimately free configured operations remain free but retain rate limits.

Retain the existing conservative cap semantics for forwarded work: the admitted
maximum remains consumed for that UTC day, while wallet settlement records
actual usage. Refund only proven not-forwarded holds; crash/expiry cleanup is
idempotent and uses the original reservation day. State this distinction in
Advanced/help copy. Exact caps cover the whole agent loop across restarts,
retries, streaming, continuations and provider failure, not just its first hop.

For personal turns, the actor is the linked person and ordinary billing resolves
each operation. Platform-key inference bills that person. An explicitly allowed
organization-owned BYOK resource retains existing resource-owner billing; this
feature must not silently change organization billing rules. The manager sponsor
never pays for personal execution. Insufficient user funds produces an app/top-up
link and stops; no public reclassification of the personal request.

## 7. Implementation sequence and completion gates

The phases below express dependencies and completion gates. Execution is split
into [five parallel workstreams](2026-10-15-public-personal-nyxbot-workstreams.md)
with exclusive file ownership, shared contracts and an integration owner. That
breakdown governs agent assignments; these phases are not six sequential jobs.

| Phase | Work and primary files | Completion evidence |
| --- | --- | --- |
| 0. Verify public runtime contract | Pin NyxAgent build/profile capabilities; inspect native tools, credential propagation, session cleanup and pricing bounds; define executable cross-service fixtures | Real contract test proves allowlist containment and all model hops reach NyxID. Document any required NyxAgent patch; this gates phase 3, not the personal routing work |
| 1. Personal consent and common runtime | Add personal-binding model/service, signup intents and purpose-bound proof reuse; extract shared NyxAgent turn execution needed by personal routing; human-only approval/status/revoke handlers and indexes | Signup/login continuation, same-chat confirmation, old binding compatibility, replace/revoke races and no duplicate agent/thread tests |
| 2. Personal runtime and delivery | Service-owned audience dispatcher in native webhook; user NyxBot/thread integration; exact origin/reply fences and independent delivery capacity | Two users concurrently isolated; same web/Telegram thread; dedup across linking; unlink/replace/disable races; normal NyxMode parity |
| 3. Public runtime and funding | Build on common execution with public credential purpose; policy enforcement in auth/MCP/proxy/LLM; extend `platform_inference_service` accounting for bounded multi-call execution | Adversarial tool/proxy bypass tests; sponsor-versus-admin-versus-user accounting; parallel caps, crashes, UTC rollover, stale profile tests |
| 4. Setup and account UX | Revise `frontend/src/pages/admin-platform-bots.tsx`, platform schemas/hooks, linking pages and assistant thread/source UI; reuse `useAppForm` and DESIGN.md | Browser-level three-field enable flow; automatic profiles; clear readiness errors; correct billing/permission consent; web history; accessible loading/retry states |
| 5. Compatibility and rollout | Separate enablement for new mode; keep old onboarding readers/child flows; lifecycle cleanup and operational docs | Existing backend/frontend suites; live two-account Telegram acceptance; verified replica/runtime deployment and controlled enablement |

Personal phases 1 and 2 can proceed while phase 0 verifies the public runtime;
their focused linking/history UI ships with them. Phase 3 waits for that public
contract. Phase 4 completes the simple admin flow once its backend is ready.
The feature is complete only when the combined acceptance gates below pass; a UI-only PR is not completion.
After migration, remove the replaced direct guide dispatcher/action adapter
once no legacy manager uses it. Preserve shared inference/accounting machinery
that still serves child installations. Each phase should be a bounded PR with
compatibility tests, not another large unreviewed replacement. Backend runtime tests require the existing MongoDB
transaction-capable test setup; avoid parallel heavyweight Rust builds on this
host. Use the repository's CI toolchain and required lint/type/build gates.

PR #1896 should be revised or superseded by phase 4, preserving its useful form
and validation improvements. Do not merge it as the final answer to this revised
requirement. Closing or changing the remote PR is outside this planning artifact.

## 8. Acceptance matrix

| Gate | Required observation |
| --- | --- |
| A1 Public tool boundary | FAQ/signup work; forged direct/wrapped/native/delegated/proxy/LLM calls and upstream built-ins cannot exceed policy |
| A2 Operator isolation | Public chat cannot read manager-owner/sponsor data; manager operators/sponsors gain no personal-history API/UI access. Logs are metadata-only |
| A3 Proof and consent | Signup alone, old onboarding link, forwarded code, wrong chat, expired approval and a changed account cannot activate personal access |
| A4 Personal parity | Consenting Alice can manage her actual connected services/account through existing native tools with her chosen mode; Bob cannot access them |
| A5 History | App and Telegram show the same post-link personal thread; pre-link public chat and old home transcripts are not imported or replayed; neither home-adoption nor recovery paths adopt the Telegram thread |
| A6 Revocation | Unlink/replacement/manager stop end new Telegram work, exactly stop its active turn and refuse late sends/cards; committed specialist work and unrelated web turns continue in the app. Account disable/delete retain stronger global cleanup |
| A7 Replay and races | Duplicate updates before/after link, parallel confirmation, webhook retry and process restart never run both audiences or double-charge |
| A8 Billing | Public calls charge only authorized sponsor; personal inference charges user; existing org resource billing persists; low funds never switch payer |
| A9 Full-loop limits | Repeated model calls, streaming, concurrent senders, timeout/crash, continuation and midnight rollover remain within exact public admission caps |
| A10 Public groups | A linked user in a group gets no personal access or personal confirmation, even when quoting a private reply/code |
| A11 Setup simplicity | From valid sponsor/manager prerequisites, source plus two budgets and Enable suffice; defaults require no raw model/API knowledge |
| A12 Existing product | Ordinary channel bots, manager creation webhook, child setup/review/test/activation, web NyxBot, grants and billing keep their existing behavior |
| A13 Cross-service live proof | Two real NyxID accounts/two Telegram identities exercise public FAQ, signup, linking, account tools, web history, billing and unlink against pinned deployed builds |
| A14 Stable consent and controls | Onboarding expiry, public model/budget edits and same-identity token rotation do not break personal routing. Exhausted public budget/quotas/semaphore/reply window leave personal capacity available; emergency stop blocks both |
| A15 Public retention | Separate senders cannot share a session; linking/24-hour expiry revokes public use and cleans local/upstream context; cleanup retries retain no execution authority |
| A16 Signup completion | A signup lasting over ten minutes resumes through a fresh proof challenge; invitation policy stays enforced; expired/replayed/wrong-account intents cannot link |
| A17 Personal lifecycle | Thread deletion permits safe one-winner recreation without old history; web continuation and web card decisions remain usable after unlink; missing engine/funds/pool capacity has no public fallback |

## 9. Compatibility, rollout and rollback

Ship additive models and optional fields with defaults that mean legacy/onboarding
only. No automatic manager conversion, new personal consent, grant widening or
backfill of user-owned chat from public data. Keep existing bot markers and
installation tombstones, and retain the old onboarding profile reader. Keep the
existing `PlatformExecutionProfile` enum value: old readers cannot deserialize
a new variant. Add optional mode/config fields using targeted updates and
preserve them through typed writes. New personal/public conversation fields
must also survive key rotation and worker settlement; mixed-version activation
remains prohibited even when deserialization succeeds.

New execution modes and credentials are enabled only after every backend auth,
MCP, proxy, webhook and worker replica enforces them. Public-purpose activation
also waits for the verified NyxAgent contract; personal parity is independent. Old binaries may ignore additive policy fields; serde
compatibility alone does not establish security compatibility. Flag-off behavior
for the new mode must be closed, never fallback to the old manager owner route.

Use separate runtime controls for public AI and personal-channel execution;
public budget exhaustion must not disable user-funded personal NyxBot. An
overall manager emergency stop disables both. Preserve existing profile `enabled`
semantics for old installations so their customer bots are not accidentally
paused by a new guide-only toggle. Deployment changes need documented mapping
between the old profile generation and new runtime policy revisions.

Roll back by pausing new traffic, stopping/revoking purpose-bound work and keeping
authority tombstones on a compatible enforcing binary. Do not downgrade to a
binary that interprets new restricted credentials as ordinary full-authority
keys. Ordinary older clients/nodes remain supported; this design does not require
a CLI or machine protocol change unless phase 0 identifies a concrete dependency.

## 10. Review and unresolved evidence

- Primary source review: completed for the relevant NyxID identity, routing,
  credential, profile, quota, billing and history boundaries listed above.
- Opus review: **converged, no remaining architectural blockers**, after source
  review and revision of B1–B7. See
  [the independent review and final verdict](2026-10-15-public-personal-nyxbot-opus-review.md).
  Resolved: consent/configuration separation, independent personal capacity,
  exact transport revocation, phase dependencies, signup lifetime/invitations,
  owner-thread lifecycle and mixed-version proof handling. The final non-blocking
  implementation conditions are incorporated above.
- NyxAgent native-tool/cost contract: unverified in this repository; phase 0 gate.
- Product defaults proposed here: 1 credit total/day, 0.1 credit/public person/day,
  daily UTC windows, durable revocable personal consent, 24-hour public context,
  deterministic group handoff and text/approval delivery with app-only images.
  Preserve existing configured monetary limits rather than resetting them.
- Signup policy: respect the existing invitation gate. A permitted registration
  path is a launch requirement; public tools do not manufacture invitations.
- Client bot provisioning: retained as an optional existing feature; removal is
  not inferred from the revised manager requirements.
- No builds, integration tests, live Telegram messages or production changes are
  claimed by this planning work.
