# Device login protocol and approval contract

This describes the implementation in the device-login branch. Deployment and
physical-device validation are separate steps. See [ADR-015](ADR-015-auth-device-login.md)
for the required staged issuance gate and [API.md](API.md#selectable-device-login-v2)
for HTTP response shapes.

## Request, approval and delivery

- A requester obtains one public `user_code` and a secret `device_code` from
  `/api/v1/auth/device/request` (legacy account only) or
  `/api/v1/auth/device/v2/request` (selectable account/Agent Key). The independent
  `/auth/agent-key` exchange is restricted to Agent Keys. The private poll secret
  selects the protocol; human code spelling does not grant capability.
- Legacy codes remain eight characters, displayed `XXXX-XXXX`. V2 defaults to
  `2-XXXX-XXXX`; `AUTH_DEVICE_EIGHT_CHAR_CODES=true` enables eight-character v2
  issuance after incompatible readers/writers are drained. Both readers remain.
  Normalize case, separators and I/L→1, O→0, U→V. Ambiguous retained matches in
  either collection fail closed. Reservations survive insertion failures until TTL.
- Use the server-returned `verification_uri_complete`, built from configured
  `FRONTEND_URL` and `/login/device?user_code=…`. The installed scanner fixture
  accepts trusted host/query payloads with eight-character codes. A QR identifies
  a request; scanning/signing in never approves it. No new browser handoff or
  deep-link scheme is introduced.
- Step 1 shows actual preview/code and requires “This is my request — continue”.
  Step 2 offers full account or restricted access only when `supports_grant_choice`
  permits it. Step 3 shows requested filters, matching keys and the actual grant,
  and requires a final explicit approval or “Create & continue”. Denial is terminal.
  The approving account is visible and can be signed out.
- Human authorization and resource/organization eligibility remain server-owned.
  Existing-key approval issues an independent child credential, without revealing
  or rotating the parent's secret. New-key creation and approval are transactional.
  An account grant produces an account session. Only the requester holding the
  private poll secret can collect the selected result once. Approval starts the
  bounded delivery window; abandoned delivery is cleaned up.
- Normal `/login` retains the legacy account-only browser exchange. Restricted
  credentials are not silently installed as browser sessions. The existing v2
  browser-poll login-code result is a separate explicit exchange, not a scoped
  browser session.

## Requester hints in `/login/device` and `/login/agent-key`

| Parameter | Accepted values / limit | Meaning |
|---|---|---|
| `user_code` | One valid eight-character code or prior v2 marker | Public request lookup; never sufficient authority to approve |
| `login_type` | `full` or `agent`; device default `full`, agent-key default `agent` | Suggested access tab; agent-key cannot choose full |
| `key_source` | `existing` or `new`; default `existing` | Suggested source, without selecting/creating a key |
| `key_name` | 1–64 characters | New draft name |
| `expiry_days` | `7`, `30`, `90`, `365`, `none`; default `90` | Draft expiry choice |
| `platform` | `generic`, `codex`, `claude-code`, `openclaw` | Attribution hint |
| `permissions` | Repeated/CSV; ≤1,024 decoded combined characters | NyxID scopes: `read`, `write`, `admin`, `openid`, `profile`, `email`, `services:read`, `services:write`, `proxy` |
| `services` | Repeated/CSV; ≤4,096 decoded combined characters | Catalog slugs, each 1–128 lowercase letters/digits/underscore/hyphen, starting alphanumeric |
| `service_permissions` | Repeated/CSV; ≤8,192 decoded combined characters | Catalog `slug::scope`; bare provider scopes remain accepted as filters |

`expiry_days` prefills the date picker with the UTC date that many days ahead;
date-only choices expire at 23:59:59 UTC on that date, following backend parsing.
The actual-grant summary and approval payload use that same instant. `none` clears
the expiry. These remain editable draft suggestions.

Raw query length is bounded to 49,152 characters. Unknown names, malformed percent
encoding, control characters, empty list entries, invalid enum values and duplicate
single-value parameters fail visibly. `key_name`/`key_source` require agent mode.
Unknown provider permissions/services are displayed as unresolved and block a match
until the user edits/removes them. URLs supply suggestions only: no automatic
approval, grant, provider reconnection or silent broadening follows from a hint.

Never trust URL-supplied owner/org IDs, actual service/node UUIDs, key IDs, allow-all
flags, capability flags, provider grants, consent snapshots, authentication kinds,
redirect destinations, credentials, access/refresh tokens or requester identity.
Those are not supported request-hint parameters. Browser `resume` is a narrowly
scoped internal resumption token, not a grant/configuration hint.

Valid hints survive password, MFA and social identity login. Return targets are
absolute trusted frontend URLs. If their encoded value exceeds 2,500 characters,
the explicit identity-verification click saves the validated query in tab-local
session storage for at most ten minutes and sends a short, random resumption URL.
Anonymous preview writes no storage. A missing/expired resumption fails visibly;
it never falls back to approving a different request. Terminal approval/denial
clears that resumption entry.

## Matching and consent freshness

The standalone search edits requested filters. Selected permissions remain in a
separate service-grouped panel with counts and removable pills. Group selection
uses the full group even while searching. Enter without a highlighted item makes
no selection; Escape returns focus to search; pill removal keeps focus in the panel.

Options return live `connections`. Each eligible key returns `effective_services`
and `permission_snapshot`; effective provider credentials resolve that key's
`AgentServiceBinding` override before the default user connection. Platform-bound
services use the catalog platform credential path, which precedes user overrides
in execution; consent reads availability metadata without decrypting credentials.
Their provider scopes are unreported and cannot satisfy specific provider-scope
filters or an exact-access claim. Catalog joins use
`catalog_service_slug`. Every requested permission/service must match, including
usable NyxID `proxy`/legacy `proxy:*` access. Exact matches precede extras; identity
permissions, extra services/nodes, future wildcards and unreported access remain
visible. Unknown access is never called exact. Legacy OAuth provider-token fallback
is classified as unreported provider access because execution may synchronize
scopes from another row. Provider filters do not narrow a stored connection.

Restricted keys with `allow_auto_connected_services` resolve their current IDs
through `key_service::effective_allowed_service_ids`: active rows owned by the key
owner with `source=auto_provision` or `credential_binding=platform` are included.
Both `ResourceSummary.auto_connected` and `EffectiveService.auto_connected` use
that classification. The grant includes future platform rows for that same owner,
and is always disclosed as extra access. It never implicitly grants another
owner's platform rows. Personal keys may separately select eligible organization
resources under the existing scope policy; organization keys stay within their
owner. `personal_owner_id` from options anchors draft ownership.

The draft's platform-grant setting uses the existing platform scope control. Owner
changes clear selected resources and wildcard grants. Approval snapshots include
all current implied platform connections as well as explicit connections; provider
filters cannot narrow those grants. The durable flag itself is part of existing-key
consent. New platform rows added later remain covered by the disclosed future grant.

A new draft prefills a connection only when exactly one eligible exact connection
exists for that requested service. Broader/ambiguous connections require explicit
choice. Detailed key settings remain available; the actual grant and all extras
are shown before creation.

New web clients send `selection.permission_snapshot` for existing keys or
`selection.connection_snapshots: [{service_id, permission_snapshot}]` for new keys.
The server compares the snapshot inside the issuance transaction and fences the
reviewed resource records against concurrent writes. A changed grant returns a
refresh/review conflict. Unchanged unavailable extras do not invalidate an otherwise
usable existing key. Fields are optional for installed clients predating this
extension. Snapshots do not replace ownership, scope or session checks. Ordinary
OAuth access-token refresh does not change a permission snapshot when stable
credential identity and scopes remain equal.

## Source and test pointers

- Request lookup, issuance/reservations: `backend/src/services/auth_device_service.rs`,
  `auth_device_service/public_code_tests.rs`; indexes: `backend/src/db.rs`.
- Capability, verification URI, legacy decisions/delivery: `backend/src/handlers/auth_device.rs`.
- Key options, binding resolution and consent fences:
  `backend/src/services/auth_agent_key_login_service/consent.rs` and `tests.rs`.
- UI and terminal/request state: `frontend/src/components/auth/device-approval.tsx`.
- Bounded raw query parser: `frontend/src/schemas/login-request.ts`; social resumption:
  `frontend/src/lib/login-resume.ts`; comparisons: `frontend/src/lib/login-permissions.ts`.
- Real React runtime/browser regressions: `frontend/e2e/device-approval.spec.ts`.
- Unchanged installed scanner: `mobile/src/features/auth/__fixtures__/installed-deviceUserCode.ts.txt`
  from `78cb26b3^`, exercised by `installedScanner.test.ts`. This proves source/parser
  compatibility, not physical-iPhone behavior.
