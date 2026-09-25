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
- Step 1 shows the actual preview/code. Signed-in approvers acknowledge “This is my request — continue”; signed-out approvers verify inline after choosing browser persistence.
  Step 2 offers full account or restricted access only when `supports_grant_choice`
  permits it. Step 3 shows requested filters, matching keys and the actual grant,
  and requires a final explicit approval or “Create & continue”. Denial is terminal.
  The approving account is visible and can be signed out.
  Existing and new keys share one authorization card with the key, owner and
  login expiry above service rows. Each row shows its selected accounts and
  extra-access count; expand it to inspect permissions or choose a connection
  for a new key. Missing accounts are labelled “Choose account” in that row.
  “Customize” contains requested filters and secondary key settings. Selection
  and review use the same rows; there is no separate review appended below the
  editor. Current/future resource authority remains visible when details are
  closed. Expanding, collapsing and selecting do not approve the request.
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
| `show_details` | `true` or `false`; default `false` | Expand the public requester details on initial load; presentation only, with no effect on identity, scope or approval |
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

Valid hints remain in the request page during inline password/MFA verification.
For a review link with requester details already expanded, append
`&show_details=true` to the server-issued verification URL. The user can collapse
the details again; returning through the stepper keeps that choice. Clicking
“Review request” returns to step 1 and expands the details. Neither action exposes
credentials or skips identity verification.
On an explicit social-verification click, tab-local storage retains the context ID,
expiry and query (never a credential). The callback returns to a fixed trusted
frontend request URL and restores the saved hints only for that same code and
unexpired context. Anonymous preview writes no storage. Legacy bounded `resume`
links remain readable; missing/expired resumption fails visibly. Terminal decisions
clear the resumption entry.

## Constructing an approval link

Start a real request on the intended deployment. For an AI agent that must receive
restricted access, use the independent Agent Key exchange:

```sh
nyxid login --agent-key --no-wait --output json --profile mail-agent \
  --key-source new --key-name "Mail and code assistant" \
  --scopes read,proxy \
  --service-permission 'api-google-gmail::https://www.googleapis.com/auth/gmail.readonly' \
  --service-permission 'api-github::repo' \
  --expiry-days 30 --platform codex
```

Use `--device` instead of `--agent-key` when the human may choose either full
account or restricted access. The two flags are mutually exclusive. On that
selectable exchange, `login_type=agent` suggests a choice; it does not prevent
the human from granting full account access. The CLI accepts the following options
and encodes their values into the approval URL:

| CLI option | URL parameter | Behavior |
|---|---|---|
| `--login-type full\|agent` | `login_type` | Defaults to `agent` when any preferences are supplied; `--agent-key` rejects `full` |
| `--key-source existing\|new` | `key_source` | Suggest the key-selection view |
| `--key-name` | `key_name` | Prefill the new-key name |
| `--scopes` (alias `--permissions`) | `permissions` | Requested NyxID scopes |
| `--service` (alias `--services`) | `services` | Requested catalog slugs |
| `--service-permission` (alias `--service-permissions`) | `service_permissions` | Requested `catalog-slug::provider-scope` values |
| `--expiry-days` | `expiry_days` | Prefill new-key expiry |
| `--platform` | `platform` | Prefill attribution |

Scope/service options accept repeated or comma-separated values. The CLI requires
catalog qualification for provider scopes so it cannot accidentally match an
unrelated service. It validates enums, control characters, empty entries and the
browser's decoded length limits before creating a request. Preferences conflict
with `--password`, `--callback`, `--code` and `login resume`; scoped preferences also
conflict with `--login-type full`. Hinted requests never fall back to the full-account
browser callback if the backend lacks the requested protocol.

The CLI prints `request_id`, `user_code`, the unchanged bare `verification_uri`,
**`verification_uri_complete`**, `expires_at`, `interval`, `profile` and `auth_kind`.
Share `verification_uri_complete` directly; no URL-building script is needed. Text
output and interactive browser opening use that same completed URL. With
`--no-wait`, the CLI stores the private polling secret locally and returns without
opening a browser or polling. The new JSON field is additive.

Direct API clients can build their own hinted URL from the returned challenge.
Device requests include `verification_uri_complete`; Agent Key requests provide
`verification_uri` and `user_code`. Build from those server-returned fields;
never generate a code locally, substitute a different approval route, or put
`device_code` in a link.

For a direct API integration, this builder supports either challenge shape:

```js
function buildAgentApprovalUrl(challenge) {
  const url = new URL(
    challenge.verification_uri_complete ?? challenge.verification_uri,
  );
  if (!challenge.verification_uri_complete) {
    url.searchParams.set("user_code", challenge.user_code);
  }
  const hints = {
    login_type: "agent",
    key_source: "new",
    key_name: "Mail and code assistant",
    permissions: "read,proxy",
    service_permissions:
      "api-google-gmail::https://www.googleapis.com/auth/gmail.readonly,api-github::repo",
    expiry_days: "30",
    platform: "codex",
  };
  for (const [key, value] of Object.entries(hints)) {
    url.searchParams.set(key, value);
  }
  return url.href;
}
```

`URLSearchParams` encodes spaces, commas, colons and provider-scope URLs once. Do
not pre-encode the values. Here `permissions=read,proxy` describes NyxID access;
`service_permissions` describes requested provider access. Discover catalog slugs
and `scope_catalog` entries from `GET /api/v1/catalog?include_all=true` on the
target deployment. Use its scope values, not human labels or guessed permission
names. The Gmail/GitHub example must be adapted if that deployment lacks them.

Show the resulting link to the human. After they review and explicitly approve,
resume using the CLI's local `request_id`, not the public `user_code`:

```sh
nyxid login resume <request_id> --output json
```

The saved request retains its destination profile. Inspect the delivered
`auth_kind` and grant before using it. Never self-approve or silently substitute
full account credentials when restricted access was required. Expired or denied
requests require a new request and human approval. URL prefilling requires the
updated approval frontend; this branch specification is not a deployment guarantee.

## Service discovery before and after approval

Before authentication, discover the deployment's supported services and scope
metadata without sending any saved credential:

```sh
nyxid catalog list --public --all --base-url https://nyx-api.chrono-ai.fun --output json
nyxid catalog show api-google-gmail --public --base-url https://nyx-api.chrono-ai.fun --output json
nyxid catalog endpoints api-google-gmail --public --base-url https://nyx-api.chrono-ai.fun --output json
```

The public catalog describes what can be requested. It does not reveal a person's
connected accounts or prove the requester may execute a service. `scope_catalog`
is a curated provider menu, not proof of a connection's actual granted scopes.
The approval page resolves the human's eligible connections after sign-in.

After delivery, discover operations using the granted profile:

```sh
nyxid whoami --profile mail-agent --output json
nyxid mcp discover --profile mail-agent --output json
```

`mcp discover` calls `GET /api/v1/mcp/config` with that credential. It returns the
server's service/node-scope-filtered services, operation IDs, input schemas,
recommended skills and diagnostics. It does not merge in the public catalog or
switch to another profile when access is refused. A revoked Agent Key is not
retried with a full account session. Discovery is a current view; live approval
policies, provider permissions and service availability still apply at execution.
`mcp config` remains the separate command for client connection configuration.

## Signed-out approvers and insufficient authority

1. Public preview shows the requester without granting credentials or exposing the
   approver's service connections. Browsers and QR payloads use
   `/login/device?user_code=XXXX-XXXX` with the documented hint parameters.
   The code is always a query parameter, preserving installed scanner support.
2. The signed-out approver chooses **Only for this request** (default) or
   **Keep me signed in** on the request page. Password and MFA verification stay
   inline. Configured social providers return to this request with tab-local hints.
   The original request expiry keeps running throughout verification.
3. **Only for this request** creates an opaque, HttpOnly browser proof accepted
   exclusively by `/api/v1/auth/approval`. It is bound to the actual request ID,
   flow, code, verified human and expiry (at most ten minutes). It cannot authorize
   `/users/me`, general account APIs, or another request. It creates no ordinary
   session or access/refresh token. An existing browser session is not revoked.
4. **Keep me signed in** also creates a normal browser session, atomically with
   completed identity verification. It survives approval or denial. Browser
   persistence and the requester's access are independent choices.
5. The approver chooses full account access or an eligible existing/new Agent Key,
   then explicitly approves or denies. Verification never approves the requester.
   MFA-enabled humans must complete MFA before any grant. Successful decisions
   close the proof; replay cannot grant again. Cancellation and expiry also end it.
6. An identity change clears key/connection choices. Pending responses from the
   previous identity cannot settle approval. An ended proof must be verified again.

Under `/api/v1/auth/approval`, the dedicated identity API supports begin
(`POST /` with `flow`, `user_code`, `keep_signed_in`), status/cancel (`GET`/`DELETE
/{id}`), password/MFA (`POST /{id}/password`, `POST /{id}/mfa`), inventory
(`GET /{id}/inventory`), and final decision (`POST /{id}/approve` or `POST /{id}/deny`).
Unsafe requests require the configured frontend Origin. The browser proof is a
host-only cookie scoped to `/api/v1/auth`; responses are `no-store`. Context IDs
and validated hints may be held in session storage; credentials never appear in
URLs or storage. Social verification requires tab storage so a provider redirect
cannot discard the context and silently become a normal login.

Being signed out differs from lacking authority. An Agent Key or third-party OAuth
credential cannot approve a new login. The approver must prove their human identity
through an existing first-party session or the dedicated verification flow. Neither
choice creates missing organization rights, provider connections or scopes. Choose
an eligible key/connection, arrange access with its owner, or deny; never silently
substitute full account access. Normal `/login` retains its existing behavior.

## What choosing an account grants

For a **new Agent Key**, the account choices in a service row are existing
connections (`UserService` IDs), not NyxID sign-in identities. Selecting multiple
connections grants the key access to each one. Two Gmail rows may represent two
mailboxes or two connections to the same mailbox; a label or slug alone does not
prove the upstream identity. Only show an email address when authoritative
connection data supplies it.

Each selected connection retains its actual provider permissions. A request for
Gmail read access does not remove sending access from a selected read-and-send
connection. The user must review the disclosed extras and choose a narrower
connection when appropriate. The final create/approve action binds the reviewed
connections and permission snapshots; selecting a row does not connect a new
provider account or create a key. NyxID account-only scopes need no provider
connection.

For an **existing Agent Key**, the card reviews its effective service bindings,
including per-key credential overrides. It does not edit the parent key or narrow
its permissions. Changing which connections are granted requires choosing another
key or creating a new one.

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
choice inside that service's expandable row. Selected connections, actual
permissions and extras share those rows with the final review. Requested filters
and secondary key settings live under Customize; no separate service-selection
page or appended review is required.

New web clients send `selection.permission_snapshot` for existing keys or
`selection.connection_snapshots: [{service_id, permission_snapshot}]` for new keys.
The server compares the snapshot inside the issuance transaction and fences the
reviewed resource records against concurrent writes. A changed grant returns a
refresh/review conflict. Unchanged unavailable extras do not invalidate an otherwise
usable existing key. Fields are optional for installed clients predating this
extension. Snapshots do not replace ownership, scope or session checks. Ordinary
OAuth access-token refresh does not change a permission snapshot when stable
credential identity and scopes remain equal.

## AI discovery and distribution

- This document is the canonical URL and approval contract; [API.md](API.md#selectable-device-login-v2)
  defines HTTP shapes and `frontend/src/schemas/login-request.ts` enforces hints.
- The NyxID skill routes login-link tasks to
  [references/device-login.md](../skills/nyxid/references/device-login.md). The CLI
  skill installer includes that reference in its fallback file list.
- [AI_AGENT_PLAYBOOK.md](AI_AGENT_PLAYBOOK.md#human-approved-agent-login-links)
  provides the public login recipe. Both `/llms.txt` and `/llms-full.txt` serve this
  playbook without authentication, with deployment-specific URLs. The server
  embeds it at build time; editing the source does not update running deployments.
- The MCP transport currently exposes tools, with no device-login specification
  resource or login-link builder. Public documentation and the installed skill
  are the available discovery paths, including before an agent has credentials.

## Source and test pointers

- CLI preference validation and URL construction: `cli/src/auth/login_hints.rs`
  and `login_exchange.rs`; command-level regressions: `cli/tests/login_resume.rs`.
- Public metadata and credential-scoped operation discovery:
  `cli/src/commands/catalog.rs`, `cli/src/commands/mcp.rs` and `backend/src/handlers/mcp.rs`.
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
