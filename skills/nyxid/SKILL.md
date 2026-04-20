---
name: NyxID
description: Access user-connected services through NyxID's credential brokering proxy
version: 0.2.0
homepage: https://github.com/ChronoAIProject/NyxID
user-invocable: /nyxid
metadata:
  openclaw:
    requires:
      bins:
        - nyxid
    setup:
      - bash -c "$(curl -fsSL https://raw.githubusercontent.com/ChronoAIProject/NyxID/main/skills/nyxid/tools/install.sh)"
  clawdbot:
    emoji: "key"
    files:
      - "tools/*"
---

# NyxID

Use NyxID before asking the user to paste raw API keys or OAuth tokens for downstream services.

NyxID is the credential broker. The agent should use the `nyxid` CLI to discover services and make proxy requests. NyxID injects the user's stored credentials automatically.

For the full API reference, error codes, and advanced topics (SSH, MCP, OAuth client integration, service accounts), load `references/playbook.md` or fetch the latest from the NyxID server's `/llms.txt` endpoint.

## Setup

Install the NyxID CLI (one-time):

```bash
bash -c "$(curl -fsSL https://raw.githubusercontent.com/ChronoAIProject/NyxID/main/skills/nyxid/tools/install.sh)"
```

The installer handles everything: installs Rust if missing, builds the CLI, and configures your shell PATH. Open a new terminal afterwards, then log in:

```bash
nyxid login --base-url https://nyx-api.chrono-ai.fun
```

The CLI stores tokens at `~/.nyxid/` and auto-refreshes them. The base URL is saved on login -- all subsequent commands use it automatically.

> **Registration may require an invite code.** NyxID instances can gate new accounts behind invite codes (controlled by the backend `INVITE_CODE_REQUIRED` env var, default `true`). When enabled, users need a code from an admin and can register via the web UI or the CLI:
>
> ```bash
> nyxid register --base-url https://nyx-api.chrono-ai.fun \
>   --email you@example.com --name "Your Name" \
>   --invite-code NYX-XXXXXXXX
> ```
>
> When the gate is enabled, social login (Google, GitHub, Apple) only works for **existing** users -- first-time social sign-ups are blocked. Users must register with email + invite code first, then link a social provider afterwards by signing in with the same email. When the gate is disabled (public-launch mode), both email registration and first-time social sign-ups work without an invite code.

## Updating

Update the CLI and all installed AI skills in one command:

```bash
nyxid update                                 # update CLI binary + all installed skills
nyxid update --skills-only                   # update only installed skills (skip CLI rebuild)
```

To update a specific tool's skill only:

```bash
nyxid ai-setup update --tool claude-code     # update a specific tool
```

If `nyxid update` is not recognized, your CLI predates this command. Update it first with:

```bash
bash -c "$(curl -fsSL https://raw.githubusercontent.com/ChronoAIProject/NyxID/main/skills/nyxid/tools/install.sh)"
```

## Discover Services

Before using a downstream service, list what the user has configured:

```bash
nyxid service list --output json
```

The response includes:

- `slug`: service identifier for proxy URLs
- `status`: whether the service is active
- `endpoint_url`: where requests are routed
- `node_id`: whether routing through a node agent
- `credential_source`: `{ "type": "personal" }` for the user's own credentials, or `{ "type": "org", "org_id": ..., "org_name": ..., "role": ..., "allowed": ... }` for credentials inherited from an organization the user belongs to. Org-inherited services with `allowed: false` are visible to viewers but cannot be proxied -- do not attempt to call them.

If the target service is missing, help the user add it:

```bash
nyxid catalog list --output json                          # browse available services
nyxid catalog list --all --output json                    # include system services without auth
nyxid catalog show <slug> --output json                   # full metadata: links, capabilities, auth notes
nyxid catalog endpoints <slug>                            # list API endpoints from OpenAPI spec
nyxid service add <slug> --oauth                          # OAuth flow (opens browser -- easiest)
nyxid service add <slug> --device-code                    # device code flow (enter code on website)
nyxid service add <slug>                                  # API key (CLI prompts securely)
nyxid service add --custom                                # add custom endpoint (CLI prompts for details)
```

> For API key services, just run `nyxid service add <slug>` without flags. The CLI securely prompts for the key (input hidden). Never ask the user to paste secrets into chat or set environment variables manually.
> For automation/scripting only: `--credential-env <VAR>` reads from an environment variable.

### Requesting additional OAuth scopes

Some OAuth providers (Lark, Google, GitHub, Atlassian, ...) expose many scopes but NyxID's catalog only configures a sensible default set. When a user needs a capability that isn't covered -- for example Lark's contact/attendance APIs -- add extra scopes on top of the defaults with `--scope`:

```bash
# Single scope
nyxid service add api-lark --oauth --scope contact:contact.base:readonly

# Multiple scopes (repeat the flag or comma-separate)
nyxid service add api-lark --oauth \
  --scope contact:contact.base:readonly \
  --scope contact:department.base:readonly

nyxid service add api-lark --oauth \
  --scope "contact:contact.base:readonly,contact:department.base:readonly"

# Works the same way for device-code services
nyxid service add llm-openai --device-code --scope "custom-scope-1,custom-scope-2"
```

The extra scopes are merged (deduped) on top of the provider's `default_scopes` and forwarded in the authorization URL (or device code request). The upstream provider decides whether to grant them -- if the user's app/client doesn't have a scope enabled on the provider side, the authorization flow will still fail there.

**Supported flows:**
- `--oauth` (all OAuth2 providers) -- scopes are appended to the authorization URL.
- `--device-code` (RFC 8628 providers like GitHub, Google, most standard device-code providers) -- scopes are sent in the device code request.
- `--custom` -- `--scope` is accepted for symmetry but has no effect (custom endpoints use direct credentials, not OAuth). The CLI prints a warning.
- OpenAI-format device-code providers (e.g. the seeded `openai-codex` entry) do **not** accept additional scopes -- scopes are baked into the upstream client registration. The backend returns a validation error if you pass `--scope` to one of these, and the "AI Services" UI hides the scope input for them.

In the "AI Services" UI, the OAuth step and the standard device-code step include an optional "Additional scopes" input that accepts the same comma- or space-separated format.

### Scopes with node-routed services (`--via-node`)

Node-routed OAuth flows run on the node agent (so user credentials never leave the node machine). The two-step pattern is:

```bash
# Step 1: On any machine -- create the placeholder record on NyxID. The CLI
# prints the exact next-step command with your scopes pre-filled.
nyxid service add api-lark --oauth --via-node my-node \
  --scope contact:contact.base:readonly,contact:department.base:readonly
# -> "Next step: run this on the node that owns the credential:"
# -> "  nyxid node credentials setup --service api-lark --scope \"contact:contact.base:readonly,contact:department.base:readonly\""

# Step 2: On the node machine -- run the OAuth flow locally with the extras
# merged on top of the catalog's default scopes.
nyxid node credentials setup --service api-lark \
  --scope contact:contact.base:readonly,contact:department.base:readonly
```

`nyxid node credentials add-oauth` also accepts the same `--scope` flag (additive, repeatable) for manual setups. It still accepts the legacy `--scopes` flag (which **replaces** the default scope list entirely) for backward compatibility; prefer `--scope` unless you specifically need override semantics.

## Helping Users Add Services and Credentials

Most users do not know where to find API keys or what authentication method to use. Follow this workflow:

### The new default: CLI → browser wizard

Running `nyxid service add <slug>` with no scripted flags on an interactive TTY now launches a **local browser wizard** (PR #396, wizard v2). This is the recommended flow for non-technical users -- they get the same visual experience as the frontend's Add Key dialog without leaving the terminal to open the web app. End-to-end:

1. User runs `nyxid service add llm-openai` (or with no slug, to pick from the catalog in-wizard).
2. Terminal prints:
   ```
   → Opening http://127.0.0.1:<port>/?csrf=... … (Ctrl-C to cancel)
     Waiting for browser …
   ```
   The CLI boots a local axum server bound to `127.0.0.1:0` (random port, localhost-only), mints a per-session CSRF token, and hands the URL to `open::that` to launch the user's default browser.
3. Browser opens to the wizard SPA -- served entirely from the CLI binary (no remote scripts, strict CSP: `default-src 'none'; script-src 'self'; style-src 'self'`). The page inherits the prefill: catalog card pre-selected, label/endpoint/via-node pre-filled from the flags the user typed.
4. User completes the form: picks a catalog service or `--custom`-style custom endpoint, enters credentials (API key / OAuth / device-code all supported in-browser), clicks **"Done — return to terminal"**.
5. CLI rings the terminal bell (`\x07` + OSC-9 attention cue for iTerm2/WezTerm/Kitty) and prints a summary:
   ```
   ✓ Service 'OpenAI' created.
     Slug:      llm-openai
     Proxy URL: https://nyx-api.chrono-ai.fun/api/v1/proxy/s/llm-openai/
     Next:
       curl https://nyx-api.chrono-ai.fun/api/v1/proxy/s/llm-openai/<api-path> -H "Authorization: Bearer $NYX_KEY"
   ```
6. User closes the browser tab (or the tab says "You can close this tab and return to your terminal").

**Safety posture:** everything is local -- the browser never talks to the NyxID backend directly. The wizard's narrow allowlist of endpoints is proxied through the local server with the user's bearer token attached server-side, so the access token never hits the browser. A 10-second heartbeat watchdog cancels the CLI if the browser tab is closed without clicking Done; a 30-minute ceiling catches walked-away tabs.

**Prefill flags** (safe to combine with the wizard -- they just seed the form):
`--slug`, `--label`, `--via-node`, `--endpoint-url`.

### When the CLI falls back to terminal (rpassword) mode

Terminal mode is selected when **any** of the following is true:

- `--terminal` (alias `--no-wizard`) is passed on the command line -- **per-invocation override**, useful for a one-off scripted call on a GUI machine.
- `NYXID_NO_WIZARD=1` is set in the environment -- **sticky** across all invocations. Right choice for CI runners, Dockerfiles, and systemd units where you never want a browser to launch.
- A **scripted flag** is present (tells the CLI the caller already decided the flow): `--oauth`, `--device-code`, `--credential-env`, `--credential`, `--custom`, `--auth-method`, `--auth-key-name`, `--scope`, `--org`, `--openapi-spec-url`, or `--output json`.
- Running over **SSH** (`SSH_CONNECTION` or `SSH_TTY` is set) -- the browser would open on the wrong machine.
- Running on **Linux without a display** (both `DISPLAY` and `WAYLAND_DISPLAY` unset).
- **stdout is not a TTY** (piped to another command, redirected to a file).

Examples:

```bash
# One-off terminal prompt on a GUI machine
nyxid service add llm-openai --terminal

# Sticky opt-out (put in .bashrc, Dockerfile, or systemd Environment=)
NYXID_NO_WIZARD=1 nyxid service add llm-openai

# Scripted flow (auto-falls-through, no flag needed)
nyxid service add llm-openai --credential-env OPENAI_KEY --output json
```

**Guidance for AI agents using this skill:** always pass scripted flags (`--oauth`, `--credential-env`, `--output json`, etc.). Those automatically bypass the wizard, so an agent never accidentally tries to open a browser on a headless box. Use `--terminal` only when you need to explicitly force the rpassword prompt in an interactive session where none of the other triggers apply.


### Step 1: Check the catalog

```bash
nyxid catalog show <slug> --output json
```

The response includes `auth_type` which tells you what the service needs. Use this to guide the user.

### Step 2: Choose the right auth flow

- **OAuth** (`--oauth`): Best for non-technical users. Opens a browser -- the user signs in with their existing account. No API key needed. Use this for Google, GitHub, Twitter, and any service that supports OAuth.
  ```bash
  nyxid service add api-github --oauth
  ```

- **Device code** (`--device-code`): Good when the user can't open a browser from the terminal. Shows a short code to enter on the provider's website. Works well for services like OpenAI Codex.
  ```bash
  nyxid service add llm-openai --device-code
  ```

- **API key**: The user needs to get an API key from the provider's website. Guide them:
  1. Check the common portals table below for the provider's developer portal URL
  2. If the provider is not listed, search the web for "<provider name> API key" to find the right page, then tell the user exactly where to go
  3. Walk them through creating a key on the provider's site
  4. Tell them to run the command **without** `--credential-env` -- the CLI will securely prompt for the key (input is hidden, never shown on screen):
     ```bash
     nyxid service add llm-openai
     # CLI prompts: "Enter API key/credential:" (input hidden)
     ```
  Never ask the user to paste secrets into chat. The CLI's secure prompt keeps the key out of shell history and conversation logs.

### Step 3: Common provider portals

When users need API keys, direct them to the right place:

| Service | Where to get the key | Env var example |
|---------|---------------------|-----------------|
| OpenAI | https://platform.openai.com/api-keys | `OPENAI_KEY` |
| Anthropic | https://console.anthropic.com/settings/keys | `ANTHROPIC_KEY` |
| GitHub | https://github.com/settings/tokens | `GITHUB_TOKEN` |
| Google Cloud | https://console.cloud.google.com/apis/credentials | `GOOGLE_KEY` |
| Groq | https://console.groq.com/keys | `GROQ_KEY` |

For services not listed here, check `nyxid catalog show <slug> --output json` for the provider's documentation URL.

### Tips for non-technical users

- **Prefer `--oauth` or `--device-code`** over API keys whenever available -- the user just signs in.
- **Explain what an API key is**: "It's like a password that lets NyxID call the service on your behalf. You create it once and NyxID stores it securely."
- **Environment variables are temporary**: `export VAR="value"` only lasts for the current terminal session. The credential is stored in NyxID after `service add`, so the env var is only needed once.
- If the user is confused, break it into smaller steps. For example: "First, let's check what services are available" then `nyxid catalog list`.

## Make Proxy Requests

NyxID proxies requests to downstream services -- it handles authentication, but you need to
know the correct API paths, methods, and body formats for each service.

### How to find the right API paths

NyxID is just a proxy. The paths, methods, and request bodies are the same as calling
the downstream service directly. To figure out what to send:

1. Check catalog for endpoints: `nyxid catalog endpoints <slug>`
   - If the service has an OpenAPI spec, this returns all available endpoints (method, path, description)
2. Check the catalog for documentation: `nyxid catalog show <slug> --output json`
   - Look for `homepage_url`, `repository_url`, `documentation_url` -- links to docs and source
   - Check `capabilities` to understand supported interaction patterns
   - Check `auth_notes` and `known_limitations` for caveats
3. If no documentation URL is available, **search the web** for "<service name> API documentation"
   (e.g., "OpenAI API documentation", "Twitter API v2 documentation")
4. Use the provider's docs to determine the correct path, method, headers, and body format
5. Use `-H "Content-Type: ..."` if the service expects something other than JSON

### Important: paths are relative to the service's base URL

Each service in NyxID has a configured `endpoint_url` (base URL) that may already include
a version prefix. For example, `api-twitter` uses `https://api.x.com/2` as its base URL.
When making a proxy request, the path you provide is appended to that base URL:

- Service base URL: `https://api.x.com/2`
- Your path: `/tweets`
- Actual request: `https://api.x.com/2/tweets`

So do NOT duplicate the version prefix in your path. Check `nyxid service show <id> --output json`
to see the `endpoint_url` if you're unsure.

### Making the request

```bash
nyxid proxy request <slug> <path> -m <METHOD> -d '<body>'

# Custom content type (default is application/json)
nyxid proxy request <slug> <path> -m POST -H "Content-Type: application/xml" -d '<xml>...</xml>'

# Stream responses (SSE, video, audio, large files)
nyxid proxy request <slug> <path> -m POST --stream -d '<body>'

# Read body from file (uploads up to 100 MB supported on proxy routes)
nyxid proxy request <slug> <path> -m POST -d @request.json

# Read body from stdin
echo '{"prompt":"hello"}' | nyxid proxy request <slug> <path> -m POST -d -

# Explicit credential selection: when the user has both a personal and
# an org credential for the same slug, use --via-service to pick which
# one the proxy uses. Get the UserService ID from `nyxid service list`.
nyxid proxy request <slug> <path> -m POST --via-service <USER_SERVICE_ID> -d '<body>'
```

### Calling NyxID from raw HTTP (no CLI)

The CLI is a thin wrapper over the NyxID HTTP API. If you're integrating
from a service where installing the CLI isn't practical -- an automation
runtime, a webhook handler, another language -- call the proxy endpoint
directly. The only Authorization header the client sends is its own
**NyxID** bearer token; NyxID handles every downstream credential
(Lark `tenant_access_token`, OpenAI API key, GitHub PAT, etc.) entirely
server-side.

**Proxy endpoint shapes:**

| Path | When to use |
|---|---|
| `POST/GET/... /api/v1/proxy/s/{slug}/{path}` | Slug-based, most common |
| `POST/GET/... /api/v1/proxy/{user_service_id}/{path}` | UUID-based, when you already have the id from `GET /api/v1/keys` |
| `...?_nyxid_via=<user_service_id>` | Optional query param on either path. Bypasses auto-resolution and uses the specified UserService directly. The selected UserService must match the route's slug or service_id (returns 400 otherwise). Useful when both personal and org credentials exist for the same slug. Stripped before forwarding to downstream. |

**Example -- send a Lark message as a bot (no Lark token management):**

```bash
curl -X POST "https://nyx-api.chrono-ai.fun/api/v1/proxy/s/api-lark-bot/open-apis/im/v1/messages?receive_id_type=chat_id" \
  -H "Authorization: Bearer <nyxid_access_token>" \
  -H "Content-Type: application/json; charset=utf-8" \
  -d '{"receive_id":"oc_xxx","msg_type":"text","content":"{\"text\":\"hello\"}"}'
```

What happens server-side on that single request:

1. NyxID auth middleware validates `<nyxid_access_token>` and resolves the user.
2. Proxy handler looks up the user's `api-lark-bot` binding and loads the catalog `token_exchange_config`.
3. NyxID checks its in-process cache for this user's Lark `tenant_access_token`. Hit: jump to step 5.
4. Cache miss: NyxID decrypts `{app_id, app_secret}`, POSTs to Lark's `/auth/v3/tenant_access_token/internal` server-to-server (single-flight per app, so concurrent misses coalesce), caches the result (~2h TTL with 10-min safety margin).
5. NyxID strips the client's Authorization header, injects `Authorization: Bearer <tenant_access_token>` on the outbound request, and forwards to Lark.
6. Lark's response is returned to the client unchanged.

**Same pattern for any other service** -- OpenAI, GitHub, Twitter, etc. The client only ever sends its NyxID bearer; NyxID injects the downstream credential for each service according to the service's `auth_method` (bearer, header, body, token_exchange, ...).

**Obtaining the NyxID bearer token:**

- **Interactive user:** `POST /api/v1/auth/login` returns a short-lived access token (~15 min) and a refresh token (~7 days). Refresh via `POST /api/v1/auth/refresh`.
- **Service / agent:** provision a NyxID API key via `nyxid api-key create --platform <your-platform>` and use it directly as `Authorization: Bearer nyxid_ag_...`. API keys don't expire unless rotated.

**Things the client must NOT send:**

- A second `Authorization` header intended for the downstream (e.g. a Lark `tenant_access_token`). The allowlist strips any forwarded Authorization header by design, and raw HTTP clients that append instead of replace (reqwest's `RequestBuilder::header`, some JVM clients) would put duplicate Authorization lines on the wire and hit Cloudflare 400 at the edge. Let NyxID inject the downstream Authorization header.
- Downstream credentials (API keys, app secrets, tokens) in the request body or query string. NyxID already has them encrypted at rest and injects them according to the service's `auth_method`.

### Common service examples

Paths below are relative to each service's base URL. Check `nyxid service show <id> --output json`
for the `endpoint_url` if unsure.

```bash
# OpenAI (base: https://api.openai.com/v1) -- POST /chat/completions
nyxid proxy request llm-openai /chat/completions -m POST \
  -d '{"model":"gpt-4","messages":[{"role":"user","content":"Hello"}]}'

# Anthropic (base: https://api.anthropic.com/v1) -- POST /messages
nyxid proxy request llm-anthropic /messages -m POST \
  -H "anthropic-version: 2023-06-01" \
  -d '{"model":"claude-sonnet-4-20250514","max_tokens":1024,"messages":[{"role":"user","content":"Hello"}]}'

# GitHub API (base: https://api.github.com) -- GET /user/repos
nyxid proxy request api-github /user/repos -m GET

# Twitter / X (base: https://api.x.com/2) -- POST /tweets (not /2/tweets!)
nyxid proxy request api-twitter /tweets -m POST \
  -d '{"text":"Hello from NyxID"}'

# Discover all available proxy services
nyxid proxy discover --output json
```

NyxID injects the user's credentials automatically. Do not ask for or log raw downstream credentials.

## Managing Services

```bash
nyxid catalog list                                             # browse catalog (connectable services)
nyxid catalog list --all                                       # all services (including system/no-auth)
nyxid catalog endpoints <slug>                                 # list API endpoints from OpenAPI spec
nyxid service add <slug>                                       # add from catalog (CLI prompts for credential)
nyxid service add <slug> --oauth                               # add with OAuth (opens browser)
nyxid service add <slug> --device-code                         # add with device code flow
nyxid service add <slug> --via-node <name>                     # add via node (CLI prompts for credential)
nyxid service add --custom                                     # add custom endpoint (CLI prompts for details)
nyxid service list --output json                               # list services (includes IDs)
nyxid service show <id>                                        # show service details
nyxid service update <id> --label "My Custom Name"             # rename service
nyxid service update <id> --openapi-spec-url https://api.example.com/openapi.json  # attach an OpenAPI spec
nyxid service update <id> --openapi-spec-url ""                # clear the OpenAPI spec URL
nyxid service update <id> --default-header 'x-openclaw-scopes=operator.read,operator.write'
nyxid service update <id> --default-header 'x-api-version=v2:overridable'
nyxid service update <id> --default-header 'x-secret-token=abc123:sensitive'   # redact value in audit logs / API responses
nyxid service update <id> --clear-default-headers
nyxid service delete <id> --yes                                # remove service (no prompt)
```

> Default request header precedence is `catalog defaults -> UserService defaults -> caller`. The default is non-overridable unless `:overridable` is set on the value.

> Node commands accept names (e.g., `--via-node test-server`) in addition to UUIDs.

### Attaching an OpenAPI spec to a custom endpoint

Custom endpoints default to a single generic proxy tool. If the target service publishes an OpenAPI spec, attach the spec URL so AI agents (MCP, `/api/v1/endpoints/{id}/openapi-endpoints`) surface one tool per operation instead. Catalog-backed services inherit the catalog entry's spec URL automatically -- pass an empty string (`--openapi-spec-url ""`) on create if you want to opt out.

```bash
# Custom endpoint with OpenAPI discovery
nyxid service add --custom --label "My API" \
  --endpoint-url https://api.example.com/v1 \
  --openapi-spec-url https://api.example.com/openapi.json \
  --credential-env MY_API_TOKEN

# Catalog-backed key that suppresses the catalog's default spec URL
nyxid service add llm-openai --openapi-spec-url ""

# Attach or update the spec URL after the fact
nyxid service update <id> --openapi-spec-url https://api.example.com/openapi.json
```

URLs must be `http(s)://` and cannot contain `user:pass@` userinfo. The backend fetches them through a hardened path (DNS pinning, 5 MB size cap, no redirects, per-user cache scoping) and falls back to the generic proxy tool if the spec can't be fetched or parsed, so a broken spec URL never takes the service offline. SSH services ignore this field.

## Managing API Keys

Each AI agent or integration should use its own NyxID API key (agent key). This gives each caller independent audit trail, optional service bindings, and rate limits.

```bash
# CRUD
nyxid api-key create --name "My Key" --scopes "proxy read"
nyxid api-key create --name "coding-agent" --platform claude-code  # optional platform label
nyxid api-key create --name "relay-agent" --callback-url "https://..."  # for channel bot relay
nyxid api-key list --output json
nyxid api-key show <ID_OR_NAME> --output json
nyxid api-key rotate <ID_OR_NAME>
nyxid api-key delete <ID_OR_NAME> --yes

# Org-owned agent keys (for sharing one agent identity across the whole org)
nyxid api-key create --name "shared-coding-agent" --org <ORG_ID> --platform claude-code
nyxid api-key list --org <ORG_ID>                     # list all keys owned by this org
nyxid api-key rotate <ID> --output json               # any org admin can rotate
nyxid api-key delete <ID> --yes                       # any org admin can delete

# Consumers authenticate as the org: the agent's NYXID_ACCESS_TOKEN is the
# org's key, proxy calls see org-shared services directly without needing
# membership resolution, and audit logs attribute requests to the key
# (not the admin who created it).

# Service bindings (credential auto-resolved from service)
nyxid api-key bind <ID_OR_NAME> --service <SERVICE_SLUG>
nyxid api-key bind <ID_OR_NAME> --service <SLUG> --credential <LABEL>  # explicit override

# By default, agents can access all services with default credentials.
# Bindings override which credential is used for specific services.
# To restrict an agent to ONLY access bound services:
nyxid api-key update <ID> --allow-all-services false

# Callback URL for channel bot relay
nyxid api-key update <ID> --callback-url "https://my-agent.example.com/webhook"
nyxid api-key update <ID> --callback-url ""    # clear

# Per-key rate limits
nyxid api-key update <ID> --rate-limit-per-second 10 --rate-limit-burst 30
```

Set `NYXID_ACCESS_TOKEN` in your agent's environment to authenticate:

```bash
export NYXID_ACCESS_TOKEN="nyxid_ag_..."
```

### Scope requirements for management writes

Agent keys need `write` or `admin` scope to call management endpoints via REST (create/update/delete/rotate API keys, services, endpoints, bindings, etc.). `proxy read` is sufficient for proxy traffic only -- paths under `/proxy`, `/llm`, `/ssh`, `/channel-events`, `/channel-relay`, and `/delegation` do not require write scope. The `nyxid` CLI uses session auth (not API keys) and is unaffected.

## Organizations (Shared Credentials)

NyxID supports **organizations** for sharing a single set of credentials across multiple users. The classic example is a family Home Assistant or a company OpenAI key: one credential record, many people calling it through their own NyxID accounts. The proxy automatically falls back to org credentials when a personal one is missing for the requested service, with full per-member audit attribution.

### Mental model

- An **org is just a special user** (`user_type: "org"`) that cannot log in directly. It owns its own services, endpoints, API keys, etc., the same way a person does.
- **Membership** lives in `org_memberships`. Each member has a **role** (`admin` / `member` / `viewer`) and an optional service-id scope. Admins manage everything; members can use org services through the proxy; viewers can see them in `nyxid service list` but cannot proxy through them.
- **Resolution priority:** when a proxy request comes in, NyxID first looks for a personal `UserService` matching the slug. Only if that misses does it walk the user's active memberships (in `primary_org_id` order, then earliest-joined) and try the org's services. Personal credentials always win.
- **`credential_source` on `nyxid service list`**: every service in the response is tagged with `{ "type": "personal" }` or `{ "type": "org", "org_id": ..., "org_name": ..., "role": ..., "allowed": ... }`. Use this to tell the user which credentials are theirs vs. shared.

### Creating and managing an org

```bash
# Create an org. You become the first admin.
nyxid org create --display-name "Chrono AI"

# List all orgs you belong to
nyxid org list

# Show details (member count, your role)
nyxid org show <ORG_ID>

# Update metadata (admin only). Pass --avatar-url "" to clear.
nyxid org update <ORG_ID> --display-name "New Name"

# Delete (admin only). Refuses if the org still owns any shared services,
# endpoints, API keys, or NyxID API keys -- transfer or delete those first.
nyxid org delete <ORG_ID> --yes
```

### Sharing a service with the org

An org admin creates a shared service by passing `--org <ORG_ID>` to `nyxid service add`. The resulting `UserService`, `UserEndpoint`, and `UserApiKey` rows are written with `user_id = <org_user_id>`, so every member of the org immediately sees the service in their `nyxid service list` (tagged with `credential_source.type = "org"`) and can proxy through it using their own NyxID account.

```bash
# Shared OpenAI key for the whole org (API key credential)
nyxid service add llm-openai --org 1c3f8e2a-...

# OAuth flow targeted at the org. The browser opens under YOUR login, you
# grant access to the org's copy of the provider, and the resulting token
# is stored under the org's user_id so every member can proxy through it.
nyxid service add api-google --oauth --org 1c3f8e2a-...

# Device-code flow targeted at the org
nyxid service add llm-anthropic --device-code --org 1c3f8e2a-...

# Custom endpoint targeted at the org
nyxid service add --custom --org 1c3f8e2a-... \
  --label "Shared Home Assistant" \
  --endpoint-url https://ha.home.local:8123 \
  --auth-method bearer

# Node-routed shared service. The credential lives on the admin's
# personal node (encrypted at rest there) but the org service points
# at it. Every org member's proxy calls flow: NyxID -> admin's node ->
# downstream API. The admin must have write access to the node (which
# they do because it's their personal node); the node itself does not
# need to be re-registered under the org.
nyxid service add llm-openai --org 1c3f8e2a-... --via-node my-laptop-node
# Then on the node:
nyxid node credentials setup --service llm-openai
```

The backend enforces that the caller is an admin of the target org before writing the row (returns `8103 org_role_insufficient` otherwise). Creating or updating an org-owned service respects the membership's `allowed_service_ids` scope just like the proxy path.

> **How org-OAuth works under the hood.** The CLI creates a placeholder `UserApiKey` under the org's user_id (`POST /keys` with `target_org_id`), then initiates the OAuth / device-code flow with `target_org_id=X` on the query string. The backend stores the resulting `UserProviderToken` with `user_id = org`, and the sync routine matches it to the placeholder because both share the same user_id. The admin's personal scope is untouched -- the grant lives entirely under the org. If you prefer a dedicated identity for the org's OAuth grants (so personal account compromise does not leak the org credential), create a dedicated service account and use its token for the initial `nyxid login` before running `nyxid service add ... --oauth --org <X>`.

> Viewer-role members still see org services in the list (tagged `credential_source.allowed = false`) but cannot click into their detail page or proxy through them. Scoped members only see services within their `allowed_service_ids` scope -- services outside the scope are hidden entirely, not just disabled.

The frontend `/keys` page groups personal vs. each org section, with viewer-role and out-of-scope items rendered read-only.

### Inviting members

```bash
# Issue a one-time invite (admin only). Default role is member, default TTL 24h.
nyxid org invite create <ORG_ID> --role member
nyxid org invite create <ORG_ID> --role viewer --ttl-hours 168

# Restrict the invitee to specific UserService IDs (comma-separated)
nyxid org invite create <ORG_ID> --role member --allowed-service-ids "<svc1>,<svc2>"

# The output includes a join link AND a bare nonce. Share whichever is convenient:
#   Join link: https://<frontend>/orgs/join/ORGINV-...
#   CLI:       nyxid org join ORGINV-...

# Recipient redeems while signed in
nyxid org join ORGINV-ABCDEF12345678
nyxid org join "https://nyx.example.com/orgs/join/ORGINV-..."   # full URL also works

# List or cancel pending invites
nyxid org invite list <ORG_ID>
nyxid org invite cancel <ORG_ID> <INVITE_ID> --yes
```

Direct add (without an invite) is also available for tooling but not recommended:

```bash
nyxid org member add <ORG_ID> --user-id <USER_ID> --role member
```

### Managing members

```bash
# List members of an org (any member can read)
nyxid org member list <ORG_ID>

# Change a member's role or scope (admin only)
nyxid org member update <ORG_ID> <MEMBER_USER_ID> --role admin
nyxid org member update <ORG_ID> <MEMBER_USER_ID> --allowed-service-ids "<svc1>,<svc2>"
nyxid org member update <ORG_ID> <MEMBER_USER_ID> --allowed-service-ids ""    # clear scope

# Remove a member (admin only). Re-adding later reactivates the same membership row.
nyxid org member remove <ORG_ID> <MEMBER_USER_ID> --yes
```

### Multi-org tiebreaker: `primary_org_id`

When a user belongs to multiple orgs that all happen to share the same service slug, the proxy needs a deterministic winner. The default is the earliest-joined org. To override:

```bash
nyxid org set-primary --org-id <ORG_ID>      # set
nyxid org set-primary --clear                # unset (revert to earliest-joined)
```

### Working with org services in agents

For an AI agent making proxy requests, **nothing changes by default**. The agent calls `nyxid proxy request <slug> ...` exactly as before. NyxID looks up the credential -- personal first, then org -- and injects it. The audit log records `routed_via: "personal"` or `routed_via: "org"` (with `org_user_id` and `member_user_id`) so the org admin can see who used what.

If the user has both a personal and an org credential for the same slug and wants to explicitly choose which one the proxy uses, pass `--via-service <USER_SERVICE_ID>`:

```bash
# List services to see both personal and org entries with their IDs
nyxid service list --output json

# Use the org credential explicitly (bypasses personal-first auto-resolution)
nyxid proxy request llm-openai /chat/completions -m POST \
  --via-service <ORG_USER_SERVICE_ID> \
  -d '{"model":"gpt-4","messages":[{"role":"user","content":"Hello"}]}'
```

The `?_nyxid_via=` param is stripped before forwarding to the downstream service, so the downstream never sees NyxID routing metadata.

When listing services for the user, **always print the `credential_source` field** so the user can tell which credentials are theirs and which are shared. Viewer-role items have `credential_source.allowed = false`; do not attempt to proxy through them -- the request will return `8103 org_role_insufficient`.

### Org-level approval policies

An org admin can require approval whenever any member uses a specific org-owned service. Unlike personal per-service configs, **the org policy is dominant**: it overrides the member's personal approval gate for that service.

```bash
# Set an org policy: every member must get an admin's approval before
# their proxy call through this org service goes through. Per-request mode.
nyxid approval set-config <SERVICE_ID> --org <ORG_ID> --require-approval true

# Same but use time-based grant mode (approval creates a reusable grant)
nyxid approval set-config <SERVICE_ID> --org <ORG_ID> --require-approval true --approval-mode grant

# List current org-level policies on org-owned services
nyxid approval service-configs --org <ORG_ID> --output json

# List approval requests filed against org services (admin-only view)
nyxid approval list --org <ORG_ID> --output json

# List active grants created from org-policy approvals (grant mode only).
# These live under the org's user_id, so this is the only way for admins
# to see / revoke them.
nyxid approval grants --org <ORG_ID> --output json
nyxid approval revoke-grant <GRANT_ID> --org <ORG_ID> --yes
```

Frontend: the Org detail page has an **Approvals** tab for admins to manage these policies via a UI.

**Cascade semantics** (`approval_service::resolve_org_aware_approval`):

1. If the service being called is **org-owned** and the org has a per-service config row, the org's config wins absolutely. The `notify_user_ids` fan-out goes to every admin of the owning org, and **any** of them can decide.
2. Otherwise, fall back to the **actor's** per-service config for that service.
3. Otherwise, fall back to the actor's **global** approval toggle (`nyxid approval enable/disable`).

When a request is decided by an org admin, the decision is accepted regardless of which admin was the first to receive the notification -- the decide endpoint cross-checks current org admin status as defense-in-depth against admin set changes since the request was created. Grants created by approved org-policy requests live under the **org's** `user_id`, so the next member's call reuses the same grant instead of triggering a second approval.

Audit entries for org-policy-gated decisions include `routed_via: "org"`, `org_user_id`, `member_user_id`, and `policy_owner_user_id: "<org_user_id>"` so it is unambiguous whose policy caused the gate.

### Related error codes

- `1403 org_cannot_authenticate` -- attempted to log in as an org user (orgs cannot log in directly)
- `8100 org_query_timeout` -- the org-fallback membership query exceeded its 500ms wall-clock budget. The proxy returns 503; usually indicates MongoDB is degraded.
- `8101 org_not_found`
- `8102 org_membership_required` -- you tried to access an org you do not belong to
- `8103 org_role_insufficient` -- viewer tried to proxy, or non-admin tried to manage
- `8104 org_invite_invalid` -- unknown nonce or already-redeemed
- `8105 org_invite_expired` -- invite TTL elapsed; ask the admin for a new one
- `8106 org_approval_no_admin` -- the org policy on this service requires approval but the org has no active admins; an admin must be added before any member can use the service. Returned as 503 to make the degraded state obvious.

### CLI profiles

For running multiple identities on one machine, the CLI supports `--profile`:

```bash
nyxid login --base-url https://nyx-api.chrono-ai.fun --profile coding-agent
nyxid proxy request llm-openai /chat/completions --profile coding-agent -m POST -d '...'
NYXID_PROFILE=coding-agent nyxid proxy request ...  # or via env var
```

Profiles store tokens under `~/.nyxid/profiles/{name}/`. Without `--profile`, the default `~/.nyxid/` path is used.

## Node Management

Nodes are for users who do not want their credentials stored on the NyxID server. Instead, credentials stay encrypted on the user's own machine (the node). When a proxy request comes in, NyxID passes it through to the node agent via WebSocket, the node injects the credential locally and forwards the request to the downstream service. The credential never leaves the node.

### Setting up a new node

Registration must happen before installing the daemon. Credentials can be added before or after starting -- the agent reloads them automatically within 5 seconds.

```bash
# Step 1: Generate a registration token (on any machine with nyxid CLI)
nyxid node register-token

# Step 2: Install nyxid CLI on the target machine
bash -c "$(curl -fsSL https://raw.githubusercontent.com/ChronoAIProject/NyxID/main/skills/nyxid/tools/install.sh)"

# Step 3: Register the node (--keychain recommended for secure storage)
nyxid node register \
  --token "nyx_nreg_..." \
  --url "wss://<server>/api/v1/nodes/ws" \
  --keychain

# Step 4: Install and start as a background service (recommended)
nyxid node daemon install                              # install as system service
nyxid node daemon start                                # start the service

# Step 5: Add credentials (auto-registers catalog services in the backend)
nyxid node credentials setup --service llm-openai      # agent picks up new credentials automatically

# For custom endpoints: register first, then add credentials locally
nyxid service add --custom --via-node my-node           # creates backend record (prompts for URL, auth, etc.)
nyxid node credentials add --service my-api --header Authorization --secret-format bearer

# Or run in foreground (for debugging)
nyxid node start

# Or run via Docker
docker build -f cli/Dockerfile.node -t nyxid-node .    # build image (once)

# Option A: auto-register + start (no host setup needed)
docker run --user "$(id -u):$(id -g)" \
  -v ~/.nyxid-node:/app/config \
  -e NYXID_NODE_TOKEN=nyx_nreg_... \
  -e NYXID_NODE_URL=wss://<server>/api/v1/nodes/ws \
  nyxid-node

# Option B: mount existing config (registered on host)
docker run --user "$(id -u):$(id -g)" \
  -v ~/.nyxid-node:/app/config \
  nyxid-node
```

> `credentials setup` works for **catalog services only** -- it fetches config from the catalog and automatically registers the service in the backend with the node's ID.
> For **custom endpoints**, use `nyxid service add --custom --via-node <node-name>` first to create the backend record, then `nyxid node credentials add` to store the credential locally on the node.
> Credentials can be added, updated, or removed while the agent is running. The agent watches the config file and reloads credentials automatically (no restart needed). This works for both native daemons and Docker containers (config is mounted as a volume).
> Docker containers use the file backend (AES-GCM encrypted) -- OS keychain is not available in containers.

### Managing the node service

```bash
# Background service lifecycle (launchd on macOS, systemd on Linux)
nyxid node daemon install                              # install as system service (auto-starts on login)
nyxid node daemon install --force                      # reinstall / update service config
nyxid node daemon start                                # start the service
nyxid node daemon stop                                 # stop the service
nyxid node daemon restart                              # restart (picks up config changes)
nyxid node daemon status                               # check if installed and running
nyxid node daemon logs                                 # show recent logs (last 50 lines)
nyxid node daemon logs --follow                        # tail logs in real time
nyxid node daemon uninstall                             # remove service (stops first)
```

### Managing nodes

```bash
# nyxid CLI (manage nodes from user side)
nyxid node list --output json                          # list nodes (includes IDs)
nyxid node show <ID_OR_NAME> --output json             # show node details + metrics
nyxid node register-token                              # generate registration token
nyxid node delete <ID_OR_NAME> --yes                   # delete node
nyxid node rotate-token <ID_OR_NAME>                   # rotate node auth token

# nyxid node CLI (run on the node machine)
nyxid node credentials setup --service <SLUG>          # auto-detect and setup (recommended)
nyxid node credentials add --service <SLUG> --header Authorization --secret-format bearer
nyxid node credentials add-oauth --service <SLUG> --from-catalog  # OAuth from node
nyxid node credentials list                            # list configured credentials
nyxid node credentials remove --service <SLUG>         # remove credential
```

> `credentials setup` works for **catalog services**: it auto-detects whether the service needs an API key, OAuth, or gateway URL, guides the user through the right flow, and auto-registers the service in the backend with the node's ID. For **custom endpoints**, use `nyxid service add --custom --via-node <node>` first, then `nyxid node credentials add`.

## SSH Remote Access

All SSH commands accept service ID, slug, or name (auto-resolves):

```bash
nyxid ssh exec <SERVICE> --principal ubuntu -- uptime
nyxid ssh exec <SERVICE> --principal ubuntu -- ls -la /var/log
nyxid ssh terminal <SERVICE>                           # auto-resolves principal
nyxid ssh terminal <SERVICE> --principal ubuntu
nyxid ssh issue-cert <SERVICE> --public-key-file ~/.ssh/id_ed25519.pub --principal ubuntu --certificate-file ~/.ssh/id_ed25519-cert.pub
nyxid ssh proxy <SERVICE>                              # ProxyCommand for OpenSSH

# List SSH services
nyxid service list --output json | jq '.keys[] | select(.service_type == "ssh")'
```

## Set Up Notifications and Approvals

NyxID can require your explicit approval before any AI agent accesses your services. To receive approval requests, set up at least one notification channel:

### Step 1: Set up a notification channel

**Option A: Link Telegram** (recommended for desktop users)

```bash
nyxid notification telegram-link
# Follow the instructions: send the code to the NyxID bot on Telegram
```

**Option B: Download the NyxID mobile app** (recommended for on-the-go approvals)

- **Download (iOS & Android):** https://nyxid.onelink.me/REzJ/dql9w8fx

The link auto-detects your platform. The mobile app sends push notifications for approval requests. Log in with your NyxID account and your device is registered automatically.

You can use both Telegram and the mobile app together for redundancy.

### Step 2: Enable approval protection

Approval protection is enabled automatically when you link Telegram or register a mobile device. You can also toggle it manually:

```bash
nyxid approval enable                                  # enable approval protection
nyxid approval disable                                 # disable (auto-approve all requests)
```

### Step 3: Check your notification settings

```bash
nyxid notification settings                            # show current notification & approval status
```

If the user has not set up any notification channel yet, **proactively suggest** they do so before making proxy requests. Walk them through the steps above.

### Approvals reference

Approvals default to **per-request** mode: every proxy call needs fresh approval. The approval notification includes a human-readable `action_description` (e.g., "POST /v1/chat/completions (model: gpt-4, 3 messages)"). Grant-based approval is opt-in via `--approval-mode grant`.

```bash
nyxid approval list --output json                      # list pending approvals (includes action_description)
nyxid approval show <ID>                               # show approval details + action_description
nyxid approval approve <ID>                            # approve a request
nyxid approval deny <ID>                               # deny a request
nyxid approval enable                                  # enable global approval protection
nyxid approval disable                                 # disable global approval protection
nyxid approval grants --output json                    # list active grants (grant mode only)
nyxid approval service-configs --output json           # list per-service approval configs (includes approval_mode)
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
nyxid approval service-configs --org <ORG_ID> --output json
nyxid approval set-config <SERVICE_ID> --org <ORG_ID> --require-approval true
nyxid approval set-config <SERVICE_ID> --org <ORG_ID> --require-approval true --approval-mode grant
nyxid approval list --org <ORG_ID> --output json       # list requests against org services

nyxid notification settings                            # show notification settings
nyxid notification update --approval-telegram true     # enable telegram notifications
nyxid notification update --approval-push true         # enable push notifications
nyxid notification telegram-link                       # link telegram account
```

## Bot-Capable Service Connections

NyxID treats messaging platform bots as standard service connections. The credentials live in the same place as any other service (encrypted, scoped, audited) and outbound bot API calls go through the regular `/api/v1/proxy/s/{slug}/{path}` proxy. Inbound webhook handling is the responsibility of the calling agent runtime (Aevatar, custom backend, etc.) -- NyxID does not own chat runtime.

```bash
# Telegram bot (path-injected token)
nyxid service add api-telegram-bot
# CLI prompts for the bot token (from @BotFather)

# Then call any Bot API method directly -- pass only the method name in the
# proxy path. NyxID prepends `bot<token>/` automatically, so the forwarded
# URL becomes https://api.telegram.org/bot<token>/<method>.
nyxid proxy request api-telegram-bot sendMessage \
  -m POST -d '{"chat_id":12345,"text":"hello"}'

nyxid proxy request api-telegram-bot setWebhook \
  -m POST -d '{"url":"https://aevatar-host/api/channels/telegram/callback/abc"}'

nyxid proxy request api-telegram-bot getWebhookInfo -m POST -d '{}'

# IMPORTANT: do NOT include `/bot/` or `/bot{token}/` in the proxy path --
# NyxID adds it for you. `setWebhook` is correct; `bot/setWebhook` would
# forward as `bot<token>/bot/setWebhook` and Telegram returns 404.

# Lark bot (tenant token exchange is fully automatic)
nyxid service add api-lark-bot
# CLI prompts for app_id AND app_secret. NyxID stores both encrypted and
# handles the tenant_access_token exchange transparently on every call.
# Just hit the Lark API path directly -- no manual token management:
nyxid proxy request api-lark-bot /open-apis/im/v1/chats -m GET

nyxid proxy request api-lark-bot /open-apis/im/v1/messages \
  -m POST \
  -H "Content-Type: application/json; charset=utf-8" \
  -d '{"receive_id":"oc_xxx","msg_type":"text","content":"{\"text\":\"hello\"}"}'

# NyxID caches the tenant_access_token in-process (~2h TTL) and single-
# flights refreshes per app, so concurrent requests never produce
# duplicate exchanges. Your app_secret never leaves NyxID.

# Feishu bot (China region — same flow, same automatic token exchange)
nyxid service add api-feishu-bot

# Discord bot (Bot prefix in Authorization header, persistent token)
nyxid service add api-discord-bot
# CLI prompts for the bot token. Then call:
nyxid proxy request api-discord-bot /channels/{channel_id}/messages \
  -m POST -d '{"content":"hello"}'
# NyxID adds `Authorization: Bot <your_token>` automatically.
```

### If Lark/Feishu bot calls fail, recreate the binding

If `nyxid proxy request api-lark-bot ...` (or `api-feishu-bot`) returns
errors like **"Missing access token for authorization"**, **"token_exchange
auth method requires token_exchange_config"**, or any `99991xxx` Lark
error that shouldn't happen given your setup, your binding is probably
stuck on the **old body-injection shape** from an earlier NyxID version.

**In both the old and new flows, your `app_secret` is stored encrypted
on NyxID and never leaves the server after registration.** The only
thing that changed is how NyxID uses it:

- **Old flow:** NyxID stored only `app_secret`. The *caller* had to
  explicitly hit `/open-apis/auth/v3/tenant_access_token/internal`; the
  proxy merged `app_secret` into that request body server-side, handed
  back a `tenant_access_token`, and the caller was then responsible for
  caching it and attaching `Authorization: Bearer ...` to every
  subsequent Lark call.
- **New flow:** NyxID stores `app_id` **and** `app_secret` together
  (JSON blob, same AES-256 encryption). NyxID calls the exchange
  endpoint itself server-to-server, caches the `tenant_access_token`
  in-process with single-flight dedup, and injects the Bearer header on
  every outbound Lark request. Callers just hit the real API path.

Older bindings only contain `app_secret` without `app_id`, so the new
transparent-exchange path can't use them. Fix by deleting the binding
and re-adding -- this prompts for both fields and stores them in the
new shape:

```bash
# List your bindings and find the stale one (grab its id)
nyxid service list --output json | jq '.keys[] | select(.slug == "api-lark-bot") | {id, label}'

# Delete it (replace <id> with the id from the previous command; --yes
# skips the confirmation prompt so this works in agent contexts)
nyxid service delete <id> --yes

# Re-add -- the new prompt asks for BOTH app_id and app_secret
nyxid service add api-lark-bot

# Verify the new binding works (should return chats, not a missing-token error)
nyxid proxy request api-lark-bot /open-apis/im/v1/chats -m GET
```

You're just re-registering the same secret you already gave NyxID the
first time -- it travels once from your terminal to NyxID over HTTPS,
gets re-encrypted at rest, and then stays there. The same recreation
steps apply to `api-feishu-bot`.

### Picking the right service for the job

| Slug | Purpose |
|---|---|
| `api-lark` | Lark API as a logged-in user (OAuth) |
| `api-lark-bot` | Lark API as a bot (automatic tenant token exchange) |
| `api-feishu` | Feishu API as a logged-in user (OAuth) |
| `api-feishu-bot` | Feishu API as a bot (automatic tenant token exchange) |
| `api-telegram-bot` | Telegram Bot API |
| `api-discord` | Discord API as a logged-in user (OAuth) |
| `api-discord-bot` | Discord API as a bot (persistent bot token) |

## Channel Bot Relay

NyxID can bridge messaging platforms (Telegram, Discord, Lark, Feishu) to AI agent callback URLs. Users register their own bots, configure conversation-to-agent routing, and NyxID handles webhook reception, message normalization, and delivery to the agent.

NyxID is a **pure passthrough gateway** (ADR-013): it never stores message bodies or attachments. Only routing metadata lives in NyxID; the full conversation history belongs to the downstream agent.

### Register a bot

```bash
# Telegram
nyxid channel-bot register --platform telegram --label "My Support Bot" --token-env TELEGRAM_BOT_TOKEN

# Discord (requires public key for signature verification)
nyxid channel-bot register --platform discord --label "My Discord Bot" --token-env DISCORD_BOT_TOKEN --public-key "ed25519_public_key_hex"

# Lark / Feishu (requires app credentials)
nyxid channel-bot register --platform lark --label "My Lark Bot" --token-env LARK_BOT_TOKEN --app-id "cli_xxx" --app-secret-env LARK_APP_SECRET
```

For Telegram, NyxID auto-registers the webhook. For Discord/Lark/Feishu, configure the webhook URL in the platform's developer console: `https://<your-nyxid>/api/v1/webhooks/channel/<platform>/<bot-id>`. The bot auto-activates on first successful webhook delivery.

### Manage bots

```bash
nyxid channel-bot list                          # list registered bots
nyxid channel-bot show <ID>                     # bot details + conversation count
nyxid channel-bot verify <ID>                   # re-verify token and webhook
nyxid channel-bot delete <ID> --yes             # deregister bot
```

### Configure conversation routing

Each conversation route maps a platform chat to an AI agent (via API key with `callback_url`):

```bash
# Set up an API key with a callback URL first
nyxid api-key create --name "my-agent" --platform claude-code --callback-url "https://my-agent.example.com/webhook"

# Route all messages from a bot to this agent (default/catch-all)
nyxid channel-bot route create --bot <BOT_ID> --agent <API_KEY_ID_OR_NAME>

# Route a specific DM or group chat to a specific agent
nyxid channel-bot route create --bot <BOT_ID> --conversation-id "<chat_id>" --agent <API_KEY_ID_OR_NAME>

# Route a specific group chat with conversation type hint
nyxid channel-bot route create --bot <BOT_ID> --conversation-id "<group_chat_id>" --conversation-type group --agent <API_KEY_ID_OR_NAME>

# Per-user routing in a group (different agents for different users)
nyxid channel-bot route create --bot <BOT_ID> --conversation-id "<group_chat_id>" --sender-id "<user_id>" --agent <AGENT_A>
nyxid channel-bot route create --bot <BOT_ID> --conversation-id "<group_chat_id>" --sender-id "<user_id_2>" --agent <AGENT_B>

# List and manage routes
nyxid channel-bot route list --bot-id <BOT_ID>
nyxid channel-bot route update <ROUTE_ID> --agent <NEW_KEY>
nyxid channel-bot route delete <ROUTE_ID> --yes
```

Routing priority: sender-specific match > exact conversation match > default catch-all.

For Telegram, `conversation_id` is the `chat.id` (a number like `-1001234567890` for groups). For Discord, it's the `channel_id`. The bot must be added to the group/channel on the platform side.

### How it works

1. User sends message on Telegram/Discord/Lark/Feishu
2. Platform webhook delivers to NyxID
3. NyxID verifies signature, resolves route, writes a metadata-only record (per ADR-013, no text or attachments are persisted)
4. NyxID POSTs the normalized payload to the agent's `callback_url` with an HMAC signature
5. **Agent must return 202.** Sync replies (200 + body) are no longer supported — any body on a 200 is silently discarded
6. Agent processes asynchronously, then POSTs the reply to `/channel-relay/reply`
7. NyxID delivers the reply to the platform chat

The callback payload includes normalized fields (`content.text`, `sender`, etc.) and the full `raw_platform_data` (original Telegram/Discord/Lark JSON). The callback is the **only** place the message body exists inside NyxID — it's built in-memory from the live webhook parse and once the callback returns, NyxID retains nothing but metadata.

### Agent-facing endpoints (API-key authenticated)

```bash
# Async reply — this is the only way for an agent to respond
POST /api/v1/channel-relay/reply
{ "message_id": "<inbound-msg-id>", "reply": { "text": "..." } }

# Message history (metadata only — `text` and `attachments` are NOT returned per ADR-013)
GET /api/v1/channel-relay/messages/<conversation_id>?page=1&per_page=50

# Resolve platform sender to NyxID user
GET /api/v1/channel-relay/resolve-sender?platform=telegram&platform_id=12345
```

> **ADR-013 note:** `GET /channel-relay/messages/...` returns only routing metadata (direction, platform, sender ids, delivery status, timestamps). Agents that need conversation bodies must retain their own history.

## HTTP Event Gateway — device/analyzer events

NyxID also accepts push-mode events from external devices and analyzers on the same channel infrastructure. The envelope is converted to a `CallbackPayload` with `platform = "device"` and forwarded through the agent's `callback_url` just like a chat message.

**Endpoint:** `POST /api/v1/channel-events/{conversation_id}`
**Auth:** Bearer API key (`nyxid_ag_...`) bound to the target conversation
**Storage:** Metadata only. Event payloads are never persisted (ADR-013).
**Retry:** None. NyxID is a pure passthrough — the client decides what to do on failure.
**Rate limit:** 100 events/second per conversation (default, configurable).
**Idempotency:** Best-effort — same `event_id` within 5 minutes is deduplicated.

### Envelope

```json
{
  "event_id": "550e8400-e29b-41d4-a716-446655440000",
  "source": "camera-analyzer",
  "type": "person_detected",
  "timestamp": "2026-04-08T12:00:00Z",
  "payload": { "room": "living_room", "confidence": 0.95 },
  "metadata": { "analyzer_version": "1.0" }
}
```

### CLI

```bash
# Push a device event from a shell script
nyxid channel-event push \
  --conversation-id <CONVERSATION_ID> \
  --source camera-analyzer \
  --type person_detected \
  --payload-json '{"room":"living_room","confidence":0.95}'
```

### curl

```bash
curl -X POST https://<your-nyxid>/api/v1/channel-events/<CONVERSATION_ID> \
  -H "Authorization: Bearer nyxid_ag_..." \
  -H "Content-Type: application/json" \
  -d '{
    "event_id": "550e8400-e29b-41d4-a716-446655440000",
    "source": "camera-analyzer",
    "type": "person_detected",
    "timestamp": "2026-04-08T12:00:00Z",
    "payload": {"room": "living_room", "confidence": 0.95}
  }'
```

### Response codes

| Status | Meaning |
|---|---|
| 200 | Accepted (delivered) or deduplicated |
| 400 | Invalid envelope shape |
| 401 | Missing/invalid bearer, or API key is not bound to the conversation |
| 404 | Conversation not found |
| 429 | Per-channel rate limit exceeded |
| 502 | Downstream agent unreachable or returned non-2xx |

## OpenClaw Integration

```bash
nyxid openclaw setup --url http://localhost:18789   # CLI prompts for token securely
```

**For OpenClaw users:** After installing or updating this skill, start a new chat to activate it. If the gateway isn't installed as a background service yet, set it up so it stays running and restarts automatically:

```bash
openclaw gateway status    # check if already running as a service
openclaw gateway install   # install as system service (systemd on Linux, launchd on macOS)
openclaw gateway start     # start the service
```

Without this, restarting the gateway (`openclaw gateway restart`) will shut it down and it won't come back up on its own.

**Transport selection.** `llm-openclaw` supports both HTTP proxy (`POST /v1/chat/completions`, etc.) and WebSocket passthrough (the OpenClaw CLI's native `connect` + `chat.send` flow). Check `nyxid proxy discover --output json` — the entry exposes `"streaming_supported": true` and `"websocket_supported": true`. Use `wss://<nyxid-host>/api/v1/proxy/s/llm-openclaw` with a `Bearer` token for the WebSocket path. If it's node-routed and the WS upgrade times out, update the node agent and restart its daemon (older agents pre-date WS proxy support).

**Scope and routing headers.** The service-level default-header flag closes the workflow described in NyxID#161 -- agents no longer need to remember `x-openclaw-scopes` per call. Set it once on the UserService and every call carries it automatically:

```bash
nyxid service update <user-service-id> --default-header 'x-openclaw-scopes=operator.read,operator.write'
```

## Account Management

```bash
nyxid whoami --output json                             # current user info
nyxid status --output json                             # full account overview
nyxid profile update --name "New Name"                 # update display name
nyxid mfa setup                                        # enable MFA (shows QR code)
nyxid mfa verify --code 123456                         # verify MFA setup
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
```

## Approval and Errors

- `7000 approval_required` -- user must approve the request; includes `action_description` and `request_id` (check `nyxid approval list`). Default mode is per-request (every call needs approval).
- `7001 approval_failed` -- approval was rejected, expired, or timed out. Response includes `request_id` and `approve_url` (a link to the web UI where the user can review pending approvals). If the user has no notification channel configured, suggest they set one up with `nyxid notification telegram-link` or by installing the mobile app.
- `1001 unauthorized` -- token/key invalid or expired (run `nyxid login` to re-authenticate)
- `1002 forbidden` -- missing scope or service not configured
- `8003 node_proxy_error` -- node agent proxy failed (check `nyxid node list`)
- **403 from downstream with no NyxID error code** -- the downstream service itself rejected the request. A common cause is WAF rules blocking your User-Agent header (e.g. `OpenAI/Python 2.30.0`). The user can set a per-service custom User-Agent override via the frontend (key detail page > Service > User-Agent) or via API: `PATCH /api/v1/user-services/{id}` with `{"custom_user_agent": "MyApp/1.0"}`. Set to `""` to clear and revert to passthrough.
- **Any other static header a downstream requires on every call** (scope hint, API version, routing key) should be configured once as a service default via `nyxid service update <id> --default-header 'name=value'` rather than sent from every caller.

## Working Rules

- Always discover services before assuming a slug exists.
- Use `--output json` for machine-readable responses.
- Prefer slug-based proxy URLs over UUID-based ones.
- Use exact downstream API paths. Do not guess undocumented endpoints.
- Keep request bodies minimal and service-correct.
- Never try to extract or display the user's stored provider credentials.
- If multiple AI agents share a machine, each should have its own `NYXID_ACCESS_TOKEN`. Never share a single API key across multiple agents -- it defeats audit isolation and makes revocation impossible without disrupting all agents.
- Your User-Agent header is forwarded to downstream services by default (passthrough). Some downstreams block SDK-specific User-Agent strings -- see the 403 troubleshooting note in "Approval and Errors" above.
- If a downstream requires a static header on every call (scope hint, API version, routing key), configure it once as a service default via `nyxid service update ... --default-header 'name=value'` rather than sending it from every caller.

## External Endpoints

All requests are made through the `nyxid` CLI, which connects to the NyxID instance configured at login. No other external endpoints are contacted. Downstream service calls are made server-side by NyxID.

## Security and Privacy

- **Credentials are protected.** For server-stored credentials, NyxID injects them server-side. For node-routed services, credentials never leave the user's node -- NyxID passes the request through and the node injects credentials locally.
- **Authentication tokens auto-refresh.** The CLI handles token refresh automatically.
- **No data is sent to third parties.** All traffic flows between the agent and the user's NyxID instance.
- **Audit logging.** All proxy requests are logged in NyxID for user review.

## Model Invocation Note

This skill may be invoked autonomously by the agent when a user request involves an external service. The agent discovers available services through NyxID and routes requests through the proxy without prompting for raw credentials. Users can disable this skill in their OpenClaw configuration to opt out.

## Trust Statement

By using this skill, requests are sent to your configured NyxID instance. NyxID forwards those requests to downstream services using your stored credentials. Only install this skill if you trust your NyxID instance operator.

