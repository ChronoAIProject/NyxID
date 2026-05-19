# Account, admin, MCP, approvals, and error codes

## Table of contents

- [Account Management](#account-management)
- [Admin Operations](#admin-operations)
  - [Platform roles](#platform-roles)
  - [User Management](#user-management)
  - [Invite Codes](#invite-codes)
- [MCP Configuration](#mcp-configuration)
- [Approval and Errors](#approval-and-errors)

## Account Management

```bash
nyxid whoami --output json                             # current user info (shows platform role: admin / operator / user)
nyxid status --output json                             # full account overview
nyxid profile update --name "New Name"                 # update display name
nyxid mfa setup                                        # enable MFA (shows QR code)
nyxid mfa verify --code 123456                         # verify MFA setup
nyxid session list --output json                       # list active sessions
```

## Admin Operations

### Platform roles

NyxID has three platform-level roles, ordered low-to-high:

- **`user`** — regular user; cannot read or write anything under `/admin/*`.
- **`operator`** — read-only platform admin. Can call every `/admin/*` GET endpoint (users, invite codes, audit log, OAuth clients, nodes, service accounts) but cannot mutate. Intended for strategy / share-ops / observability accounts that need authoritative cross-org data without holding admin keys.
- **`admin`** — full read + write on everything under `/admin/*`.

Operator reads are audited via fire-and-forget `admin.read.by_operator` entries that include the calling endpoint marker (e.g. `admin.users.list`, `admin.invite_codes.list`), so the audit trail can answer "operator X read endpoint Y at time T" independently of HTTP access logs.

`admin` overrides `operator` — granting `admin` implies all operator capabilities; you don't need to set both.

> The platform-level `operator` role is **separate from** the org-level `OrgRole::Viewer`. Org roles (`admin / member / viewer`) live on `org_memberships` and only affect access within one organization. Platform roles live on the `User` row and apply NyxID-wide.

### User Management

```bash
nyxid admin user list                                  # list platform users (paginated, 50/page default)
nyxid admin user list --search alice@                  # case-insensitive email substring search
nyxid admin user list --page 2 --per-page 100
nyxid admin user list --output json                    # machine-readable
nyxid admin user show <USER_ID>                        # one user's profile + role + verified / MFA / last login
nyxid admin user set-role <USER_ID> --role admin       # promote to admin (full read + write)
nyxid admin user set-role <USER_ID> --role operator    # promote to operator (read-only platform admin)
nyxid admin user set-role <USER_ID> --role user        # demote to regular user
```

Role-change semantics:

- `set-role` refuses to change the caller's own role (self-protection — admins can't demote themselves and lock the platform out).
- The CLI sends the legacy `{is_admin: bool}` body for `admin` / `user` transitions and the new `{role: "operator"}` body only when granting operator. Older backends without operator support stay compatible for `admin` / `user`; granting `operator` against an unupgraded backend surfaces a clear "backend upgrade required" error.
- `list` and `show` are operator-readable. `set-role` requires `admin`.

`nyxid admin` (without a subcommand) lists every available admin operation. Listing operations require operator OR admin; mutating operations require admin. Non-admin / non-operator callers get `1002 forbidden`.

### Invite Codes

NyxID gates new-user registration behind invite codes. Each code grants a bounded number of registrations and can be deactivated at any time. Only admins can create or deactivate codes; operators can `list` codes (read-only) but cannot create or deactivate.

```bash
nyxid admin invite-code create                                    # default: 10 uses, no note
nyxid admin invite-code create --max-uses 5 --note "alice@corp"   # bounded uses + admin note
nyxid admin invite-code create --output json                      # machine-readable
nyxid admin invite-code list                                      # show all codes + usage
nyxid admin invite-code list --output json
nyxid admin invite-code deactivate <ID>                           # invalidate a code by ID
```

Notes for admins helping new users:

- `max-uses` must be between 1 and 1000. The default is 10.
- Codes look like `NYX-XXXXXXXX`. Share the code verbatim -- the CLI and frontend normalize casing/whitespace before hitting the server, so `nyx-abc123` and `NYX-ABC123` are treated the same.
- `list` shows `used_count/max_uses`, active state, and the per-redemption `usages` array (who used it, when).
- Deactivation is immediate and cannot be undone -- create a new code if the user needs another attempt.
- Create and deactivate are audited (`admin_invite_code_create`, `admin_invite_code_deactivate`) and visible in `nyxid` audit tooling.
- **Turning the gate off entirely:** set `INVITE_CODE_REQUIRED=false` in the backend environment and restart the server. Public registration then works without a code and first-time social sign-ups succeed normally. Set it back to `true` (or unset it) to re-enable the gate.

## MCP Configuration

```bash
nyxid mcp config --tool cursor                         # generate MCP config for Cursor
nyxid mcp config --tool claude-code                    # generate MCP config for Claude Code
nyxid mcp config --tool vscode                         # generate MCP config for VS Code
```

## Approval and Errors

- `7000 approval_required` -- user must approve the request; includes `action_description` and `request_id` (check `nyxid approval list`). Default mode is per-request (every call needs approval).
- `7001 approval_failed` -- approval was rejected, expired, or timed out. Response includes `request_id` and `approve_url` (a link to the web UI where the user can review pending approvals). If the user has no notification channel configured, suggest they set one up with `nyxid notification telegram-link` or by installing the mobile app.
- `1001 unauthorized` -- token/key invalid or expired (run `nyxid login` to re-authenticate)
- `1002 forbidden` -- missing scope or service not configured
- `8003 node_proxy_error` -- node agent proxy failed (check `nyxid node list`)
- **403 from downstream with no NyxID error code** -- the downstream service itself rejected the request. A common cause is WAF rules blocking your User-Agent header (e.g. `OpenAI/Python 2.30.0`). The user can set a per-service custom User-Agent override via the frontend (key detail page > Service > User-Agent) or via API: `PATCH /api/v1/user-services/{id}` with `{"custom_user_agent": "MyApp/1.0"}`. Set to `""` to clear and revert to passthrough.
- **Any other static header a downstream requires on every call** (scope hint, API version, routing key) should be configured once as a service default via `nyxid service update <id> --default-header 'name=value'` rather than sent from every caller.
