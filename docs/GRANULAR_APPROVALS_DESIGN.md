# Granular Approvals Design

Status: Proposal (no code yet)
Author: design exploration, 2026-05-29
Related: per-service approval config (`ServiceApprovalConfig`), `approval_service`, proxy paths

## Problem

AI-service approval is currently **binary per (user, service)**. A `ServiceApprovalConfig`
row carries only `approval_required: bool` and `approval_mode` (`per_request` | `grant`).
Either every call to a service needs approval, or none does.

Users want **finer control** -- "auto-allow reads, require approval for writes",
"always approve `DELETE`", "approve anything that touches `/repos/*/contents`" --
modeled loosely on GitHub fine-grained permission scopes.

The complication the user flagged: **AI-service operations are dynamic**. The
operation identity does not always live in the HTTP method + path:

| Protocol      | Where the operation identity actually lives             |
|---------------|---------------------------------------------------------|
| REST (OpenAI, GitHub) | HTTP method + path (+ resource id embedded in path) |
| LLM gateway   | `model` / tool-calls in the **body**; path is static    |
| MCP           | JSON-RPC `method` / `tool_name` in the **body**          |
| SSH exec      | the **command string**; there is no path                 |
| GraphQL       | operation name in the **body**; single path              |

A design scoped only on raw HTTP `method` + `path` works for GitHub/OpenAI REST but
degenerates for MCP/SSH/LLM, where "everything is `POST /`" and one rule matches all.
GitHub itself avoids raw-path rules by mapping many endpoints onto a **small set of
named permissions** (Contents: read/write, Issues: read/write, ...).

## Decisions (locked)

1. **Rule model: hybrid.** Semantic `read` / `write` / `destructive` classification
   (derived from the request, overridable) PLUS optional path-glob rules for power
   users. Feels like GitHub's coarse toggles; scales to dynamic paths.
2. **Protocol scope: generalized now.** Introduce a protocol-agnostic
   `OperationDescriptor` and populate it for **HTTP, MCP, SSH, and LLM gateway** from
   the start, rather than retrofitting later.
3. Deliverable: this design doc. Implementation gated on review.

## Current state (verified against code)

Approval call sites today (`approval_service` fns: `approval_service.rs`):

| Path        | Handler / fn                                   | resolve | check | create | operation_summary today |
|-------------|------------------------------------------------|---------|-------|--------|-------------------------|
| HTTP proxy  | `proxy.rs::execute_proxy_inner` (~1050)        | 1321    | 1367  | 1427   | `proxy:{METHOD} {path}` |
| LLM gateway | `llm_gateway.rs::check_llm_approval` (1099)    | 1111    | 1137  | 1182   | `llm:{METHOD} {path}`   |
| SSH tunnel  | `ssh_tunnel.rs` (~934)                         | 934     | 957   | 999    | `ssh:tunnel` (no command) |
| **MCP**     | `mcp_transport.rs` (`mcp_post`/`handle_tools_call`) | **none** | **none** | **none** | **unapproved** |

Key facts that shape the plan:

- At the HTTP/LLM check sites, `method`, `path`, `query`, and the **buffered body**
  are all already in scope (`proxy.rs:1273`, `:1278`, `:1341`). The data needed for
  granularity is present; only the matching logic and grant scope are missing.
- `action_description::build_action_description(method, path, body)` already extracts
  safe summary params (`model`, `tool_choice`, message count) and is PII-scrubbed.
  The descriptor builder should reuse / extend this rather than re-parsing bodies.
- **SSH** captures no command/principal in the approval request (`operation_summary =
  "ssh:tunnel"`, `action_description = None`). Granular SSH approval requires
  threading the command/principal through first.
- **MCP has no approval checks at all.** Adding the descriptor here is also adding the
  *first* approval enforcement to MCP -- treat as a distinct, higher-risk workstream.
- Grants (`ApprovalGrant`) are **service-scoped** (+ optional org-scoped). They carry
  no method/path scope, so any granularity on the request side would leak through a
  grant unless grants are also scoped.

## Proposed design

### 1. `OperationDescriptor` (protocol-agnostic operation identity)

A small struct each proxy path builds and passes into approval resolution. It is the
single seam that lets one rule engine serve every protocol.

```rust
// backend/src/services/operation_descriptor.rs  (new)

pub enum Protocol { Http, Llm, Mcp, Ssh }

/// Coarse semantic class. Derived by the per-protocol builder; the rule engine
/// matches on this when a rule omits method/resource patterns.
pub enum Verb { Read, Write, Destructive }

pub struct OperationDescriptor {
    pub protocol: Protocol,
    pub verb: Verb,
    /// HTTP method, or MCP JSON-RPC method, or "EXEC"/"TUNNEL" for SSH.
    pub method: Option<String>,
    /// HTTP path, MCP tool name, or SSH command (first token / full, see below).
    pub resource: Option<String>,
    /// Reuses action_description; shown in the approval prompt. PII-scrubbed.
    pub summary: String,
}
```

Per-protocol builders (one fn each, unit-testable, no DB):

- **HTTP / LLM** -- `method` = HTTP method; `resource` = path; `verb` from method
  (`GET`/`HEAD`/`OPTIONS` -> Read; `POST`/`PUT`/`PATCH` -> Write; `DELETE` ->
  Destructive). `summary` from `build_action_description`. LLM additionally surfaces
  `model` into `summary` (already partly done).
- **MCP** -- The only verb-bearing JSON-RPC method is `tools/call`
  (`mcp_transport.rs:615`); `initialize` / `tools/list` / `ping` are handshake/read
  and are not subject to approval. **Crucially, NyxID's MCP tools are generated from
  OpenAPI endpoints**: each `McpToolEndpoint` carries a concrete HTTP `method` + `path`
  (`mcp_service.rs:88-89`). So the MCP builder resolves the called tool name back to its
  endpoint and reuses the **exact same** `method` + `path` + verb logic as HTTP. No
  hand-maintained per-tool "destructive" map is needed. Cases:
    - endpoint-backed tool -> `method`/`path`/`verb` from the endpoint;
    - generic-proxy tool (`is_generic_proxy`) -> `method`/`path` from the call
      arguments, same verb logic;
    - `nyx__*` meta-tools (search/discovery) -> `verb = Read`.
  Consequence: a single rule (e.g. "approve `DELETE /repos/*`") applies identically
  whether the agent calls the service over HTTP proxy or via the MCP tool wrapping that
  endpoint. Same policy, both transports.
- **SSH** -- `method` = `"EXEC"` | `"TUNNEL"`; `resource` = command string (exec) or
  `""` (tunnel). `ssh exec` (and the MCP-SSH-exec path) can be matched by
  `resource_pattern` against the command; **tunnels/terminals stay coarse** -- a tunnel
  is an opaque interactive byte stream after the handshake, so approval is whole-session
  at connect time (`verb = Write`, empty resource, never matches a command pattern).
  Requires threading the command/principal into the approval call (currently dropped at
  `ssh_tunnel.rs:1011`). See "Resolved decisions" Q1 for the security caveat.

> Resource normalization: for matching we lower-case the method, strip the query
> string from HTTP paths, and for SSH use the full command string (glob can match
> `git push*`). SSH exec commands are redacted and truncated before they are
> stored or shown in approval prompts.

### 2. Rule list on `ServiceApprovalConfig`

Replace the binary flag with an ordered rule list + a default. Backward compatible:
a missing `rules` field behaves exactly like today.

```rust
pub enum Effect { RequireApproval, AutoAllow, Deny }

pub struct ApprovalRule {
    /// Match methods (case-insensitive). ["*"] or empty = any.
    pub methods: Vec<String>,
    /// Glob over the normalized resource. "*" / "" = any. e.g. "/v1/chat/*",
    /// "/repos/*/contents/**", "git push*".
    pub resource_pattern: String,
    /// Optional semantic gate: only match when verb is in this set. Empty = any.
    pub verbs: Vec<Verb>,
    pub effect: Effect,
    /// Applies when effect = RequireApproval.
    pub mode: ApprovalMode,
}

pub struct ServiceApprovalConfig {
    // ... existing fields ...
    /// Ordered; first match wins. Empty = use the legacy binary behavior below.
    #[serde(default)]
    pub rules: Vec<ApprovalRule>,
    /// Fallback when no rule matches. Defaults preserve today's behavior:
    /// if rules is empty, default_effect is derived from approval_required.
    #[serde(default)]
    pub default_effect: Option<Effect>,
}
```

Matching (`fn evaluate(descriptor, &config) -> Effect`), pure + unit-testable, lives
beside `action_description.rs`. **Three-state fallback** (resolves Q2 -- default-allow,
explicit + opt-in):

1. Walk `rules` in order; first rule whose `methods` AND `resource_pattern` AND
   `verbs` all match returns its `effect`.
2. No rule matched: use `default_effect` if the user set one.
3. `default_effect` is `None` AND `rules` is empty -> fall back to legacy
   `approval_required` (exact current behavior; zero-migration safety).
4. `default_effect` is `None` AND `rules` is non-empty -> `AutoAllow` (the user opted
   into rules as additive guards; an unlisted endpoint is allowed -- least surprise for
   dynamic APIs).
5. `Deny` short-circuits the proxy with a 403 before any credential resolution.
6. `RequireApproval` carries the rule's `mode`.

`default_effect` is a first-class, user-settable field (`AutoAllow` | `RequireApproval`
| `Deny`). A security-conscious user sets `RequireApproval` ("approve everything I
didn't explicitly auto-allow") or `Deny` ("allowlist-only -- block anything unlisted").
The system default is `AutoAllow` so granular rules never silently break a dynamic API
the user forgot to list.

Glob matching via the `globset` crate (anchored, `**` for path segments). One compiled
`GlobSet` per config, cached on the resolved config.

**Simple mode (GitHub-like):** the frontend offers a preset that generates rules from
three toggles -- "approve reads / writes / destructive" -- by emitting `verbs`-only
rules with no path pattern. Power users switch to "advanced" and edit raw rules. Same
storage, same engine.

### 3. Scope the grant (prevents granularity leak)

`ApprovalGrant` gains a `scope` field: the normalized signature of the approved
operation, e.g. `http:post:/v1/chat/*` (from the matched rule) or a concrete
`http:post:/v1/chat/completions` (from the request). `check_approval` computes the
incoming request's signature from its descriptor and only matches grants whose `scope`
covers it.

- Backward compatibility: existing grants have no `scope` -> treated as service-wide
  (current behavior) so already-approved requesters are not re-prompted after upgrade.
- A grant minted from a path-glob rule stores the **rule's** pattern as scope, so one
  approval of `POST /v1/chat/*` covers future `completions` calls -- matching user
  intent. A grant minted with no matching rule (legacy default) stays service-wide.

### 4. Wiring into `resolve_org_aware_approval`

`resolve_org_aware_approval` currently returns `{ required, mode, primary_owner,
from_org_policy }`. Extend it to take the `&OperationDescriptor` and run the rule
engine, returning additionally the matched `Effect` and the grant `scope` to use. The
org-policy cascade (`approval_service.rs:103-118`) is unchanged -- org configs simply
carry their own `rules`, and the org's rules win when the org owns the service.

**Org rules fully replace, never merge** (resolves Q4). When a service is org-owned and
the org has a `ServiceApprovalConfig`, that config -- its `rules` AND its
`default_effect` -- is the complete, authoritative policy; personal rules are not unioned
in. This keeps the current contract (org config wins absolutely for shared resources),
keeps first-match precedence unambiguous, and avoids confusing "my personal rule didn't
apply" cases. Personal rules still govern personal services.

All three existing call sites (`proxy.rs:1321`, `llm_gateway.rs:1111`,
`ssh_tunnel.rs:934`) pass a descriptor instead of just IDs. The MCP path gains a new
call site at `handle_tools_call` (`mcp_transport.rs:995`).

## Phased rollout

**Phase 0 -- descriptor seam (no behavior change).**
Add `OperationDescriptor` + per-protocol builders for HTTP/LLM/SSH. Thread the
descriptor into `resolve_org_aware_approval` but keep returning the binary result.
Backfill `operation_summary`/`action_description` from the descriptor (fixes SSH
losing the command). Pure refactor; covered by existing approval tests.

**Phase 1 -- method + verb scoping (HTTP/LLM).**
Add `rules` + `default_effect` to the model (backward-compatible serde), the
`evaluate` engine (methods + verbs only, no globs yet), and grant `scope`. Ship the
"simple mode" read/write/destructive toggles in the frontend. Covers ~80% of asks
("require approval for writes") with zero pattern authoring and no glob ambiguity.

**Phase 2 -- path globs + advanced rule editor.**
Add `resource_pattern` glob matching (`globset`) and the advanced rule-list UI
(method multiselect + pattern input + effect + mode). Add `Deny` enforcement.

**Phase 3 -- MCP & SSH-exec granularity.**
Populate the descriptor for the MCP path by resolving each `tools/call` tool name back
to its `McpToolEndpoint` (`method`/`path`) and reusing the HTTP verb logic -- no
separate destructive map. Thread the SSH command into the SSH-exec descriptor so
`resource_pattern` can match commands. MCP gains its first approval enforcement, but
because `default_effect` defaults to `AutoAllow`, an MCP user who hasn't configured
rules sees **no new prompts** -- enforcement is opt-in via the same rules as every other
transport, so this is no longer a surprising behavior change. A single rule
("approve `DELETE /repos/*`") now covers both the HTTP and MCP-tool routes to the same
endpoint.

## Data model migration

- `ServiceApprovalConfig`: add `rules: Vec<ApprovalRule>` (`#[serde(default)]`) and
  `default_effect: Option<Effect>` (`#[serde(default)]`). No migration script needed --
  absent fields deserialize to empty/None and the engine falls back to
  `approval_required`. Follows the `legacy_approval_mode_default` precedent
  (`service_approval_config.rs:29`).
- `ApprovalGrant`: add `scope: Option<String>` (`#[serde(default)]`). Absent = service-wide.
- `ApprovalRequest`: add `http_method: Option<String>`, `resource: Option<String>`,
  `verb: Option<String>` so the prompt UI and audit log can show structured operation
  identity (today only the free-text `operation_summary` carries it).
- Exact-service effect admission is atomic across rolling versions. The partial unique
  `exact_service_semantic_effect_unique` index binds requester type/id, actor user,
  operation id, and effect idempotency key, intentionally excluding caller or producer
  generation. Startup first checks for historical duplicate groups. If duplicates are
  present, or a duplicate write races the unique-index build, startup logs and persists
  an Integrity-page remediation diagnostic, skips index creation, and continues; it
  never silently deletes requests or falls back to a non-unique index. The pre-insert
  semantic lookup and legacy replay guard remain active until operators reconcile each
  group against decision/redemption/provider evidence and restart. Once the correctly
  shaped index exists, later startups verify its metadata and skip the full duplicate
  scan.

## API & CLI surface

- `PUT /api/v1/approvals/service-configs/{service_id}` accepts `rules` +
  `default_effect` alongside the existing `approval_required` / `approval_mode`.
  Validation: max N rules, pattern length cap, methods from a known set, reject
  patterns that fail to compile.
- `GET .../service-configs` returns the rule list.
- CLI: `nyxid approval ...` (if/where approval config is exposed) gains rule flags;
  defer detail to implementation.

## Frontend

- `frontend/src/types/approvals.ts` -- add `ApprovalRule`, `Effect`, `rules`,
  `default_effect` to `ServiceApprovalConfigItem` / `SetServiceApprovalConfigRequest`.
- Zod schema for rules (method set, pattern, effect, verbs).
- Service-config row: **Simple** tab (3 toggles) and **Advanced** tab (rule editor).
- Approval prompt (history + Telegram/push + mobile) shows structured method/resource
  from the new `ApprovalRequest` fields.

## Execution authority binding

Approval fences bind *what* operation was approved. They do not bind *where the
effect goes* or *which credential pays for it*. Those are the execution inputs,
and they stay owner-mutable while an approval is pending: a
`PUT /api/v1/keys/{key_id}` can repoint the destination URL, swap the
credential, change the auth method or rewrite injection config without moving
any operation-shape digest.

`services/execution_authority.rs` closes that with a producer-owned digest over
the resolved execution inputs:

- destination base URL, auth method, auth key name
- credential identity: `api_key_id`, `credential_epoch`, whether it is the
  catalog master credential, and any per-agent override id and epoch
- identity injection: propagation mode, which claims are included, JWT
  audience, access-token forwarding, delegation-token injection and scope,
  custom User-Agent
- both default-header layers (catalog and user-service), values included
- the effective proxy operation policy
- the *configured* node binding set (primary plus fallbacks)

The projection serializes under `nyxid-exact-execution-authority.v2` and is
hashed with `canonical_sha256`, so key order cannot change the digest. The
approval persists `{ projection_version, digest }`; the hash is never
interpreted without that version. Header values participate in the hash but
are never persisted in plaintext.

Version 2 also binds `service_category`, `requires_user_credential`, and an
inner digest of the execution-relevant `token_exchange_config` (endpoint,
encoding/template, response paths, TTL/injection/error mapping, and canonical
credential-field names). The literal configuration remains server-side; only
its canonical digest enters the outer projection.

The projection is built from the resolved `ProxyTarget`, which is the same
struct the execution path consults, rather than from independently re-read
rows. Re-reading would reintroduce exactly the producer/consumer split the
digest exists to close.

Create stores the digest on the approval binding. Observe and redeem first use
a read-only authority snapshot that neither materializes credentials nor runs
provider refresh/mint side effects. After the shared live gates pass, redeem
materializes the proxy target, digests that resolution again, compares it, and
executes the same resolution object. Because the outbound request is built
from the bytes just digested, there is no check-then-re-resolve window to race.
Drift returns HTTP 200 with `state: "drifted"` and
`failure_code: "execution_authority_drift"`, and dispatches no provider call.

The digest binds the **configured** node set, not the dispatchable one. A node
dropping its WebSocket between approval and redemption is a connectivity event,
not an authority change, and must not fail the redemption; at execution the
frozen route is the attested set filtered to currently-dispatchable nodes.

### Freshness semantics

Content-addressed fields accept A→B→A. If an owner changes the destination away
and back before redemption, redemption proceeds: the approver attested to
configuration content, and at effect time the live content is byte-identical to
what they saw. The interim state never received an effect.

Credential material cannot be safely content-addressed. Hashing ciphertext
would drift on encryption-key rotation and KMS migration; hashing plaintext
would place a secret-derived digest on an approval row. `UserApiKey.credential_epoch`
is used instead — a monotonic counter incremented **only** on user-initiated
credential replacement: rotation through `PUT /keys/{key_id}`, node-managed
credential promotion, and fresh OAuth authorization (which can change granted
scopes, so invalidating pending approvals is the fail-closed bias). It
deliberately does *not* increment on background or lazy OAuth token refresh,
which is authority-neutral; binding `updated_at` instead would drift every
pending approval on each refresh sweep. Being a counter rather than a hash, it
does not accept A→B→A: rotating a credential away and back bumps twice.

Any new write path that replaces credential material must bump the epoch, and
must do so with a **pipeline** update (`vec![doc! { "$set": ... }]`). The bump
is an aggregation expression; in a classic update document MongoDB stores the
literal expression sub-document instead of evaluating it, which both skips the
bump and makes the row fail to deserialize.

The authority rollout is rolling-safe. New approvals store the real v1 digest
in the legacy slot and the explicit v2 `{ projection_version, digest }` binding.
A v1 replica therefore validates exactly the projection it understands, while
a v2 replica validates the stronger projection. When a v2 replica reads a
main-era row with only the unversioned v1 digest, it recomputes and compares the
live v1 projection. A versioned v1 binding uses the same comparison. Rows that
predate authority digests skip this one gate, bounded by approval expiry and all
other fences; only genuinely unknown future projection versions return
`execution_authority_version_unsupported`.

The additive exact-view fields retain the v2 catalog contract. During the
bounded mixed-replica window, discovery and the legacy `exact_view_digest` row
slot carry the pre-additive digest, while new rows also persist the full digest
in `exact_view_digest_binding`. Old replicas validate the legacy slot; new
replicas validate both. Main-era rows without the second slot accept either the
live full or pre-additive digest, allowing pending approvals to roll forward.

Producer generation follows the same compatibility rule. New approvals bind
the live positive generation of a durable `ServiceEndpoint`. Main-era and
instance-spec rows with `producer_generation_bound: false` skip only the
generation comparison; catalog, exact-view, endpoint-contract, operation, and
execution-authority fences remain active. Instance-spec endpoints have no
durable producer row, so their delegated view publishes
`operation_generation: null` and their endpoint-contract digest is the shape
fence.

`ws_frame_injections` is not projected because it is unreachable from the HTTP
exact-approval execution path.

### Shared live evaluator

Observe and redeem do not maintain independent catalog, generation, policy, or
execution-authority gate implementations. Both call one ordered evaluator:

1. resolve the live catalog and compare catalog, exact-view, operation-shape,
   and producer-generation fences;
2. revalidate that the live approval policy still requires this exact
   per-request approval; and
3. compare a read-only execution-authority snapshot.

Observe returns that evaluation without claiming or writing the request. Its
`drifted`/`revoked` result is therefore transient: content-addressed A-to-B-to-A
configuration can return to `approved`. Redeem intentionally checks only
persisted request state before claim, claims atomically, then runs the same live
evaluator and persists terminal drift, revocation, or failure. Only a matched
result reaches credential materialization, whose separately recomputed digest
must still match before the provider call. This keeps the gates single-sourced
while preserving the different persistence semantics of observation and
effect admission.

### Provider outcome recovery

The provider effect and the MongoDB terminal receipt cannot be committed in one
transaction. Redemption therefore claims `executing` before dispatch and never
replays that claim. Buffered exact execution has a 10-minute hard deadline; a
deadline leaves provider success ambiguous and is itself persisted as
`provider_outcome_unknown`. If the process crashes, or the terminal Mongo write
fails, a later redeem retry leaves a fresh claim alone but atomically converts
an `executing` claim older than 15 minutes to terminal `failed` with
`failure_code: "provider_outcome_unknown"`. The original `admitted_at` remains
on the row for audit. No automatic retry follows because the provider may have
committed a non-idempotent effect; provider-specific read-back or human
reconciliation must resolve the ambiguity outside this approval identity.

## Security notes

- `Deny` rules must short-circuit **before** credential resolution and before the
  downstream request is built.
- Descriptor `summary` must keep `action_description`'s PII-scrubbing guarantees; never
  put request bodies, tokens, or SSH command secrets into matchable/loggable fields
  beyond what `build_action_description` already permits.
- SSH exec commands are stored in `ApprovalRequest.resource` and grant scopes only after
  redacting common secret forms (`-p...`, `--password ...`, `Authorization: ...`,
  `token=...`, `api_key=...`) and truncating the stored command string. Command glob
  matching therefore operates on the same redacted/truncated resource that is persisted.
- Glob patterns are user-supplied -- compile with `globset` (no regex backtracking),
  cap pattern count and length, anchor matches to avoid `*` matching across `/`
  unintentionally for HTTP-family paths (use `**` explicitly for multi-segment).
  SSH command resources are not path-segmented, so `*` is allowed to match `/`.
- MCP gains its first approval enforcement, but `default_effect = AutoAllow` means it
  stays opt-in (no new prompts until the user adds rules) -- not a silent behavior change.
- **Command-pattern approval is not a sandbox** (see Q1). A user with SSH tunnel/terminal
  access gets a full interactive shell that command-pattern rules cannot constrain. To
  actually restrict which commands run, disable tunnel/terminal at the service level
  (`ssh_auth_mode = proxy_only`, or omit terminal access) and allow only `ssh exec`,
  where the command is visible to the rule engine.

## Resolved decisions

1. **SSH tunnel stays coarse.** Per-command approval inside a live tunnel is infeasible
   (opaque interactive byte stream after handshake). Granular command matching applies
   only to `ssh exec` and the MCP-SSH-exec path. Tunnels/terminals get whole-session
   approval at connect time. Caveat: tunnel access bypasses command rules entirely --
   restrict via `ssh_auth_mode` if command-level control is required (see Security notes).
2. **Default-allow, explicit + opt-in.** `default_effect` defaults to `AutoAllow` so
   granular rules never silently break a dynamic API the user forgot to list. It is a
   first-class user-settable field; security-conscious users set `RequireApproval`
   (approve everything not explicitly allowed) or `Deny` (allowlist-only). Empty rules +
   no `default_effect` falls back to legacy `approval_required` (zero-migration safety).
3. **MCP reuses HTTP verb logic; no destructive map.** NyxID MCP tools are
   OpenAPI-endpoint-backed (`McpToolEndpoint.method`/`path`, `mcp_service.rs:88-89`), so
   `tools/call` resolves to a concrete method+path and runs the same verb derivation as
   HTTP. Generic-proxy tools take method+path from call args; `nyx__*` meta-tools are
   Read. One rule covers both the HTTP and MCP routes to the same endpoint.
4. **Org rules replace, not merge.** For org-owned services with an org
   `ServiceApprovalConfig`, the org's `rules` + `default_effect` are the complete policy;
   personal rules are not unioned in. Preserves the current absolute-org-wins contract
   and keeps first-match precedence unambiguous. Personal rules govern personal services.
