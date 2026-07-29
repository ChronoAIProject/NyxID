# NyxID Assistant Actions Registry — Implementation Spec

- **Status:** Ready for implementation — spec review by GPT Sol (Codex) completed 2026-07-29;
  all findings (1×P1 rate-limit availability, 5×P2) incorporated
- **Date:** 2026-07-29 · **Owner:** Calvin
- **Deliverable:** `GET /api/v1/assistant/actions` — the action manifest endpoint the Aevatar host
  snapshots at startup to enable NyxID Assistant browser actions (chat "connect cards").
- **Pipeline:** GPT Sol (Codex) reviews this spec, then implements it on a branch off `main`.
  Fable reviews the implementation and raises the PR. Conventional commit:
  `feat(assistant): actions registry endpoint`.
- **Consumer coordination:** aevatarAI/aevatar#3026
  (comment: aevatarAI/aevatar#3026 issuecomment-5116220214). The consumer-side parser this spec
  must satisfy byte-for-byte is `NyxIdAssistantActionRegistry.Load` +
  `NyxIdAssistantActionRegistryHttpSource.FetchAsync` in
  `agents/Aevatar.GAgents.NyxidChat/` on aevatar branch `feature/integrate`.

---

## 1. Purpose

Aevatar's NyxIdChat kernel gates all browser actions behind a startup-pinned registry. When
`Aevatar:NyxId:AssistantActions:Enabled=true`, the Aevatar host fetches this endpoint **once at
startup, before accepting work**, validates it strictly, and refuses to start on any failure.
There is no refresh API and no retry loop — one anonymous GET, fail-fast.

Until NyxID serves this endpoint, the entire browser-action path (service-connect cards in chat)
cannot be enabled in any Aevatar deployment. This endpoint is the single NyxID-side unblocker.

The manifest is **metadata only**: verb names, model-facing descriptions, params schemas, and
advisory risk flags. It contains no secrets, no user data, and no tenant-specific content. It is
identical for every caller.

## 2. Consumer contract (hard constraints — violating any of these bricks Aevatar startup)

Extracted from the Aevatar parser. The implementation MUST satisfy every row; §8's tests encode
them locally so a NyxID change can never silently break Aevatar boot.

### 2.1 Transport

| Constraint | Source (Aevatar) |
|---|---|
| `GET {base}/api/v1/assistant/actions`, **no auth header sent** — the route must be publicly readable | `NyxIdAssistantActionRegistryHttpSource.FetchAsync` builds a bare `HttpRequestMessage` |
| Any non-2xx status is fatal | `EnsureSuccessStatusCode()` |
| Response body must be ≤ 1,048,576 bytes (Content-Length is pre-checked AND the stream is capped) | `MaximumRegistryBytes` |
| Body is parsed as JSON (`JsonDocument.Parse`); anything unparseable is fatal | `Load` |

### 2.2 Root object

| Field | Rule |
|---|---|
| `schema_version` | JSON number, integer, exactly `4` |
| `revision` | string ≤ 128 chars, exactly `"nyxid-assistant-actions.v4"` (ordinal equality) |
| `actions` | JSON array |

Root field names are **snake_case**. Unknown extra root fields are ignored by the parser, but we
serve none (keep the payload exactly the golden body in §4).

### 2.3 Per-action entry — all six fields required

| Field | Rule |
|---|---|
| `action` | string ≤ 128. **Must be a member of Aevatar's closed `SupportedActions` set** (§2.5). An unrecognized verb doesn't get skipped — it throws and the host does not start. Duplicates are fatal. |
| `description` | string ≤ 2048, non-empty after trim, no control characters. Written as an instruction to a model, not API documentation. |
| `params_schema` | JSON object satisfying the restricted schema grammar in §2.4 |
| `risk` | exactly one of `"low"`, `"grant"`, `"destructive"` |
| `tier` | must be exactly `"v1"`. (`"v2"` parses but is then rejected with `TierUnsupported` — fatal.) |
| `remember_eligible` | JSON boolean. Must be `false` when `risk` is `"destructive"` (fatal otherwise). |

Additionally: the manifest **must contain `service.connect`** — Aevatar checks its
`ExecutableActions` set is fully present and fails startup if not.

### 2.4 `params_schema` grammar (restricted JSON-Schema subset)

`ValidateSchemaNode` accepts ONLY:

- a node with `oneOf`: non-empty array; every branch recursively validated; nothing else read
  from that node;
- `{"type": "object", ...}`: MUST have `"additionalProperties": false` (JSON literal `false`) AND
  a `"properties"` object. Every property name is checked against the secret policy (below) and
  every property value recursively validated. `required` arrays are allowed and used at request
  time;
- `{"type": "array", ...}`: MUST have `items`, recursively validated;
- `{"type": "string"}`.

`type` is required (≤ 32 chars) on every non-`oneOf` node; types other than
`object`/`array`/`string` are fatal. The validator reads ONLY the keywords named above —
unrecognized keywords (`format`, `enum`, `minLength`, …) are **ignored, not rejected** (e.g.
`{"type":"string","format":"uri"}` loads fine but the `format` does nothing). NyxID authoring
policy is stricter than the parser: use only the recognized keywords plus `required`, so the
manifest never implies validation semantics the consumer does not enforce.

**Secret policy on property names** (`NyxIdActionSecretPolicy.ValidateFieldName`): the name is
normalized by dropping every non-ASCII-alphanumeric character and lowercasing (so
`client_secret`, `clientSecret`, `Client-Secret` all normalize to `clientsecret`), then rejected
if it lands in the forbidden set:

```
token tokens accesstoken refreshtoken authorization cookie cookies secret secrets
clientsecret password passphrase usercode devicecode rawbody rawupstreambody
credential credentials
```

No property name anywhere in any `params_schema` may normalize into that set.

Property names inside `params_schema` are **camelCase** (they mirror Aevatar's wire params, e.g.
`serviceSlug`, `requestedScopes`) — note the deliberate asymmetry with the snake_case root and
entry fields. Do not "fix" this.

### 2.5 Aevatar's recognized verb set (closed, as of `feature/integrate`)

```
service.connect          service.reauthorize        provider.set_app_credentials
key.create               key.rotate                 node.register_token
node.rotate_token        node.inject_credential     service_account.create
service_account.rotate_secret                       developer_app.create
developer_app.rotate_secret                         account.mfa_setup
device.onboard
```

Only members of this set may EVER appear in the manifest, and only `service.connect` is
executable in Aevatar v1 (the rest are "manifest-only": accepted at load, not yet dispatchable).
**We serve exactly one entry — `service.connect` — in v1.** Rationale: every additional entry is
pure coordination surface with zero function today, and any future verb outside a deployed
host's set is a startup-crash for that host (see §6).

## 3. Endpoint design (NyxID backend)

### 3.1 Route and mounting

- Path: `GET /api/v1/assistant/actions` (exactly — Aevatar appends the literal
  `RegistryPath = "/api/v1/assistant/actions"` to its configured base URL's authority + trimmed
  path, so `BaseUrl = https://auth.nyxid.dev` targets
  `https://auth.nyxid.dev/api/v1/assistant/actions`).
- **Mount in the `api_v1_public` router** (`backend/src/routes.rs`, the public runtime-metadata
  router built around line 1220): `.route("/assistant/actions", get(...))`. Auth in this codebase
  is extractor-driven, not blanket middleware — a handler is anonymous simply by not taking the
  `AuthUser` extractor. The handler takes **no** `AuthUser` (do NOT copy the catalog handlers;
  they extract `AuthUser` and are mounted in `api_v1_human_only`, so they are the wrong
  template for this route).
- **Rate limiting — exact-path exemption required.** The global limiter exempts only `/health`,
  `/.well-known/`, and `/mcp` today (`backend/src/mw/rate_limit.rs`, exemption check around line
  567); everything else draws from a per-IP bucket AND a process-global bucket with defaults of
  10 rps / burst 30. Aevatar's startup fetch is one anonymous GET with **no retry** — a 429
  (cold-start fanout behind one NAT, or unrelated traffic draining the global bucket) prevents
  the consumer host from starting at all. Add `/api/v1/assistant/actions` to the exact-path
  exemption list next to `/health`, with a unit test on the exemption predicate. This is safe:
  the response is a ~1.4 KB static string, no DB access, no amplification, no per-caller
  variance — the DoS surface is strictly smaller than the already-exempt `/health`.
- Security headers middleware still applies globally (note: it unconditionally adds
  `Pragma: no-cache`; that is fine here).
- Method: GET only. No query parameters. No CORS requirements (server-to-server), but nothing
  breaks if the global CORS layer covers it.

### 3.2 Handler

- New file `backend/src/handlers/assistant_actions.rs`; register in `handlers/mod.rs`.
- No service layer and no models: the manifest is a compile-time constant with no business
  logic, no DB access, and no per-request variance. (Deviation from the handler→service→model
  rule is intentional and mirrors `handlers/llms_txt.rs`; document with a one-line comment.)
- Build the JSON once and serve it as a `&'static str`: assemble via typed structs +
  `serde_json::json!` into a `LazyLock<String>` (or `OnceLock`) and expose
  `pub fn manifest_body() -> &'static str` so tests can call it directly — this makes the
  build-once claim observable instead of inferred. Serve with
  `Content-Type: application/json`. The handler sets no cache headers; in production the global
  security-headers middleware (`main.rs` response wrapper) adds
  `Cache-Control: no-store, no-cache, must-revalidate` and `Pragma: no-cache` to any response
  that omits them, and that default is acceptable here — the consumer fetches exactly once at
  startup and never caches, so a path-specific carve-out would be complexity for zero functional
  gain. Do not assert cache-header absence in tests that bypass that wrapper.
- Always `200 OK`. There is no error path.

### 3.3 Content constants

```rust
pub const ASSISTANT_ACTIONS_SCHEMA_VERSION: u32 = 4;
pub const ASSISTANT_ACTIONS_REVISION: &str = "nyxid-assistant-actions.v4";
```

Construct the body with typed serde structs (`AssistantActionsManifest { schema_version,
revision, actions }`, `AssistantActionDescriptor { action, description, params_schema, risk,
tier, remember_eligible }`) where `params_schema` is a `serde_json::Value` built with
`serde_json::json!`. Field order in the serialized output does not matter to the parser; the
golden test in §8 compares parsed values, not bytes.

## 4. The v1 manifest (normative payload)

The endpoint returns exactly this document (whitespace/field-order insignificant):

```json
{
  "schema_version": 4,
  "revision": "nyxid-assistant-actions.v4",
  "actions": [
    {
      "action": "service.connect",
      "description": "Ask the user's browser to connect a service through NyxID. Use when a task needs a catalog service (by slug) or a custom HTTPS endpoint that the user has not connected yet. NyxID owns the entire journey - auth modality, consent copy, and credential storage - and reports back only completion or decline with a safe resource reference. Never ask the user for keys, tokens, or passwords in chat.",
      "params_schema": {
        "oneOf": [
          {
            "type": "object",
            "additionalProperties": false,
            "required": ["catalogService"],
            "properties": {
              "catalogService": {
                "type": "object",
                "additionalProperties": false,
                "required": ["serviceSlug"],
                "properties": {
                  "serviceSlug": { "type": "string" },
                  "requestedScopes": { "type": "array", "items": { "type": "string" } },
                  "viaNodeId": { "type": "string" },
                  "targetOrgId": { "type": "string" }
                }
              }
            }
          },
          {
            "type": "object",
            "additionalProperties": false,
            "required": ["customService"],
            "properties": {
              "customService": {
                "type": "object",
                "additionalProperties": false,
                "required": ["name", "endpointUrl", "authMethod"],
                "properties": {
                  "name": { "type": "string" },
                  "endpointUrl": { "type": "string" },
                  "authMethod": { "type": "string" },
                  "authKeyName": { "type": "string" },
                  "viaNodeId": { "type": "string" },
                  "targetOrgId": { "type": "string" }
                }
              }
            }
          }
        ]
      },
      "risk": "grant",
      "tier": "v1",
      "remember_eligible": true
    }
  ]
}
```

The `params_schema` above is structurally identical to the fixture Aevatar's own registry tests
load (`ServiceConnectSchema` in `test/Aevatar.AI.Tests/NyxIdAssistantActionRegistryTests.cs`) —
it is the shape Aevatar validates outgoing `service.connect` requests against. Do not extend it
unilaterally: request-side field additions are an Aevatar-side change first.

`risk: "grant"` and `remember_eligible: true` follow the action-contract interaction model
(connecting a service changes what an agent can reach → one confirmation; a standing approval
grant may cover repeats). These values are advisory to Aevatar's presentation only — NyxID
recomputes authorization at execution time regardless.

## 5. Security

- **Metadata only, forever.** Nothing user-, tenant-, or deployment-specific may enter this
  payload. No feature flags, no internal hostnames, no service inventory. Adding any of those
  turns a public static file into an information-disclosure surface.
- Public read is intentional and safe under that invariant (same reasoning as `/llms.txt`).
- Rate-limit exemption (§3.1) is justified by the same invariant: ~1.4 KB static body, no DB, no
  amplification — a strictly smaller surface than the already-exempt `/health`. Security headers
  apply globally. Body is far under the consumer's 1 MiB cap.
- The §8 conformance test enforces the secret-policy property-name rule so a future edit cannot
  ship a field name Aevatar will reject at boot.

## 6. Change control (read before ever editing the manifest)

This endpoint's consumer pins BOTH the schema version and the revision string with ordinal
equality checks, and treats unknown verbs as fatal. Consequences:

1. **Never bump or vary `revision`/`schema_version` unilaterally.** A changed string does not
   degrade gracefully — every Aevatar host with the old constant fails startup validation, and
   every host expecting the new one fails against the old body. Version bumps are coordinated
   releases, Aevatar constant first.
2. **Adding a verb is Aevatar-first.** A verb outside a deployed host's `SupportedActions`
   crashes that host at boot. Order of operations: (a) Aevatar ships recognition (parser +
   `SupportedActions` entry) everywhere, (b) only then does NyxID add the manifest entry.
   (We have asked Aevatar to consider a tolerant-reader posture in the #3026 comment; until that
   lands, assume intolerance.)
3. **Safe without coordination:** editing `description` text (≤ 2048 chars, no control chars),
   and content-preserving refactors. Everything else in an entry (`risk`, `tier`,
   `remember_eligible`, `params_schema`) is consumer-validated — run the §8 conformance suite
   and treat any change as a contract change requiring a note on the coordination issue.
4. Record every manifest change in the PR description with a line confirming which Aevatar
   version range accepts it.

## 7. Out of scope (do not build)

- Executing actions, `action.continue`/stream changes, cards, consent UI — separate work items.
- Admin CRUD, DB-backed or per-tenant manifests, hot reload — the consumer snapshots once at
  startup; dynamism buys nothing and adds failure modes (FI-003).
- The other 13 recognized verbs — manifest-only entries with no function until Aevatar makes
  them executable.
- Auth on the route, ETag/conditional GET, compression tuning.

## 8. Testing requirements

All in Rust, alongside the handler (plus an integration test if the repo's axum test harness
makes it cheap):

1. **Golden test:** parse the served body and assert `schema_version == 4`,
   `revision == "nyxid-assistant-actions.v4"`, exactly one action, `action == "service.connect"`,
   `risk == "grant"`, `tier == "v1"`, `remember_eligible == true`, and the full `params_schema`
   equals the §4 value (`serde_json::Value` equality).
2. **Conformance suite** — a local mirror of Aevatar's `Load` rules, applied to whatever the
   handler serves (this is the guard that makes future edits safe):
   - body ≤ 1,048,576 bytes;
   - all six entry fields present with the §2.3 types/limits; description non-empty, ≤ 2048,
     no control chars;
   - `risk` ∈ {low, grant, destructive}; `tier == "v1"`; destructive ⇒ `remember_eligible == false`;
   - no duplicate `action`; every `action` ∈ the §2.5 set; `service.connect` present;
   - recursive `params_schema` walk enforcing the §2.4 grammar: `oneOf` non-empty; object nodes
     have literal `additionalProperties: false` and an object `properties`; array nodes have
     `items`; only `object`/`array`/`string` types; every property name, normalized by dropping
     non-**ASCII**-alphanumeric characters (`char::is_ascii_alphanumeric` — NOT
     `char::is_alphanumeric`, which keeps Unicode letters and would diverge from the consumer)
     and lowercasing, is absent from the §2.4 forbidden set.
3. **Handler test:** `GET /api/v1/assistant/actions` with **no Authorization header** returns
   200, `Content-Type: application/json`, and a body equal to `manifest_body()`; and
   `manifest_body()` equals the §4 golden value.
4. **Rate-limit exemption test:** unit test on the exemption predicate in `mw/rate_limit.rs`
   asserting `/api/v1/assistant/actions` is exempt (alongside the existing `/health` /
   `/.well-known/` / `/mcp` expectations, in whatever form those are currently tested).

## 9. Acceptance criteria

- [ ] `GET /api/v1/assistant/actions` is publicly reachable (no auth) on a locally running
      backend and returns the §4 payload with 200 / `application/json`.
- [ ] Route registered in the `api_v1_public` router; handler takes no `AuthUser` extractor.
- [ ] Exact-path rate-limit exemption added in `mw/rate_limit.rs` with its unit test (§8.4).
- [ ] Handler serves a `&'static str` built once (`manifest_body()` exposed and tested); no DB,
      no per-request body allocation.
- [ ] All §8 tests present and green; `cargo test` passes.
- [ ] End-to-end smoke documented in the PR: Aevatar host (branch `feature/integrate`) started
      with `Aevatar:NyxId:AssistantActions:Enabled=true` and `BaseUrl` pointed at the local
      backend completes startup registry validation (or, if running the .NET host locally is
      impractical, the PR states this and relies on the conformance suite).
- [ ] No changes to any other endpoint, model, or middleware.
- [ ] CLAUDE.md "Key API Routes" section gains one line for the new route.

## 10. References

- Consumer: aevatar `agents/Aevatar.GAgents.NyxidChat/NyxIdAssistantActionRegistry.cs`
  (`Load`, `SupportedActions`, `ExecutableActions`, `ValidateSchemaNode`),
  `NyxIdAssistantActionRegistryStartup.cs` (`FetchAsync`, `StartAsync`),
  `NyxIdActionSecretPolicy.cs` (`ValidateFieldName`, `NormalizeFieldName`) — branch
  `feature/integrate`.
- Consumer fixture: `test/Aevatar.AI.Tests/NyxIdAssistantActionRegistryTests.cs`
  (`RegistryJson`, `ServiceConnectSchema`).
- Contract: `docs/canon/nyxid-chat-api.md` (aevatar repo), "NyxID browser-action handoff:
  schema v4"; NyxID↔Aevatar Action Contract (owner: Calvin).
- Coordination: aevatarAI/aevatar#3026 + NyxID-side comment (issuecomment-5116220214).
