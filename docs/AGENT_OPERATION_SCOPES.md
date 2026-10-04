# Specialist operation scopes (B1)

Status: implementation contract for the 0.42.0 feature series. B2/B3 are out of scope.

## Authority and storage

NyxID, never agent hooks or instructions, enforces specialist operation scopes.
`AssistantAgent.operation_scopes` lives beside `grants`, keyed by the granted
UserService or platform DownstreamService ID. The scope contains a monotonically
increasing revision, stable ServiceEndpoint IDs and server-compiled operation
rules. Absence means the whole granted service; a present empty selection denies
every operation. Scopes never confer service access. Removing and re-adding a
grant must not silently discard a saved restriction.

Endpoint selections bind the stored endpoint ID and its method/path contract at
selection time. Changing an endpoint cannot silently widen a saved selection;
the owner must explicitly save the new contract. Where no stored endpoint rows
exist, explicit `ProxyOperationPolicy` rules are permitted. Rules use the existing
matcher: exact method, root-anchored template, one segment per variable, no globs.
Scope rules cannot select destinations or inject credentials.

The complete compiled map is mirrored onto every thread ApiKey in the same
MongoDB transaction as the agent change. Creation, turn start, replacement and
rotation converge through the existing fenced authority path. Auth contexts
carry the map from their existing key read. Relay tokens inherit the live map
from their existing parent-key liveness read, never stale JWT claims. Unrelated keys, proxy requests, MCP
requests and turns acquire no scope-specific database reads. No global scope
cache is authoritative. An already admitted request retains its auth snapshot;
subsequent requests see committed changes.

## Execution and discovery

Every specialist execution must pass the operation scope AND service grants,
live ownership/access, guest access, owner approval, destructive confirmation
and webhook automation policy. NyxBot and non-assistant keys retain their
existing behavior. Guests cannot request or approve wider permissions.

MCP endpoint tools and `nyx__call_tool` enforce stored endpoint identity and
the final method/path. Dynamic instance-spec and generic calls have no durable
endpoint identity; they enforce the selected method/path contract, as raw HTTP does.
Search, list and discovery omit out-of-scope operations. Refusals identify the
allowed operation IDs/methods/templates, never request bodies or arguments.
Refusals show at most 20 entries within a 2 KiB message, retaining each shown
endpoint ID, shortening long paths at UTF-8 boundaries, and reporting the omitted
count. They point to MCP search/list discovery for the complete allowed list.

Raw UUID/slug proxy, LLM, pools, node, exact-approval redemption and machine gateway dispatch enforce the
same rules before service dispatch and billing admission. Pools check the actual member service;
pool admission never substitutes for member authorization. Explicit member hints
retain instance identity; scopes on sibling connections do not intersect. AI pool aliases are
visible only when a viable member permits its translated native chat operation;
discovery checks the loaded candidates without additional database reads. The assistant model
inference exception does not bypass a configured scope. Machine jobs retain
the live thread key restriction as well as their explicit service declaration.

Canonicalization reuses `proxy_authorization::CanonicalPath` and the raw URI
guard. Percent encoding is decoded exactly once; encoded separators, nested
escapes, dot segments, duplicate slashes, trailing slashes (except `/`), invalid
escapes, control characters and backslashes are rejected. Path case is
significant. Forwarding uses the matched canonical path. HEAD requires HEAD;
GET never implies it. Query parameters cannot choose another operation;
method-override headers/query fields are refused. WebSocket upgrades are denied
for any scoped service because an HTTP operation grant cannot bound later
frames. Scope checks do not replace the existing service operation policy.
Raw guest calls intersect read/use/all access using compiled endpoint effect
metadata and the live guest setting. Guests never request owner approvals or
spend the owner's approval grants, including through raw, LLM and exact-approval routes.
Management operation options intentionally
include candidates outside the current selection so an owner can review changes;
execution discovery does not. Webhook calls needing an owner action card
must use MCP; raw forwarding never spends an action card implicitly.

## Management

Human owner writes use an expected revision; stale writes return conflict.
Configuration is gated by the default-off, admin-managed runtime feature flag
`assistant:operation-scopes`, resolved for the owner through the existing feature
flag service. While disabled, owner and NyxBot writes and specialist operation
requests explain that the feature is not enabled yet; the Grants UI shows that
note instead of the selector. Pending scope requests cannot be applied either.
Enforcement of saved scopes is always on and never reads this flag.
The Grants UI offers all operations or a selection, searchable method/path/
summary rows, read-only and changes-existing marks, and select-all reads.
Services without endpoint rows accept explicit rules. Revisions remain visible
after changes, including a return to all operations.

NyxBot's `nyxid__set_agent_operations` may narrow immediately. Any widening,
including removing a restriction, requires a one-use owner action card bound to
the exact agent, service, expected revision and proposed selection; the global
skip-destructive setting cannot bypass this card. Specialists cannot mutate
their own scopes. Their requests use the existing permission flow with NyxBot
as decider; NyxBot may narrow immediately but must seek owner confirmation for
widening. Existing destructive account confirmations remain unchanged.

Audits contain agent/service/endpoint IDs, revisions and counts only. No request
bodies, tool arguments, secrets, credential material or scope-change prose.

## Compatibility and later phases

Legacy missing fields deserialize to empty maps, preserving whole-service grants.
Deploy every auth/proxy/MCP replica with enforcement support, then enable
`assistant:operation-scopes` through the existing admin feature-flag controls.
Rollback starts by disabling the flag (including any enabling cohort/user overrides)
to stop configuration. Existing scopes remain enforced; remove scoped specialists
from execution before returning traffic to old binaries, which cannot enforce them.
Older writers of `grants` cannot erase sibling scopes.

Scopes contain no person-specific ownership assumptions. B3 can make `user_id`
polymorphic and resolve maintainer ACLs without changing this model; execution
and billing remain the acting person's. B2 skill pins will be separate sibling
metadata using `SkillReference`; untrusted skill guidance never changes scopes.
