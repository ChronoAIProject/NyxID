# Service Accounts and Developer OAuth Apps

NyxID has two machine-identity surfaces beyond agent API keys (`nyxid_ag_…`). Both live under the admin gate but are user-facing for org admins.

- **Service accounts (`nyxid service-account`)** — `grant_type=client_credentials` machine identities. The SA gets a `client_id` + `client_secret` and exchanges them at `/oauth/token` for short-lived bearer tokens. Use this when a backend service (CI, scheduled job, internal tool) needs to call NyxID without holding a long-lived agent key.
- **Developer OAuth apps (`nyxid developer-app`)** — OIDC clients downstream apps register so end-users can sign in with NyxID. Distinct from service accounts: developer apps act on behalf of a user (authorization code / token exchange), not as themselves.

Both commands gate on admin role at the backend and accept `--org <ID|SLUG|NAME>` to scope creation to an organization.

> **Wizard defaults.** `service-account create / rotate-secret` and `developer-app create / rotate-secret` are wizard-capable: the bare form opens the local browser wizard or auto-falls-through to remote pairing on a headless box. Append `--no-wait --output json` only when the agent specifically can't hold a subprocess open (it returns a `{ pairing_id, pair_url, resume_cmd, … }` payload you can hand to the user, then resume with `nyxid pairing resume <pairing_id>`). Don't add `--terminal` or `--output json` (without `--no-wait`) on user-facing flows — those skip the wizard and dump the new `client_secret` to stdout where it may be captured by tool transcripts.

> **Before creating, list.** `nyxid service-account list --output json` and `nyxid developer-app list --output json` will tell you whether the user already has the identity they're describing. Most "create me a service account for CI" follow-ups are actually requests to *update* (rename, change scopes, rotate secret) an existing one.

## Table of contents

- [Service Accounts](#service-accounts)
- [Developer OAuth Apps](#developer-oauth-apps)
- [Choosing between API key, service account, developer app](#choosing-between-api-key-service-account-developer-app)

## Service Accounts

```bash
# Create a SA — bare wizard form. The wizard mints client_id + client_secret
# and shows the secret with click-to-reveal (DisplayOnce). On a headless
# agent the CLI auto-falls-through to remote pairing.
nyxid service-account create \
  --name "ci-runner" \
  --scopes "openid profile" \
  --description "Internal CI"

# Create scoped to an org (admin of that org required)
nyxid service-account create \
  --name "acme-ci" \
  --scopes "openid profile proxy" \
  --org acme-corp

# Override the per-second token-refresh rate limit (admin-only)
nyxid service-account create \
  --name "high-throughput-job" \
  --scopes "openid proxy" \
  --rate-limit-override 50

# Pre-assign role IDs at creation time (comma-separated)
nyxid service-account create \
  --name "release-bot" \
  --scopes "openid profile" \
  --role-ids "<role-id-1>,<role-id-2>"

# Headless agent — get a pairing handoff JSON and resume later
nyxid service-account create --name "ci-runner" --scopes "openid profile" \
  --no-wait --output json
nyxid pairing resume <pairing_id>

# List, search, paginate (read commands always use --output json)
nyxid service-account list --output json
nyxid service-account list --search "ci-" --page 1 --per-page 50 --output json
nyxid service-account list --org acme-corp --output json

# Show one SA
nyxid service-account show <SERVICE_ACCOUNT_ID> --output json

# Update metadata (no wizard — pure REST). Also accepts --is-active true|false
# to disable / re-enable without deleting (preferred over delete when you
# might restore later).
nyxid service-account update <SERVICE_ACCOUNT_ID> \
  --name "ci-runner-v2" \
  --description "Updated" \
  --scopes "openid profile proxy"
nyxid service-account update <SERVICE_ACCOUNT_ID> --is-active false   # disable
nyxid service-account update <SERVICE_ACCOUNT_ID> --is-active true    # re-enable

# Rotate the client secret. Wizard-capable: bare form opens DisplayOnce on
# the new secret. On headless agents, use --no-wait --output json then
# resume with `nyxid pairing resume <pairing_id>` once the user finishes.
nyxid service-account rotate-secret <SERVICE_ACCOUNT_ID>
nyxid service-account rotate-secret <SERVICE_ACCOUNT_ID> --no-wait --output json
nyxid pairing resume <pairing_id>

# Revoke every active token without rotating the secret
nyxid service-account revoke-tokens <SERVICE_ACCOUNT_ID>

# Soft-delete the SA (also revokes outstanding tokens)
nyxid service-account delete <SERVICE_ACCOUNT_ID> --yes
```

> SA tokens default to a 1-hour TTL (`SA_TOKEN_TTL_SECS`). Rotation revokes existing tokens immediately — running services keep working until their cached access token expires, then their next exchange returns the new token.

> The rate-limit-override flag controls the SA's token-refresh rate at `/oauth/token`, not the proxy throughput limit. Use per-API-key rate limits for proxy throttling.

## Developer OAuth Apps

```bash
# Create a public OAuth client (no secret; PKCE-only flows). Public clients
# don't enter the secret-display wizard at all because there's nothing to show.
nyxid developer-app create \
  --name "MyDesktopApp" \
  --redirect-uri "myapp://callback" \
  --client-type public \
  --allowed-scopes "openid profile email"

# Create a confidential client — bare wizard form. The wizard mints the
# client_secret and shows it once with click-to-reveal.
nyxid developer-app create \
  --name "BackendIntegration" \
  --redirect-uri "https://app.example.com/callback" \
  --client-type confidential \
  --allowed-scopes "openid profile email" \
  --delegation-scopes "proxy:read proxy:write" \
  --broker-capability true

# Register multiple redirect URIs (repeat the flag)
nyxid developer-app create \
  --name "MultiHostApp" \
  --redirect-uri "https://app.example.com/callback" \
  --redirect-uri "https://staging.example.com/callback" \
  --client-type confidential \
  --allowed-scopes "openid profile"

# Scope the client to an org (admin of that org required)
nyxid developer-app create \
  --name "AcmeDashboard" \
  --redirect-uri "https://dashboard.acme.com/callback" \
  --client-type confidential \
  --allowed-scopes "openid profile" \
  --org acme-corp

# Headless agent — get a pairing handoff JSON and resume later
nyxid developer-app create --name "BackendIntegration" \
  --redirect-uri "https://app.example.com/callback" \
  --client-type confidential --allowed-scopes "openid profile email" \
  --no-wait --output json
nyxid pairing resume <pairing_id>

# List, show
nyxid developer-app list --output json
nyxid developer-app list --org acme-corp --output json
nyxid developer-app show <CLIENT_ID> --output json

# Update metadata (no wizard — pure REST). Repeating --redirect-uri REPLACES
# the full list with the values you pass; pass `--allowed-scopes ""` to
# canonicalize back to "openid".
nyxid developer-app update <CLIENT_ID> \
  --name "Renamed App" \
  --redirect-uri "https://app.example.com/callback-v2" \
  --redirect-uri "https://app.example.com/callback-old" \
  --allowed-scopes "openid profile email"

# Toggle delegated-token-exchange capability on / off
nyxid developer-app update <CLIENT_ID> --broker-capability false
nyxid developer-app update <CLIENT_ID> --delegation-scopes ""    # disable token exchange

# Rotate the confidential client's secret (public clients have no secret).
# Wizard-capable: bare form opens DisplayOnce on the new secret. On headless
# agents, use --no-wait --output json then resume with `nyxid pairing resume
# <pairing_id>` once the user finishes.
nyxid developer-app rotate-secret <CLIENT_ID>
nyxid developer-app rotate-secret <CLIENT_ID> --no-wait --output json
nyxid pairing resume <pairing_id>

# Soft-delete the client
nyxid developer-app delete <CLIENT_ID> --yes
```

> The `--broker-capability` flag controls whether the client can mint OAuth broker bindings (RFC 8693 token exchange to opaque `binding_id` handles). See [`oauth-broker.md`](oauth-broker.md) for what bindings are and how end users see them at `/settings/authorizations`.

> `--delegation-scopes` controls which scopes the client may request via token exchange. Pass `""` to disable token exchange entirely on the client.

## Choosing between API key, service account, developer app

| Use case | Pick | Token shape |
|---|---|---|
| AI agent (Claude Code, Codex, Cursor, OpenClaw) calls NyxID's proxy | `nyxid api-key create` | `Authorization: Bearer nyxid_ag_…` |
| Backend / CI job calls NyxID without a human in the loop | `nyxid service-account create` | exchange `client_id` + `client_secret` at `/oauth/token` for a short-lived bearer |
| End-user-facing OAuth integration where users sign in with NyxID | `nyxid developer-app create` | standard OIDC authorization code (+ optional broker bindings) |

API keys are the simplest path for agents — no token exchange, no expiry by default. Reach for service accounts when you need rotatable secrets + automatic short-lived tokens, and developer apps when human end users are doing the consenting.
