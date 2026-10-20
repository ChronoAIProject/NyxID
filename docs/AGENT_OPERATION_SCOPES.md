# Operation scopes

Status: implementation contract. Introduced for specialist agents in 0.42.0 (B1);
extended to ordinary Agent Keys and converged with the permission engine
(`permissions/`, see [Google API permission keys](GOOGLE_API_PERMISSIONS.md)).

An operation scope limits a holder to selected operations within a service it
can already use. Scopes never confer service access; they only narrow it.

## Holders

- **Specialist agents.** `AssistantAgent.operation_scopes` lives beside `grants`,
  keyed by the granted UserService or platform DownstreamService ID. The compiled
  map is mirrored onto every thread ApiKey in the same MongoDB transaction as the
  agent change; creation, turn start, replacement and rotation converge through
  the fenced authority path. NyxBot and other non-specialist keys keep their
  existing behaviour.
- **Ordinary Agent Keys.** General-purpose keys store scopes directly in
  `ApiKey.assistant_operation_scopes`, keyed by UserService ID from the key's
  effective allowlist (or any of the owner's live services for allow-all keys).
  Conversation keys (managed through their agent), scheduled-invocation keys and
  permission-bound keys cannot be scoped this way. Agent Key login children and
  relay tokens inherit the parent's live map; rotation copies it.

Absence means the whole service; a present empty selection denies every
operation. Removing and re-adding a grant must not silently discard a saved
restriction.

## Compiled contract

A scope holds a monotonically increasing revision, stable ServiceEndpoint IDs
and server-compiled operation rules. Endpoint selections bind the stored
endpoint ID and its method/path contract at selection time; changing an endpoint
cannot silently widen a saved selection. Where no endpoint rows exist, explicit
`ProxyOperationPolicy` rules are permitted: exact method, root-anchored
template, one segment per variable, no globs, no targets, no credential
injection.

Each selected endpoint may also carry **input limits** (`ScopedOperation.inputs`,
type `nyxid_permissions::values::InputRules`), the same closed rules the Google
permission engine uses:

- `path`: template variables pinned to literal `exact` / `one_of` strings;
- `query`: when present, a closed map: undeclared parameters are refused,
  `required` stops a filter from being dropped, and authentication, method
  override and NyxID routing parameters cannot be declared;
- `body`: when present, a closed JSON schema (`object`, `array`, `string`,
  `integer`, `boolean`, `exact`, `one_of`; every object closed, bounded depth).

Absent parts stay unconstrained. Limits are validated at save time against the
operation's method and template, and checked by `check_inputs` against the final
request on every transport after `authorize`. A request satisfies a scope when
any operation it matches accepts its inputs. Streamed uploads have no buffered
body, so a body limit refuses them. Authoring is through the API or CLI
(`--inputs`); the UI shows a "Value limits" mark and preserves saved limits for
operations that stay selected.

The **contract digest** (`contract_digest`) is a SHA-256 over the compiled
catalog identity and operations, excluding the revision. A selection that
carries `contract_digest` fails with a conflict if it now compiles differently.

## Execution and discovery

Every scoped execution must pass the operation scope AND service grants, live
ownership/access, guest access, owner approval, destructive confirmation and
webhook automation policy.

- Raw UUID/slug proxy, LLM provider and gateway, pools (each actual member,
  including fallbacks; pool admission never substitutes for member
  authorization), node-routed proxy, exact-approval redemption, the machine
  gateway, MCP endpoint tools, `nyx__call_tool`, generic proxy calls, Ornn skill
  reads and voice enforce the same rules before service dispatch and billing.
- The pre-resolution preflight checks scopes only when the instance is
  explicitly selected (`_nyxid_via` or a pool member); otherwise the full check
  after resolution applies, so sibling connections to one catalog never
  intersect.
- Voice resolves the selected BYOK connection's identity first, like proxy/LLM.
- MCP endpoint tools enforce stored endpoint identity and the final method/path;
  dynamic instance-spec and generic calls enforce the selected method/path
  contract. Search, list and discovery omit out-of-scope operations. Refusals
  name at most 20 allowed operations within 2 KiB and never echo arguments.
- Canonicalization reuses `proxy_authorization::CanonicalPath` and the raw URI
  guard on REST routes; LLM routes canonicalize the Axum-decoded path (decoded
  separators, percent signs and controls are rejected). Path case is
  significant, HEAD requires HEAD, method overrides are refused, and forwarding
  uses the matched canonical path.
- Scoped requests and permission-bound requests never follow upstream
  redirects: the authorized URL is the only URL requested, and a 3xx is
  returned to the caller.
- WebSocket upgrades are denied for any scoped service.
- Services that inject a delegation token are refused for any key holding
  operation scopes: a delegated token carries service/node allowlists but no
  operation limits.
- Oracle tools and Oracle REST submission/attach/extract are refused for keys
  holding operation scopes, because pools have no operation identity.
- SSH has no HTTP operations: SSH services are excluded from key scoping, SSH
  REST routes are human-only, and MCP SSH meta-tools are hidden from scoped keys.

**Granularity.** Scopes and input limits constrain method, path, declared query
parameters and JSON bodies. Operations a service selects through an undeclared
query or body field (RPC-style `?action=…`, GraphQL operation names) are only
limited when the operation declares closed query/body rules.

**Guests and webhooks are turn authority, not scope authority.** Conversation
keys run the guest, guest-approval and webhook checks on raw proxy, LLM and
exact-approval routes whether or not the service is scoped. Without a matching
scoped operation, effects come from the stored endpoint contract, else from the
method (a POST is never a read). Guests may not send method overrides. Guests
never request owner approvals or spend the owner's approval grants. Webhook
calls needing an owner action card must use MCP. Ordinary keys acquire no
database reads for these checks.

Relay tokens intersect their claimed service/node allowlists with the parent
key's live allowlists, so a grant removed from the key stops working
immediately; they also inherit the parent's live operation map.

An already admitted request retains its auth snapshot; subsequent requests see
committed changes. No global scope cache is authoritative.

## Management

Configuration is gated by the default-off, admin-managed runtime feature flag
`assistant:operation-scopes`, resolved for the acting person. While disabled,
writes explain that the feature is not enabled yet and the UI hides the
selector. Enforcement of saved scopes never reads this flag.

Specialists:

- `GET /api/v1/assistant/nyxagent/agents/{id}/operations`,
  `PUT .../operations/{service_id}` (human owner, expected revision).
- NyxBot's `nyxid__set_agent_operations` may narrow immediately. Widening,
  including removing a restriction or changing input limits, requires a one-use
  owner action card. The card binds the selection's compiled contract digest;
  the model must retry with the refusal's `retry_arguments` plus
  `acknowledgement_id`, and an endpoint change after the card was shown makes
  the retry conflict. The global skip-destructive setting cannot bypass it.
- Specialists cannot change their own scopes; their requests use the
  permission flow with NyxBot as decider and are bound to the contract digest
  computed when the request is raised.

Agent Keys:

- `GET /api/v1/api-keys/{key_id}/operations`,
  `PUT /api-keys/{key_id}/operations/{service_id}` on the human-only key router.
  Personal keys require their owner; org keys require org write access, with
  member resource ACLs applied to listed services. The stored scope revision
  fences concurrent writers. A human owner may widen directly.
- UI: the key detail page's **Service operations** card.
- CLI: `nyxid api-key operations <key>` lists; add `--service <slug|id>` with
  `--allow-endpoint <id>…`, `--allow METHOD:/path…`, `--deny-all` or `--all`,
  and optionally `--inputs <file.json>` keyed by operation ID.

Widening detection compares the whole compiled operation: an identical entry
covers; otherwise only replacing a single-use, unconstrained variable with a
literal it matches, or adding input limits, narrows. Renaming variables,
changing parameter grammars, narrowing a repeated variable, or changing or
removing input limits widens.

Audits contain holder/service/endpoint IDs, revisions and counts only
(`assistant_agent_operations_changed`, `api_key_operations_changed`). No request
bodies, tool arguments, input-limit values, secrets or credential material.

## Relationship to permission-bound keys

Permission-bound keys (`purpose: permission_bound`) remain an immutable,
single-connection pilot with their own ingress and Drive folder adapter. Both
systems now share one input-rule evaluator (`permissions/src/values.rs`); the
Google adapter's query and body checks call the same functions. The intended
next steps are to express Google policies as scoped operations with input
limits, move the Drive ancestry check into the shared pipeline as a resource
adapter, and fold `/permission-execution` into `/proxy`.

## Compatibility

Legacy missing fields deserialize to empty maps or `None`, preserving
whole-service grants. **Upgrade every auth/proxy/MCP replica before enabling the
flag or saving Agent Key scopes or input limits**: older binaries ignore
`inputs` (under-enforcing), and refuse scoped Agent Keys on raw proxy and LLM
routes because they require a live conversation. Rollback starts by disabling the flag; existing scopes remain enforced by
current binaries, so remove scopes (or the keys/specialists holding them) before
returning traffic to old binaries. Older writers of `grants` cannot erase
sibling scopes.

Scopes contain no person-specific ownership assumptions; execution and billing
remain the acting person's.
