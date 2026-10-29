# Shared agents and multi-person groups

Status: approved with corrections; Phase 1 implementation complete. Baseline: `origin/main` at `38bae2f78`
(NyxID 0.75.0, including #1996 guest-access helpers and #1995 guest model routing). Phase 2 remains planned.

This extends [NyxBot groups and guests](09-nyxbot-orchestrator.md) and the
[organization-agent contract](../ORG_AGENTS.md). The existing
[NyxAgent engine](08-nyxagent-engine.md), [agent isolation](../AGENT_ISOLATION.md),
[connection selection](../API_DISCOVERY.md#connection-selection-and-slug-collisions),
and [service execution](../NYXBOT_SERVICE_EXECUTION.md) remain the foundations.
Where this proposal changes behavior, it applies only to explicitly shared
specialists and the new multi-person group surface. Legacy personal groups,
organization groups and channel guests retain their contracts.

## 1. Outcome and fixed boundaries

A specialist's owner can share it with named people or an organization. Each
recipient gets private conversations, their own thread key, action cards and
billing. Owners and managers cannot read those conversations. In a multi-person
group, participants see one transcript while each agent answers through an
isolated execution thread for the original requester.

Shared chats use the agent's pinned skills for instructions and package reads,
just as the owner's turns do. Skills are untrusted behavior and never grant a
service, connection, operation, approval or tool; the live shared ACL and the
recipient's connection-consent path still govern every effect. Only the owner
can change pins, and a pin change changes the behavior digest so existing
durable connection consents stop authorizing the old behavior.

The Grok Team Bot comparison informs the share dialog, roles, shared memory and
private notes. It does not authorize access to every owner connection. A shared
turn can use only explicitly shared owner services within their shared-users
policy, or the acting person's exact connection to the same catalog service
after an explicit **Allow once / Always / Skip** consent card. Changing accounts
never happens silently. These consents remain necessary in Full access mode.

Personal NyxBot is never shareable. New multi-person groups contain hosted
specialists, not personal NyxBots or agents executing on participants' computers.
Shared execution does not inherit machine nodes, saved logins, SSH, Oracle,
owner-account `nyxid__` tools, owner approvals, channel links or automation
authority. For these two phases, both credential lanes use server transport;
an actor's node-backed connection is also ineligible. Existing personal use of
machines and existing organization-agent paths are unchanged.

Only a live agent owner, an eligible organization member, or a current share
recipient gets an authenticated turn. An unauthorized group participant may get
a guest turn only with the agent owner's explicit opt-in for that exact group
and agent. The default is off. Guests retain the channel guest contract: owner
execution identity and billing, no consent/action/approval cards, no owner
approval reuse, and no NyxBot tools. Otherwise NyxID posts a fixed, polite notice
attributed to the addressed agent: “Ask this agent's owner to share it with you.”
The refusal starts no model turn and incurs no model charge.

## 2. Roles and publishing authority

Agent ownership stays `AssistantAgent.user_id`, a person or organization User.
There is no ownership transfer and no new `AgentKind`. Roles are agent-specific;
they are not new `OrgRole` values or account permissions.

| Capability | Owner | Shared manager | Shared user |
| --- | --- | --- | --- |
| Use the agent in a private chat or eligible group | Yes | Yes | Yes |
| Edit display name, description, specialty and persona | Yes | Yes | No |
| Edit explicitly shared team memory | Yes | Yes | No |
| Edit own private notes and own thread model/settings | Yes | Yes | Yes |
| Add/remove people or org audiences; appoint managers | Yes | No | No |
| Change grants, shared/guest levels, operation selection or credentials | Yes | No | No |
| Change skills, model defaults, access policy, nodes, logins, channels or automations | Existing owner authority | No | No |
| Enable guest fallback in a group; destroy/purge the agent | Yes | No | No |
| Read another person's private chats, notes, cards or context | No | No | No |

Managers use an explicit field allowlist; reusing `require_maintain` would be
unsafe because org maintenance currently includes grants and credentials.
Profile edits are untrusted instructions, never authority. They advance a
configuration revision and reset affected sessions through the existing
instruction fingerprint. An actor's durable personal-connection consent is
invalidated by a changed behavior revision (profile, persona, skills or model
default), so a manager cannot silently change the bot that received that consent.

Direct person shares may grant `manager` or `user`; org audiences grant `user`
only. An org audience may name only an organization where the personal owner is
an active member, so the owner can recognize its slug/name without turning a
sharing preview into an organization directory. Its effective recipients remain
active members with `can_proxy()` of that active org, including its live
membership lifecycle, not an expanded list copied at share time. Viewers are
excluded. A direct share is an independent explicit grant;
removing an org audience does not revoke a surviving direct share. The owner UI
shows all sources of effective access and supports removing all such sources.

Publishing is personal-specialist-only in both phases. Organization-owned
specialists cannot be shared externally or opted into guest turns. Existing
organization Admin/Member maintenance, use and resource authority stay intact.
Personal specialists may be shared with an organization's live `can_proxy()`
members. External organization publishing and owner-paid organization guests
are future extensions requiring explicit owner approval.

## 3. Additive data model

New model collections have `COLLECTION_NAME`, UUID-v4 string `_id` values,
redacted Debug implementations where needed, and BSON datetime helpers. Optional
dates use `bson_datetime::optional`. Model fields must serialize to MongoDB;
there is no `skip_serializing`. API responses are dedicated, role-filtered DTOs.
The Phase 1 storage and API names below track this implementation. Group models
remain Phase 2 proposals and are not present in 0.73.1.

### Agent sharing and memory

| Location | Proposed data and invariants |
| --- | --- |
| `assistant_agent_sharing` | Agent UUID `_id`, authoritative personal `owner_id`, defaulted `enabled: false`, write `version: 0`, authority `revision: 0`, `shared_access` map and explicitly published `memory`. `version` is the optimistic-concurrency token and increments on every non-no-op save; `revision` changes only when sharing is first enabled or an existing recipient's authority changes. A side collection prevents legacy agent replacement from erasing the policy. Empty/absent policy means withheld. Behavior is bound by a digest of persona, description, model, pinned skills and operation grants. Shared recipient projections retain those pinned skill references and metadata; skill package content is untrusted behavior, not owner-private memory. |
| `assistant_agent_shares` | `agent_id`, authoritative `owner_id`, `subject_kind` (`person`/`org`), `subject_id`, owner-visible `subject_email` for person labels (defaulted for legacy rows), `role` (`manager`/`user`), revision and creator/timestamps. Unique `(agent_id, subject_kind, subject_id)`; indexes for subject-to-agent discovery and agent cleanup. No stored `owner` role: it is derived from ownership. |
| `assistant_agent_consents` | Actor, agent, exact catalog and connection IDs, credential owner/epoch, execution-authority digest, behavior/share revisions, consent scope, decision and timestamps. Consents bind the actor's exact conversation and key as well as the connection/scope; actor and conversation indexes support listing/revocation. `always` remains valid until revoked or any binding changes. No raw credentials or tokens. |
| `assistant_agent_private_notes` | Actor, agent, bounded notes, revision and timestamps; unique `(agent_id, actor_user_id)`. Reuse the existing 50-note/500-character bounds. Only the actor and that actor's private execution context can read them. |
| Published team memory | Bounded notes in `assistant_agent_sharing.memory`, edited by owner/managers under the policy revision. No automatic transcript extraction. It is never read through the existing owner-memory store. |

`AssistantAgent.memory` remains the owner's private memory for personal agents;
sharing never publishes or copies it. Existing organization memory remains
organization data, visible only under its existing membership contract. External
share recipients see the new explicitly published personal-agent memory.
Organization agents cannot be externally published in either phase. Owners may explicitly copy selected notes into
team memory after a preview. Automatic learning and checkpoint creation must
never write a person's private content into team memory.

`shared_access` maps the exact granted owner service identity to the **same
`GuestAccess` type and policy evaluator delivered by #1996**, including withhold,
NyxID's append-only marker and operation selection. The shared-users audience has
its own stored policy; it must not change channel guest policy. Unlike the
legacy guest map's implicit `use`, absence here is deny/withhold. Reuse #1996's
operation-selection representation and intersection semantics rather than
creating a second permission language. Rebase after `fix/guest-access-1996`
lands, then use the final names and helpers from that implementation.

Removing an owner grant atomically removes its shared policy and invalidates
affected consent/authority generations. Regranting starts withheld, even if the
same service ID is reused. Editing a grant must not restore an old share policy.
Policy writes use `expected_version`/`version` CAS. A true no-op advances neither
counter. Add-only audience or service changes advance `version` but preserve the
authority `revision`, existing Always consents, active turns and sessions.
Removing or downgrading a subject, withholding/changing service authority, or
disabling sharing advances both and revokes/fences affected authority. The
authority revision remains the value bound into shared keys and consents; stale
write versions always return 409, including two concurrent add-only saves.
Owner-only widening never follows from a manager profile edit or a recipient's
consent.

### Groups and invitations

| Location | Proposed data and invariants |
| --- | --- |
| `AssistantGroup` | Defaulted optional `collaboration` object containing `version: 1`, creator ID and membership revision. For new groups `user_id` remains the personal creator; reuse `participant_user_ids` (1–16 active people), `member_agent_ids` (1–8 specialists), per-actor sticky recipients and lead. |
| `assistant_group_invites` | Group, creator/issuer, SHA-256 token hash, optional intended person, expiry, acceptance/cancellation timestamps and accepting person. Unique token hash; group/state lookup and expiry cleanup indexes. Raw random 256-bit token is returned once and never logged. |
| `assistant_group_agent_access` | Unique `(group_id, agent_id)`, authoritative agent owner, adding participant, membership generation, `guest_enabled: false`, consenting owner representative, revision/timestamps. Only the live owner/authorized org Admin can enable; the creator can disable or remove the agent. Removing/readding the agent resets opt-in to off. |
| `GroupRequest` | Reuse immutable original `actor_user_id`, request ID, trigger sequence, pending recipients, attachments and handoff budget; add defaulted collaboration marker and membership generation. Client input cannot set requester identity. |
| `GroupMessage` | Add a defaulted collaboration routing marker independent of author, plus optional execution attribution and original requester. Keep role strings `user`/`agent`/`notice`; do not add persisted variants to old role enums. |

`org_group_service::is_org` currently tests `created_by_user_id.is_some()`.
Do not set that field on new personal multi-person groups. Centralize group
classification: explicit collaboration marker first, otherwise the existing
org discriminator, otherwise legacy personal. All group handlers, sweepers,
purges, upload readers and live routing must use that classifier. Unknown
collaboration versions fail closed. No lazy conversion of a legacy group occurs
when someone reads it; these phases create new groups to avoid exposing old
private transcripts through a membership edit.

The creator manages the name, agent list, lead, invitations and people. Other
participants may propose agents they can use; adding one is a creator-authorized
group mutation. People join by accepting a one-time invitation while signed in
to a NyxID person account. Preview is non-mutating and does not expose the
transcript or token in logs. Accept atomically claims the unused invitation and
adds the person under the live creator/group revision and 16-person cap. Two
acceptors cannot consume the same invite. TTL is cleanup, not the expiry check.
Creator cancellation and group deletion win against stale acceptance.

Quick-add is creator-only and requires both people to share at least one active
organization, checked live in the membership transaction. An organization's
Viewer may qualify as a group person through this path; that does not grant use
of its agents or services. Later departure from that common org does not undo
explicit personal-group participation; it does immediately remove org-derived
agent use. Invite/quick-add grants transcript participation, never an agent share.

Anyone can leave immediately. Revoke participation, invalidate cards and queued
work, stop live turns and revoke hidden keys before asynchronous cleanup; do not
retain access until a busy turn finishes. This strengthens the new-group path
relative to today's org-group “refuse while live” removal. The creator can leave
after explicitly transferring group management to a consenting participant;
otherwise leaving deletes the group after a destructive-action confirmation.
The last participant's departure deletes the group. Group-management transfer
never transfers agents, credentials, consents or guest authorizations.

Proposed invite lifetime is seven days, with bounded issuance and acceptance
rate limits. Invitations can be bound to an existing account or be one-use bearer
links for a signed-in account; acceptance always names the actual person. There
is no anonymous chat access, email-suffix policy or automatic message delivery.

### Threads, keys and consent bindings

An authenticated shared conversation stores `user_id = acting person` and
`agent_owner_id = real owner`, as org-agent conversations already do. Extend the
key/conversation/active-turn binding with defaulted optional collaboration
metadata: agent, original requester, access source, optional group/request,
participant generation, authority generation and billing policy version. It is
server-authored and must agree across the conversation, live turn and key.
Never use `agent_owner_id` as a replacement for conversation ownership.

Private home lookup is `(actor, agent)`; the shared recipient never uses
`AssistantAgent.home_conversation_id`. Hidden group execution slots are unique
per `(group, agent, original requester, execution lane)`, where the lane isolates
authenticated from guest execution. A guest slot has owner-owned credentials,
but still records the original requester for membership, budget and attribution.
Use a separate slot/binding collection with a unique compound index to avoid
changing legacy hidden-thread uniqueness semantics; each slot points to a normal
restricted conversation. Concurrent creation must produce one key/credential.

Sharing/unsharing never converts a guest key into an actor key, or attaches a
recipient to an owner session. A change of effective authority clears the
upstream session, checkpoint and pending continuation capabilities as needed;
the next admissible turn reconstructs only that lane's allowed history. No new
global enum variants are needed in `AgentKind`, `TurnOrigin`, `CredentialClass`,
`OrgRole` or existing acknowledgement kinds.

## 4. One identity rule

Resolve the original requester from the authenticated human or persisted request
chain, never from text, display names, mention strings or model tool arguments.
First require live group participation for a group turn. Then resolve the target:

| Live authority to the target agent | Turn identity, cards and payer |
| --- | --- |
| Requester owns it | Requester; their key/cards/approvals/billing |
| Requester has active org `can_proxy()` use rights | Requester; retain that org's resource scopes and policy |
| Requester has an effective person/org-audience share | Requester; `shared` restrictions and their key/cards/billing |
| No use right; exact group+agent guest opt-in is live | Agent owner; `guest` restrictions and owner billing; no cards |
| None of the above | Fixed refusal; no execution |

Use rights take precedence over guest fallback. A failed authorized turn (missing
connection, consent skipped, balance insufficient, approval denied, service
withheld) never retries as an owner-paid guest. An ACL lookup failure is an error,
not evidence of “no use rights.” A queued authorized turn whose access was
revoked is cancelled; it cannot silently change principal. A later, new human
request may select the currently available guest lane with clear UI attribution.

Each handoff copies the original requester and request ID, decrements the same
persisted budget and re-evaluates the destination with this rule. Guest A handing
to B does not make A's owner the requester. No current agent, group creator or
last speaker can donate identity to the chain. Apply existing per-request and
hourly handoff limits (defaults 6 and 60) to the original requester across these
groups. Owner-paid guests also consume the owner's existing guest/channel pool
and rate limits. Retries and continuations do not refill budgets. Keep the
existing pool, turn duration and tool-call bounds; dedupe handoff admission.

Guest busy handling follows channel behavior: give a bounded retry notice rather
than enqueueing a guest as the owner's ordinary event work. Authorized group work
can use the existing bounded pending-request mechanism. Multi-person groups do
not register personal NyxBot followers, copy transcripts to an owner's home, or
send unsolicited events to another person's NyxBot.

## 5. Live ACL and request-access snapshots

Compose the private `shared_agent_service::RequestAccess` snapshot into
`org_agent_service::RequestAccess`. Keep existing org helpers for unrelated
paths. New requests carry one immutable `Arc<RequestAccess>` through authentication,
chat authority, discovery, approval, billing and final execution. Its private
fields bind actor, agent, real owner, execution mode, optional group/request,
effective role/source, membership/share/policy revisions and eligible exact
connections. It may contain the already-resolved org snapshot. A pure matcher
must reject reuse for any other actor, owner, agent, group or request.

For a list or group admission, a batch loader:

1. Uses already-loaded agents/group and gathers distinct owners and candidate IDs.
2. Loads the actor's active memberships once and active referenced owners/orgs in
   one batch; reuses authentication's membership snapshot if available.
3. Loads matching direct/org share rows for all candidates, and group-agent
   opt-ins in one query each. Loads participants/agent profiles in batches.
4. Fetches granted/shared service rows, providers and relevant consents by `$in`
   only if this request discovers or executes those services. Applies live ACLs
   before same-slug resolution and dense preference ranking.
5. Computes capabilities in memory; per-agent/per-service predicates do no I/O.

Paginate lists and chunk bounded `$in` queries; no per-agent membership queries,
no per-service consent query loop, no decryption for discovery. A normal personal
key, unshared personal turn or existing org-only turn takes its existing path
without share, consent, collaboration-group or new flag lookups. The key's
defaulted binding and the loaded group's discriminator select the new path.
Test query counts for this property.

Snapshots are request-local, never stored as reusable ACLs or cached across
requests. Persist only attribution, versions and immutable request bindings.
Every new turn, retry, continuation, queued-work admission, key authentication,
approval decision, async-watch effect and handoff resolves live rights. A turn
snapshot is not authority for later tool HTTP requests.

Share/membership removal transactionally advances the relevant generation,
revokes affected execution bindings, expires pending cards/consents and requests
Stop for active turns. Live auth reads remain the authority while cleanup catches
up. Decisions and admission use CAS/transactional fences against those mutations.
Final egress follows the existing service-execution fresh-authority check after
approval/billing awaits, validating the snapshot's revisions and exact selected
resource. Streaming delivery and long-lived transports check cancellation/live
membership too. No new request or effect is authorized after revocation commits;
an already dispatched provider action cannot be recalled. Settlement still runs.
This is not an atomic transaction with an external provider's network receive.

## 6. Credential selection and approvals

The effective authority is an intersection, not a copy of the owner's key:
agent grants/scopes, the turn's lane policy, exact selected connection authority,
live share/group ACL, and the actor's consent where required. Ordinary keys never
gain access to someone else's resources merely because that person shared an
agent. `AgentServiceBinding` must not become a generic cross-owner credential
alias. The new resolver lives inside the bound assistant execution path.

### Owner-shared connection

Require an active granted connection, an explicit non-withheld `shared_access`
entry, and #1996's service level plus shared-users operation selection. Reuse its
trusted catalog effects, append-only metadata and method-override guard, and
intersect agent operation scopes and input constraints. Remote metadata may
only narrow authority. There are no service-name, operation-name substring or
word-based policy checks. Select the exact `UserService.id` before dispatch.

All owner-resource policy stays in force. Deny remains deny. If an operation
requires the resource owner's approval, refuse it in the shared lane; do not
create an owner card, borrow an owner grant or let the acting person authorize
the owner's account. The actor's own action confirmation can further restrict
an otherwise permitted operation, but cannot widen the shared level. This
matches the guest machinery's owner-approval ceiling.

For a platform-bound connection, apply the live platform-key ACL to the acting
person before decryption. The owner's restricted platform grant is not
transferable through sharing. Reuse the request membership/provider snapshot.
Exclude Internal account-service exposure and node routes. Never forward an
owner access token, delegation token or owner machine/login authority. If the
service needs those to execute, it is ineligible for sharing; a shared level
cannot authorize them. Ordinary permitted downstream credentials are injected
server-side and never exposed in tool results, cards or logs.

Pinned Ornn skill package reads use the same server-side, read-only adapter as
the owner's turn, with the recipient's bound conversation key and identity.
Fetch exactly the pinned reference (and its pinned dependency closure); never
fall back to the owner's personal credential, a recommendation or an
auto-granted service. Package guidance remains subject to the shared operation
and service ACLs above.

### Acting person's connection

A candidate must belong to the acting person, reference the **same immutable
catalog service ID**, and satisfy the agent's service/operation contract. A
matching slug, base URL, provider name or description is insufficient. Custom
services without a catalog identity have no automatic personal alternative.
Other orgs' connections are not “my connection” in this lane. The guest-only
owner-operation selection applies to owner material; an actor connection is
bounded by the agent's operation scope mapped against the catalog contract and
the actor's own permissions. Unmappable/changed contracts fail closed.

Before any call with actor material, present the exact service, safe account
label, agent identity, permitted scope, intended operation, billing attribution
and reply audience. The model can propose a candidate but cannot accept it.
Only the acting person decides through an authenticated human endpoint:

| Decision | Effect |
| --- | --- |
| Allow once | One atomic consumption for the exact operation/input digest and connection, bound to actor/agent/conversation/turn/request. A visible continuation may carry that one capability; sibling turns and handoffs may not. |
| Always | Authorize this exact actor/agent/connection and reviewed operation scope, for the reviewed audience class. It is revocable and checked live each time; it does not bypass ordinary operation approvals. |
| Skip | No actor credential is used. Continue without that operation; no automatic owner-account retry. Suppress repeated identical prompts within the turn. |

Private-chat consent never authorizes disclosure into a group. Group consent is
bound to the exact group and membership generation; adding/removing people
requires renewed consent. New agent behavior, owner/share policy generation,
credential replacement, changed endpoint/configuration/contract or revoked
share invalidates the applicable consent. Normal OAuth refresh preserves
`credential_epoch`; replacement bumps it through the existing pipeline update.

Use the existing acknowledgement rendering/decision/continuation machinery with
a separate consent payload record. Do not add an unreadable variant to an old
stored acknowledgement enum. Decisions bind actor, exact connection, agent,
thread/key, request, versions and deadline; pending `allow once`/`skip` records
expire with the card/turn. Durable `always` has no automatic expiry in this
proposal, but is removable from the user's agent settings. A decision never
reveals that user's connection metadata to the owner/manager.

The actor's own approval policies and action cards still apply after credential
consent. The actor's access mode governs their confirmations; an agent owner's
Full access override cannot suppress another person's cards. The consent
exception to Full access is deliberate: it decides account delegation, not risk
classification. Shared specialists cannot route missing grants or consent to
the owner's NyxBot. Even when the actor assigned work via their own NyxBot, only
the human can consent to their connection and only the agent owner can widen
owner-service policy.

### Selection is stable across transports

Discovery shows permitted owner connections and actor alternatives as distinct,
sanitized entries, with `credential_source` and `consent_required`. Listing an
alternative never grants it. Phase 1 carries these fields in additive
`shared_connection` metadata on MCP discovery and `/mcp/config`, and assigns
each shared connection a stable UUID-derived tool prefix so same-slug accounts
remain distinct. An exact call stays exact. Preferences, pools,
credential refresh, retries and outages cannot swap between these accounts;
another pool member needs the same fresh explicit admission. Unknown or
withheld owner resources are not disclosed through search or fallback hints.

Require a live bound turn unconditionally for new shared keys, independent of
the older `assistant:live-turn-gate` rollout. Set node authority empty, do not
copy `assistant:account`, and enforce forbidden protocols/meta-tools after
resolution as well as in discovery. A key allowlist alone is insufficient.
NyxAgent's bounded model-transport exception remains available as the actor;
it must never select an owner's unshared inference connection. Actor-owned
model credentials also require the applicable explicit connection consent;
an eligible platform model can bootstrap the first turn. A model bootstrap
consent uses the existing bounded stateless-model turn scope, not a blanket
provider grant, and excludes stateful provider actions.

## 7. Billing and audit attribution

The current `BillingOwnerResolver::resolve_for_execution` bills platform master
usage to the billing principal but other credential classes to the resource
owner via org ACL. Passing a foreign person's connection to it will correctly
refuse; manufacturing `OwnerAccess::Direct` would be a security bug.

Add a resolver entry point for an already-authorized assistant execution, taking
an opaque server-created billing authority derived from `RequestAccess` and
the final credential selection. Only the `shared` lane returns the acting
person's wallet for every credential class. `org_member` uses the existing
`resolve_for_execution`: org BYOK bills the org and platform-key usage bills the
person, including in new multi-person groups. `owner` retains current billing. It must not be an arbitrary caller-supplied
`payer_id` or weaken `resolve_for_resource`. Existing non-collaborative paths
keep `resolve_for_execution` unchanged.

For supported execution, carry the resulting payer through initial `llm-nyx` transport, downstream model
callbacks/continuations, metered tools, pool dispatch, managed/async jobs,
reservations, exact funding split, settlement, cancellation and retries. Freeze
the resolved payer on durable usage work so recovery never re-infers it from
the connection owner. Failure to fund the actor refuses the request, never
falls back to an owner or org wallet. Benefits, taxes and exact accounting use
that payer through existing billing machinery; no new money arithmetic.

Guest fallback retains owner identity, owner billing and the existing
channel pool. Organization-owned guest publishing is outside both phases.
Existing channel billing is unchanged.
External provider charges associated with a BYOK account still reach that
account's provider bill; caller-pays here means NyxID model and metered-service
charges, not moving an external subscription.

Append metadata-only events through `audit_service`, preserving hash chaining:
sharing/role changes, shared-service policy changes, consent decisions/revocation,
invitation lifecycle, group membership/guest opt-in, turn refusal and execution.
Include real `actor_user_id` (original requester), `agent_owner_id`, actual
execution principal, credential owner/source, billing owner, agent/thread/group/
request IDs, mode (`owner`, `org_member`, `shared`, `guest`) and outcome. Keep
these as additive metadata, not new values in a legacy credential enum. Guest
service logs keep owner execution identity and add the authenticated requester;
they must not mislabel the guest as the owner who requested the action.
Never log prompts, transcript/notes/skill bodies, consent arguments, invite
tokens, raw keys, ciphertext or upstream session bindings. Audit access itself
must not give owners/managers a private-chat activity feed.

## 8. Privacy and group delivery

| Data | Readers/context |
| --- | --- |
| Private shared-agent chat, context/checkpoint, uploads, tool results and cards | Acting person only; ownership/management of the agent gives no read access |
| Personal owner memory, pending work and roster | Owner-only paths; never supplied to another person's turn |
| Private notes | Actor's private conversations only; not injected into group turns, including that actor's own group execution |
| Published team memory | Current authorized users/managers/owner; not channel/group guests |
| Group transcript and explicitly published group attachments | Every current participant; no unrelated agent owner or manager access |
| Group action cards | Group sees safe status and who must decide; exact account labels/arguments and decision controls are actor-only |
| Hidden execution threads and credentials | Runtime only; no participant or agent-owner enumeration through ordinary history routes |

New participants see the existing group transcript; the invitation/quick-add UI
must state this before membership is changed. Former participants lose server
access immediately. Previously read/exported content cannot be recalled. A
person's departure preserves shared messages under group retention while deleting
their hidden execution artifacts after cancellation and settlement. Private
conversations remain that person's data; unshare disables execution, not their
ability to read/export/delete their own retained transcript and notes. Deleting
an agent never transfers these artifacts to its owner. User purge and group
deletion include consents, slots, credentials, notes, cards, uploads and async
bindings, retaining required billing/audit records under existing policies.

Even when the requester owns the agent, a new multi-person group turn receives
only group-visible context and explicitly shared team memory. It never injects
owner private memory/roster, private notes, direct chat history, home-thread
pending work or unrelated card decisions. Guest turns receive only permitted
group transcript text, omit team/private memory and retain channel guest upload
restrictions. They cannot fetch other people's uploads or call upload/machine
tools. Prompt and recap builders must enforce these projections in code.

Each lane has independent session/checkpoint state. A private chat cannot resume
a group session, and guest/authenticated lanes cannot resume each other. Privacy
projection changes advance the instruction fingerprint. Test recovery, reset,
voice/steering if supported, automatic title/learning work and late tool calls,
not only initial prompt assembly. Unimplemented entrances refuse new shared
bindings; they must never fall back to an owner path.

Only safe final replies and explicitly group-deliverable attachments are copied
into the group transcript. Hidden tool traces and consent details stay private.
Group upload reads require a live participant ACL, transcript reference and
retention check; tool reads additionally bind the upload to the triggering
request. A handoff can carry only that request's authorized attachments.
Approvals are authenticated clicks, never group text such as “yes 4821”. Only
the original requester can approve/deny/continue or Stop their execution; the
creator can cancel group work when removing someone or deleting the group,
without becoming its approver.

Extend `assistant_live`'s bounded, coalescing group worker to collaboration
markers; batch current participant eligibility before identifier-only fan-out.
The change-stream loop never awaits membership lookups. Recheck subscriptions
and delivery; on removal send no subsequent content and close/revalidate group
streams. Dropped identifier events retain polling/sweep as backstop. Do not route
by `GroupMessage.user_id` or author: even system notices need the group marker.

## 9. Frontend experience

Use the existing Assistant shell, agent details sheet, Radix/shadcn components
and [DESIGN.md](../../DESIGN.md) tokens. Domain Zod schemas, TanStack Query hooks
and `useAppForm` keep current conventions; forms with non-text controls must
become dirty and enable Save. The server returns caller capabilities so UI
visibility never substitutes for enforcement.

- **Share dialog:** owner adds new people by normalized email; existing person
  recipients are edited by their owner-visible UUID/email row. Organizations use
  owner-visible IDs and labels, and only organizations the owner belongs to are
  eligible. The owner sees effective roles,
  access sources, remove actions and an explicit enabled state. Manager and user
  descriptions say what they can edit. A “Services for shared users” section
  lists only the agent's grants, initially Withheld, with #1996's level and
  operation picker. Pinned skills are shown as part of the shared agent behavior
  and are not replaced by owner-private notes. Channel guest access is visibly
  separate. The dialog states that private chats stay private and recipients pay
  NyxID usage.
- **Shared with me:** sidebar section alongside Personal and organization agents,
  deduped by agent ID, with safe owner identity and caller role. Agent IDs or
  qualified owner/agent labels resolve collisions; bare names are accepted only
  when unambiguous. It contains only the viewer's private threads, never other
  recipients or the owner's work. Read-only retained chats remain accessible
  after unshare with execution disabled.
- **Consent:** exact account card with Allow once / Always / Skip; scope and
  audience are readable before deciding. Show a selected-account indicator on
  the turn and an actor-only list of remembered consents with Revoke. Others
  in a group see “Waiting for <person>”, not the account details.
- **Groups:** creator invites by one-time link, quick-adds eligible org peers,
  manages people and accepts agent additions. Join preview names the creator,
  history visibility and current participants; Leave is always reachable.
  Each participant has their own recipient pills/sticky choices. Mentions use
  server-resolved IDs with owner labels for duplicate names.
- **Per-agent guest toggle:** off initially, with an owner-only enable action
  stating that unauthorized participants use the owner's guest service policy
  and bill the owner. The creator may disable it. Other people see enabled/off
  and whether an addressed reply runs as them or as a guest. Adding participants
  invalidates actor group consents; guest opt-in explicitly covers future group
  participants until revoked, which the owner must see when enabling it.

## 10. REST, NyxBot tools and MCP parity

All assistant management, invitation acceptance and consent decisions remain
first-party human surfaces under `/api/v1/assistant/nyxagent`; conversation keys,
generic API keys, OAuth-client, delegated, service-account and relay credentials
cannot call them. Use the shared human guard and engine flag, not only a handler
ownership comparison. Phase 1 additions and Phase 2 proposed group surfaces:

| Surface | Contract |
| --- | --- |
| `GET/PUT /agents/{id}/sharing` | Combined audience/role and per-service policy; new people use `{subject_kind: "person", email}`, existing person rows use their owner-visible UUID, and org rows require an owner-active membership; responses include owner-visible labels plus `version` and authority `revision`; owner writes send `expected_version`, projected caller reads; stale versions return 409 and the route never creates grants |
| `GET /agents` | Includes live authorized shared summaries and actor-only retained-history entries; existing owner/org entries remain |
| `GET/PUT/DELETE /agents/{id}/private-notes` | Actor's own notes only |
| `GET/PUT /agents/{id}/shared-memory` | Authorized read, owner/manager revision-fenced edit |
| `GET /agents/{id}/consents` | Actor-only consent inventory, bounded to 100 current cards/grants |
| `POST /shared-consents/{consent_id}` | Actor-only `once` / `always` / `skip` / `revoked`; exact binding checked transactionally; separate from legacy action-card enums |
| Existing group CRUD/messages/attachments/live routes | Dispatch by explicit group classifier; current participant ACL |
| `POST /groups/{id}/invites`, `DELETE /groups/{id}/invites/{invite_id}` | Creator issues/cancels; raw token returned once |
| `POST /group-invites/preview`, `POST /group-invites/accept` | Body token, human account, non-mutating preview and single-use accept |
| `POST/DELETE /groups/{id}/participants/{user_id}`, `POST /groups/{id}/leave` | Creator quick-add/remove and self-leave; membership generation fence |
| `PUT /groups/{id}/agents/{agent_id}/guest-access` | Owner enable/disable, creator disable; revision-fenced |

Group transfer uses a human proposal/acceptance flow and cannot execute from a
model tool. New secret-bearing, streaming, execution-shaped or protocol GETs
must be denied by delegated-read classification before mounting. No URL returns
keys/session bindings. Not-found-shaped denials protect private resource IDs;
public group notices disclose only that the addressed agent is unavailable.
New errors, if necessary, are allocated only in `errors/mod.rs`, not here.

Personal NyxBot may discover and delegate to a shared specialist the actor can
use, just as it can discover eligible org specialists. `list_subagents` returns
role/owner/use metadata; `message_subagent` carries the original actor into their
private shared thread. `wait_for_subagents` includes only that actor's shared
thread replies, never the owner's home-thread reply. Shared discovery happens
in explicit tools; ordinary turn roster construction adds no sharing reads.
Report-back goes only to the exact assigning thread of
that actor. It never grants owner resources or forwards the owner's memory.

Add owner-scoped `get_agent_sharing` and `set_agent_sharing` native tools; the
latter replaces the combined audience and per-service policy at an exact write
`version` and returns both `version` and authority `revision`. It uses existing
human confirmation classification. Any widening delegation (enabling sharing,
adding or upgrading an audience subject, or adding/raising an owner service
level) always requires a one-use owner action card, even in Full access, because
it delegates account authority. Narrowing and no-op saves follow the normal
access mode; Full access may auto-allow them with its receipt. For a new person,
the tool accepts `{subject_kind: "person", email}` and echoes that normalized
email in the owner card. A UUID person selector is accepted only for an existing
audience row; organization selectors require the owner to be an active member of
that organization. Owner-only GET surfaces return only already-visible audience
labels. In Full access,
NyxBot may apply a widening only after the owner approves that card; this remains
an owner-scoped policy mutation and does not grant new services. Extend group management
tools with invite/quick-add/remove/leave and per-agent guest settings, calling
the same services as REST. Personal NyxBot cannot join/follow new multi-person
groups or receive their transcript as an unsolicited follower. Invite acceptance,
manager-role acceptance if added, and personal-credential consent remain human
decisions. No tool sends an invitation message automatically.

Shared specialists cannot invoke account/team/share/grant/connection management
tools against either person's account. The limited note interface, if exposed
through existing `remember`/`forget`, must route to the actor's private-note store
only in private shared threads; shared-memory edits require explicit authorized
human management. Guest turns expose none of these native tools.

Execution parity means one access/selection/policy evaluator covers
`tools/list`, `nyx__list_connected_services`, `nyx__search_tools`, `/mcp/config`,
`nyx__call_tool`, direct MCP calls, UUID/slug HTTP proxy, LLM callbacks,
WebSocket opens, pools, managed workers and async results. REST must not bypass
a restriction implemented only in today's `guest_service_refusal` MCP handler.
Unsupported transports refuse bound shared keys before dispatch. Existing
`nyxid mcp tools/call` exercises the MCP surface; human CLI commands, if added,
wrap the same REST DTOs rather than minting a broader key. No general Agent Key
gains assistant-management authority for CLI convenience.

## 11. Compatibility, flag and rollback

Register **`assistant:shared-agents`**, default off, in the existing feature flag
system. Add its enforcement capability to security readiness, using the existing
roster/drain configuration rather than a new environment variable. The flag
gates publication, new shared-thread admissions/delegation and new multi-person
group creation/join/turns. Require enabled authority for the publisher/creator
and acting person for new feature work; guest admission also checks the owner.
Existing owner-only conversations do not acquire flag reads because an agent
has been published. Revocation, disabling, leaving, cleanup and private-history
reads remain available when off. Turning off never removes a stored restriction.

New fields deserialize with safe defaults. New role/mode/consent discriminators
live in new collections or optional new payloads that old structs ignore; no
new variants are emitted into enum fields old readers consume. Unknown versions
on upgraded readers deny use. For #1996's new `GuestAccess` value, obey its
reader-first rollout: old serde enum readers can fail even when the surrounding
field is defaulted. Do not write that value into legacy fields until every
reader supports it. Preserve unrelated fields with targeted updates and CAS;
do not assume older `replace_one` writers retain new fields.

Wire compatibility is not authorization compatibility. Old replicas do not know
shared credential consent, caller-pays BYOK or collaboration-group participation.
Some old personal group/history paths would still trust creator equality. A
defaulted discriminator or revoked key does not fix those readers.

Rollout sequence:

1. Land #1996, rebase this work and verify shared policy uses its final helpers.
   Ship additive readers, classifiers, indexes and unconditional stored-binding
   enforcement with the flag off. Install indexes before activating writers;
   do not change legacy thread indexes underneath old writers.
2. Drain old API/auth/proxy/MCP/LLM/worker replicas, including alternate ingress
   and sweepers. Upgrade the full roster and verify the new readiness capability.
   Readiness is an operational assertion plus observations, not proof of a
   hidden old process's absence.
3. Enable Phase 1 for test publishers and actors. Verify ACL, billing, privacy,
   query counts and revocation with two real identities and no shared secrets.
4. Ship Phase 2 through the same reader-first/all-replica gate. Enable group UI
   only after backend capability is present; unknown request fields currently
   fail closed on some handlers. Do not activate it on a partly upgraded fleet.

Safe feature rollback turns the flag off, stops new admissions and continuations,
cancels/drains active collaboration turns and workers, invalidates pending cards
and revokes their execution bindings, while upgraded readers and financial
settlement remain online. Restore the flag only after fresh ACL and consent
validation; do not resurrect a revoked key or one-time consent.

Binary rollback requires more: keep all affected read routes on upgraded
replicas, or securely archive/remove collaboration groups, hidden/private
thread artifacts, cards and attachments from old-readable collections after
export and settlement. Revoke all associated keys, including guest keys, and
quiesce their queues/watches/sweepers first. Inventory must show zero old-readable
active bindings and queued effects. Follow #1996's data downgrade requirements
too. Retain new side collections safely for recovery, never downgrade them to
owner-only records. Disabling the flag or deleting keys alone is not a safe
binary rollback. Unrelated legacy data stays untouched.

## 12. Phased implementation after review

Phase 1 currently supports private shared conversations and foreground HTTP/MCP
service execution, including model callbacks. Pools, detached/managed jobs,
async-watch continuations, voice sessions, WebSocket/CONNECT and group/channel publishing are
refused for shared keys before effects. These transports require a separate
extension carrying the same durable payer, consent and revocation fences.
Consent uses separate human-only inline cards; after choosing, the person
continues the chat. The current Share dialog accepts exact person/org UUIDs;
there is no public user-directory enumeration.


**Phase 1 — shared agents.** Land the access/role model, owner publishing and
shared-service policy, batched snapshots, private threads/notes and explicit
team memory. Implement both credential lanes and human consent, all execution
gates and billing/audit attribution before exposing discovery. Add share dialog,
Shared with me, remembered-consent management and NyxBot discovery/delegation.
Cover privacy/lifecycle/rollback first; then enable the flag for a small cohort.
No multi-person group UI or guest sharing is implicitly enabled by this phase.

**Phase 2 — multi-person groups.** Add the explicit group discriminator,
membership/invite transactions, creator transfer/leave, agent membership and
owner guest opt-in. Generalize org-group admission/hidden-thread/request-chain
machinery with the same identity evaluator, retaining legacy entry points.
Implement actor cards, consent audiences, uploads, live delivery, handoffs,
queued work and cancellation, then participant/invite/guest-toggle UI and tools.
Validate mixed owners and concurrent requesters before widening rollout.

Expected code touchpoints are `models/assistant_{agent,group,conversation}.rs`,
`models/api_key.rs`, new share/consent/memory/binding models and services,
`org_agent_service`, `org_group_service`, `assistant_team_service`,
`assistant_nyxagent*`, `assistant_acknowledgement_service`, `assistant_live`,
upload/attachment/learning cleanup, auth and MCP/proxy/LLM execution adapters,
`billing/owner_resolver.rs`, readiness/index registration and assistant UI
schemas/hooks/components. Keep business logic in services, HTTP in handlers and
models as serde data. Update the normative engine/org/billing docs in the
implementation phases after the policy is approved.

## 13. Test and verification plan

| Area | Required evidence |
| --- | --- |
| Role/ACL matrix | Person owner, manager, user, unrelated person; direct and org-audience sources; active/inactive org, Admin/Member/Viewer, scoped service membership; no cross-agent snapshot reuse or manager grant/credential escalation |
| Identity | Two people and two owners; all identity-rule branches, owner turn inside shared group, org member restrictions, no paid-guest downgrade after denial; guest-to-shared and shared-to-guest use new contexts |
| Credentials | Shared withhold/read/use/all and #1996 append-only/operation selection; scopes/inputs/method overrides/remote metadata; exact catalog match; same-slug wrong account; personal alternative consent; platform ACL before decryption; no forbidden node/SSH/login/Oracle/account route |
| Consent | Allow once raced twice; Always revoked; Skip; Full access still prompts; replacement vs OAuth refresh; changed behavior/contract/policy/membership; private consent in group refused; no handoff, retry or continuation replay |
| Revocation | Race unshare/leave/demotion/disable with turn/key creation, approval, queue admission, final egress, stream delivery and async watch; stop wins for later work, dispatched work settles once, cleanup cannot recreate access |
| Privacy | Owner/manager cannot list/read/reset/recover/export/delete another person's chat or notes; no private data in prompts, recap, checkpoint, skills/learning, title, cards, search, notifications or live events; no private history/session reused in groups |
| Invites/groups | Expiry/cancel/single-use concurrent acceptance, full group cap, existing-account requirement, active shared-org quick-add, creator-only management, transfer/leave/last-person delete, agent remove/readd resets guest opt-in; no arbitrary owner joins through agent attachment |
| Handoffs | Original actor preserved across owner/shared/org/guest targets; per-request and hourly limits survive retries and concurrent agents; revoked pending request drops without switching actor; no personal NyxBot follower leak |
| Billing | Shared-lane actor wallet charged for supported model, BYOK and platform calls; unsupported pool, detached/managed-job, async-watch and WebSocket calls refuse before effect; org-member BYOK bills org and platform bills person regardless of group type; owner billing unchanged; guest owner charged; insufficient actor balance never uses owner funds; cancellation/partial stream/retry settle exactly once with persisted payer |
| API parity | REST UUID/slug proxy, LLM, WebSocket if supported, MCP search/list/direct/call_tool, exact approvals and worker egress agree; conversation-key management denied; human REST/CLI and NyxBot service mutations have identical role fences |
| Compatibility | Deserialize pre-feature rows; default sharing off/withheld; no unknown variants in legacy fields; old fixture readers tolerate additive output; full-fleet activation/rollback rehearsal including history readers and #1996 downgrade |
| Performance/UI | Constant batched ACL/consent reads as agent count grows; no new reads on unrelated turn paths; bounded live worker under overflow; dirty selects, keyboard dialogs, account choice, invite acceptance, live removal and both themes |

Use real MongoDB transactions on the dedicated replica-set test database and
provider stubs for egress/billing assertions. Test no-effect refusals by asserting
the downstream stub was never called; prompt snapshots alone are insufficient.
Use multi-request/concurrent tests for one-time claims and revocation, and
query-count instrumentation for request snapshot reuse. Run relevant existing
org-agent/group, guest policy, acknowledgement, model transport and billing
regressions alongside new behavior tests.

For implementation builds and tests, acquire the shared Cargo lock first, then
check free disk and stop below 10 GiB. Serialize Cargo and disable incremental
output, for example:

```sh
lockf -k /tmp/nyxid-cargo.lock env CARGO_INCREMENTAL=0 cargo check -p nyxid
lockf -k /tmp/nyxid-cargo.lock env CARGO_INCREMENTAL=0 \
  NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27022/?replicaSet=rs0&directConnection=true' \
  cargo test -p nyxid <relevant_test_filter> -- --test-threads 2
```

After targeted tests, run the complete `cargo test -p nyxid --bin nyxid-server --
--test-threads 2` with the same lock, environment and database. Run clippy and
`cargo fmt --all -- --check`, plus frontend lint, test and build. Run
`npm run build:wizard` only when wizard sources change. Record actual outcomes
and blockers; the default-off flag is not a substitute for validation.

## 14. Review decisions

Approved by the coordinating reviewer:

- Publishing is personal-only. Org audiences for personal specialists retain live
  `can_proxy()` membership. External org publishing/guests need future approval.
- Managers edit profile/persona and published team memory only.
- New participants see full retained group history, disclosed before invite or
  quick-add. Creator departure requires accepted management transfer or deletion.
- Always consent lasts until revoked or invalidated; Allow once covers one exact
  operation or the bounded stateless-model scope. Invites last seven days and
  are single-use.
- Caller-pays is exclusive to `shared`; owner/org-member/guest billing stays as
  specified in section 7.

Phase 1 is implemented without a commit in this worktree. The implementation uses a thin
`shared_access` adapter over the v0.75.0 #1996 helpers; absent shared policy is
withheld and no new enum values are written to legacy fields. Phase 2 is not
implemented here.
