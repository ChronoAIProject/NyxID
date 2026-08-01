# Assistant Readiness Snapshot Design

**Date:** 2026-08-01

**Status:** Approved for implementation

**Requirements baseline:** `ChronoAIProject/NyxID#1307` and `nyxid-chat/docs/requirements/aevatar-platform-prd.md` sections 12–14

**Visual baseline:** None; this issue adds an authenticated JSON read endpoint and fixture only.

## 1. Goal and scope

Add `GET /api/v1/assistant/readiness`, a safe user-scoped snapshot consumed by `nyxid-chat` before its first turn. The snapshot includes the two required platform capabilities (`model` and `runtime`) and the authenticated user's currently visible connector capabilities.

This issue does not add a readiness database, mutate connections or grants, expose credentials, calculate task state, or infer a task-specific required connector set from prompt text.

## 2. Confirmed decisions

- NyxID remains authoritative for identity, connection, credential, grant, approval, and management URLs.
- The verified `AuthUser.user_id` is the only caller identity accepted by the endpoint.
- `runtime` is derived from the active admin-managed `aevatar` `DownstreamService`. `model` checks the exact assistant LLM catalog slug (`chrono-llm-public` by the current deployment contract), not any arbitrary `llm-*` row.
- Existing visible `UserService` and `UserApiKey` safe views supply optional connector capabilities; the endpoint does not enumerate every unconnected catalog entry.
- Connector `capabilityId` is the canonical catalog slug when present, otherwise the active `UserService.slug`.
- The assistant approval requester is the standard delegated identity emitted by the existing bridge: `requester_type = "delegated"` and `requester_id = "aevatar"`.
- A task-specific connector becomes required only when a future Aevatar contract supplies a typed required-capability set. Until then connectors are optional; prompt heuristics are forbidden.
- `managementUrl` is the configured frontend origin joined with `/keys` only when `frontend_url` is HTTPS. An invalid or non-HTTPS configured URL produces `null`, never a caller-controlled fallback.
- Database/query failure fails the whole request through the existing safe `AppError` boundary. It is not converted to `missing`.

## 3. Contract

The response has exactly `revision`, `evaluatedAt`, and `capabilities`. Each capability has exactly:

```json
{
  "capabilityId": "api-github",
  "label": "GitHub",
  "required": false,
  "status": "available",
  "connectionState": "connected",
  "grantState": "granted",
  "requestedScopes": ["repo"],
  "managementUrl": "https://nyx.example/keys",
  "reasonCode": null
}
```

Closed enums:

- `status`: `available | missing | cannot_use | cannot_check`
- `connectionState`: `not_connected | connecting | verifying | connected | expired | revoked | unknown`
- `grantState`: `not_required | granted | partial | missing | expired | revoked | unknown`

Capabilities are sorted by `required` first, then `capabilityId`. Duplicate connector identities are collapsed conservatively: identical evidence stays exact; conflicting connection or grant evidence becomes `unknown`, which forces `status = cannot_check`.

`revision` is the lowercase SHA-256 digest of the stable serialized capability list. It excludes `evaluatedAt`, so unchanged evidence produces the same revision. `evaluatedAt` is the current UTC timestamp.

## 4. Evidence mapping

| Evidence | Connection state | Grant state | Status |
| --- | --- | --- | --- |
| Active configured core service | `connected` | `not_required` | `available` |
| Missing or inactive configured core service | `not_connected` | `not_required` | `missing` |
| Active connector key | `connected` | resolved approval state | `available` only for `granted` or `not_required` |
| `pending_auth` / `expired` / `revoked` key | `connecting` / `expired` / `revoked` | matching known evidence or `unknown` | never `available` |
| Connector visible but caller role/scope cannot use it | preserved connection evidence | preserved grant evidence | `cannot_use` |
| Unknown key state, conflicting duplicate evidence, or unresolvable approval evidence | `unknown` where applicable | `unknown` where applicable | `cannot_check` |

`requestedScopes` is empty in this endpoint until Aevatar supplies a typed task-specific required set. `KeyView.granted_scopes` describes scopes already granted to a connection and must not be relabelled as scopes requested by the current task. Approval grant evaluation reuses the existing org-aware policy and grant lookup for the delegated Aevatar requester. No session-bypass shortcut is used.

Core checks are configuration checks, not network health probes, and use `grantState = not_required`. `runtime` requires an active `aevatar` row that does not require a user credential. `model` requires the exact assistant LLM row to be active and usable through the existing public/internal master-credential or no-auth path. A row that exists but violates those provisioning predicates is `cannot_use`, not `missing`. Operation-specific approvals remain part of the actor execution loop; inventing an operation descriptor during readiness is forbidden.

## 5. Module boundaries

| Module | Responsibility | Depends on |
| --- | --- | --- |
| `assistant_readiness_service` | Read authoritative safe evidence, map closed states, aggregate duplicates, and compute revision | existing assistant, key, proxy-owner, and approval services |
| `approval_service` | Summarize existing org-aware approval policy and grant rows for one requester without inventing an operation descriptor | existing approval models and policy source selection |
| `handlers::assistant` | Convert `AuthUser.user_id` to a service call and serialize the dedicated response DTO | `assistant_readiness_service` |
| `routes` | Mount the GET under the existing human-only `/assistant` router with exempt billing classification | handler |
| versioned fixture | Freeze a secret-free consumer contract covering all enum values | service contract |

The handler never reads MongoDB collections directly. The service does not serialize existing model structs.

## 6. Consistency and security

- The endpoint is read-only; no transaction, idempotency key, or new persistence is required.
- Every query is scoped by the verified caller or by an existing admin-managed core-service lookup.
- HTTP success means only that a snapshot was evaluated; it does not approve or mutate anything.
- `connected + missing/partial/expired/revoked/unknown grant` is never `available`.
- No credential, encrypted credential, token, cookie, authorization header, provider response, endpoint URL, internal owner ID, grant requester ID, or raw approval payload is serialized.
- Unknown or contradictory evidence fails closed as `cannot_check`; authoritative access denial is `cannot_use`; absence is `missing` only where the queried authority proved absence.

## 7. Error behavior

Authentication and database failures use NyxID's existing `AppError` response mapping. The endpoint adds no new error envelope. Internal error strings, MongoDB details, stack traces, paths, and secrets never cross the boundary.

## 8. Testing and acceptance

- Pure mapping tests cover every closed enum, duplicate aggregation, scope normalization, status derivation, HTTPS management URL validation, stable revision, and secret-free serialization.
- Mongo-backed service tests prove user scoping, core evidence, delegated Aevatar grant identity, org-aware denial, and database-backed connector mapping. Tests use the repository existing no-local-Mongo skip guard.
- A route test proves the production path is mounted inside the existing human-only router and returns the dedicated JSON contract for an authenticated session.
- A versioned fixture is parsed by NyxID tests and then copied unchanged into `nyxid-chat`, where the consumer contract test reads the file.
- Completion commands are `cargo fmt --check`, focused readiness tests, and `cargo test --manifest-path backend/Cargo.toml`.

## 9. Deferred simplification

`biz-defer: connectors remain optional because this endpoint has no typed task-specific required-capability input; add required-set projection when Aevatar publishes that authority.`
