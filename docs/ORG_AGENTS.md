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
attachments, queued events, session binding or cards. B3a introduces no shared
chat; B3b will require a separate explicit participant ACL.

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
