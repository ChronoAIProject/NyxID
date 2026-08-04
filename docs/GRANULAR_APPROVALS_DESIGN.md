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
