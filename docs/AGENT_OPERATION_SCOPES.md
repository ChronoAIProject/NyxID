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
  `integer`, `number`, `boolean`, `exact`, `one_of`; every object closed, bounded depth).

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

### Decimal number limits

`number` accepts finite JSON numbers, including integers, within inclusive
`minimum` and `maximum` bounds (both signed 64-bit integers). Its
`max_fraction_digits` must be between 0 and 12; the minimum cannot exceed the
maximum. Strings such as `"12.34"` are refused. Use `integer` for dates expressed
as milliseconds.

For example, an Amount property in a closed receipt `fields` object can use:

```json
{"type":"number","minimum":0,"maximum":1000000,"max_fraction_digits":2}
```

Fraction precision is checked on serde_json's canonical decimal representation
of the parsed value, with exponents expanded and insignificant trailing decimal
zeros removed. `12.34` and `12.340` have two fractional digits; `12.345` has three
and is refused by this example. `12.0` and `1e2` have none. `1.5e-3` expands to
`0.0015` and has four. The original byte spelling does not preserve extra
precision after JSON parsing. Bounds are compared as scaled integers so the
inclusive i64 edges do not widen through floating-point conversion.

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

## Receipt-Bot recipe

This example restricts the managed Lark Base receipt-write workflow using the
published field-list and managed-create operations. The connection and rollout
guide is [Lark Base](site/cli/guides/connect-a-service.md#lark-base).

Grant one Lark/Feishu UserService to a specialist, then select only
`bitable_fields_list` and `bitable_record_create_with_attachments`. A grant
still supplies service access; this scope only narrows it. Do not grant plain
`bitable_record_create` or standalone `bitable_media_upload` to this specialist.
The composite's internal field reads/uploads/create run under its managed
guard, so it does not need those child operations as separate grants.

First read the table's field definitions and current options with
`bitable_fields_list`. The example uses Base `bascnReceiptApp`, table
`tblReceipts`, select options `Expense`/`Refund` for `Type` and `1234`/`5678`
for `Card Number`, a UTC date in 2026 as integer milliseconds, Amount between
0 and 1,000,000 with at most two decimal places, and the attachment field
`Receipt Photos`. Replace these values with the owner's exact schema and
policy. A newly added dropdown option requires an explicit scope change; the
agent cannot invent options.

Identify the two operations through catalog/MCP discovery, then read
`GET /api/v1/assistant/nyxagent/agents/{agent_id}/operations` as the human
owner. Find the granted UserService and match each operation's method/path
to the returned scope options to obtain its endpoint ID. The options include
method, path, and summary; they do not expose operation names. Endpoint IDs
are the returned server IDs, not operation-name strings. In the following
selection, replace
`<fields-endpoint-id>` and `<composite-endpoint-id>` everywhere, including the
`inputs` object keys. Replace `expected_revision: 0` with that service's current
revision. Save the resulting JSON with:

```http
PUT /api/v1/assistant/nyxagent/agents/{agent_id}/operations/{service_id}
Content-Type: application/json
```

```json
{
  "expected_revision": 0,
  "all_operations": false,
  "endpoint_ids": ["<fields-endpoint-id>", "<composite-endpoint-id>"],
  "inputs": {
    "<fields-endpoint-id>": {
      "path": {
        "app_token": {"type": "exact", "value": "bascnReceiptApp"},
        "table_id": {"type": "exact", "value": "tblReceipts"}
      }
    },
    "<composite-endpoint-id>": {
      "path": {
        "app_token": {"type": "exact", "value": "bascnReceiptApp"},
        "table_id": {"type": "exact", "value": "tblReceipts"}
      },
      "body": {
        "type": "object",
        "required": ["fields", "attachments"],
        "properties": {
          "fields": {
            "type": "object",
            "required": ["Type", "Card Number", "Date", "Amount"],
            "properties": {
              "Type": {"type": "one_of", "values": ["Expense", "Refund"]},
              "Card Number": {"type": "one_of", "values": ["1234", "5678"]},
              "Date": {"type": "integer", "minimum": 1767225600000, "maximum": 1798761599999},
              "Amount": {"type": "number", "minimum": 0, "maximum": 1000000, "max_fraction_digits": 2}
            }
          },
          "attachments": {
            "type": "object",
            "required": ["Receipt Photos"],
            "properties": {
              "Receipt Photos": {
                "type": "array",
                "max_items": 10,
                "items": {"type": "string", "max_length": 128}
              }
            }
          }
        }
      }
    }
  }
}
```

Every `object` in this vocabulary is closed without an
`additionalProperties` keyword. This selection refuses missing fields,
unknown fields or top-level properties, unknown dropdown options, a different
Base/table, Amount `12.345`, and more than ten photo IDs. Amount `12.34` is a
number, not a quoted string. The scope vocabulary's `array` has a maximum but
no minimum length; the managed composite separately refuses empty photo
arrays, repeated IDs/content, incomplete inbound photo sets, and inaccessible uploads
before any provider call. The required `Receipt Photos` property makes an
empty `attachments` object fail the scope itself.

An ordinary Agent Key's equivalent management routes are
`GET /api/v1/api-keys/{key_id}/operations` and
`PUT /api/v1/api-keys/{key_id}/operations/{service_id}`, using that key's own
endpoint IDs and revision. The CLI can apply the same input map to an ordinary
key after saving the JSON object's `inputs` value as `receipt-inputs.json`:

```bash
nyxid api-key operations <key-id> --service <user-service-id> \
  --allow-endpoint <fields-endpoint-id> \
  --allow-endpoint <composite-endpoint-id> \
  --inputs receipt-inputs.json
```

This demonstrates scope management only: an ordinary Agent Key cannot execute
chat attachment writes. For the working Receipt-Bot, use the specialist's
scope endpoint above; its live owner conversation key inherits the saved
scope. The specialist's grants UI and the Agent Key **Service operations**
card can select operations and retain existing input limits, but cannot author
this nested JSON. Configuration requires `assistant:operation-scopes` for the
acting person; execution also needs `assistant:attachment-writes` and a live,
non-guest owner turn. Upgrade all replicas before saving the Number rule.

Success returns `{record_id, already_created:false}`; an authorized identical
replay returns `{record_id, already_created:true}`, even after credential
rotation. Active writes report `in progress; try again shortly`. For
`uncertain`, follow `outcome unknown; check the Base; NyxID will not create it
again`. Never call plain create or standalone upload to evade that decision.
A succeeded-photo conflict names the existing record and directs field changes
to `bitable_record_update`. An incomplete source set reports `N of M photos
from that message could not be read; ask the owner to resend`. Definite
non-application releases reservations; `failed_definite` / `not_sent` proves
the create request did not leave NyxID, permitting one create on a corrected
new call. See the guide for all outcomes, the 400-day protection horizon, and
[compatibility](#compatibility) for scope removal before rollback.

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

**Upgrade all replicas before saving a scope that uses `number`**
(`JsonShape::Number`). A previous enum reader cannot deserialize this variant;
the typed API-key load fails closed instead of discarding the input limits or
falling back to unrestricted access. This may surface as an authentication or
database error on an old replica. Remove every Number scope, including its
compiled copies on keys, before rolling back to a binary without this variant.

Scopes contain no person-specific ownership assumptions; execution and billing
remain the acting person's.
