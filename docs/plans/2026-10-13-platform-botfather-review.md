# Platform BotFather consultation and plan review

Date: 2026-10-13. Plan: [construction plan](2026-10-13-platform-botfather.md).
Scope: source/design review only. No implementation, production disconnection
or deployment occurred during this planning turn.

## Fable 5.1 consultation

Requested through the [Heca skill](/Users/chronoai/.agents/skills/heca/SKILL.md),
using the configured Claude provider's discovered `claude-fable-5-1` model,
high effort. Agent `01a135a6-63b0-7b02-8919-fa4320cc827d` inspected the current
repository and recovered restaurant discussion. Its substantive report was
delivered to the parent in this session. This document is a synthesis, not a
verbatim transcript or implementation sign-off.

### Findings adopted

1. **Observed discovery does not prove execution.** Owner specialists can list
   the personal directory to request grants. The operator's notification
   Telegram ID can make them a verified channel owner, selecting the specialist
   home thread. Empty execution grants do not hide owner metadata. Confirm
   deployed versions and thread activities; discovery is not fully represented
   in ordinary MCP execution audits.
2. **No owner token is not enough.** Manager relay omits owner delegation, but
   ordinary NyxBot linking independently chooses a person-bound conversation
   key. New platform authority must persist across transport and turn layers.
3. **Reuse guest exclusion, then close its remaining gaps.** In platform mode
   no sender is a personal owner. Exclude private context and permission cards;
   close `tools/list`, catalog/connected discovery and direct/wrapped dispatch.
   Templates are the guide's public offering directory.
4. **Manager deep links need their own protocol.** Manager `/start` consumes
   its creation challenges; a normal NyxBot owner pairing payload is not a
   functioning account-link contract there. A browser-approved identity watch
   can resume the guide without a `/start` completion payload.
5. **Use human registration and explicit review.** Telegram identity and NyxID
   account identity remain distinct. Device-code-style review can bind them
   without delivering account tokens to the guide. Keep invitation, MFA and
   account policy authoritative. Avoid notification/OpenClaw mappings as proof.
6. **Deliver human handoffs before autonomous setup.** Secure guide plus saved
   facts and existing trusted pages is a useful first release. Add digest-bound
   deterministic provisioning only after the authority contract is tested.
7. **A service account is optional.** In-process provisioning needs no external
   credential. An external worker may receive an onboarding-only purpose and
   scopes, but still needs client-approved step authority. Broad admin access
   and human-route impersonation are excluded.
8. **Budgets and capacity must be separate.** Public help must not share the
   operator's personal channel pool or silently charge their wallet. Add sender
   admission and explicit platform funding before public model traffic.
9. **Restaurant is a template over shared onboarding.** Start with published
   FAQ and reservation-form links and zero customer connector grants. Keep the
   client's own NyxBot as their management interface.
10. **Revocation and cleanup are first-class.** Bindings, guide sessions,
    requests and keys need account/channel deletion hooks and generation fences.
    Verify actual current cascades; generic bot deletion alone must not be
    assumed to disconnect every assistant link or cancel late effects.

### Deliberate differences from Fable's proposal

- The plan specifies a typed platform execution profile but leaves the exact
  channel/config/key representation to a reviewed PR 1 contract. The report's
  “3–4 mode checks” is not an effort estimate: credential, discovery, upstream
  session, funding, recovery and effect boundaries must all be traced.
- Identity bindings are scoped to platform configuration and manager identity,
  with a generation. A global Telegram-to-user index is not adopted without an
  explicit cross-bot consent and environment policy.
- One setup record becomes the installation. Minimal per-sender security state
  and binding records are separate; a second competing setup/session/job source
  of truth is avoided. Step attempts/outbox may be separate operational records.
- V1 follows existing signup policy. Automatic bot-issued invitation codes are
  deferred; a source label never bypasses the global invitation gate.
- V1 is personal destination ownership. Organization schemas remain compatible,
  but enabling org-specialist channels needs separate live-ACL, key, funding and
  revocation proof; current ordinary channel behavior is not assumed to support
  every org-specialist combination.
- UUID v4 model IDs remain the repository convention. Idempotency uses stable
  setup/step/revision keys and unique indexes, rather than broadly introducing
  UUID v5 resource IDs as suggested in the consultation.
- Existing guest-discovery/cleanup findings should receive bounded regression
  work, but are not permission to redesign all personal specialist semantics.
  Platform privacy has its own enforced boundary.

## Independent adversarial review

GPT-6-Astra reviewed the draft read-only, via the Blueprint skill's required
independent review step. It found no additional critical issue in the overall
authority design, and identified three high-priority gaps. This is a plan review;
final implementation still needs independent adversarial review and execution.

| Finding | Source evidence | Disposition |
| --- | --- | --- |
| Exact child correlation unavailable upstream | `telegram_new_service.rs` active-request indexes and sender-based pending request selection; Telegram does not echo arbitrary request ID | Preserve serialization, queue other setups, review exact child in trusted UI; stale old form cancellation/restart test; A14 |
| Owner-only private setting does not gate groups | `nyxbot_chats.rs::members_may_talk`, owner presence and ordinary access setters | Installation-wide admission/effect gate across groups/private/follow/native/gateway/rebuild/reply; bounded test mode; A15 |
| Generic auto-connect worker ignores future setup authority | `telegram_new_worker.rs` claims ready/provisioning; `telegram_new_connect.rs` handles token/webhook/registration effects | Carry setup approval/binding generations into request, enforce worker/connect/registration transaction, reconcile effects already begun; A16 |

The reviewer also confirmed that person-only v1, default-off rollout, old-writer
drain, no personal-route rollback, explicit FAQ publication and no account-token
delivery are appropriate boundaries.

After revision, the same independent reviewer rechecked all three findings and
confirmed they are addressed at the plan level, with no unresolved high finding
from that scoped review. Implementation and execution evidence are still required.

## Implementation gates still open

PR 1 must decide and prove execution/funding identity representation and native
versus gateway platform transport. Numerical budgets, retention durations and
upstream minimum versions need concrete reviewed configuration before pilot.
The current personal public route is still active unless separately contained;
writing this plan did not change it.
