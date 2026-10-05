# Organization specialist agents

## Ownership and roles

`AssistantAgent.user_id` is the polymorphic owner (person or organization User),
matching nodes and services. Existing agents remain personal. Organizations MUST
NOT have a NyxBot. No separate organization ownership field is added to agents.
Authorization MUST use `org_service::resolve_owner_access` and live organization
and membership state. Admin and Member may maintain organization agents; Viewer
may inspect their profiles but may neither maintain nor use them. This agent-only
maintenance policy does not expand Member permissions on other organization
resources. Agent IDs are not service IDs: membership service scopes constrain
resource grants and execution, not visibility of the agent profile.

Maintaining includes profile, grants, operation scopes, skills, shared memory and
destruction. Using requires active membership with `can_proxy()`. UI, native
tools, turn admission, queued work, and thread-key authentication MUST enforce
these checks. Removed members, inactive organizations and Viewers MUST be refused
on the next request. Existing scopes and skills retain their B1/B2 contracts.

## Actor, threads and execution

The organization owns the specialist; the acting person owns each conversation,
credential, approval and action card. There is no shared organization home thread.
Home-thread lookup and every history query MUST include the acting person.
Organization membership MUST NOT confer access to another person's transcript,
attachments, queued events, session binding or cards. Shared organization group
transcripts use the separate explicit participant ACL below; they confer no
access to another person's private or hidden thread.

Thread keys retain the acting person's identity and carry an organization-owner
binding, checked against live membership on every authentication, including
machine gateway jobs. Authentication MUST resolve this access once and reuse its
actor/owner-bound snapshot for service and node authority, chat lookup and final
execution checks within that request. An admitted request may finish with its
snapshot; a new request MUST resolve live access again. Snapshots MUST NOT be
persisted or shared across requests. Effective authority is the intersection of agent grants,
B1 scopes and the person's live organization resource access. Grant and scope
changes update every member's thread keys transactionally. Credential resolution
MUST NOT fall back to a member's personal resources or another organization.
The assistant engine/model transport retains its existing narrowly bounded
exception. Ordinary service execution and platform-key billing use the existing
`BillingOwnerResolver::resolve_for_execution`; platform usage bills the person.

Organization agents may receive only their organization's services, connections
and machine nodes, and catalog/platform services using existing organization
bindings. Personal resources and saved logins MUST be refused. Maintenance must
not grant resources the maintaining person cannot access. Account-wide personal
management access and guest/channel sharing are not organization-agent grants.

## Management, skills and memory

Personal NyxBot tools accept an explicit `org` selector (ID, slug or unambiguous
name). They resolve ownership and recheck the maintaining person's authority;
specialists cannot maintain other agents or self-escalate. Widening operation
scopes and adding/re-pinning skills retain one-use cards decided by the requesting
maintainer, bound to the exact agent and change. Destruction retains confirmation.
Ornn search, pinning and reads use the managing/acting person's own access.

The acting person's personal NyxBot may discover and delegate to an organization
specialist that person can use when `assistant:org-agents` is enabled for that
person. Disabling this discovery/admission gate never disables existing thread
enforcement. Its roster and turn instructions show only live
Admin/Member memberships with `can_proxy()`, using `org-slug/agent-name` labels;
Viewers and inactive memberships are omitted. `nyxid__message_subagent` accepts
the qualified label or agent ID. Bare names prefer personal specialists and are
accepted for an organization agent only when the usable match is unambiguous.
The delegated conversation remains private to the acting person, carries the
organization owner binding, uses only the organization's grants/skills/memory,
and reports back to the assigning NyxBot. Membership is checked again at
admission and on every execution. If an organization agent lacks a grant and the
acting person maintains it, the existing orchestrator card flow remains available:
that person's NyxBot or the person may decide the card, subject to the existing
resource ACLs and one-use owner confirmations. Otherwise refuse with
`organization_grant_required` and ask an organization maintainer to change the
agent's grants. The same maintainer check applies to skill, operation-scope,
machine and account permission requests. Decisions, including legacy cards,
MUST recheck live maintainer authority inside the decision transaction; a personal
NyxBot never confers organization authority of its own. Requests reuse the
authentication snapshot when present, and granted calls add no maintainer reads.
Admin and Member retain the existing agent-maintenance role semantics. Ordinary
action confirmations remain decided by the acting person; they do not confer
new grants or permit personal account access for an organization agent.

Agent memory is shared organization data, visible and editable by maintainers.
Turn instructions MUST identify this shared memory and prohibit storing a
member's private content in it. Shared memory is never populated automatically
from another member's transcript. Viewers receive profile metadata without memory
or private thread data. Audit contains actor person, owner organization, agent ID
and change kind only; never prompts, memory, skills bodies or credentials.

The Assistant agent list separates Personal and organization ownership. Detail
shows ownership, the maintaining roles, and the caller's read-only/use state.

## Rollout

Creation is gated by the default-off `assistant:org-agents` feature flag, evaluated
for the managing person. Deploy every auth, proxy, MCP and worker replica before
enabling it. Old replicas do not understand the live organization binding on
person-owned thread keys and could retain authority after membership removal.
Enforcement is always enabled independently of this creation gate. Disable the
flag before rollback, drain organization-agent turns and revoke their thread keys
before running old binaries. Personal agents remain backward compatible.

## Organization group chats

`AssistantGroup.user_id` is the polymorphic owner. Organization groups store an
explicit, unique `participant_user_ids` list (at most 16 people, including the
creator at creation) and `created_by_user_id`. Personal groups ignore these
defaulted fields and retain their existing behavior. Organization groups contain
1–8 live specialist agents owned by that same organization; NyxBot, personal and
other-organization agents MUST be refused. Creation requires the existing
`assistant:org-agents` flag and live `can_proxy()` membership.

Every group read, post, attachment read, card decision, live subscription/delivery
and queued-work admission MUST require both explicit participation and live
active `can_proxy()` organization access. Resolve owner access once per request
and share only that request's snapshot. Non-participants and ineligible members
receive not-found-shaped errors. The creator and participating organization
Admins may manage the name, agents, lead and participant list or delete the group.
Every added person MUST have active Admin/Member access. Leaving or removing a
participant MUST transactionally delete that person's hidden threads, keys,
credentials, cards and pending requests with the participant-list update. Refuse
while that person's turn is live. Shared messages and their uploads retain their
normal retention policy. A live creator or participating Admin MUST remain;
otherwise refuse the change and ask the manager to delete the group. The last
participant's departure cascades through group deletion. Deletion removes the
transcript, attachments, and every participant's hidden member threads and keys.

Each human message records its author and shows that person's display name.
Every message chain retains its triggering person, including agent hand-offs and
their bounded budget. Hidden execution threads are unique per (group, agent,
triggering person), created through `create_thread_for` with that person as actor.
Their keys, grants/scopes, live organization checks and person billing retain the
B3a contract. Transcript cursors belong to each hidden thread. Turn instructions
identify the shared group, triggering person and participants by display name,
and prohibit copying private member content into shared agent memory.

Cards belong to the triggering person. Participants may see the card and whom it
awaits; only that person may decide it, with a not-found-shaped refusal otherwise.
Plain-text/channel approval shortcuts MUST NOT apply. Proxy/exact-service cards
also bind the group, hidden thread, person and request chain; notifications go
only to that person, and these cards appear in the group's ACL-protected view.
Existing organization approval policies still apply: an Admin-only service
policy also requires the triggering person to be an authorized Admin. Uploads retain the acting
person's ownership for retention and sweep, while group attachment reads require
the live group ACL. Only agents handling the upload's turn receive its content.
Ineligible participants' queued work is dropped on admission with an
identifier-only notice; it MUST NOT acquire another participant's authority.

Live events carry identifiers only and are routed to currently eligible explicit
participants, never the organization user ID. Both subscription and every
delivery decision recheck eligibility; unrelated personal paths incur no new
database reads. The change-stream loop MUST NOT await organization lookups:
enqueue group identifiers to one bounded worker, coalesce each group over 250 ms,
and resolve eligibility once per batch. Overflow may drop identifiers; polling
and the 15-second sweep remain the backstop. Every organization message,
including system notices, stores a server-only routing marker independent of
its author. Pending-card reads batch acknowledgements over the hidden thread IDs.
Responses project owner, participant display names and caller
role through dedicated DTOs. Audit records actor, owner, group and change kind,
never message, upload, memory or credential content.

Existing group routes and management tools accept the B3a organization selector
(ID, slug or unambiguous name). Personal NyxBot may manage authorized groups but
MUST NOT post into or follow organization groups; channel links are unsupported.
Keep `assistant:org-agents` off while deploying, including when upgrading from
B3a, then enable it only after every auth/proxy/MCP/worker replica is upgraded.
Old group routes fail closed through actor/owner equality, but B3a binaries do
not understand a group key's participation binding. Rollback MUST first disable
creation, stop/drain group turns and revoke their thread credentials/keys before
restarting older replicas. Disabling creation alone does not revoke existing
keys. Older member-thread history/card readers also lack the participation
check: keep those routes on upgraded replicas while group data remains, or
securely archive/remove the org-group hidden threads, messages and cards before
rollback. Key revocation alone does not protect those reads. Existing
organization groups otherwise require upgraded replicas.
