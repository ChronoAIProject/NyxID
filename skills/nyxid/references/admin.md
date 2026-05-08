# Account, admin, MCP, approvals, and error codes

## Table of contents

- [Account Management](#account-management)
- [Admin Operations](#admin-operations)
  - [Invite Codes](#invite-codes)
- [MCP Configuration](#mcp-configuration)
- [Repo, Info, Telemetry](#repo-info-telemetry)
- [Approval and Errors](#approval-and-errors)

## Account Management

```bash
nyxid whoami --output json                             # current user info
nyxid status --output json                             # full account overview
nyxid info                                             # CLI version + project links
nyxid doctor                                           # run local install + auth health checks
nyxid doctor --json                                    # machine-readable doctor output
nyxid profile update --name "New Name"                 # update display name
nyxid profile consents --output json                   # list OAuth consents (web UI: /settings/consents)
nyxid profile revoke-consent <CLIENT_ID> --yes         # revoke an OAuth consent
nyxid profile delete --yes                             # delete the account (irreversible)
nyxid mfa setup                                        # enable MFA (idempotent: re-running before verify rotates the secret)
nyxid mfa setup --terminal                             # scripted enrollment (skip browser wizard)
nyxid mfa verify --code 123456                         # complete enrollment from the scripted path
nyxid mfa status --output json                         # show current MFA enrollment state
nyxid session list --output json                       # list active sessions
```

## Admin Operations

Commands under `nyxid admin` require the caller to have `is_admin=true` on their account. Non-admin callers get `1002 forbidden` from the server.

### Invite Codes

NyxID gates new-user registration behind invite codes. Each code grants a bounded number of registrations and can be deactivated at any time. Only admins can create or deactivate codes.

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
nyxid mcp config --tool generic                        # default: generic MCP shape (any compatible tool)
```

## Repo, Info, Telemetry

```bash
# Project links
nyxid repo                                             # print the NyxID GitHub repo URL
nyxid repo --open                                      # also open it in the default browser
nyxid info                                             # CLI version, build commit, project links

# Telemetry consent (~/.nyxid/config.toml)
nyxid telemetry status                                 # show resolved consent state and source
nyxid telemetry enable                                 # opt in (persists {enabled=true, asked=true})
nyxid telemetry disable                                # opt out (clears the local anon UUID so a re-enable starts fresh)
```

`nyxid telemetry` is the canonical editor for the persisted consent flag. Set `NYXID_TELEMETRY=0` in the environment for a per-invocation opt-out without touching the persisted state. See `docs/TELEMETRY.md` §3 for the precedence ladder.

## Approval and Errors

- `7000 approval_required` -- user must approve the request; includes `action_description` and `request_id` (check `nyxid approval list`). Default mode is per-request (every call needs approval).
- `7001 approval_failed` -- approval was rejected, expired, or timed out. Response includes `request_id` and `approve_url` (a link to the web UI where the user can review pending approvals). If the user has no notification channel configured, suggest they set one up with `nyxid notification telegram-link` or by installing the mobile app.
- `1001 unauthorized` -- token/key invalid or expired (run `nyxid login` to re-authenticate)
- `1002 forbidden` -- missing scope or service not configured
- `8003 node_proxy_error` -- node agent proxy failed (check `nyxid node list`)
- **403 from downstream with no NyxID error code** -- the downstream service itself rejected the request. A common cause is WAF rules blocking your User-Agent header (e.g. `OpenAI/Python 2.30.0`). The user can set a per-service custom User-Agent override via the frontend (key detail page > Service > User-Agent) or via API: `PATCH /api/v1/user-services/{id}` with `{"custom_user_agent": "MyApp/1.0"}`. Set to `""` to clear and revert to passthrough.
- **Any other static header a downstream requires on every call** (scope hint, API version, routing key) should be configured once as a service default via `nyxid service update <id> --default-header 'name=value'` rather than sent from every caller.
