# Set Up Notifications and Approvals

## Table of contents

- [Step 1: Set up a notification channel](#step-1-set-up-a-notification-channel)
- [Step 2: Enable approval protection](#step-2-enable-approval-protection)
- [Step 3: Check your notification settings](#step-3-check-your-notification-settings)
- [Approvals reference](#approvals-reference)

NyxID can require your explicit approval before any AI agent accesses your services. To receive approval requests, set up at least one notification channel:

## Step 1: Set up a notification channel

**Option A: Link Telegram** (recommended for desktop users)

```bash
nyxid notification telegram-link
# Follow the instructions: send the code to the NyxID bot on Telegram
```

**Option B: Download the NyxID mobile app** (recommended for on-the-go approvals)

- **Download (iOS & Android):** https://nyxid.onelink.me/REzJ/dql9w8fx

The link auto-detects your platform. The mobile app sends push notifications for approval requests. Log in with your NyxID account and your device is registered automatically.

You can use both Telegram and the mobile app together for redundancy.

## Step 2: Enable approval protection

Approval protection is enabled automatically when you link Telegram or register a mobile device. You can also toggle it manually:

```bash
nyxid approval enable                                  # enable approval protection
nyxid approval disable                                 # disable (auto-approve all requests)
```

## Step 3: Check your notification settings

```bash
nyxid notification settings                            # show current notification & approval status
```

If the user has not set up any notification channel yet, **proactively suggest** they do so before making proxy requests. Walk them through the steps above.

## Approvals reference

Approvals default to **per-request** mode: every proxy call needs fresh approval. The approval notification includes a human-readable `action_description` (e.g., "POST /v1/chat/completions (model: gpt-4, 3 messages)"). Grant-based approval is opt-in via `--approval-mode grant`.

```bash
nyxid approval list --output json                      # list pending approvals (includes action_description)
nyxid approval show <ID>                               # show approval details + action_description
nyxid approval approve <ID>                            # approve a request
nyxid approval deny <ID>                               # deny a request
nyxid approval enable                                  # enable global approval protection
nyxid approval disable                                 # disable global approval protection
nyxid approval grants --output json                    # list active grants (grant mode only; shows scope)
nyxid approval service-configs --output json           # list per-service approval configs (approval_mode, default_effect, rules)
nyxid approval set-config <SERVICE_ID> --require-approval true                    # per-request (default)
nyxid approval set-config <SERVICE_ID> --require-approval true --approval-mode grant  # grant mode

# `<SERVICE_ID>` is a UserService ID from `nyxid service list --output json`
# (recommended — works for both catalog-backed and custom user services) or
# a legacy catalog `DownstreamService.id`. Custom services (added via
# `nyxid service add --custom`) have no catalog backing, so the UserService
# ID is the only way to target their per-service policy.

# Org-level per-service approval policies (admin only). When set, the
# policy is dominant over the member's personal gate: every member of
# the org must get an admin's approval before the proxy call goes through.
nyxid approval service-configs --org <ID|SLUG|NAME> --output json
nyxid approval set-config <SERVICE_ID> --org <ID|SLUG|NAME> --require-approval true
nyxid approval set-config <SERVICE_ID> --org <ID|SLUG|NAME> --require-approval true --approval-mode grant
nyxid approval list --org <ID|SLUG|NAME> --output json # list requests against org services

nyxid notification settings                            # show notification settings
nyxid notification update --approval-telegram true     # enable telegram notifications
nyxid notification update --approval-push true         # enable push notifications
nyxid notification telegram-link                       # link telegram account
```

## Granular approval rules

`--require-approval` is a single on/off switch for the whole service. For finer
control, attach **rules** that match a request's HTTP method, path, and verb and
decide per-operation. Rules work across every proxy protocol — HTTP, the LLM
gateway, MCP `tools/call`, and SSH `exec` (which matches on the command string).
SSH tunnels and terminals stay coarse (one decision per session).

A rule has these fields, given as a `key=value;key=value` string to `--rule`:

- `effect` — `auto_allow` (allow silently), `require_approval`, or `deny`
  (reject before any credential is used). Defaults to `require_approval`.
- `methods` — comma-separated, e.g. `GET,POST` (also `EXEC` / `TUNNEL` for SSH).
  Omitted means any method.
- `path` — glob over the request path (HTTP/LLM/MCP) or the command (SSH exec),
  e.g. `/v1/chat/*` or `rm *`. Defaults to `*`.
- `verbs` — comma-separated `read` / `write` / `destructive` (derived from the
  method). Omitted means any verb.
- `mode` — `per_request` or `grant`; only meaningful when `effect=require_approval`.

`--default-effect` decides operations that match no rule. It defaults to
`auto_allow`, so adding a few rules never silently blocks an unlisted endpoint of
a dynamic API. Set `--default-effect deny` (or `require_approval`) to turn the
service into an allow-list. Rules are evaluated in the order given; first match
wins. Limits: 50 rules, 16 methods per rule, 256-char patterns.

```bash
# Require approval for any write, auto-allow everything else (reads pass through).
nyxid approval set-config <SERVICE_ID> --rule 'verbs=write'

# Deny deletes outright, ask for writes (issuing a reusable grant), allow reads.
nyxid approval set-config <SERVICE_ID> \
  --rule 'effect=deny;methods=DELETE' \
  --rule 'verbs=write;mode=grant'

# Allow-list: only POST /v1/chat/completions is auto-allowed; everything else denied.
nyxid approval set-config <SERVICE_ID> \
  --default-effect deny \
  --rule 'effect=auto_allow;methods=POST;path=/v1/chat/completions'

# SSH: require approval before any `rm` command; allow other exec.
nyxid approval set-config <SSH_SERVICE_ID> --rule 'methods=EXEC;path=rm *'

nyxid approval set-config <SERVICE_ID> --clear-rules   # drop all rules, back to require/mode policy
```

With no rules configured, behavior is unchanged: the binary
`--require-approval` / `--approval-mode` gate applies. Approving an operation
issues a **scoped grant** (visible in `nyxid approval grants`): a grant for
`POST /v1/chat/completions` covers later calls to that same endpoint (across HTTP,
LLM, and MCP transports) but not, say, `DELETE /v1/files/*`. SSH grants stay
isolated to SSH.
