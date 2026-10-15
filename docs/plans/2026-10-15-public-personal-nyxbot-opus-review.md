# Opus review: public guide and personal NyxBot through one Telegram bot

Reviewed: `docs/plans/2026-10-15-public-personal-nyxbot.md` as of 2026-10-15
(branch `guide-setup-simplify`, `2dd4e0297`). Read-only review; no builds, tests
or product edits. Source anchors were checked in this worktree.

## Verdict

**Converges, conditional on the blocking revisions below.** The architecture is
right: personal parity with the user's existing NyxBot, a dedicated origin
instead of a synthetic `NyxbotChannel`, a new Telegram thread excluded from both
home-adoption paths, yes-code decisions, a public-only sponsor quota with no
personal fallback, and phase 0 gating only the public NyxAgent guide.

The resolutions you have already accepted close my earlier findings, provided
the text below lands in the document:

- durable personal consent until unlink/revoke
- text plus approval delivery, with images available in the app
- 24-hour public context retention with explicit upstream cleanup
- `nyxagent/chat` only when a restricted context is verified, otherwise a
  dedicated public profile as a prerequisite
- no `PlatformExecutionProfile` variant
- the frontend path correction

The remaining blockers are about which authority each lifecycle event touches.
The draft states them as goals ("must not disable", "must not stop an unrelated
web turn") without saying which records and filters enforce them.

## Blocking omissions

### B1. Guide setting changes must not touch personal consent

Today every guide save and every pause increments `PlatformBotConfig.generation`:

- `services/platform_bot_config_service.rs:1256-1257` (configure/enable)
- `services/platform_bot_config_service.rs:380` (pause)

Onboarding bindings are filtered on that generation:

- `live_for_sender` filters on `profile_generation` (`services/platform_binding_service.rs:363-364`)
- `fence_reply` checks it again before each reply (`:378-395`)
- `confirm_from_chat` and `preview` fence on it (`:264-269`, `:332`)

So with the current mechanics, editing the guide's model, profile or budget, or
pausing the public guide, silently ends every link. That is acceptable for
onboarding. It would break durable personal consent. The plan must state:

- A personal binding is keyed on the stable profile ID plus `manager_bot_id`. It
  never stores or fences on the guide `generation`, guide configuration
  revision, public budget or `profile.enabled`.
- Personal admission and delivery fence on exactly these:
  - the binding's own generation and active state
  - the user being active
  - a separate personal-routing control on the manager
  - the manager emergency stop
  - manager identity: same `manager_bot_id`, and the current `observation_id`
    checked at send time, as `send_reply` already does
    (`services/platform_guide_service.rs:367-372`)
- A token rotation that keeps the same `manager_bot_id` preserves consent. A
  different Telegram bot identity, retirement, deletion or un-conversion revokes
  every personal binding.
- Personal linking requests also must not fence on the guide generation.
  Otherwise an admin's budget edit invalidates every in-progress signup.
- Add a test: change the guide profile/budget, pause the public guide, rotate the
  manager token, then show personal routing still works. A changed manager
  identity, or the emergency stop, closes it.

### B2. Public budget exhaustion and public outbound limits must not disable personal routing

Section 9 states the goal. The draft misses that the shared native reply
admission is a public-only throttle today:

- `admit_native_reply_in_session` (`services/platform_binding_service.rs:802-863`)
  admits at most 10 replies per sender and **100 per profile per minute** across
  all senders.
- Guide work uses one 32-permit process semaphore (`GUIDE_WORK`,
  `platform_guide_service.rs:253`).

If personal deliveries reuse these paths, a public flood starves every linked
user, and personal volume starves the guide. The plan must specify:

- Personal turns never pass through `GUIDE_WORK`, `platform_inference_service`
  admission, public quota scopes or `platform_native_replies` admission.
- Personal outbound sends use their own per-chat Telegram pacing and the user's
  existing owner turn pool (live/concurrent turns per settings).
- A manager-wide outbound budget sits under Telegram's per-bot limits and is
  reserved for personal traffic, so public replies yield first when the bot
  token approaches Telegram limits. Treat the exact Telegram limit values as
  unverified external facts. Make the split a configured safe value, not a
  hard-coded claim.
- The hidden "10 starts/hour, one concurrent turn" limits in section 3 apply only
  to public senders. State this explicitly.
- Add a test: exhaust the public budget and sender quota, saturate the public
  reply window and the guide semaphore, then show a linked user's turn is
  admitted and delivered while public senders get the fixed quota reply.

### B3. In-flight work on revocation: exact scope

Section 4 says revocation "requests Stop for the exact admitted turn, and
invalidates queued work and async continuations under its old generation". It
does not define which work counts as "Telegram work". Under parity, most delegated
work is the user's own authorized account work that also appears on the web.
Required semantics:

- **The transport is revoked, not the user's authority.** Unlink, replacement,
  account disable and manager stop revoke the Telegram sender's ability to start
  turns, decide cards and receive output. They do not revoke the user's NyxBot
  authority.
- **Exact Stop of the Telegram turn only.** The Stop filter is the conversation ID
  plus `active_turn.turn_id` equal to the turn recorded on the ingress receipt,
  plus that turn's persisted origin binding ID/generation. A web-origin turn
  running on the same Telegram thread is not stopped. Neither is any turn on
  another conversation.
  - Reuse the existing conversation-scoped Stop and node cancel
    (`docs/chat/08-nyxagent-engine.md:923-933`).
  - Never use the standalone desktop Stop, which stops all agent activity on a
    machine.
  - Machine cancellation follows the BotFather rule: exact cancellation only
    where the node advertises it, with no broad-cancel fallback
    (`docs/PLATFORM_BOTFATHER.md:110-113`).
- **Queued Telegram messages** admitted under the old generation are discarded.
  Never re-run them under another identity or audience.
- **Delegated and asynchronous work is not cascade-cancelled.** This covers
  specialist turns, automations, connect-link and channel watches, machine jobs
  and proxy approvals started from a Telegram turn. It continues as the user's
  own work and reports to the web. NyxID does not record a reliable spawned-by
  provenance chain for all of these. Adding a broad "everything this thread
  touched" cancel would stop web-originated work, which the draft forbids. If a
  compromise-response cascade is wanted later, it needs explicit provenance on
  each artifact, which is a separate feature. Consent copy and the unlink
  confirmation should say: "Work already started keeps running in NyxID; stop it
  from the app."
- **After revocation the thread becomes an ordinary web thread.** Event wake-ups
  for it (watch resumes, specialist results routed to NyxBot, `group_settled`,
  card continuations) run without a Telegram destination. Every send path
  re-fences the destination proof, including late delivery and image notices.
  Late settlements are saved to the web transcript and never sent. The plan must
  say this explicitly instead of only "late sends refused".
- **Pending cards** raised by the Telegram turn stay decidable in the app. Telegram
  yes-code replies are refused after revocation, because the binding generation
  check happens before `decide_reply`.
- **Replacement** (Telegram identity relinked to account B) revokes A's binding in
  the same transaction as B's confirmation, as `confirm_from_chat` already does
  for setups (`platform_binding_service.rs:338-350`). A's thread and credentials
  are never reassigned. A's running Telegram-origin turn is stopped by the exact
  filter above.
- Add a test: during a Telegram turn that has delegated to a specialist and has a
  web-origin turn queued on another thread, unlink. The Telegram turn stops. The
  specialist and the other thread continue. The specialist's result lands in the
  web thread with no Telegram send. A late "yes 1234" from Telegram is refused.

### B4. Remove the stale 30-day text and fix phase dependencies

These sections still contradict the accepted resolutions:

- **Section 4 "Proposal for personal session lifetime" and section 10 defaults**
  still say 30 days. Replace them with durable consent until unlink, revoke,
  replacement, account disable/delete or manager retirement. Onboarding binding
  expiry and guide configuration have no effect (see B1).
- **Section 7, line 314** says phases 1 and 2 follow phase 0. The accepted
  position is that only public activation waits for phase 0.
- **Phase 2 currently owns "Factor shared NyxAgent execution"**, which phase 3
  (personal) needs. Move that factoring into phase 1 or a phase 1b, so personal
  work has no dependency on the public runtime or on the upstream proof.
- **Phase 4 cites `frontend/src/pages/admin/platform-bots.tsx`.** The real file is
  `frontend/src/pages/admin-platform-bots.tsx` (with
  `admin-platform-bots.test.tsx`).

### B5. Signup continuation will not survive the current request lifetime or the registration gate

Two existing facts break the journey in section 3:

- **Request lifetime.** Linking requests expire after 10 minutes
  (`platform_binding_service.rs:223`). Sign-up plus email verification routinely
  takes longer.
- **Registration gate.** Registration is gated by the default-on
  `auth:invitation-code` flag (CLAUDE.md, Environment Variables). A public visitor
  sent to sign up hits an invitation wall unless the platform issues codes.

The plan must decide both:

- **Request lifetime:** either a longer-lived, single-use, server-held linking
  intent for the personal purpose (still HMAC-only, sender-bound, revocable), or
  "after signing up, send /link again". Pick one and test it.
- **Invitation gate:** whether the public guide can issue or attach an invitation
  (a product and abuse decision), or only explains how to get one. The
  `public_signup_link` tool contract depends on this answer.

### B6. Personal turns must be first-class owner turns with explicit availability failures

- The admitted personal turn is an owner turn, not a guest turn, of the user's
  NyxBot on the dedicated thread, with Telegram as the chat-app location.
- Check the user's `assistant:nyxagent-engine` flag and assistant readiness
  before admission. Missing engine, missing wallet or funds, or a full owner pool
  returns a fixed, metadata-only reply. It never falls back to the public guide
  (section 4 step 4 says this only for lookup failure; extend it to these cases).
- **Dedicated thread lifecycle.**
  - Section 4 binds one conversation ID per binding. Define what happens when the
    user deletes that thread in the app. Recommended: the binding's conversation
    is cleared atomically by the delete transaction, and the next Telegram message
    creates a new dedicated thread.
  - The thread must be created through the engine path whose home-adoption guard
    excludes `channel` threads (`services/assistant_nyxagent.rs:2273-2283`), and
    never through `assistant_team_service.rs:612-620`, which adopts home without a
    channel check. Both exclusions need tests.

### B7. Rolling compatibility for the proof flow itself

Section 4 binds a closed purpose into link requests. Old replicas still handle
`/link`, `decide` and `confirm_from_chat`. They would treat a personal-purpose
request as onboarding, or confirm it without writing personal consent. Neither
is a privilege escalation, but either gives the user a silent wrong outcome.

Require one of these:

- Store personal requests in a separate collection that old replicas never read.
- Issue them only behind the personal-routing flag, enabled after every replica
  enforces it.

In both cases, the personal flag-off path is closed: those messages go to the
public guide, with no personal fallback. The same goes for the receipt: a single
ingress receipt shared across audiences (section 2, last row) must not be issued
by old replicas in a shape new replicas misread. Use a new receipt kind rather
than widening the existing purpose-scoped one in place.

## Non-blocking conditions

- **Operator privacy (personal).** A2 covers admins chatting in the public
  context. Add the matching personal gate: the manager admin and the sponsor have
  no API/UI read access to personal threads, and manager-level logs and audit
  carry identifiers and counts only.
  - The consent page must disclose that messages pass through a
    platform-operated Telegram bot, and that Telegram chats are not end-to-end
    encrypted.
  - Personal replies should keep `protect_content` like the guide does.
- **Word decisions.** Parity reuses `reply_decision` / `decide_reply` scoped to the
  dedicated conversation (`handlers/nyxbot.rs:2675-2717`). The "since the owner's
  previous message" anchor must count only that thread's owner messages,
  including web-origin owner messages in the same thread.
- **Public key ownership.** State who owns the public-purpose credential row (for
  example, the sponsor organization) and confirm platform-key billing then
  resolves to that owner through `resolve_for_execution`'s managed-master branch
  (`services/billing/owner_resolver.rs:53-67`). Prove the org-owned-key inventory
  semantics ("org-owned keys act as the org") are fully closed by the public
  purpose on every route, not only MCP.
- **Public model hop.** Keep the explicit exclusion of public keys from
  `assistant_model_call` (`handlers/proxy.rs:3204-3228`, call sites at 3488 and
  3666). Also validate the proxied request's model against the pinned value. A
  pinned service alone still allows an expensive model on that service.
- **Public context retention.** The 24-hour retention covers NyxID records, the
  encrypted credential and the upstream `store: true` session. If upstream cannot
  delete sessions, record that as a phase 0 finding rather than claiming cleanup.
- **Readiness spend.** Profile discovery and readiness spend sponsor money (current
  docs already say this). Count it against the total public budget so repeated
  Enable clicks cannot exceed the cap.
- **Document style.** The plan has several tables. They are fine for an
  engineering artifact. If you intend to show the plan to the user directly,
  convert the key tables to lists, per their stated preference.

## Status of earlier findings

All of these are resolved, by the accepted resolutions above or by the draft as written:

- Parity, not a per-Telegram ceiling (sections 1 and 4 of the draft).
- Never touching `owner_sender_ids`, and the dedicated origin.
- Dispatch order and the single receipt (section 4), subject to B7.
- Sponsor billing on the outer and inner hops, with exact caps (section 6).
- The upstream unverified gate (sections 2 and 5, phase 0), with the accepted
  dedicated-profile fallback.
- BotFather kept as an optional, separate flow.
- No sponsor fallback for personal turns (section 6).

Once B1 to B7 are reflected in the document, I have no remaining architectural
objection. Implementation review should then concentrate on:

- the fence sets in B1 and B3
- the outbound partition in B2
- the phase 0 contract evidence

## Convergence verdict (2026-10-15, final review of the revised plan)

**Converged. No remaining architectural blockers.** I re-read the revised
`docs/plans/2026-10-15-public-personal-nyxbot.md` in full (541 lines). Each
earlier blocker is now resolved in the text:

- **B1, consent independent of guide configuration.** Fixed in section 4
  (lines 172-203): `platform_bot_runtime_controls` keeps a stable manager
  authority epoch separate from `public_policy_revision`. Personal fences never
  read the guide `generation`, `profile.enabled`, funding or quotas. Same-identity
  token rotation preserves consent. A different bot identity, retirement or
  deletion advances the epoch. Whole-profile Pause/Retire set the emergency stop.
  Guide-only endpoints change public controls only, and stale old guide edits are
  rejected with an actionable conflict. A14 tests it.
- **B2, capacity isolation.** Fixed in section 4 "Capacity separation" (lines
  335-350): personal work stays out of `GUIDE_WORK`, public quota scopes and
  `platform_native_replies`. Personal sends get a separate per-chat pacing policy,
  plus a manager send scheduler with reserved personal capacity. Exact Telegram
  limits are marked as an external contract to verify. Section 3 marks the 10/hour
  sender limit public-only. A14 tests it.
- **B3, transport-only revocation.** Fixed in section 4 "Revocation" (lines
  295-326):
  - Stop matches conversation, turn ID, origin binding ID and generation exactly.
  - Cancellation is exact only, with no broad desktop or node fallback.
  - Committed specialist, automation, watch and proxy work continues on the web
    and is explicitly not cascade-cancelled.
  - Cards stay decidable in the app, late sends are fenced, and the thread stays
    usable on the web.
  - Account disable/delete keeps stronger global cleanup, and replacement is a
    single transaction.

  A6 and A17 match these semantics.
- **B4, stale text and phase order.** Fixed: durable consent replaces the 30-day
  wording in sections 4 and 10. Phase 0 gates only public activation (phase 3).
  Common execution moved into phase 1, and the frontend path is corrected.
- **B5, signup journey.** Fixed in section 3 (lines 122-139): a 24-hour,
  HMAC-only, sender- and epoch-bound intent, followed by a fresh 10-minute
  post-authentication challenge. The invitation policy is enforced unchanged, and
  open registration or invite distribution is an explicit launch prerequisite,
  with no bypass. A16 tests it.
- **B6, owner turn, failure behaviour, thread lifecycle.** Fixed in sections 4
  (lines 228-232, 260-269) and A17:
  - Personal work is an owner turn (`guest = false`).
  - Engine, readiness, funding and pool failures get a fixed response, with no
    public fallback.
  - Thread deletion clears the binding atomically, and recreation has one winner.
  - Both home-adoption paths, and the recovery fallback, exclude `platform_thread`.
- **B7, rolling compatibility.** Fixed in sections 4 and 9: separate personal
  intent and challenge collections, and a new `platform_manager_ingress_receipts`
  collection. Legacy receipts are never reinterpreted. Mixed-version activation is
  prohibited, and no `PlatformExecutionProfile` variant is added.

The public cleanup design (lines 402-413) is also acceptable. A revoked,
cleanup-only key whose single permitted action is deleting one exact session is
consistent with "no execution authority", provided phase 0 proves the protocol.
The plan states that it does not promise deletion until then.

### Non-blocking conditions for implementation

These need definitions in the PRs. None changes the architecture.

1. **Event turns while linked.** Event, watch, card and specialist-result
   continuations on a `platform_thread` take their Telegram destination from the
   live binding generation recorded on the thread, not from a past ingress receipt.
   Specify this alongside "delayed results carry the same destination proof"
   (section 4). The send fence still rechecks it.
2. **Clearing the emergency stop.** Resuming the whole profile must not silently
   clear the emergency stop and re-enable personal routing. Clearing it should be
   an explicit, audited admin action on the runtime controls.
3. **Several Telegram identities per user.** The unique index prevents two owners
   per sender, but does not say whether one user can link two Telegram accounts to
   the same manager. Recommendation: allow it, with one dedicated thread per binding.
4. **Consent disclosure of decision power.** `decide_reply` has no
   action-class restriction (`services/assistant_acknowledgement_service.rs`,
   around lines 2028-2043). A Telegram `yes 4821` can therefore approve
   destructive, machine and saved-login cards, as it already can on personal
   channel bots. The consent copy should say so in one sentence. Under the accepted
   parity design this is not a defect.
5. **Sponsor-owned public keys in generic management.** Line 248 rejects them from
   inventory APIs. Org-admin rotate, delete and edit on `/api-keys` must also refuse
   them, or preserve the public purpose. A successor must never become an ordinary
   org key.
6. **Status lines.** Update section 10's "Opus review: requested" line and the
   header status once this verdict is recorded.

Remaining disagreement: none. The plan is ready to hand to implementation in the
stated phase order, subject to the phase 0 evidence it already requires.
