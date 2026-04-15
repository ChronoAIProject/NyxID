# NyxID AI Agent Playbook

This document is a reference for AI agents (Claude, Codex, ChatGPT, Gemini, etc.) to help users configure services, credentials, providers, nodes, and integrations on a running NyxID deployment. It uses placeholder URLs that the server replaces with real values when served via `/llms-full.txt`.

**Audience:** AI coding assistants helping developers use NyxID.

**Server URLs (replaced dynamically when served via /llms-full.txt):**
- Backend API: `http://localhost:3001`
- Frontend Dashboard: `http://localhost:3000`

---

## Table of Contents

1. [What is NyxID](#1-what-is-nyxid)
1b. [Install CLI Tools](#1b-install-cli-tools)
2. [Key Concepts](#2-key-concepts)
3. [Getting Started](#3-getting-started)
4. [Register a Service (External API)](#4-register-a-service-external-api)
5. [Register a Service (Internal / Shared Credential)](#5-register-a-service-internal--shared-credential)
6. [Register a Service (OIDC / SSO Provider)](#6-register-a-service-oidc--sso-provider)
7. [Add a Service (AI Services)](#7-add-a-service-ai-services)
8. [Set Up MCP Proxy for AI Clients](#8-set-up-mcp-proxy-for-ai-clients)
9. [Use the Credential Proxy](#9-use-the-credential-proxy)
10. [Set Up a Provider (OAuth / API Key / Device Code)](#10-set-up-a-provider-oauth--api-key--device-code)
11. [Deploy a Node Agent (On-Premise Credentials)](#11-deploy-a-node-agent-on-premise-credentials)
12. [Add Login to a React App (OAuth Client)](#12-add-login-to-a-react-app-oauth-client)
13. [Add Login to Any Web App (Raw OAuth / OIDC)](#13-add-login-to-any-web-app-raw-oauth--oidc)
14. [Server-to-Server Authentication (Service Accounts)](#14-server-to-server-authentication-service-accounts)
15. [Approval Workflow](#15-approval-workflow)
16. [SSH Services](#16-ssh-services)
17. [API Keys (Programmatic Access)](#17-api-keys-programmatic-access)
18. [LLM Gateway](#18-llm-gateway)
19. [API Quick Reference](#19-api-quick-reference)
20. [Error Code Reference](#20-error-code-reference)
21. [Troubleshooting](#21-troubleshooting)
22. [Common Pitfalls](#22-common-pitfalls)
23. [NyxID CLI](#23-nyxid-cli)
24. [Using NyxID in OpenClaw](#24-using-nyxid-in-openclaw)
25. [Channel Bot Relay](#25-channel-bot-relay)

---

## 1. What is NyxID

NyxID is an Auth/SSO and credential management platform. Users interact with it primarily to:

- **Store and proxy credentials** -- Register external APIs as services, store API keys/tokens, and let NyxID inject them into proxied requests automatically.
- **Expose APIs as MCP tools** -- AI clients (Cursor, Claude Code, Codex) connect to a single MCP endpoint and get tools from all connected services.
- **Run on-premise node agents** -- Keep sensitive credentials on your own infrastructure; NyxID routes requests through nodes without the credentials ever leaving your machines.
- **Act as an OAuth 2.0 / OIDC identity provider** -- Add "Sign in with NyxID" to your apps.
- **Manage providers** -- Connect external OAuth services (Google, GitHub, OpenAI, etc.) so users can link their accounts.

The dashboard is at http://localhost:3000. The API is at http://localhost:3001.

### CLI Tools

NyxID provides two CLI tools:

- **`nyxid`** -- User CLI for managing services, keys, catalog, nodes, and more. Preferred over curl for AI agents. Includes the `nyxid node` subcommand for on-premise credential management and proxy routing.

See [section 1b](#1b-install-cli-tools) for installation instructions.

---

## 1b. Install CLI Tools

### Prerequisites: Rust toolchain

Both CLIs are Rust binaries. Install Rust if you don't have it:

```bash
# macOS / Linux
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"

# Verify
rustc --version && cargo --version
```

> On macOS, you may also need Xcode command line tools: `xcode-select --install`

### nyxid (User CLI)

The `nyxid` CLI manages services, API keys, catalog browsing, MCP setup, and more. Authenticates via browser SSO or API key.

```bash
# Install from git
cargo install --git https://github.com/ChronoAIProject/NyxID nyxid-cli

# Or clone and build locally
git clone https://github.com/ChronoAIProject/NyxID && cd NyxID
cargo install --path cli

# Verify
nyxid --help
```

**First-time setup:**

```bash
# Login via browser SSO (opens browser, stores token at ~/.nyxid/access_token)
# --base-url is saved to ~/.nyxid/base_url -- all subsequent commands use it automatically
nyxid login --base-url http://localhost:3001

# Or use password login (for headless/AI-agent environments)
nyxid login --base-url http://localhost:3001 --password --password-env NYXID_PASSWORD

# Check connection (no --base-url needed after login)
nyxid status
```

> **Note:** After `nyxid login --base-url <URL>`, the URL is persisted at `~/.nyxid/base_url`. You do not need to pass `--base-url` on subsequent commands.

**Updating the CLI:**

To update the CLI to the latest version, reinstall it:

```bash
cargo install --git https://github.com/ChronoAIProject/NyxID nyxid-cli
```

If a command fails with an unrecognized flag or missing subcommand, the CLI is likely outdated. Reinstall it first.

**Without the CLI (API key only):**

If you can't install the CLI, set a NyxID API key as an environment variable and use curl:

```bash
export NYXID_API_KEY="nyxid_..."
export NYXID_BASE_URL="http://localhost:3001"
curl -H "X-API-Key: $NYXID_API_KEY" "$NYXID_BASE_URL/api/v1/keys"
```

### nyxid node (Node Agent Subcommand)

The `nyxid node` subcommand manages on-premise credential storage and proxying. It is built into the `nyxid` CLI. See [section 11](#11-deploy-a-node-agent-on-premise-credentials) for full setup.

```bash
# Install from git (requires Rust toolchain) -- nyxid node is included
cargo install --git https://github.com/ChronoAIProject/NyxID nyxid-cli

# Or clone and build locally
git clone https://github.com/ChronoAIProject/NyxID && cd NyxID
cargo install --path cli

# Verify
nyxid node version
```

### CLI vs API

Throughout this playbook, both CLI commands and API calls are shown. **AI agents should prefer CLI commands** when the `nyxid` binary is available -- they are shorter, handle authentication automatically, and avoid manual JSON construction.

---

## 2. Key Concepts

### Core Concepts

| Concept | Description |
|---------|-------------|
| **Catalog** | Read-only list of pre-configured service templates (OpenAI, Anthropic, etc.). Admin-managed via DownstreamService. Users browse the catalog to add services. |
| **UserEndpoint** | A target URL owned by a user (e.g., `https://api.openai.com/v1`). Auto-provisioned from catalog defaults or set manually for custom endpoints. |
| **UserApiKey** | A user's external credential (API key, OAuth token, bearer token). Encrypted at rest. |
| **UserService** | Proxy routing config that binds a UserEndpoint + UserApiKey + auth method. Defines the slug used in `/proxy/s/{slug}/*`. Optionally routes through a node. |
| **AI Services page** | Unified dashboard page at `/keys` with 2 tabs: **External Services** (UserEndpoint + UserApiKey + UserService) and **NyxID API Keys** (ApiKey with scope). Replaces the old Services, Connections, and Providers pages. |
| **OAuth Client** | An app registered to use NyxID as its identity provider (gets a client_id). |
| **Service Account** | A non-human identity for server-to-server auth (client_credentials grant). |
| **Node** | An on-premise agent that holds credentials locally and proxies requests through NyxID. Node routing is configured per-service on the AI Services page. |
| **MCP Proxy** | Exposes connected service endpoints as MCP tools at `/mcp`. |

### How Users Add Services

Users add services via a single action: `nyxid service add <slug>` (CLI) or `POST /api/v1/keys` (API). This auto-provisions all 3 records (UserEndpoint + UserApiKey + UserService) from catalog defaults or custom input. The old separate steps (register service, then connect credential, then bind node) are replaced by this one operation.

### Auth Methods for Credential Injection

- `bearer` -- Inject credential as `Authorization: Bearer <credential>`
- `header` -- Inject credential as a custom HTTP header (e.g., `X-API-Key: <credential>`)
- `query` -- Inject credential as query parameter
- `path` -- Inject credential as a URL path prefix (e.g., Telegram Bot API: `/bot<token>/method`)
- `basic` -- Inject credential as HTTP Basic auth
- `none` -- No credential injection

### Admin-Only Concepts (Service Catalog Management)

These are managed by admins and serve as templates for the user-facing catalog:

| Concept | Description |
|---------|-------------|
| **DownstreamService** | A service template in the catalog. Has default base URL, auth method, slug. Admin-managed. |
| **ProviderConfig** | OAuth/device-code plumbing for a provider (authorization_url, token_url, etc.). |
| **ServiceProviderRequirement** | Links a catalog service to a provider config. |

The old **Connection** and **Provider** concepts are now unified into the AI Services flow. Users no longer need to visit separate pages -- everything is managed from `/keys`.

---

## 3. Getting Started

### What the user needs before starting

- A NyxID account. Registration requires an **invite code** from an admin. Register via http://localhost:3000/register (web) or `nyxid register ... --invite-code NYX-XXXXXXXX` (CLI). Social login (Google/GitHub/Apple) only works for users who already have a NyxID account -- first-time social sign-ups are rejected because the OAuth redirect cannot carry an invite code.
- For admin operations: an admin account. Admins can mint new invite codes with `nyxid admin invite-code create` (defaults to 10 uses per code; `--max-uses` accepts 1..1000).
- An API key for AI agent access (see below)

### Set up an API key for AI agent access (recommended)

Create an API key so AI agents can make API calls on the user's behalf. The key should be stored as an environment variable -- **never paste it into AI chat**.

**Step 1:** Create an API key via the AI Services page at http://localhost:3000/keys (NyxID API Keys tab), via CLI, or via API:

**Via CLI (recommended):**

```bash
nyxid login --base-url http://localhost:3001   # one-time; saves URL
nyxid api-key create --name "AI Agent Key" --scopes "read write"
# Returns the key value (one-time display)
```

**Via API:**

```bash
# Login first to get an access token
ACCESS_TOKEN=$(curl -s -X POST http://localhost:3001/api/v1/auth/login \
  -H "Content-Type: application/json" \
  -d '{"email": "USER_EMAIL", "password": "USER_PASSWORD"}' \
  | jq -r '.access_token')

# Create an API key with read + write scopes
curl -X POST http://localhost:3001/api/v1/api-keys \
  -H "Authorization: Bearer $ACCESS_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"name": "AI Agent Key", "scopes": "read write"}'
# Response includes full_key -- save it immediately (shown only once)
```

**Step 2:** Store the key as an environment variable:

```bash
export NYXID_API_KEY="nyxid_..."
```

Add it to your shell profile (`~/.zshrc`, `~/.bashrc`) to persist across sessions.

**Step 3:** Use the key in API calls (via `X-API-Key` header or `Authorization: Bearer`):

```bash
curl http://localhost:3001/api/v1/users/me \
  -H "X-API-Key: $NYXID_API_KEY"

# Or as Bearer token:
curl http://localhost:3001/api/v1/users/me \
  -H "Authorization: Bearer $NYXID_API_KEY"
```

**IMPORTANT for AI agents -- credential safety rules:**

AI coding tools (Claude Code, Codex, Cursor) run commands in non-interactive shells with no TTY. `read -s` will not work. Use these methods instead:

1. **`$NYXID_API_KEY` env var** -- The user's NyxID API key. Always reference the variable name, never ask for the value.
2. **Environment variables for other secrets** -- Ask the user to set secrets as env vars before or during the session:
   - **Claude Code:** Tell the user to run `! export SECRET_NAME="value"` (the `!` prefix runs it in their real terminal -- the AI never sees the value).
   - **Codex / Cursor / other tools:** Tell the user to set the env var in a separate terminal, then reference `$SECRET_NAME` in commands.
3. **Dashboard UI** -- For entering credentials (service connections, provider keys, etc.), direct users to the relevant dashboard page where they can type secrets into form fields securely.
4. **`nyxid node credentials setup`** (recommended) or **`nyxid node credentials add`** -- For node agent credentials, these commands prompt for the secret value interactively (the user runs them themselves, not through the AI). `credentials setup` also auto-registers the service in the backend.
5. **Never** include secret values in commands, echo them, or ask the user to paste them into chat. Use placeholder text like `<your-api-key>` in examples.

### Alternative: Login with email/password

For short-lived sessions without an API key:

```bash
ACCESS_TOKEN=$(curl -s -X POST http://localhost:3001/api/v1/auth/login \
  -H "Content-Type: application/json" \
  -d '{"email": "USER_EMAIL", "password": "USER_PASSWORD"}' \
  | jq -r '.access_token')
```

Access tokens expire in 15 minutes. Refresh:

```bash
curl -X POST http://localhost:3001/api/v1/auth/refresh \
  -H "Content-Type: application/json" \
  -d '{"refresh_token": "REFRESH_TOKEN"}'
```

### A note on `$ACCESS_TOKEN` in examples

The API examples throughout this document use `$ACCESS_TOKEN`. If the user has set up an API key (recommended), replace with:

```bash
-H "X-API-Key: $NYXID_API_KEY"
# instead of:
-H "Authorization: Bearer $ACCESS_TOKEN"
```

### Or use the dashboard

Most operations can also be done in the NyxID dashboard at http://localhost:3000. The dashboard is the easiest way to get started -- the API examples below are for when the user wants to automate or script things.

---

## 4. Register a Service (External API)

> **Admin-only.** This section covers registering a new service in the catalog. Normal users add services from the catalog via `POST /api/v1/keys` (see [section 7](#7-add-a-service-ai-services)). The catalog is browsable at `GET /api/v1/catalog`.

**Goal:** Register an external API in the service catalog so users can add it from the AI Services page.

### Via Dashboard (admin)

1. Go to http://localhost:3000/services (admin section)
2. Click "New Service"
3. Fill in: name, base URL, auth type (e.g., API Key / Bearer / Basic), auth key name (e.g., `Authorization`)
4. Set category to "External" (users bring their own credentials)
5. Save -- the service now appears in the catalog for all users

### Via API (admin)

```bash
curl -X POST http://localhost:3001/api/v1/services \
  -H "Authorization: Bearer $ACCESS_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "OpenAI API",
    "slug": "openai",
    "base_url": "https://api.openai.com",
    "auth_method": "header",
    "auth_key_name": "Authorization",
    "service_category": "connection",
    "visibility": "public"
  }'
```

Save the returned `id` -- this is the service ID.

### Add API Endpoints (for MCP tools)

Endpoints define which API paths are exposed as MCP tools. You can auto-discover them from an OpenAPI spec or add them manually.

**Auto-discover from OpenAPI:**

```bash
# First set the spec URL on the service
curl -X PUT http://localhost:3001/api/v1/services/$SERVICE_ID \
  -H "Authorization: Bearer $ACCESS_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"openapi_spec_url": "https://api.example.com/openapi.json"}'

# Then trigger discovery (no request body)
curl -X POST http://localhost:3001/api/v1/services/$SERVICE_ID/discover-endpoints \
  -H "Authorization: Bearer $ACCESS_TOKEN"
```

**Add manually:**

```bash
curl -X POST http://localhost:3001/api/v1/services/$SERVICE_ID/endpoints \
  -H "Authorization: Bearer $ACCESS_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "create_chat_completion",
    "method": "POST",
    "path": "/v1/chat/completions",
    "description": "Create a chat completion"
  }'
```

---

## 5. Register a Service (Internal / Shared Credential)

> **Admin-only.** This creates a catalog entry with an admin-provided credential.

**Goal:** Register a service where the admin provides a shared credential. Users just enable access without managing keys.

### Via API

**Via Dashboard (recommended):** Create the service at http://localhost:3000/services (admin section) and enter the shared credential in the form.

**Via CLI:** First set the credential as an env var (user runs this themselves -- in Claude Code use `!` prefix):

```bash
# User runs this in their terminal (not through the AI):
! export SHARED_CRED="<the-shared-api-key>"
```

Then the AI can run:

```bash
curl -X POST http://localhost:3001/api/v1/services \
  -H "X-API-Key: $NYXID_API_KEY" \
  -H "Content-Type: application/json" \
  -d "{
    \"name\": \"Internal Analytics API\",
    \"slug\": \"analytics\",
    \"base_url\": \"https://analytics.internal.example.com\",
    \"auth_method\": \"header\",
    \"auth_key_name\": \"X-API-Key\",
    \"service_category\": \"internal\",
    \"credential\": \"$SHARED_CRED\",
    \"visibility\": \"public\"
  }"
```

Users connect with an empty body (no credential needed):

```bash
curl -X POST http://localhost:3001/api/v1/connections/$SERVICE_ID \
  -H "Authorization: Bearer $ACCESS_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{}'
```

---

## 6. Register a Service (OIDC / SSO Provider)

> **Admin-only.** This registers an OIDC-enabled service in the catalog.

**Goal:** Register a service where NyxID acts as the OIDC identity provider. Users sign in via NyxID's OAuth flow and the downstream app gets a client_id/client_secret pair.

### Via Dashboard (admin)

1. Go to http://localhost:3000/services (admin section)
2. Click "New Service"
3. Set auth type to **OIDC**
4. NyxID auto-generates a client_id and client_secret for the service
5. The service category is automatically set to "provider"

### Via API

```bash
curl -X POST http://localhost:3001/api/v1/services \
  -H "Authorization: Bearer $ACCESS_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Customer Portal",
    "slug": "customer-portal",
    "base_url": "https://portal.example.com",
    "auth_method": "oidc",
    "visibility": "public"
  }'
```

When `auth_method` is `oidc`, NyxID automatically:
- Creates an OAuth client with a generated `client_id` and `client_secret`
- Sets the default redirect URI to `{base_url}/callback`
- Sets `service_category` to `provider`

### Retrieve OIDC credentials

```bash
curl http://localhost:3001/api/v1/services/$SERVICE_ID/oidc-credentials \
  -H "Authorization: Bearer $ACCESS_TOKEN"
```

Response:

```json
{
  "client_id": "generated-uuid",
  "client_secret": "generated-secret",
  "redirect_uris": ["https://portal.example.com/callback"],
  "allowed_scopes": "openid profile email",
  "delegation_scopes": "",
  "issuer": "http://localhost:3001",
  "authorization_endpoint": "http://localhost:3001/oauth/authorize",
  "token_endpoint": "http://localhost:3001/oauth/token",
  "userinfo_endpoint": "http://localhost:3001/oauth/userinfo",
  "jwks_uri": "http://localhost:3001/.well-known/jwks.json"
}
```

Give these values to the downstream app to configure its OIDC client.

### Update redirect URIs

```bash
curl -X PUT http://localhost:3001/api/v1/services/$SERVICE_ID/redirect-uris \
  -H "Authorization: Bearer $ACCESS_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "redirect_uris": [
      "https://portal.example.com/callback",
      "https://portal.example.com/auth/callback"
    ]
  }'
```

### Regenerate client secret

```bash
curl -X POST http://localhost:3001/api/v1/services/$SERVICE_ID/regenerate-secret \
  -H "Authorization: Bearer $ACCESS_TOKEN"
```

Returns the new `client_secret` (one-time display).

---

## 7. Add a Service (AI Services)

Adding a service is now a single action. Use `nyxid service add <slug>` (CLI) or `POST /api/v1/keys` (API) to auto-provision all 3 records (UserEndpoint + UserApiKey + UserService) from catalog defaults or custom input.

### Option A: Add from catalog (most common)

**Via Dashboard (recommended):**

1. Go to http://localhost:3000/keys (AI Services page)
2. Click "+ Add Key"
3. Pick a service from the catalog (OpenAI, Anthropic, etc.)
4. Enter your API key and an optional label
5. Done -- the endpoint, credential, and proxy path are auto-configured

**Via CLI (preferred for AI agents):** First have the user set the credential as an env var (in Claude Code use `!` prefix, in other tools use a separate terminal):

```bash
# User runs this themselves (AI never sees the value):
! export SERVICE_CREDENTIAL="sk-proj-..."
```

Then the AI can run:

```bash
# CLI (preferred)
nyxid service add llm-openai --credential "$SERVICE_CREDENTIAL" --label "Production"

# API equivalent
curl -X POST http://localhost:3001/api/v1/keys \
  -H "X-API-Key: $NYXID_API_KEY" \
  -H "Content-Type: application/json" \
  -d "{\"service_slug\": \"llm-openai\", \"credential\": \"$SERVICE_CREDENTIAL\", \"label\": \"Production\"}"
```

This auto-creates:
- **UserEndpoint** with the catalog's default URL (e.g., `https://api.openai.com/v1`)
- **UserApiKey** with the encrypted credential
- **UserService** with the proxy slug, auth method, and auth key name from catalog defaults

### Option B: Add from catalog with custom endpoint URL

For services like OpenClaw where the user provides their own instance URL:

```bash
# User sets env var: ! export SERVICE_CREDENTIAL="my-bearer-token"

# CLI
nyxid service add llm-openclaw --credential "$SERVICE_CREDENTIAL" \
  --endpoint-url "http://localhost:18789" --label "Local OpenClaw"

# API equivalent
curl -X POST http://localhost:3001/api/v1/keys \
  -H "X-API-Key: $NYXID_API_KEY" \
  -H "Content-Type: application/json" \
  -d "{
    \"service_slug\": \"llm-openclaw\",
    \"credential\": \"$SERVICE_CREDENTIAL\",
    \"endpoint_url\": \"http://localhost:18789\",
    \"label\": \"Local OpenClaw\"
  }"
```

### Option C: Fully custom endpoint (no catalog)

For internal APIs or services not in the catalog:

```bash
# User sets env var: ! export SERVICE_CREDENTIAL="secret-token"

# CLI
nyxid service add-custom --label "Internal API" \
  --endpoint-url "https://internal.corp.com/api" \
  --credential "$SERVICE_CREDENTIAL" \
  --auth-method header --auth-key-name "X-API-Key"

# API equivalent
curl -X POST http://localhost:3001/api/v1/keys \
  -H "X-API-Key: $NYXID_API_KEY" \
  -H "Content-Type: application/json" \
  -d "{
    \"label\": \"Internal API\",
    \"endpoint_url\": \"https://internal.corp.com/api\",
    \"credential\": \"$SERVICE_CREDENTIAL\",
    \"auth_method\": \"header\",
    \"auth_key_name\": \"X-API-Key\"
  }"
```

The slug is auto-generated from the label (e.g., `internal-api`).

### Option D: OAuth provider flow

For services that use OAuth (e.g., GitHub, Codex):

```bash
# CLI -- opens browser for OAuth flow
nyxid service add github --oauth

# API equivalent -- start OAuth flow
curl -X POST http://localhost:3001/api/v1/keys/oauth/authorize \
  -H "X-API-Key: $NYXID_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"service_slug": "github"}'
# Returns: { "authorization_url": "https://..." }
# Open the URL in a browser to complete the OAuth flow.
# Tokens are stored automatically as a UserApiKey.

# Device code flow (e.g., Codex) -- no secrets needed
curl -X POST http://localhost:3001/api/v1/providers/$PROVIDER_ID/connect/device-code/initiate \
  -H "X-API-Key: $NYXID_API_KEY"
# Returns: { "user_code": "ABCD-1234", "verification_uri": "https://..." }
# Show user_code to the user, then poll:
curl -X POST http://localhost:3001/api/v1/providers/$PROVIDER_ID/connect/device-code/poll \
  -H "X-API-Key: $NYXID_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"state": "STATE_FROM_INITIATE"}'
```

**Never** put actual credential values in commands. Always use env vars or the dashboard.

### Browse the catalog

```bash
# CLI
nyxid catalog list                                # connectable services only
nyxid catalog list --all                          # all services (including system/no-auth)
nyxid catalog show llm-openai                     # full metadata: links, capabilities, auth notes, limitations
nyxid catalog endpoints llm-openai                # parsed API endpoints from OpenAPI spec

# API equivalent
curl http://localhost:3001/api/v1/catalog \
  -H "X-API-Key: $NYXID_API_KEY"
curl "http://localhost:3001/api/v1/catalog?include_all=true" \
  -H "X-API-Key: $NYXID_API_KEY"
curl http://localhost:3001/api/v1/catalog/llm-openai \
  -H "X-API-Key: $NYXID_API_KEY"
curl http://localhost:3001/api/v1/catalog/llm-openai/endpoints \
  -H "X-API-Key: $NYXID_API_KEY"
```

The catalog response includes rich metadata when available:
- `homepage_url`, `repository_url`, `issues_url` -- links to docs, source, and issue tracker
- `openapi_spec_url`, `asyncapi_spec_url` -- spec URLs for API discovery
- `capabilities` -- structured flags (e.g., `supports_proxy_read`, `supports_streaming`)
- `auth_notes` -- notes on downstream auth expectations
- `known_limitations` -- caveats for agents and users
- `required_permissions` -- downstream permissions needed for key actions

The `/endpoints` sub-resource parses the service's OpenAPI spec and returns structured endpoint data (method, path, name, description, parameters, request body schema).

### Manage existing services

```bash
# CLI
nyxid service list                              # List all your services
nyxid service show <slug>                       # Get details
nyxid service update <slug> --label "My Custom Name"  # Rename service
nyxid service update <slug> --endpoint-url "http://localhost:8080/openai"  # Update endpoint
nyxid service update <slug> --node-id "NODE_UUID"  # Route through a node
nyxid service remove <slug>                     # Delete a service

# API equivalents
# List all your services (combined view: endpoint + key + service)
curl http://localhost:3001/api/v1/keys \
  -H "X-API-Key: $NYXID_API_KEY"

# Get details for a specific service
curl http://localhost:3001/api/v1/keys/$KEY_ID \
  -H "X-API-Key: $NYXID_API_KEY"

# Update service (label, endpoint, routing, etc.) via unified endpoint
curl -X PUT http://localhost:3001/api/v1/keys/$KEY_ID \
  -H "X-API-Key: $NYXID_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"label": "My Custom Name", "endpoint_url": "http://localhost:8080/openai"}'

# Update the credential -- user sets env var first:
# ! export NEW_CREDENTIAL="sk-new-key-..."
curl -X PUT http://localhost:3001/api/v1/api-keys/external/$API_KEY_ID \
  -H "X-API-Key: $NYXID_API_KEY" \
  -H "Content-Type: application/json" \
  -d "{\"credential\": \"$NEW_CREDENTIAL\"}"

# Update service routing (e.g., add node routing)
curl -X PUT http://localhost:3001/api/v1/keys/$KEY_ID \
  -H "X-API-Key: $NYXID_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"node_id": "NODE_UUID"}'

# Delete a service (deactivates service + key)
curl -X DELETE http://localhost:3001/api/v1/keys/$KEY_ID \
  -H "X-API-Key: $NYXID_API_KEY"
```

### Deprecated routes (still functional)

The old `/api/v1/connections` and `/api/v1/providers/*/connect/*` routes still work during the migration period. New integrations should use `/api/v1/keys`.

---

## 8. Set Up MCP Proxy for AI Clients

**Goal:** Let Cursor, Claude Code, or Codex call APIs through NyxID's MCP proxy with automatic credential injection.

### Prerequisites

1. At least one service with endpoints registered in the catalog (see section 4, admin-only)
2. User has added the service via AI Services (see [section 7](#7-add-a-service-ai-services))

### Via CLI (auto-configures your AI tool)

```bash
nyxid mcp setup cursor     # Generates .cursor/mcp.json
nyxid mcp setup claude     # Generates .claude/settings.json MCP entry
nyxid mcp setup codex      # Generates ~/.codex/config.toml entry
```

### Cursor (manual)

Create or edit `.cursor/mcp.json` in your project root:

```json
{
  "mcpServers": {
    "nyxid": {
      "url": "http://localhost:3001/mcp"
    }
  }
}
```

Restart Cursor. Authenticate via browser when prompted.

### Claude Code

Run in terminal:

```bash
claude mcp add --transport http nyxid http://localhost:3001/mcp
```

Or edit `~/.claude/settings.json` or project-level `.claude/settings.json`:

```json
{
  "mcpServers": {
    "nyxid": {
      "type": "http",
      "url": "http://localhost:3001/mcp"
    }
  }
}
```

Restart Claude Code. Authenticate via browser when prompted.

### Codex

Edit `~/.codex/config.toml`:

```toml
[mcp_servers.nyxid]
url = "http://localhost:3001/mcp"
```

Restart Codex.

### How it works

1. MCP client connects and authenticates via OAuth in the browser
2. NyxID aggregates API endpoints from all connected services into MCP tools
3. Tools are named `{service-slug}__{endpoint-name}`
4. When a tool is called, NyxID injects the user's stored credential
5. Request is proxied to the downstream service, response returned to the MCP client

Built-in meta-tools let you search for tools and discover/connect to new services from within the MCP client.

---

## 9. Use the Credential Proxy

**Goal:** Proxy API requests through NyxID with automatic credential injection (without MCP).

### Proxy by slug (recommended)

```bash
# CLI
nyxid proxy request llm-openai v1/chat/completions \
  -m POST -d '{"model": "gpt-4", "messages": [{"role": "user", "content": "Hello"}]}'

# With streaming
nyxid proxy request llm-openai v1/chat/completions \
  -m POST --stream \
  -d '{"model": "gpt-4", "stream": true, "messages": [{"role": "user", "content": "Hello"}]}'

# API
curl http://localhost:3001/api/v1/proxy/s/openai/v1/chat/completions \
  -H "Authorization: Bearer $ACCESS_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"model": "gpt-4", "messages": [{"role": "user", "content": "Hello"}]}'
```

### Proxy by service ID

```bash
# CLI
nyxid proxy request $SERVICE_ID v1/chat/completions --by-id \
  -m POST -d '{"model": "gpt-4", "messages": [{"role": "user", "content": "Hello"}]}'

# API
curl http://localhost:3001/api/v1/proxy/$SERVICE_ID/v1/chat/completions \
  -H "Authorization: Bearer $ACCESS_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"model": "gpt-4", "messages": [{"role": "user", "content": "Hello"}]}'
```

NyxID automatically injects the user's stored credential (e.g., `Authorization: Bearer sk-proj-...`) into the outgoing request.

### Streaming and large files

The proxy streams video, audio, images, and large file responses directly to the client without buffering in memory. HTTP Range requests (byte-range seeking) are supported for services that return `Accept-Ranges` headers. Request body uploads up to 100 MB are supported on proxy routes (configurable via `PROXY_MAX_BODY_SIZE`). Use `--stream` in the CLI to consume streaming responses incrementally.

### List proxyable services

```bash
# CLI (recommended -- shows all user services including custom slugs)
nyxid service list --output json

# CLI (legacy catalog-only discovery)
nyxid proxy discover

# API (all user services)
curl http://localhost:3001/api/v1/keys \
  -H "Authorization: Bearer $ACCESS_TOKEN"

# API (legacy catalog-only)
curl http://localhost:3001/api/v1/proxy/services \
  -H "Authorization: Bearer $ACCESS_TOKEN"
```

Use `nyxid service list` for complete discovery. If a user has multiple keys for the same service (e.g., `llm-anthropic` and `llm-anthropic-2`), only `service list` / `GET /api/v1/keys` shows both.

### Identity Propagation (optional)

Services can be configured to forward the NyxID user's identity to the downstream API:

- `X-User-ID` -- NyxID user ID
- `X-User-Email` -- User's email
- `X-User-Name` -- User's display name
- `X-NyxID-Authenticated` -- Always `true`

---

## 10. Set Up a Provider (OAuth / API Key / Device Code)

> **Admin-only.** This section covers registering provider configurations in the catalog. Users connect to providers through the AI Services page (`POST /api/v1/keys` or the OAuth flow at `POST /api/v1/keys/oauth/authorize`). See [section 7](#7-add-a-service-ai-services).

**Goal:** Register an external provider configuration that enables OAuth/device-code/API-key flows when users add services from the catalog.

### OAuth 2.0 Provider (admin credentials)

Admin provides a shared OAuth app. Users authorize via OAuth when adding the service from AI Services.

```bash
curl -X POST http://localhost:3001/api/v1/providers \
  -H "Authorization: Bearer $ACCESS_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "GitHub",
    "slug": "github",
    "provider_type": "oauth2",
    "credential_mode": "admin",
    "authorization_url": "https://github.com/login/oauth/authorize",
    "token_url": "https://github.com/login/oauth/access_token",
    "default_scopes": ["repo", "read:user"],
    "client_id": "YOUR_GITHUB_CLIENT_ID",
    "client_secret": "YOUR_GITHUB_CLIENT_SECRET",
    "supports_pkce": false
  }'
```

Users connect via: `GET /api/v1/providers/{provider_id}/connect/oauth`

### OAuth 2.0 Provider (user credentials)

Each user provides their own developer OAuth app credentials (e.g., X/Twitter where each developer creates their own app, or Codex CLI auth).

```bash
curl -X POST http://localhost:3001/api/v1/providers \
  -H "Authorization: Bearer $ACCESS_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "X (Twitter)",
    "slug": "x-twitter",
    "provider_type": "oauth2",
    "credential_mode": "user",
    "authorization_url": "https://twitter.com/i/oauth2/authorize",
    "token_url": "https://api.twitter.com/2/oauth2/token",
    "default_scopes": ["tweet.read", "users.read"],
    "supports_pkce": true
  }'
```

Users first set their own OAuth credentials, then authorize:

```bash
# Step 1: User provides their own developer app client_id/client_secret
curl -X PUT http://localhost:3001/api/v1/providers/$PROVIDER_ID/credentials \
  -H "Authorization: Bearer $ACCESS_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "client_id": "users-own-client-id",
    "client_secret": "users-own-client-secret",
    "label": "My X Developer App"
  }'

# Step 2: User initiates OAuth flow (uses their own credentials)
# GET /api/v1/providers/{provider_id}/connect/oauth
# Returns: { "authorization_url": "https://twitter.com/i/oauth2/authorize?..." }
```

### OAuth 2.0 Provider (both modes)

Admin provides default credentials but users can override with their own:

```bash
curl -X POST http://localhost:3001/api/v1/providers \
  -H "Authorization: Bearer $ACCESS_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Codex",
    "slug": "codex",
    "provider_type": "oauth2",
    "credential_mode": "both",
    "authorization_url": "https://auth.openai.com/authorize",
    "token_url": "https://auth.openai.com/oauth/token",
    "default_scopes": ["openid", "profile"],
    "client_id": "default-client-id",
    "client_secret": "default-client-secret",
    "supports_pkce": true
  }'
```

Users who want the default just call `GET /api/v1/providers/{id}/connect/oauth`. Users who want their own credentials set them first via `PUT /api/v1/providers/{id}/credentials`.

### User credential management

**Via Dashboard (recommended):** Go to http://localhost:3000/keys (AI Services page) to manage credentials.

**Via CLI:** User sets their OAuth app credentials as env vars first (in Claude Code use `!` prefix):

```bash
# User runs these themselves:
# ! export OAUTH_CLIENT_ID="my-client-id"
# ! export OAUTH_CLIENT_SECRET="my-client-secret"

curl -X PUT http://localhost:3001/api/v1/providers/$PROVIDER_ID/credentials \
  -H "X-API-Key: $NYXID_API_KEY" \
  -H "Content-Type: application/json" \
  -d "{\"client_id\": \"$OAUTH_CLIENT_ID\", \"client_secret\": \"$OAUTH_CLIENT_SECRET\", \"label\": \"My App\"}"

# Get user's credentials for a provider
curl http://localhost:3001/api/v1/providers/$PROVIDER_ID/credentials \
  -H "X-API-Key: $NYXID_API_KEY"

# Delete user's credentials (fall back to admin credentials if mode is "both")
curl -X DELETE http://localhost:3001/api/v1/providers/$PROVIDER_ID/credentials \
  -H "X-API-Key: $NYXID_API_KEY"
```

### API Key Provider

```bash
curl -X POST http://localhost:3001/api/v1/providers \
  -H "Authorization: Bearer $ACCESS_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Anthropic",
    "slug": "anthropic",
    "provider_type": "api_key",
    "credential_mode": "user",
    "api_key_instructions": "Get your API key from https://console.anthropic.com/settings/keys",
    "api_key_url": "https://console.anthropic.com/settings/keys"
  }'
```

Users connect via:

```bash
POST /api/v1/providers/{provider_id}/connect/api-key
{"api_key": "sk-ant-api03-...", "label": "My Anthropic Key"}
```

### Self-Hosted Gateway Provider (e.g., OpenClaw)

OpenClaw is pre-seeded as a provider with `requires_gateway_url: true`. Users must provide their instance URL alongside the bearer token when connecting.

**Connect via API:**

```bash
# User sets their token as an env var first:
# ! export OPENCLAW_TOKEN="my-gateway-bearer-token"

curl -X POST http://localhost:3001/api/v1/providers/{openclaw_provider_id}/connect/api-key \
  -H "Authorization: Bearer $ACCESS_TOKEN" \
  -H "Content-Type: application/json" \
  -d "{
    \"api_key\": \"$OPENCLAW_TOKEN\",
    \"gateway_url\": \"http://localhost:18789\",
    \"label\": \"My OpenClaw\"
  }"
```

**Connect via node agent (recommended -- one command):**

```bash
nyxid node openclaw connect --url http://localhost:18789 --access-token $ACCESS_TOKEN
# Prompts for bearer token, stores locally, registers with NyxID, creates binding
```

**Proxy through OpenClaw after connecting:**

```bash
# Chat completions
curl http://localhost:3001/api/v1/proxy/s/llm-openclaw/v1/chat/completions \
  -H "Authorization: Bearer $ACCESS_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"model": "claude-3-5-sonnet", "messages": [{"role": "user", "content": "Hello"}]}'

# Invoke tools/skills
curl http://localhost:3001/api/v1/proxy/s/llm-openclaw/tools/invoke \
  -H "Authorization: Bearer $ACCESS_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"tool": "sessions_list", "action": "json", "args": {}}'
```

**Channel integration (map OpenClaw channels to NyxID users):**

```bash
# Create a mapping (returns a one-time webhook_secret)
curl -X POST http://localhost:3001/api/v1/integrations/openclaw/mappings \
  -H "Authorization: Bearer $ACCESS_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"channel": "whatsapp", "channel_user_id": "+1234567890"}'
# Returns: { "webhook_secret": "abc123...", ... }
# Configure this secret in your OpenClaw channel plugin
```

### Device Code Provider (for CLI tools)

```bash
curl -X POST http://localhost:3001/api/v1/providers \
  -H "Authorization: Bearer $ACCESS_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "OpenAI Codex",
    "slug": "openai-codex",
    "provider_type": "device_code",
    "device_code_url": "https://auth.openai.com/api/accounts/deviceauth/usercode",
    "device_token_url": "https://auth.openai.com/api/accounts/deviceauth/token",
    "authorization_url": "https://auth.openai.com/authorize",
    "token_url": "https://auth.openai.com/oauth/token"
  }'
```

Users connect via:

```bash
# Initiate
POST /api/v1/providers/{provider_id}/connect/device-code/initiate
# Returns: { user_code, verification_uri, state, expires_in, interval }

# Poll (repeat until status != "pending")
POST /api/v1/providers/{provider_id}/connect/device-code/poll
{"state": "STATE_FROM_INITIATE"}
# Returns: { status: "pending" | "success" | "expired" | "denied" }
```

### Credential Modes

- `admin` -- Admin provides client credentials. Users just authorize via OAuth.
- `user` -- Users bring their own client_id/client_secret.
- `both` -- Admin provides defaults; users can override with their own.

### Via Dashboard

Admin provider management: http://localhost:3000/providers/manage (admin section).
Users add services and manage credentials from the AI Services page: http://localhost:3000/keys.

---

## 11. Deploy a Node Agent (On-Premise Credentials)

**Goal:** Keep sensitive credentials on your own infrastructure. NyxID routes proxy requests through the node agent, which injects credentials locally -- they never leave your machine.

### How it works

1. The node agent runs on your infrastructure and connects to NyxID via WebSocket
2. You store API credentials locally on the node (encrypted at rest)
3. When a user makes a proxy request for a node-bound service, NyxID routes it through the node
4. The node injects the credential and forwards the request to the downstream API
5. The credential never transits the NyxID server

### Step 1: Install the node agent

The node agent is built into the `nyxid` CLI. Build from source:

```bash
# Requires Rust toolchain (https://rustup.rs)
# Clone the NyxID repo, then:
cargo build --release -p nyxid-cli

# Binary is at: target/release/nyxid
# Copy it to the target machine or add to PATH:
cp target/release/nyxid /usr/local/bin/
```

Or install directly:

```bash
cargo install --path cli
```

Verify:

```bash
nyxid node version
```

### Step 2: Generate a registration token

Via dashboard: http://localhost:3000/nodes, via CLI, or via API:

```bash
# CLI
nyxid node register-token

# API equivalent
curl -X POST http://localhost:3001/api/v1/nodes/register-token \
  -H "Authorization: Bearer $ACCESS_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"name": "Production Node"}'
```

Returns: `{ "token": "nyx_nreg_...", "token_id": "...", "expires_at": "..." }`

Registration tokens expire after 1 hour by default.

### Step 3: Register the node

On the target machine where the agent will run:

```bash
# Recommended: use OS keychain for secure credential storage
nyxid node register \
  --token "nyx_nreg_..." \
  --url "wss://localhost:3001/api/v1/nodes/ws" \
  --keychain

# Or file-based encryption (creates .keyfile)
nyxid node register \
  --token "nyx_nreg_..." \
  --url "wss://localhost:3001/api/v1/nodes/ws"
```

**Options:**
- `--keychain` (recommended) -- Use OS keychain (macOS Keychain, Windows Credential Manager, Linux Secret Service) for credential storage
- `--config <PATH>` -- Custom config directory (default: `~/.nyxid-node`)

### Step 4: Add credentials

Credentials can be added before or after starting the agent. The agent watches the config file and reloads credentials automatically within 5 seconds (no restart needed). You can also update or remove credentials while the agent is running.

Add credentials for each service the node will handle:

```bash
# Recommended for catalog services: auto-setup
# Fetches config from catalog, prompts accordingly, and auto-registers in backend.
nyxid node credentials setup --service llm-openai
# Auto-detects: API key service → prompts for key, shows where to get it
# Auto-detects: OAuth service → runs device code flow from the node
# Auto-detects: gateway URL required → prompts for instance URL first
# Auto-registers: creates the backend UserService with node_id (no separate step needed)

# Custom endpoints: register in backend first, then add credentials locally
nyxid service add --custom --via-node <node-name-or-id>   # creates backend record (prompts for URL, auth, etc.)
nyxid node credentials add --service "my-api" --header "Authorization" --secret-format Bearer

# Manual for catalog services (requires separate backend registration, see Step 5)
nyxid node credentials add --service "llm-openai" --header "Authorization" --secret-format Bearer

# Manual: header injection (requires separate backend registration, see Step 5)
nyxid node credentials add --service "llm-google-ai" --header "x-goog-api-key"

# OAuth: run device code flow from the node
nyxid node credentials add-oauth --service "api-twitter" --from-catalog
```

> **Important:** `credentials setup` works for **catalog services only** -- it auto-registers the service in the backend with the node's ID. For **custom endpoints**, use `nyxid service add --custom --via-node <node>` first to create the backend record, then `nyxid node credentials add` to store the credential locally. If using `credentials add` for a catalog service, register it manually first:
> ```bash
> nyxid service add <slug> --via-node <node-name-or-id>
> ```

**Manage credentials:**

```bash
nyxid node credentials list                      # List all configured credentials
nyxid node credentials remove --service "openai"  # Remove a credential
```

### Step 5: Route services through the node (manual)

> **Skip this step for catalog services if you used `credentials setup` in Step 4** -- it auto-registers with node routing. This step is required for **custom endpoints** (handled in Step 4 via `service add --custom --via-node`) and for changing routing on existing services.

For services added via `credentials add` or when changing routing on an existing service:

**Via CLI (preferred):**

```bash
# Add a service routed through a node (creates backend record + sets node routing)
nyxid service add <slug> --via-node <node-name-or-id>

# Change routing on an existing service
nyxid service route $SERVICE_ID --node $NODE_ID

# Switch back to direct routing
nyxid service route $SERVICE_ID --direct
```

**Via Dashboard:**

1. Go to http://localhost:3000/keys (AI Services page)
2. Click on a service card to expand it
3. Under "Routing", click "Route via Node"
4. Select your node from the picker

**Via API:**

```bash
# Update a user service to route through a node
curl -X PUT http://localhost:3001/api/v1/user-services/$SERVICE_ID \
  -H "X-API-Key: $NYXID_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"node_id": "NODE_UUID"}'
```

You can also set node routing when adding a service:

```bash
curl -X POST http://localhost:3001/api/v1/keys \
  -H "X-API-Key: $NYXID_API_KEY" \
  -H "Content-Type: application/json" \
  -d "{
    \"service_slug\": \"llm-openai\",
    \"node_id\": \"NODE_UUID\",
    \"label\": \"OpenAI via Node\"
  }"
```

Services without a `node_id` use NyxID's direct proxy. The old `POST /api/v1/nodes/{id}/bindings` route still works during migration.

### Step 5b: Add credentials with target URL (optional)

The node agent can store a custom target URL alongside the credential, useful when the node should forward requests to a local endpoint:

```bash
nyxid node credentials add --service "my-api" --header "Authorization" --target-url "http://localhost:8080"
# Prompts for the secret value
```

### Step 5c: Add OAuth credentials locally (optional)

For OAuth-based services, the node agent can run a local OAuth flow and store the resulting tokens:

```bash
nyxid node credentials add-oauth --service "github" --provider-slug "github"
# Opens browser for OAuth authorization, stores tokens locally
```

### Step 6: Install and start as a background service

**Prerequisite:** The node must be registered (steps 2-3) before installing the daemon. Credentials (step 4) can be added before or after starting -- the agent picks them up automatically.

The recommended way to run the node agent in production is as a background service. The CLI handles platform-specific service installation automatically (launchd on macOS, systemd on Linux):

```bash
# Install as a background service (auto-starts on login, restarts on crash)
nyxid node daemon install

# Start the service
nyxid node daemon start
```

The agent connects via WebSocket and automatically reconnects with exponential backoff (100ms to 60s) if the connection drops.

**Managing the service:**

```bash
nyxid node daemon status                               # check if installed and running
nyxid node daemon restart                              # restart (picks up config changes)
nyxid node daemon stop                                 # stop the service
nyxid node daemon logs                                 # show recent logs
nyxid node daemon logs --follow                        # tail logs in real time
nyxid node daemon uninstall                            # remove background service
```

**Service files:**
- macOS: `~/Library/LaunchAgents/dev.nyxid.node.plist`
- Linux: `~/.config/systemd/user/nyxid-node.service`

On Linux, the installer enables user lingering (`loginctl enable-linger`) so the service keeps running after logout.

**Foreground mode** (for debugging):

```bash
nyxid node start                                       # runs in foreground, Ctrl+C to stop
nyxid node start --log-level debug                     # verbose logging
```

### Configuration file reference

The config is stored at `~/.nyxid-node/config.toml` (created during registration):

```toml
storage_backend = "file"  # "file" (AES-256-GCM) or "keychain" (OS keychain)

[server]
url = "wss://localhost:3001/api/v1/nodes/ws"

[node]
id = "node-uuid-here"
auth_token_encrypted = "..."  # Encrypted auth token (file backend only)

[signing]
shared_secret_encrypted = "..."  # HMAC signing secret (file backend only)

[ssh]
max_tunnels = 10           # Max concurrent SSH tunnels (default: 10)
io_timeout_secs = 3600     # Idle timeout per tunnel in seconds (default: 3600)
# Restrict which SSH hosts the node can connect to:
# allowed_targets = [
#   { host = "internal.example.com", port = 22 },
#   { host = "db.example.com", port = 22 },
# ]

# Credentials are stored per-service:
[credentials.openai]
injection_method = "header"
header_name = "Authorization"
header_value_encrypted = "..."  # AES-256-GCM encrypted (file backend)

[credentials.weather-api]
injection_method = "query_param"
param_name = "api_key"
param_value_encrypted = "..."
```

### Secret storage backends

**File backend (default):**
- Secrets encrypted with AES-256-GCM
- Encryption key stored at `~/.nyxid-node/.keyfile` (mode `0600`)
- Works on all platforms, no daemon required

**Keychain backend:**
- macOS: Keychain
- Windows: Credential Manager
- Linux: Secret Service D-Bus (GNOME Keyring / KDE Wallet)
- Register with `--keychain` flag, or migrate later

**Migrate between backends:**

```bash
nyxid node migrate --to keychain   # File -> OS keychain
nyxid node migrate --to file       # OS keychain -> file
```

### All CLI commands

```bash
# Registration and foreground operation
nyxid node register       --token <TOKEN> [--url <WS_URL>] [--config <PATH>] [--keychain]
nyxid node start          [--config <PATH>] [--log-level <LEVEL>]   # foreground mode
nyxid node agent-status   [--config <PATH>]                         # read-only local config check
nyxid node rekey          --auth-token <TOKEN> --signing-secret <HEX> [--config <PATH>]

# Background service lifecycle (launchd on macOS, systemd on Linux)
nyxid node daemon install    [--config <PATH>] [--log-level <LEVEL>] [--force]
nyxid node daemon uninstall  [--config <PATH>]
nyxid node daemon start      [--config <PATH>]
nyxid node daemon stop       [--config <PATH>]
nyxid node daemon restart    [--config <PATH>]
nyxid node daemon status     [--config <PATH>]
nyxid node daemon logs       [--config <PATH>] [--follow] [--lines <N>]

# Credential management (setup auto-registers catalog services in the backend)
nyxid node credentials setup     --service <SLUG> [--api-url <URL>]  # Catalog services: auto-detect, setup, and register (recommended)
nyxid node credentials add       --service <SLUG> [--header <NAME> | --query-param <NAME>] [--secret-format Raw|Bearer|Basic] [--target-url <URL>]  # Local only; for custom endpoints run `nyxid service add --custom --via-node <NODE>` first
nyxid node credentials add-oauth --service <SLUG> --from-catalog [--api-url <URL>]  # OAuth flow from node
nyxid node credentials list      [--config <PATH>]
nyxid node credentials remove    --service <SLUG> [--config <PATH>]

# OpenClaw integration
nyxid node openclaw connect      --url <GATEWAY_URL> [--token <TOKEN>] [--access-token <JWT>] [--api-url <URL>]
nyxid node openclaw status       [--config <PATH>]
nyxid node openclaw disconnect   [--config <PATH>]

# Secret storage migration
nyxid node migrate        --to <file|keychain> [--config <PATH>]
nyxid node agent-version
```

The node agent also supports live credential updates via WebSocket `credential_update` messages from the server, enabling remote credential rotation without restarting the agent.

Global option: `--log-level <trace|debug|info|warn|error>` (default: `info`)

### Manage nodes from the nyxid CLI

```bash
nyxid node list                        # List your nodes
nyxid node show $NODE_ID               # Node details (status, metrics, services)
nyxid node delete $NODE_ID             # Delete a node (--yes to skip confirmation)
nyxid node rotate-token $NODE_ID       # Rotate node auth token
```

---

## 12. Add Login to a React App (OAuth Client)

**Goal:** Add "Sign in with NyxID" to a React app using the official SDK.

### Step 1: Register an OAuth client

Via dashboard at http://localhost:3000/developer/apps, or via API:

```bash
curl -X POST http://localhost:3001/api/v1/developer/oauth-clients \
  -H "Authorization: Bearer $ACCESS_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "My React App",
    "redirect_uris": ["https://myapp.example.com/auth/callback"],
    "client_type": "public",
    "allowed_scopes": ["openid", "profile", "email"]
  }'
```

**Save the returned `id`** -- this is your `client_id`.

### Step 2: Install the SDK

```bash
npm install @nyxids/oauth-core @nyxids/oauth-react
```

### Step 3: Configure the provider

```tsx
// src/main.tsx
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { NyxIDProvider, createNyxClient } from "@nyxids/oauth-react";
import App from "./App";

const nyxClient = createNyxClient({
  baseUrl: "http://localhost:3001",
  clientId: "YOUR_CLIENT_ID",
  redirectUri: `${window.location.origin}/auth/callback`,
  scope: "openid profile email",
});

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <NyxIDProvider client={nyxClient}>
      <App />
    </NyxIDProvider>
  </StrictMode>,
);
```

### Step 4: Add login button and callback handler

```tsx
// src/App.tsx
import { useEffect, useState } from "react";
import { useNyxID } from "@nyxids/oauth-react";
import type { OAuthUserInfo } from "@nyxids/oauth-core";

export default function App() {
  const {
    isAuthenticated,
    tokens,
    loginWithRedirect,
    handleRedirectCallback,
    getUserInfo,
    clearSession,
  } = useNyxID();

  const [user, setUser] = useState<OAuthUserInfo | null>(null);

  useEffect(() => {
    if (window.location.pathname === "/auth/callback") {
      handleRedirectCallback()
        .then(() => window.history.replaceState({}, "", "/"))
        .catch(() => { /* handle error */ });
    }
  }, [handleRedirectCallback]);

  useEffect(() => {
    if (isAuthenticated && tokens?.accessToken) {
      getUserInfo(tokens.accessToken)
        .then(setUser)
        .catch(() => { /* handle error */ });
    }
  }, [isAuthenticated, tokens, getUserInfo]);

  if (!isAuthenticated) {
    return <button onClick={() => loginWithRedirect()}>Sign in with NyxID</button>;
  }

  return (
    <div>
      <h1>Welcome, {user?.name ?? user?.email ?? "User"}</h1>
      <button onClick={clearSession}>Sign out</button>
    </div>
  );
}
```

### SDK API Reference

```typescript
interface NyxIDClientConfig {
  baseUrl: string;           // NyxID API URL (e.g., "http://localhost:3001")
  clientId: string;          // OAuth client ID from step 1
  redirectUri: string;       // Must match a registered redirect_uri
  scope?: string;            // Default: "openid profile email"
  storage?: StorageLike;     // Default: localStorage
  fetchFn?: typeof fetch;    // Custom fetch (for testing/SSR)
}

// useNyxID() hook returns:
interface NyxIDContextValue {
  client: NyxIDClient;
  tokens: NyxIDTokenSet | null;
  isAuthenticated: boolean;
  loginWithRedirect(options?: LoginRedirectOptions): Promise<void>;
  handleRedirectCallback(url?: string): Promise<NyxIDTokenSet>;
  clearSession(): void;
  getUserInfo(accessToken?: string): Promise<OAuthUserInfo>;
}

interface NyxIDTokenSet {
  accessToken: string;       // Bearer token (15 min TTL)
  tokenType: string;         // "Bearer"
  expiresIn: number;         // Seconds until expiration
  refreshToken?: string;
  idToken?: string;
  scope?: string;
}

interface OAuthUserInfo {
  sub: string;
  email?: string;
  email_verified?: boolean;
  name?: string;
  picture?: string;
  roles?: string[];
  groups?: string[];
  permissions?: string[];
}
```

---

## 13. Add Login to Any Web App (Raw OAuth / OIDC)

**Goal:** Integrate NyxID login without the SDK. Works with any language/framework.

### Using OIDC Discovery (recommended)

Point your OIDC library at:

```
http://localhost:3001/.well-known/openid-configuration
```

#### NextAuth.js

```typescript
import NextAuth from "next-auth";

export const { handlers, auth } = NextAuth({
  providers: [{
    id: "nyxid",
    name: "NyxID",
    type: "oidc",
    issuer: "http://localhost:3001",
    clientId: "YOUR_CLIENT_ID",
    clientSecret: "YOUR_CLIENT_SECRET",
  }],
});
```

#### Passport.js

```javascript
const { Issuer, Strategy } = require("openid-client");
const issuer = await Issuer.discover("http://localhost:3001");
const client = new issuer.Client({
  client_id: "YOUR_CLIENT_ID",
  redirect_uris: ["https://myapp.example.com/callback"],
  response_types: ["code"],
});
passport.use("nyxid", new Strategy({ client }, (tokenSet, userinfo, done) => {
  done(null, userinfo);
}));
```

### Manual OAuth 2.0 + PKCE

#### Step 1: Generate PKCE challenge

```python
import hashlib, base64, os

code_verifier = base64.urlsafe_b64encode(os.urandom(48)).rstrip(b"=").decode()
code_challenge = base64.urlsafe_b64encode(
    hashlib.sha256(code_verifier.encode()).digest()
).rstrip(b"=").decode()
state = base64.urlsafe_b64encode(os.urandom(24)).rstrip(b"=").decode()
```

#### Step 2: Redirect user to authorize

```
GET http://localhost:3001/oauth/authorize?
  response_type=code&
  client_id=YOUR_CLIENT_ID&
  redirect_uri=https://myapp.example.com/callback&
  scope=openid profile email&
  code_challenge=<code_challenge>&
  code_challenge_method=S256&
  state=<state>
```

#### Step 3: Exchange code for tokens

```bash
curl -X POST http://localhost:3001/oauth/token \
  -H "Content-Type: application/x-www-form-urlencoded" \
  -d "grant_type=authorization_code" \
  -d "code=AUTHORIZATION_CODE" \
  -d "redirect_uri=https://myapp.example.com/callback" \
  -d "client_id=YOUR_CLIENT_ID" \
  -d "code_verifier=CODE_VERIFIER"
```

#### Step 4: Get user info

```bash
curl http://localhost:3001/oauth/userinfo \
  -H "Authorization: Bearer ACCESS_TOKEN"
```

#### Step 5: Refresh tokens

```bash
curl -X POST http://localhost:3001/oauth/token \
  -H "Content-Type: application/x-www-form-urlencoded" \
  -d "grant_type=refresh_token" \
  -d "refresh_token=REFRESH_TOKEN" \
  -d "client_id=YOUR_CLIENT_ID"
```

#### Step 6: Verify JWTs (optional)

```bash
curl http://localhost:3001/.well-known/jwks.json
```

Use any JWT library to verify tokens with the RS256 algorithm and these keys.

---

## 14. Server-to-Server Authentication (Service Accounts)

**Goal:** Authenticate a backend service (no browser) using client credentials.

### Step 1: Create a service account (admin only)

```bash
curl -X POST http://localhost:3001/api/v1/admin/service-accounts \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"name": "My Backend Service", "description": "Automated data pipeline"}'
```

Response includes a one-time `secret` -- **save it immediately**.

### Step 2: Get an access token

```bash
curl -X POST http://localhost:3001/oauth/token \
  -H "Content-Type: application/x-www-form-urlencoded" \
  -d "grant_type=client_credentials" \
  -d "client_id=SERVICE_ACCOUNT_ID" \
  -d "client_secret=SERVICE_ACCOUNT_SECRET"
```

### Step 3: Use the token

```bash
curl http://localhost:3001/api/v1/some-endpoint \
  -H "Authorization: Bearer ACCESS_TOKEN"
```

Token TTL defaults to 1 hour.

---

## 15. Approval Workflow

**Goal:** Require approval before users can access sensitive services. Approvals can be delivered via Telegram or mobile push notifications.

### Approval modes

NyxID supports two approval modes:

| Mode | Behavior | Default? |
|------|----------|----------|
| `per_request` | Every proxy call requires fresh approval. No grants are created. | Yes (default) |
| `grant` | Approval creates a time-based grant. Subsequent requests within the grant period pass automatically. | No (opt-in) |

When a service has approval enabled, the default mode is **per-request**: every proxy call triggers a new approval notification with a human-readable `action_description` (e.g., "POST /v1/chat/completions (model: gpt-4, 3 messages)"). The approver sees exactly what the request will do before approving.

To use grant-based approvals instead (legacy behavior), set `approval_mode` to `"grant"`.

### Configure a service to require approval

```bash
# CLI -- per-request mode (default)
nyxid approval set-config $SERVICE_ID --require-approval true

# CLI -- grant mode (approval creates a time-based grant)
nyxid approval set-config $SERVICE_ID --require-approval true --approval-mode grant

# API -- per-request mode (default)
curl -X PUT http://localhost:3001/api/v1/approvals/service-configs/$SERVICE_ID \
  -H "Authorization: Bearer $ACCESS_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"approval_required": true}'

# API -- grant mode
curl -X PUT http://localhost:3001/api/v1/approvals/service-configs/$SERVICE_ID \
  -H "Authorization: Bearer $ACCESS_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"approval_required": true, "approval_mode": "grant"}'
```

When a user tries to proxy a request to this service, they get a 403 with `error_code: 7000`, a `request_id`, and an `action_description` describing the request. If the approval is rejected, expires, or times out, the error changes to `error_code: 7001` (`approval_failed`) and includes an `approve_url` linking to the web UI where the user can review pending approvals.

### View and manage approval requests

```bash
# CLI
nyxid approval list                           # List all approval requests (includes action_description)
nyxid approval show $REQUEST_ID               # Show request details (includes status + action_description)

# API
curl http://localhost:3001/api/v1/approvals/requests/$REQUEST_ID/status \
  -H "Authorization: Bearer $ACCESS_TOKEN"
# Returns: { "status": "pending" | "approved" | "denied", "expires_at": "...", "action_description": "..." }
```

### Approve or deny (admin / approver)

```bash
# CLI
nyxid approval approve $REQUEST_ID
nyxid approval deny $REQUEST_ID --reason "Not authorized for production data"

# API
curl -X POST http://localhost:3001/api/v1/approvals/requests/$REQUEST_ID/decide \
  -H "Authorization: Bearer $ACCESS_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"approved": true}'
```

### View approval grants

Grants are only created in `grant` mode. Services using `per_request` mode do not create grants.

```bash
# CLI
nyxid approval grants                         # List active grants
nyxid approval revoke-grant $GRANT_ID         # Revoke a grant

# API
GET /api/v1/approvals/grants
DELETE /api/v1/approvals/grants/{grant_id}
```

### View per-service approval configs

```bash
# CLI
nyxid approval service-configs                # List all per-service configs (includes approval_mode)
```

### Set up Telegram notifications

```bash
# CLI
nyxid notification telegram-link              # Link Telegram account
nyxid notification update --approval-telegram true
nyxid notification telegram-disconnect        # Disconnect Telegram

# API
POST /api/v1/notifications/telegram/link
PUT /api/v1/notifications/settings {"approval_requests": true, "approval_grants": true}
DELETE /api/v1/notifications/telegram
```

### Configure notification preferences

```bash
# CLI
nyxid notification settings                   # Show current settings
nyxid notification update \
  --approval-email true \
  --approval-push true \
  --approval-telegram true
```

### Via Dashboard

- Approval history: http://localhost:3000/approvals/history
- Active grants: http://localhost:3000/approvals/grants (grant mode only)
- Notification settings: http://localhost:3000/approvals/settings

---

## 16. SSH Services

**Goal:** Register an SSH service for certificate-based authentication, remote command execution, or interactive terminal sessions.

### Register an SSH service

```bash
# CLI
nyxid service add-ssh \
  --label "Production Server" \
  --host 10.0.0.5 \
  --port 22 \
  --cert-auth \
  --principals "ubuntu,deploy" \
  --ttl 30 \
  --via-node $NODE_ID

# API (admin -- registers in the catalog)
curl -X POST http://localhost:3001/api/v1/services \
  -H "Authorization: Bearer $ACCESS_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Production Server",
    "slug": "prod-server",
    "base_url": "ssh://10.0.0.5",
    "service_type": "ssh",
    "visibility": "private",
    "ssh_config": {
      "host": "10.0.0.5",
      "port": 22,
      "certificate_auth_enabled": true,
      "certificate_ttl_minutes": 30,
      "allowed_principals": ["ubuntu", "deploy"]
    }
  }'
```

SSH services allow private IPs and localhost (they're admin-configured infrastructure, not user-supplied URLs).

### Issue an SSH certificate

```bash
# CLI (accepts service ID, slug, or name)
nyxid ssh issue-cert <SERVICE_ID_OR_SLUG> \
  --public-key-file ~/.ssh/id_ed25519.pub \
  --principal ubuntu \
  --certificate-file ~/.ssh/id_ed25519-cert.pub

# API
curl -X POST http://localhost:3001/api/v1/ssh/$SERVICE_ID/certificate \
  -H "Authorization: Bearer $ACCESS_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"public_key": "ssh-ed25519 AAAA..."}'
# Returns: { "certificate": "ssh-ed25519-cert-v01@openssh.com ...", "validity_period": "30m" }
```

### Execute a remote command

```bash
# CLI (accepts service ID, slug, or name -- auto-resolves to DownstreamService ID)
nyxid ssh exec <SERVICE_ID_OR_SLUG> --principal ubuntu -- uptime
nyxid ssh exec kw-office-spare-mac --principal ubuntu -- ls -la /var/log

# API
curl -X POST http://localhost:3001/api/v1/ssh/$SERVICE_ID/exec \
  -H "Authorization: Bearer $ACCESS_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"command": "uptime"}'
# Returns: { "stdout": "...", "stderr": "...", "exit_code": 0 }
```

### Interactive terminal

```bash
# CLI (auto-resolves principal from service config if --principal omitted)
nyxid ssh terminal <SERVICE_ID_OR_SLUG>
nyxid ssh terminal kw-office-spare-mac
nyxid ssh terminal kw-office-spare-mac --principal ubuntu

# API (WebSocket upgrade)
GET /api/v1/ssh/{service_id}/terminal?principal=ubuntu
```

### SSH tunnel (ProxyCommand)

```bash
# CLI -- use as OpenSSH ProxyCommand (accepts ID, slug, or name)
nyxid ssh proxy <SERVICE_ID_OR_SLUG>

# With auto certificate issuance
nyxid ssh proxy kw-office-spare-mac \
  --issue-certificate \
  --public-key-file ~/.ssh/id_ed25519.pub \
  --principal ubuntu \
  --certificate-file ~/.ssh/id_ed25519-cert.pub

# In ~/.ssh/config:
# Host kw-office
#   ProxyCommand nyxid ssh proxy kw-office-spare-mac
#   User chronoai
#   CertificateFile ~/.ssh/id_ed25519-cert.pub

# API (WebSocket upgrade, bidirectional SSH protocol)
GET /api/v1/ssh/{service_id}
```

### Generate OpenSSH config

```bash
# CLI -- prints a config stanza you can append to ~/.ssh/config
nyxid ssh config \
  --host-alias prod-server \
  --base-url http://localhost:3001 \
  --service-id $SERVICE_ID \
  --principal ubuntu \
  --identity-file ~/.ssh/id_ed25519 \
  --certificate-file ~/.ssh/id_ed25519-cert.pub
```

---

## 17. API Keys (Programmatic Access)

**Goal:** Create NyxID API keys for CLI or programmatic access without going through the OAuth flow. Managed from the AI Services page under the "NyxID API Keys" tab at http://localhost:3000/keys.

```bash
# CLI (list shows ID, scopes, service bindings, node scope)
nyxid api-key create --name "CI Pipeline Key" --scopes "proxy read"
nyxid api-key create --name "relay-agent" --callback-url "https://..."  # for channel bot relay
nyxid api-key list                                     # Shows: ID, Name, Scopes, Services, Nodes
nyxid api-key show <ID>                                # Full details with bindings info
nyxid api-key rotate <ID>                              # Rotate
nyxid api-key delete <ID>                              # Delete

# Service bindings (credential auto-resolved from service)
nyxid api-key bind <ID> --service <SLUG>
nyxid api-key bind <ID> --service <SLUG> --credential <LABEL>  # explicit override

# By default, agents access all services with default credentials.
# To restrict to only bound services:
nyxid api-key update <ID> --allow-all-services false

# Callback URL for channel bot relay
nyxid api-key update <ID> --callback-url "https://my-agent.example.com/webhook"

# Node scope (restrict which nodes the agent can route through)
nyxid api-key update <ID> --allowed-nodes "node-id" --allow-all-nodes false

# API equivalents
curl -X POST http://localhost:3001/api/v1/api-keys \
  -H "Authorization: Bearer $ACCESS_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"name": "CI Pipeline Key"}'
# Returns the key value (one-time display)

GET /api/v1/api-keys               # List API keys
POST /api/v1/api-keys/{key_id}/rotate   # Rotate
DELETE /api/v1/api-keys/{key_id}        # Delete
```

Use API keys as Bearer tokens: `Authorization: Bearer nyxid_key_...`

### Scope fields

API keys can be scoped to limit which services and nodes they can access:

```bash
curl -X POST http://localhost:3001/api/v1/api-keys \
  -H "Authorization: Bearer $ACCESS_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Limited Agent Key",
    "scopes": "read write",
    "allowed_service_ids": ["service-uuid-1", "service-uuid-2"],
    "allowed_node_ids": ["node-uuid-1"],
    "allow_all_services": false,
    "allow_all_nodes": false
  }'
```

| Field | Default | Description |
|-------|---------|-------------|
| `allowed_service_ids` | `[]` | List of service UUIDs this key can proxy through. Empty + `allow_all_services: false` = no proxy access. |
| `allowed_node_ids` | `[]` | List of node UUIDs this key can route through. |
| `allow_all_services` | `true` | If true, key can access all services (ignores `allowed_service_ids`). |
| `allow_all_nodes` | `true` | If true, key can route through all nodes (ignores `allowed_node_ids`). |

Scope is enforced at proxy time. A key with `allow_all_services: false` and an empty `allowed_service_ids` list has no proxy access (useful for management-only keys).

---

## 18. LLM Gateway

**Goal:** Proxy LLM API calls through NyxID with automatic credential injection. Provides an OpenAI-compatible API interface.

### Check available LLM providers

```bash
curl http://localhost:3001/api/v1/llm/status \
  -H "Authorization: Bearer $ACCESS_TOKEN"
```

### Make LLM requests (OpenAI-compatible)

```bash
# Route through the unified gateway
curl http://localhost:3001/api/v1/llm/gateway/v1/chat/completions \
  -H "Authorization: Bearer $ACCESS_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"model": "gpt-4", "messages": [{"role": "user", "content": "Hello"}]}'

# Or route to a specific provider by slug
curl http://localhost:3001/api/v1/openai/v1/chat/completions \
  -H "Authorization: Bearer $ACCESS_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"model": "gpt-4", "messages": [{"role": "user", "content": "Hello"}]}'
```

NyxID injects the user's stored provider token automatically.

### Route through OpenClaw

If you've connected an OpenClaw gateway (see section 10), you can route LLM requests through it:

```bash
# OpenClaw chat completions (uses your gateway_url automatically)
curl http://localhost:3001/api/v1/proxy/s/llm-openclaw/v1/chat/completions \
  -H "Authorization: Bearer $ACCESS_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"model": "openclaw:main", "messages": [{"role": "user", "content": "Hello"}]}'

# OpenClaw tools/skills
curl http://localhost:3001/api/v1/proxy/s/llm-openclaw/tools/invoke \
  -H "Authorization: Bearer $ACCESS_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"tool": "sessions_list", "action": "json", "args": {}}'

# OpenClaw OpenResponses API
curl http://localhost:3001/api/v1/proxy/s/llm-openclaw/v1/responses \
  -H "Authorization: Bearer $ACCESS_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"input": "Analyze this", "stream": true}'
```

Each user's requests route to their own OpenClaw instance (per-user gateway URL). If a node binding exists, requests are proxied through the node agent with local credential injection.

---

## 19. API Quick Reference

Base URL: `http://localhost:3001`

### Authentication

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | `/api/v1/auth/register` | Register new user |
| POST | `/api/v1/auth/login` | Login (returns access_token + refresh_token) |
| POST | `/api/v1/auth/logout` | Logout |
| POST | `/api/v1/auth/refresh` | Refresh access token |
| POST | `/api/v1/auth/forgot-password` | Request password reset email |
| POST | `/api/v1/auth/reset-password` | Complete password reset |

### User

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/v1/users/me` | Get current user profile |
| PUT | `/api/v1/users/me` | Update profile |
| GET | `/api/v1/sessions` | List active sessions |
| GET | `/api/v1/api-keys` | List API keys |
| POST | `/api/v1/api-keys` | Create API key |

### AI Services (Unified Key Management) -- NEW

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | `/api/v1/keys` | Add service from catalog or custom (auto-provisions endpoint + key + service) |
| GET | `/api/v1/keys` | List all services (combined view) |
| GET | `/api/v1/keys/{id}` | Get combined view |
| DELETE | `/api/v1/keys/{id}` | Revoke (deactivates service + key) |
| POST | `/api/v1/keys/oauth/authorize` | Start OAuth flow for a provider |
| GET | `/api/v1/keys/oauth/callback` | OAuth callback |
| POST | `/api/v1/keys/{id}/refresh` | Force token refresh |

### Catalog (Read-Only) -- NEW

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/v1/catalog` | List available service templates |
| GET | `/api/v1/catalog/{slug}` | Get template details |

### User Endpoints -- NEW

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/v1/endpoints` | List user's endpoints |
| PUT | `/api/v1/endpoints/{id}` | Update endpoint URL |
| DELETE | `/api/v1/endpoints/{id}` | Delete endpoint |

### User External API Keys -- NEW

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/v1/api-keys/external` | List user's external credentials |
| PUT | `/api/v1/api-keys/external/{id}` | Update label, rotate credential |
| DELETE | `/api/v1/api-keys/external/{id}` | Revoke key |

### User Services (Proxy Routing) -- NEW

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/v1/user-services` | List user's service bindings |
| PUT | `/api/v1/user-services/{id}` | Update auth config, node routing |
| DELETE | `/api/v1/user-services/{id}` | Deactivate service |

### Services (Admin -- Catalog Management)

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/v1/services` | List services (admin) |
| POST | `/api/v1/services` | Create service (admin) |
| GET | `/api/v1/services/{id}` | Get service details |
| PUT | `/api/v1/services/{id}` | Update service |
| DELETE | `/api/v1/services/{id}` | Delete service |
| POST | `/api/v1/services/{id}/endpoints` | Add API endpoint |
| POST | `/api/v1/services/{id}/discover-endpoints` | Auto-discover from OpenAPI |
| GET | `/api/v1/services/{id}/oidc-credentials` | Get OIDC client credentials |
| PUT | `/api/v1/services/{id}/redirect-uris` | Update OIDC redirect URIs |
| POST | `/api/v1/services/{id}/regenerate-secret` | Regenerate OIDC client secret |

### Connections (DEPRECATED -- use `/api/v1/keys`)

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/v1/connections` | List user's connections |
| POST | `/api/v1/connections/{service_id}` | Connect to service |
| DELETE | `/api/v1/connections/{service_id}` | Disconnect from service |
| PUT | `/api/v1/connections/{service_id}/credential` | Update credential |

### Proxy

| Method | Endpoint | Description |
|--------|----------|-------------|
| ANY | `/api/v1/proxy/{service_id}/{path}` | Proxy by UUID |
| ANY | `/api/v1/proxy/s/{slug}/{path}` | Proxy by slug |
| GET | `/api/v1/proxy/services` | List proxyable services |

### Providers (Admin config + DEPRECATED user connect -- use `/api/v1/keys`)

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/v1/providers` | List providers (admin) |
| POST | `/api/v1/providers` | Create provider (admin) |
| GET | `/api/v1/providers/{id}` | Get provider |
| PUT | `/api/v1/providers/{id}` | Update provider (admin) |
| DELETE | `/api/v1/providers/{id}` | Delete provider (admin) |
| GET | `/api/v1/providers/{id}/connect/oauth` | Initiate OAuth connect (deprecated -- use `/keys/oauth/authorize`) |
| POST | `/api/v1/providers/{id}/connect/api-key` | Connect with API key (deprecated -- use `POST /keys`) |
| POST | `/api/v1/providers/{id}/connect/device-code/initiate` | Start device code flow |
| POST | `/api/v1/providers/{id}/connect/device-code/poll` | Poll device code status |
| POST | `/api/v1/providers/{id}/refresh` | Refresh provider token |
| DELETE | `/api/v1/providers/{id}/disconnect` | Disconnect provider |
| GET | `/api/v1/providers/{id}/credentials` | Get user's own OAuth credentials |
| PUT | `/api/v1/providers/{id}/credentials` | Set user's own OAuth credentials |
| DELETE | `/api/v1/providers/{id}/credentials` | Delete user's own credentials |

### Nodes

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | `/api/v1/nodes/register-token` | Create registration token |
| GET | `/api/v1/nodes` | List user's nodes |
| GET | `/api/v1/nodes/{id}` | Get node details |
| DELETE | `/api/v1/nodes/{id}` | Delete node |
| POST | `/api/v1/nodes/{id}/rotate-token` | Rotate node auth token |
| POST | `/api/v1/nodes/{id}/bindings` | Bind service to node (deprecated -- use `PUT /user-services/{id}` with `node_id`) |
| DELETE | `/api/v1/nodes/{id}/bindings/{binding_id}` | Remove binding (deprecated) |

### Developer Apps (OAuth Clients)

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/v1/developer/oauth-clients` | List your OAuth clients |
| POST | `/api/v1/developer/oauth-clients` | Create OAuth client |
| GET | `/api/v1/developer/oauth-clients/{id}` | Get client details |
| PATCH | `/api/v1/developer/oauth-clients/{id}` | Update client |
| DELETE | `/api/v1/developer/oauth-clients/{id}` | Delete client |
| POST | `/api/v1/developer/oauth-clients/{id}/rotate-secret` | Rotate secret |

### OAuth / OIDC

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/.well-known/openid-configuration` | OIDC discovery |
| GET | `/.well-known/jwks.json` | Public signing keys |
| GET | `/oauth/authorize` | Authorization endpoint |
| POST | `/oauth/token` | Token endpoint |
| GET/POST | `/oauth/userinfo` | Get authenticated user info |
| POST | `/oauth/introspect` | Token introspection |
| POST | `/oauth/revoke` | Token revocation |

### Admin

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/v1/admin/users` | List all users |
| POST | `/api/v1/admin/service-accounts` | Create service account |
| GET | `/api/v1/admin/audit-log` | View audit log |

### MFA

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | `/api/v1/auth/mfa/setup` | Start MFA setup (TOTP) |
| POST | `/api/v1/auth/mfa/confirm` | Confirm MFA setup |
| POST | `/api/v1/auth/mfa/verify` | Verify MFA code |
| POST | `/api/v1/auth/mfa/disable` | Disable MFA |

### Approvals

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/v1/approvals/requests` | List approval requests |
| GET | `/api/v1/approvals/requests/{id}` | Get request details |
| POST | `/api/v1/approvals/requests/{id}/decide` | Approve or deny |
| GET | `/api/v1/approvals/requests/{id}/status` | Poll approval status |
| GET | `/api/v1/approvals/grants` | List active grants |
| DELETE | `/api/v1/approvals/grants/{id}` | Revoke grant |
| GET | `/api/v1/approvals/service-configs` | List service approval configs |
| PUT | `/api/v1/approvals/service-configs/{service_id}` | Configure approval requirement and mode (`per_request` or `grant`) |
| DELETE | `/api/v1/approvals/service-configs/{service_id}` | Remove approval requirement |

### Notifications

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/v1/notifications/settings` | Get notification preferences |
| PUT | `/api/v1/notifications/settings` | Update notification preferences |
| POST | `/api/v1/notifications/telegram/link` | Link Telegram account |
| DELETE | `/api/v1/notifications/telegram` | Disconnect Telegram |
| POST | `/api/v1/notifications/devices` | Register push notification device |
| GET | `/api/v1/notifications/devices` | List registered devices |
| DELETE | `/api/v1/notifications/devices/{id}` | Remove device |

### SSH

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | `/api/v1/ssh/{service_id}/certificate` | Issue SSH user certificate |
| POST | `/api/v1/ssh/{service_id}/exec` | Execute remote command |
| GET | `/api/v1/ssh/{service_id}/terminal` | Interactive terminal (WebSocket) |
| GET | `/api/v1/ssh/{service_id}` | SSH tunnel (WebSocket) |

### LLM Gateway

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/v1/llm/status` | List available LLM providers |
| ANY | `/api/v1/llm/gateway/v1/{path}` | Unified LLM proxy (OpenAI-compatible) |
| ANY | `/api/v1/{provider_slug}/v1/{path}` | Provider-specific LLM proxy |

### MCP

| Method | Endpoint | Description |
|--------|----------|-------------|
| * | `/mcp` | MCP proxy endpoint (Streamable HTTP) |

### OpenClaw Integration

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | `/api/v1/integrations/openclaw/channel` | Channel webhook (unauthenticated, per-user HMAC-verified) |
| POST | `/api/v1/integrations/openclaw/mappings` | Create channel-to-user mapping (returns one-time webhook secret) |

### Public Endpoints (no auth)

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/health` | Health check |
| GET | `/llms.txt` | Short AI agent context |
| GET | `/llms-full.txt` | Full AI agent playbook |
| GET | `/.well-known/openid-configuration` | OIDC discovery |
| GET | `/.well-known/jwks.json` | JWT public keys |

---

## 20. Error Code Reference

All errors return JSON: `{ "error": "...", "error_code": N, "message": "..." }`

| Code | Error Key | HTTP | Description |
|------|-----------|------|-------------|
| 1000 | bad_request | 400 | Invalid request body or parameters |
| 1001 | unauthorized | 401 | Missing or invalid auth token |
| 1002 | forbidden | 403 | Insufficient permissions |
| 1003 | not_found | 404 | Resource not found |
| 1004 | conflict | 409 | Resource already exists (e.g., duplicate email) |
| 1005 | rate_limited | 429 | Too many requests |
| 1006 | internal_error | 500 | Internal server error |
| 1007 | database_error | 500 | Database operation failed |
| 1008 | validation_error | 422 | Input validation failed |
| 2000 | authentication_failed | 401 | Wrong email or password |
| 2001 | token_expired | 401 | Access or refresh token has expired |
| 2002 | mfa_required | 403 | MFA verification needed (includes `session_token`) |
| 3000 | pkce_verification_failed | 400 | PKCE code verifier mismatch |
| 3001 | invalid_redirect_uri | 400 | Redirect URI not registered |
| 3002 | invalid_scope | 400 | Requested scope not allowed |
| 3003 | consent_required | 403 | User consent needed (includes `consent_url`) |
| 3004 | unsupported_grant_type | 400 | Grant type not supported |
| 4006 | duplicate_slug | 409 | Slug already in use |
| 5000 | service_account_not_found | 404 | Service account does not exist |
| 5001 | service_account_inactive | 403 | Service account is disabled |
| 6000 | social_auth_failed | 400 | Social login provider error |
| 6001 | social_auth_conflict | 409 | Social account already linked to another user |
| 6004 | external_token_invalid | 401 | External provider token is invalid or expired |
| 7000 | approval_required | 403 | Service requires approval (includes `request_id` and `action_description`) |
| 7001 | approval_failed | 403 | Approval was rejected, expired, or timed out (includes `request_id` and `approve_url`) |
| 8000 | node_not_found | 404 | Node does not exist |
| 8001 | node_offline | 502 | Node is not connected |
| 8002 | node_proxy_timeout | 504 | Node did not respond in time |
| 8003 | node_registration_failed | 400 | Node registration failed |

---

## 21. Troubleshooting

### "unauthorized" on all requests

- Check that the `Authorization: Bearer <token>` header is set
- Verify the token has not expired (default 15 min). Refresh with `/api/v1/auth/refresh`

### "mfa_required" error on login

Login returns `error_code: 2002` with a `session_token`. Complete the two-step flow:

```bash
# Step 1: Login attempt returns MFA challenge
curl -X POST http://localhost:3001/api/v1/auth/login \
  -H "Content-Type: application/json" \
  -d '{"email": "user@example.com", "password": "password"}'
# Response: { "error": "mfa_required", "error_code": 2002, "session_token": "..." }

# Step 2: Submit TOTP code with the session token
curl -X POST http://localhost:3001/api/v1/auth/mfa/verify \
  -H "Content-Type: application/json" \
  -d '{"session_token": "SESSION_TOKEN_FROM_STEP_1", "code": "123456"}'
# Response: { "access_token": "...", "refresh_token": "...", "expires_in": 900 }
```

### Token expired (error_code 2001)

Access tokens expire after 15 minutes. Refresh:

```bash
curl -X POST http://localhost:3001/api/v1/auth/refresh \
  -H "Content-Type: application/json" \
  -d '{"refresh_token": "REFRESH_TOKEN"}'
```

For OAuth clients:

```bash
curl -X POST http://localhost:3001/oauth/token \
  -H "Content-Type: application/x-www-form-urlencoded" \
  -d "grant_type=refresh_token" \
  -d "refresh_token=REFRESH_TOKEN" \
  -d "client_id=YOUR_CLIENT_ID"
```

### OAuth callback fails with "invalid_grant"

- PKCE code verifier must match the one used to generate the code challenge
- Authorization code can only be used once
- `redirect_uri` must exactly match between authorize request and token exchange
- Code expires after ~60 seconds

### Service proxy returns 403 with "approval_required"

- The service requires approval before access. The error response includes a `request_id` and an `action_description` (e.g., "POST /v1/chat/completions (model: gpt-4, 3 messages)").
- By default, approval mode is `per_request`: every proxy call needs fresh approval. There are no reusable grants.
- To use grant-based approvals instead, configure the service with `--approval-mode grant`.
- Use the `request_id` from the error to poll: `GET /api/v1/approvals/requests/{request_id}/status`
- Admin can approve via: `POST /api/v1/approvals/requests/{request_id}/decide {"approved": true}`

### Service proxy returns 403 with "approval_failed"

- The approval request was rejected, expired, or timed out. The error response includes:
  - `request_id`: the approval request ID
  - `approve_url`: a link to the web UI where the user can review pending approvals
  - `message`: the reason for failure (e.g., "Approval request timed out")
- If the user has no notification channel configured (Telegram or mobile app), they may not have seen the approval request. Suggest they set up notifications: `nyxid notification telegram-link` or install the NyxID mobile app.
- The user can always approve pending requests from the web UI at `/approvals/history`, even without notifications.

### MCP client can't find tools

1. Verify at least one service has endpoints defined
2. Verify the user has added the service via AI Services (`GET /api/v1/keys`)
3. Restart the MCP client after configuration changes

### Node agent connection issues

- Check the node is running: `nyxid node status`
- Verify the WebSocket URL is correct and reachable
- For production, use `wss://` (not `ws://`)
- Check node heartbeat timeout (default 90s) -- node marked offline if no heartbeat

---

## 22. Common Pitfalls

1. **`allowed_scopes` is an array, not a string.** When creating OAuth clients, pass `["openid", "profile", "email"]`, not `"openid profile email"`.

2. **`discover-endpoints` takes no request body.** Set `openapi_spec_url` on the service first (via PUT), then call `POST .../discover-endpoints` with no body.

3. **Endpoint creation requires a `name` field.** E.g., `"name": "list_users"`.

4. **Redirect URIs must match exactly.** Trailing slashes matter. The `redirect_uri` in the token exchange must be byte-identical to what was registered.

5. **Authorization codes are single-use.** A second attempt returns `invalid_grant`.

6. **SDK packages use `@nyxids` scope.** `@nyxids/oauth-core` and `@nyxids/oauth-react`.

7. **PKCE is required for public clients.** `code_challenge` + `code_challenge_method=S256` are mandatory.

8. **Internal services don't need user credentials.** Pass an empty body `{}` when connecting.

9. **MFA error code is 2002, not 2001.** 2001 is `token_expired`.

10. **Node WebSocket URL must use `wss://` in production.** Use `ws://` only for local development.

11. **Use `POST /api/v1/keys` for adding services, not the old routes.** The old `/connections` and `/providers/*/connect/*` routes still work but are deprecated.

12. **Slugs are auto-generated from labels.** When using `POST /api/v1/keys` with a custom endpoint, the slug is derived from the label (e.g., "Internal API" becomes `internal-api`).

---

## 23. NyxID CLI

The `nyxid` CLI manages services, API keys, catalog, nodes, MCP setup, approvals, notifications, SSH, and more. It reads `$NYXID_API_KEY` from the environment (via `--access-token-env`, default `NYXID_ACCESS_TOKEN`) or uses a stored session from `nyxid login`. **AI agents should prefer CLI commands over raw API calls** when the binary is available.

### Installation

```bash
# Build from source (requires Rust toolchain)
cargo install --path cli

# Or build manually
cargo build --release -p nyxid-cli
cp target/release/nyxid /usr/local/bin/

# Verify
nyxid --help
```

### Updating

To update the CLI to the latest version, reinstall it:

```bash
# From git (recommended)
cargo install --git https://github.com/ChronoAIProject/NyxID nyxid-cli

# Or from local checkout
git pull && cargo install --path cli
```

To update installed AI skills (fetches latest skill + playbook from server):

```bash
nyxid ai-setup update                        # update all installed tools
nyxid ai-setup update --tool claude-code     # update a specific tool
nyxid ai-setup status                        # check what's installed
```

If a command fails with an unrecognized flag or missing subcommand, the CLI is likely outdated. Reinstall it first.

### Global Options

All authenticated commands accept:

| Flag | Env Var | Description |
|------|---------|-------------|
| `--base-url <URL>` | `NYXID_URL` | NyxID server URL. **Saved to `~/.nyxid/base_url` on `nyxid login`** -- only needed on first login. |
| `--access-token <TOKEN>` | -- | Override saved token |
| `--access-token-env <VAR>` | -- | Env var to read token from (default: `NYXID_ACCESS_TOKEN`) |
| `--output <FORMAT>` | -- | Output format: `table` (default) or `json` |

### Non-Interactive Secret Handling

Every command that accepts a secret (API keys, credentials, tokens, passwords) supports three modes:

| Mode | Flag Example | Best For |
|------|-------------|----------|
| **Interactive prompt** | *(default)* | Recommended -- secure `rpassword` prompt, input never echoed |
| **Env var** | `--credential-env MY_API_KEY` | Automation, CI/CD pipelines, scripting |
| **Direct value** | `--credential <VALUE>` | Scripts (hidden from shell history on supported shells) |

Specific env-var flags by context:

| Flag | Used By |
|------|---------|
| `--credential-env <VAR>` | `service add`, `service rotate-credential`, `openclaw setup` |
| `--token-env <VAR>` | `service add --oauth`, `service add --device-code` |
| `--password-env <VAR>` | `login --password`, `register` |
| `--client-id-env <VAR>` | `service credentials` |
| `--client-secret-env <VAR>` | `service credentials` |

> **Prefer the interactive prompt (no flags)** for credential entry -- the CLI securely prompts with hidden input. Never ask the user to paste secrets into chat. Use `--credential-env` only for automation/scripting where no human is present.

### Complete Command Reference

#### Authentication and Account

```bash
nyxid login --base-url <URL>           # Log in (opens browser); saves URL to ~/.nyxid/base_url
  [--password]                         #   Use email/password instead of browser
  [--password-env <VAR>]               #   Read password from env var (non-interactive)
  [--email <EMAIL>]                    #   Email (only with --password)
nyxid logout                           # Log out and clear stored token
nyxid register                         # Register a new account
  --email <EMAIL>
  [--name <NAME>]
  [--password-env <VAR>]               #   Read password from env var (non-interactive)
nyxid verify-email                     # Verify email with token from email
  --token <TOKEN>
nyxid forgot-password                  # Request password reset email
  --email <EMAIL>
nyxid reset-password                   # Reset password using reset token
  --token <TOKEN>
nyxid whoami                           # Show current user info
nyxid status                           # Show account overview (services, keys, nodes)
```

> After `nyxid login --base-url <URL>`, the URL is saved. All subsequent commands use it automatically.

#### Profile Management

```bash
nyxid profile update                   # Update profile
  [--name <NAME>]
nyxid profile delete                   # Delete your account
  [--yes]                              #   Skip confirmation prompt
nyxid profile consents                 # List OAuth consents
nyxid profile revoke-consent <CID>     # Revoke an OAuth consent
  [--yes]
```

#### Multi-Factor Authentication

```bash
nyxid mfa setup                        # Set up MFA (displays QR code URL and secret)
nyxid mfa verify --code <CODE>         # Verify MFA setup with a TOTP code
nyxid mfa status                       # Show current MFA status
```

#### Sessions

```bash
nyxid session list                     # List active sessions
```

#### Service Catalog

```bash
nyxid catalog list                     # Browse available service templates
nyxid catalog show <SLUG>              # Show details for a catalog entry
```

#### Service Management (AI Services)

```bash
nyxid service add <SLUG>               # Add from catalog (auto-fetches label from catalog)
  [--credential-env <VAR>]             #   Read credential from env var (non-interactive)
  [--credential <VALUE>]               #   Pass credential directly (hidden)
  [--custom]                           #   Add a fully custom endpoint
  [--oauth]                            #   Use OAuth flow for authentication
  [--device-code]                      #   Use device code flow for authentication
  [--token-env <VAR>]                  #   Read OAuth/device-code token from env var
  [--via-node <NODE_ID|NAME>]          #   Route traffic through a node (accepts name or ID)
  [--endpoint-url <URL>]               #   Endpoint URL override
  [--label <LABEL>]                    #   Display label (auto-fetched from catalog if omitted)
  [--auth-method <METHOD>]             #   Auth method: bearer, header, query, path, basic
  [--auth-key-name <NAME>]             #   Auth key name (e.g., Authorization, X-API-Key)
nyxid service add-ssh                  # Add an SSH service
  --label <LABEL>
  --host <HOST>
  [--port <PORT>]                      #   Default: 22
  [--cert-auth]                        #   Enable certificate authentication
  [--principals <LIST>]                #   Comma-separated SSH principals
  [--ttl <MINUTES>]                    #   Certificate TTL in minutes (default: 30)
  --via-node <NODE_ID|NAME>            #   Node to route through (required for SSH)
nyxid service list                     # List user's configured services (includes ID column)
nyxid service show <ID>                # Show service details
nyxid service update <ID>              # Update service configuration
  [--label <LABEL>]                    #   New display label
  [--endpoint-url <URL>]
  [--node-id <NODE_ID|NAME>]           #   New node for routing (accepts name or ID)
  [--no-node]                          #   Remove node routing (direct mode)
  [--active]                           #   Set service to active
  [--inactive]                         #   Set service to inactive
nyxid service delete <ID>              # Delete a service
  [--yes]
nyxid service rotate-credential <ID>   # Rotate external credential for a service
  [--credential-env <VAR>]             #   Read new credential from env var
nyxid service route <ID>               # Change service routing
  [--node <NODE_ID|NAME>]              #   Route through this node (accepts name or ID)
  [--direct]                           #   Use direct routing (no node)
nyxid service credentials <SLUG>       # Set OAuth client credentials for a provider
  --client-id <CID>
  --client-secret <SECRET>
  [--client-id-env <VAR>]              #   Read client ID from env var
  [--client-secret-env <VAR>]          #   Read client secret from env var
```

> **Catalog slugs auto-fetch labels:** `nyxid service add llm-openai` auto-fills the label from the catalog. No `--label` needed for catalog services.
>
> **Unknown slug error:** If the slug is not in the catalog, the CLI shows a helpful message suggesting `nyxid catalog list` to browse available services or `--custom` to add a custom endpoint.
>
> **Service list shows IDs:** `nyxid service list` includes an ID column so AI agents can reference specific services by ID.

#### NyxID API Keys

```bash
nyxid api-key create                   # Create a new NyxID API key
  [--name <NAME>]
  [--scopes <SCOPES>]                  #   Space-separated: read write proxy
  [--expires-in-days <N>]              #   Expiry in days (0 = no expiry)
  [--allowed-services <IDS>]           #   Comma-separated service IDs
  [--allowed-nodes <IDS>]              #   Comma-separated node IDs
  [--allow-all-services]               #   Allow access to all services
  [--allow-all-nodes]                  #   Allow access to all nodes
nyxid api-key list                     # List API keys
nyxid api-key show <ID>                # Show key details
nyxid api-key update <ID>              # Update key scope
  [--name <NAME>]
  [--scopes <SCOPES>]
  [--allowed-services <IDS>]
  [--allowed-nodes <IDS>]
  [--allow-all-services <BOOL>]
  [--allow-all-nodes <BOOL>]
nyxid api-key rotate <ID>              # Rotate a key
nyxid api-key delete <ID>              # Revoke a key
  [--yes]
```

#### Node Management

```bash
nyxid node list                        # List user's nodes (includes ID column)
nyxid node show <ID_OR_NAME>           # Show node details (accepts name or ID)
nyxid node register-token              # Generate a registration token
nyxid node delete <ID_OR_NAME>         # Delete a node (accepts name or ID)
  [--yes]
nyxid node rotate-token <ID_OR_NAME>   # Rotate node auth token (accepts name or ID)
```

> **Node commands accept names:** `nyxid node show test-server` resolves the name to an ID automatically. Same for `node delete` and `node rotate-token`.

#### Proxy Requests

```bash
nyxid proxy discover                   # List proxyable services (service discovery)
nyxid proxy request <SERVICE> [PATH]   # Send a request through the NyxID proxy
  [-m, --method <METHOD>]              #   HTTP method (default: GET)
  [-d, --data <BODY>]                  #   Request body (JSON, @file, or - for stdin)
  [-H, --header <K:V>]                 #   Extra headers (repeatable)
  [--stream]                           #   Stream the response (SSE, video, audio, large files)
  [--by-id]                            #   Use service ID instead of slug
```

#### SSH

```bash
# All SSH commands accept service ID, slug, or name (auto-resolves)
nyxid ssh issue-cert <SERVICE>         # Issue a short-lived SSH certificate
  --public-key-file <PATH>
  --principal <NAME>
  --certificate-file <PATH>
  [--ca-public-key-file <PATH>]
nyxid ssh proxy <SERVICE>              # SSH-over-WebSocket tunnel (ProxyCommand)
  [--issue-certificate]                #   Auto-issue certificate before connecting
  [--public-key-file <PATH>]
  [--principal <NAME>]
  [--certificate-file <PATH>]
  [--ca-public-key-file <PATH>]
nyxid ssh config                       # Print an OpenSSH config stanza
  --host-alias <ALIAS>
  --base-url <URL>
  --service-id <ID>
  --principal <NAME>
  --identity-file <PATH>
  --certificate-file <PATH>
  [--access-token-env <VAR>]
  [--ca-public-key-file <PATH>]
nyxid ssh exec <SERVICE>               # Execute a command on a remote host
  --principal <NAME>
  <COMMAND...>
nyxid ssh terminal <SERVICE>           # Interactive SSH terminal
  [--principal <NAME>]                 #   auto-resolved from service config if omitted
```

#### MCP Configuration

```bash
nyxid mcp config                       # Generate MCP configuration for AI tools
  [--tool <TARGET>]                    #   Target: cursor, claude-code, vscode, generic (default: generic)
```

#### OpenClaw Integration

```bash
nyxid openclaw setup                   # OpenClaw setup
  [--url <GATEWAY_URL>]
  [--credential-env <VAR>]             #   Read bearer token from env var (non-interactive)
```

#### Notifications

```bash
nyxid notification settings            # Show current notification settings
nyxid notification update              # Update notification settings
  [--approval-email <BOOL>]
  [--approval-push <BOOL>]
  [--approval-telegram <BOOL>]
nyxid notification telegram-link       # Link a Telegram account
nyxid notification telegram-disconnect # Disconnect Telegram account
```

#### Approvals

```bash
nyxid approval list                    # List approval requests
nyxid approval show <ID>               # Show approval request details
nyxid approval approve <ID>            # Approve a request
nyxid approval deny <ID>               # Deny a request
  [--reason <REASON>]
nyxid approval grants                  # List approval grants
nyxid approval revoke-grant <ID>       # Revoke an approval grant
  [--yes]
nyxid approval service-configs         # List per-service approval configurations (includes approval_mode)
nyxid approval set-config <ID>         # Set approval configuration for a service
  [--require-approval <BOOL>]
  [--approval-mode <MODE>]             #   "per_request" (default) or "grant"
```

#### Endpoints (Low-Level)

```bash
nyxid endpoint list                    # List user endpoints
nyxid endpoint update <ID> --url <URL> # Update an endpoint URL
nyxid endpoint delete <ID>             # Delete an endpoint
  [--yes]
```

#### External Keys (Low-Level)

```bash
nyxid external-key list                # List external API keys/credentials
nyxid external-key rotate <ID>         # Rotate an external credential
nyxid external-key delete <ID>         # Delete an external credential
  [--yes]
```

### Using with AI Agents

AI agents use the `nyxid` CLI for all NyxID operations. After `nyxid login --base-url <URL>`, the URL is saved to `~/.nyxid/base_url`. No need to set environment variables or pass `--base-url` on every command.

**Fully non-interactive AI agent workflow:**

```bash
# One-time login (saves base URL for all future commands)
nyxid login --base-url http://localhost:3001

# Add a service non-interactively (credential from env var)
export OPENAI_KEY="sk-..."
nyxid service add llm-openai --credential-env OPENAI_KEY --output json

# List services (table includes IDs for programmatic reference)
nyxid service list --output json

# Node commands accept names instead of IDs
nyxid node show my-server --output json

# All secret-handling commands support --credential-env / --token-env / --password-env
```

### Common Workflows

**First-time setup (one-time):**

```bash
nyxid login --base-url http://localhost:3001    # saves URL; only needed once
nyxid catalog list --output json
nyxid service add llm-openai --credential-env OPENAI_KEY  # non-interactive
nyxid status
```

**Add a service and make a proxy request:**

```bash
nyxid service add llm-anthropic --credential-env ANTHROPIC_KEY  # auto-fetches label from catalog
nyxid proxy request llm-anthropic v1/messages \
  -m POST -d '{"model":"claude-sonnet-4-20250514","max_tokens":100,"messages":[{"role":"user","content":"Hello"}]}'
```

**Set up approval workflow (per-request, default):**

```bash
nyxid approval set-config <SERVICE_ID> --require-approval true
nyxid notification update --approval-telegram true
nyxid notification telegram-link
```

**SSH remote access (accepts ID, slug, or name):**

```bash
nyxid ssh exec <SERVICE_OR_SLUG> --principal ubuntu -- uptime
nyxid ssh terminal <SERVICE_OR_SLUG>                    # auto-resolves principal
```

---

## 24. Using NyxID in OpenClaw

NyxID ships an OpenClaw skill at `skills/nyxid` that lets OpenClaw agents discover and call external services through NyxID's credential proxy. The skill uses the `nyxid` CLI exclusively -- no environment variables or HTTP fallback.

For the full integration guide (plugin setup, channel integration, node agent support), see [`docs/OPENCLAW_INTEGRATION.md`](OPENCLAW_INTEGRATION.md).

### Prerequisites

Install the `nyxid` CLI on the OpenClaw machine and log in once:

```bash
# Install Rust (if needed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"

# Install the NyxID CLI
cargo install --git https://github.com/ChronoAIProject/NyxID nyxid-cli

# Log in (opens browser, saves URL for all future commands)
nyxid login --base-url https://nyx-api.chrono-ai.fun
```

After login, the CLI stores tokens at `~/.nyxid/` and auto-refreshes them. No further configuration needed.

> **Important:** `nyxid login --base-url https://nyx-api.chrono-ai.fun` must be run before the skill can work. If the agent encounters `1001 unauthorized` errors, the token has expired and the user must re-run `nyxid login` to re-authenticate.

### Install the Skill

Copy the skill to OpenClaw's managed skills directory:

```bash
mkdir -p ~/.openclaw/skills
cp -r skills/nyxid ~/.openclaw/skills/nyxid
```

Verify the skill passes the eligibility check:

```bash
openclaw skills check
```

The skill requires `nyxid` on PATH. No environment variables are needed.

### How It Works

Once installed, the OpenClaw agent can:

1. **Discover services** -- `nyxid service list --output json` returns all user-configured services with slugs, status, and endpoint URLs.
2. **Make proxy requests** -- `nyxid proxy request <slug> <path> -m <METHOD> -d '<body>'` calls any connected service through NyxID. Credentials are injected server-side.
3. **Add services** -- `nyxid catalog list` browses available services; `nyxid service add <slug> --credential-env <VAR>` adds from catalog.

The agent never handles raw downstream credentials. NyxID injects them at proxy time.

### Example Agent Interaction

```
User: "Send a message to OpenAI using my connected account"

Agent runs: nyxid service list --output json
Agent sees: llm-openai (active, slug: llm-openai)
Agent runs: nyxid proxy request llm-openai /chat/completions -m POST \
  -d '{"model":"gpt-4","messages":[{"role":"user","content":"Hello"}]}'
```

### Approval Integration

If a service has approval gating enabled (per-request by default), proxy calls return error code `7000` with an `action_description` and `request_id`. The agent should:

1. Inform the user that approval is needed
2. Show the `action_description` (e.g., "POST /v1/chat/completions (model: gpt-4, 3 messages)")
3. Wait for the user to approve via mobile app, Telegram, or `nyxid approval approve <ID>`

If the approval is rejected, expires, or times out, the error changes to `7001 approval_failed` with an `approve_url`. The agent should show the user the `approve_url` and suggest setting up Telegram or the mobile app if they haven't already (`nyxid notification telegram-link`).

### Skill Tools

The skill includes two helper scripts in `tools/`:

- `services.sh` -- Runs `nyxid service list --output json`
- `proxy.sh <service> <method> <path> [body]` -- Runs `nyxid proxy request` with the given arguments

Both are thin wrappers around the CLI.

### Optional: OAuth Plugin

For advanced use cases (RFC 8693 delegation, programmatic token exchange), install the TypeScript auth plugin:

```bash
cd integrations/openclaw && npm install && npm run build
```

Configure in `~/.openclaw/openclaw.json`:

```json
{
  "plugins": {
    "nyxid": {
      "enabled": true,
      "baseUrl": "https://nyx-api.chrono-ai.fun",
      "clientId": "your-client-id",
      "clientSecret": "your-client-secret"
    }
  }
}
```

Most users only need the skill (CLI mode). The plugin is for server-side OAuth flows.

### Troubleshooting

| Symptom | Fix |
|---------|-----|
| Skill blocked in `openclaw skills check` | Ensure `nyxid` is on PATH: `which nyxid` |
| `1001 unauthorized` from CLI | Token expired. Re-authenticate: `nyxid login --base-url https://nyx-api.chrono-ai.fun` |
| `No base URL configured` or connection refused | Login was never run. Run `nyxid login --base-url https://nyx-api.chrono-ai.fun` first |
| Empty service list | Add services: `nyxid catalog list` then `nyxid service add <slug>` |
| `7000 approval_required` | User must approve: `nyxid approval list` |
| `7001 approval_failed` | Approval rejected/expired/timed out. Check `approve_url` in response, or set up notifications: `nyxid notification telegram-link` |

See [`docs/OPENCLAW_INTEGRATION.md`](OPENCLAW_INTEGRATION.md) for the full integration guide including plugin setup, channel integration, and node agent support.

---

## 25. Channel Bot Relay

> **ADR-013 update (2026-04-09):** Per ADR-013, NyxID is a **pure passthrough gateway**. It never stores message bodies, attachments, or raw webhook payloads — only routing metadata. Synchronous agent replies (HTTP 200 + body) are no longer supported; agents **must** return 202 to the callback and post replies via `POST /api/v1/channel-relay/reply`. The earlier NyxID#191 deprecation of channel relay has been recalled.

NyxID acts as a multi-platform messaging gateway. Users register their own bots (Telegram, Discord, Lark, Feishu), and NyxID receives messages via platform webhooks, routes each message to the correct AI agent's callback URL, and relays the agent's asynchronous reply back to the chat.

### Architecture

```
Telegram/Discord/Lark/Feishu
    |
    v
NyxID Webhook Handler (signature-verified)
    |
    v
Routing Service (conversation -> agent)
    |
    v
Agent Callback URL (POST with HMAC signature)
    |
    v
Agent Reply (async only: 202 + POST /channel-relay/reply)
    |
    v
NyxID sends reply back to platform
```

### Setup

**Step 1: Create an API key with a callback URL**

```bash
nyxid api-key create --name "my-agent" --platform claude-code --callback-url "https://my-agent.example.com/webhook"
```

The `callback_url` is where NyxID will POST normalized messages when they arrive from a chat platform.

**Step 2: Register a bot**

```bash
# Telegram (auto-registers webhook)
nyxid channel-bot register --platform telegram --label "Support Bot" --token-env TELEGRAM_TOKEN

# Discord (manual webhook setup in Developer Portal)
nyxid channel-bot register --platform discord --label "Discord Bot" --token-env DISCORD_TOKEN --public-key "hex_ed25519_key"

# Lark (manual webhook setup in Developer Console)
nyxid channel-bot register --platform lark --label "Lark Bot" --token-env LARK_TOKEN --app-id "cli_xxx" --app-secret-env LARK_SECRET
```

For Discord/Lark/Feishu, set the webhook URL in the platform's developer console:
`https://<nyxid-server>/api/v1/webhooks/channel/<platform>/<bot-id>`

The bot auto-activates on first successful webhook delivery.

**Step 3: Configure routing**

```bash
# Default route: all messages from this bot go to the agent
nyxid channel-bot route create --bot <BOT_ID> --agent <API_KEY_ID>

# Specific conversation route
nyxid channel-bot route create --bot <BOT_ID> --conversation-id "12345678" --agent <API_KEY_ID>
```

### CLI Reference

```bash
# Bot management
nyxid channel-bot register --platform <PLATFORM> --label <LABEL> --token-env <ENV_VAR>
nyxid channel-bot list
nyxid channel-bot show <ID>
nyxid channel-bot verify <ID>
nyxid channel-bot delete <ID> --yes

# Conversation routing
nyxid channel-bot route create --bot <BOT_ID> --agent <API_KEY_ID> [--conversation-id <ID>] [--default]
nyxid channel-bot route list [--bot-id <BOT_ID>]
nyxid channel-bot route update <ROUTE_ID> [--agent <KEY>] [--default true|false]
nyxid channel-bot route delete <ROUTE_ID> --yes
```

### API Endpoints

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| POST | `/api/v1/channel-bots` | Session | Register a new bot |
| GET | `/api/v1/channel-bots` | Session | List user's bots |
| GET | `/api/v1/channel-bots/{id}` | Session | Bot detail |
| DELETE | `/api/v1/channel-bots/{id}` | Session | Delete bot |
| POST | `/api/v1/channel-bots/{id}/verify` | Session | Re-verify bot |
| POST | `/api/v1/channel-conversations` | Session | Create route |
| GET | `/api/v1/channel-conversations` | Session | List routes |
| PUT | `/api/v1/channel-conversations/{id}` | Session | Update route |
| DELETE | `/api/v1/channel-conversations/{id}` | Session | Delete route |
| GET | `/api/v1/channel-conversations/{id}/messages` | Session | Message history (owner) |
| POST | `/api/v1/channel-relay/reply` | API Key | Agent async reply |
| GET | `/api/v1/channel-relay/messages/{id}` | API Key | Message history (agent) |
| GET | `/api/v1/channel-relay/resolve-sender` | API Key | Resolve platform sender |
| POST | `/api/v1/webhooks/channel/{platform}/{bot_id}` | None | Platform webhook (signature-verified) |

### Callback Payload (NyxID -> Agent)

When a message arrives, NyxID POSTs a normalized payload to the agent's callback URL:

```json
{
  "message_id": "uuid",
  "platform": "telegram",
  "agent": { "api_key_id": "uuid", "name": "my-agent" },
  "conversation": { "id": "uuid", "platform_id": "12345678", "type": "private" },
  "sender": { "platform_id": "87654321", "display_name": "Alice" },
  "content": { "type": "text", "text": "Hello", "attachments": [] },
  "timestamp": "2026-04-01T12:00:00Z",
  "raw_platform_data": { "...": "full original Telegram/Discord/Lark webhook JSON" }
}
```

The payload includes both normalized fields and `raw_platform_data` (the full original webhook JSON from the platform). Most agents use the normalized fields; agents that need platform-specific features (Telegram inline keyboards, Discord embeds, Lark interactive cards) can read `raw_platform_data` directly.

Headers: `X-NyxID-Signature` (HMAC-SHA256), `X-NyxID-Message-Id`, `X-NyxID-Timestamp`, `X-NyxID-Platform`, `X-NyxID-User-Token` (short-lived access token for the bot owner -- use as `Authorization: Bearer <token>` to call NyxID APIs on behalf of the user).

### Agent Reply

**Synchronous (200):** Include reply in response body:
```json
{ "reply": { "text": "Hello back!" } }
```

**Asynchronous (202):** Return empty 202, then call later:
```
POST /api/v1/channel-relay/reply
Authorization: Bearer nyxid_ag_xxxxx
{ "message_id": "<original-message-id>", "reply": { "text": "Here's my answer..." } }
```

### Error Codes

| Code | Name | Description |
|------|------|-------------|
| 10000 | `channel_bot_not_found` | Bot ID does not exist |
| 10001 | `channel_bot_inactive` | Bot is deactivated |
| 10002 | `channel_bot_limit_reached` | Max bots per user exceeded |
| 10003 | `channel_webhook_verification_failed` | Platform signature invalid |
| 10004 | `channel_relay_failed` | Agent callback failed |
| 10005 | `channel_platform_error` | Platform API error |

### Integration with Agent Isolation

Channel relay uses the same `ApiKey` model as agent isolation. The `callback_url` on the API key is where NyxID sends messages. Proxy scope enforcement (`allowed_service_ids`, `allowed_node_ids`) applies when the agent makes proxy calls, not to the relay itself. Each agent has independent rate limits, audit trails, and credential bindings.

For full design details, see [`docs/CHANNEL_BOT_RELAY.md`](CHANNEL_BOT_RELAY.md).
