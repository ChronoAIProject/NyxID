# n8n: One Credential for Every Upstream API

Configure n8n to call any number of upstream APIs through a single NyxID `Header Auth` credential. NyxID injects the per-service authentication (API keys, OAuth tokens, path-based tokens) at proxy time; the n8n credential never carries a downstream secret.

```
n8n  ─┐
curl ─┼──(X-API-Key: nyx_…)──►  NyxID Proxy  ──►  Gemini · TwitterAPI.io · Google Sheets · Telegram · …
agent─┘                                            (NyxID injects the right auth per service)
```

This guide uses four upstream APIs as worked examples — Gemini (header auth), TwitterAPI.io (custom header), Google Sheets (OAuth2), Telegram Bot (path-based auth). The same flow works for any HTTP API.

## Prerequisites

- A NyxID account and an Agent Key with the `proxy` scope. If you don't have these yet, complete **Step 0** below; otherwise jump to the [Procedure](#procedure).
- An n8n instance (cloud or self-hosted) where you can add a credential and `HTTP Request` nodes.
- An upstream API token for each service you want n8n to call. The walkthrough below uses four; obtain each before starting:

  | Upstream service | Where to obtain the token | Auth used | NyxID slug |
  |---|---|---|---|
  | Gemini | [aistudio.google.com/apikey](https://aistudio.google.com/apikey) → `Create API key` | API key (header `x-goog-api-key`) | `llm-google-ai` (catalog) |
  | TwitterAPI.io | [twitterapi.io](https://twitterapi.io) → sign up → `API Keys` | API key (header `x-api-key`) | `twitterapi-io` (custom) |
  | Google Sheets | Google Cloud Console — full Cloud Console steps are in [Step 2](#2-register-an-oauth-service-google-sheets) | OAuth 2.0 (refresh token) | `api-google` (catalog) |
  | Telegram Bot | Chat with [@BotFather](https://t.me/BotFather) on Telegram → `/newbot` and follow the prompts | Bot token (path `/bot<token>/`) | `api-telegram-bot` (catalog) |

  Each token in this table is the *upstream* credential — what NyxID injects into the proxied request. The Agent Key from Step 0 is separate; that is what n8n authenticates to NyxID with.

### Step 0 — Get NyxID running and create an Agent Key

**Hosted (recommended).** Sign up at [nyx.chrono-ai.fun/register](https://nyx.chrono-ai.fun/register) using the invite code in the [README Getting Started](../../README.md#1-install-nyxid). After signing in, open `AI Services` → `Agent Keys` → `Create API Key`, name the key `n8n`, select the `proxy` scope, click `Create`, and copy the displayed `nyx_…` value (shown once).

**Self-host.** Follow [docs/SETUP.md](../SETUP.md) to bring up the Docker stack, register at `http://localhost:3000`, then create the Agent Key via the same web console flow.

Save the Agent Key value somewhere safe — a password manager works, or a local file with `chmod 600`. You will paste it into n8n in [Step 3](#3-paste-the-agent-key-into-an-n8n-header-auth-credential).

## Procedure

### 1. Register each upstream service in NyxID

There are two paths — the web UI (recommended for first-time setup) and the CLI (for scripting). Both produce the same result. Pick one and follow it for every API-key service. Google Sheets uses OAuth and is covered separately in [Step 2](#2-register-an-oauth-service-google-sheets).

**Web UI — catalog services (Gemini, Telegram Bot).** For each:

1. In the web console, open `AI Services` and click `Add Service`.
2. Type the service name in the catalog search (e.g. `Gemini AI`, `Telegram Bot`) and click `Connect` on the matching entry. NyxID prefills the endpoint URL and auth method from the catalog.
3. Paste the upstream token from the [Prerequisites table](#prerequisites) into the `Credential` field, set a `Label` (e.g. `Gemini AI`), and click `Save`.
4. NyxID lands on the service detail page with the assigned `Slug` at the top — record it for [Step 4](#4-configure-http-request-nodes-in-n8n).

**Web UI — custom services (TwitterAPI.io).**

1. In the web console, open `AI Services` and click `Add Service`.
2. Scroll past the catalog and click `Add custom service`.
3. Fill the form:
   - `Slug`: `twitterapi-io`
   - `Label`: `TwitterAPI.io`
   - `Endpoint URL`: `https://api.twitterapi.io`
   - `Auth method`: `header`
   - `Auth key name`: `x-api-key`
4. Paste the upstream token into the `Credential` field and click `Save`.
5. Record the `Slug` shown on the service detail page for [Step 4](#4-configure-http-request-nodes-in-n8n).

**CLI.** If you have the `nyxid` CLI installed and logged in, save each upstream token to a local file (`~/.gemini_key`, `~/.twitterapi_key`, `~/.tg_token`) with `chmod 600`, then run:

```bash
# Gemini — catalog entry llm-google-ai
GEMINI_KEY="$(cat ~/.gemini_key)" \
  nyxid service add llm-google-ai \
  --credential-env GEMINI_KEY \
  --label "Gemini AI"

# TwitterAPI.io — not in the catalog; register as custom
TWITTER_KEY="$(cat ~/.twitterapi_key)" \
  nyxid service add --custom \
  --slug twitterapi-io \
  --label "TwitterAPI.io" \
  --endpoint-url "https://api.twitterapi.io" \
  --auth-method header \
  --auth-key-name x-api-key \
  --credential-env TWITTER_KEY

# Telegram Bot — catalog entry api-telegram-bot (path auth handled by NyxID)
TG_TOKEN="$(cat ~/.tg_token)" \
  nyxid service add api-telegram-bot \
  --credential-env TG_TOKEN \
  --label "Telegram Bot"
```

Each `nyxid service add` prints the user-side slug NyxID assigned. On a fresh account it matches the catalog or `--slug` value; if you already have a service with that slug, NyxID appends `-2`, `-3`, or a random suffix to keep slugs unique. Record the printed slugs — they go into the `HTTP Request` URLs in [Step 4](#4-configure-http-request-nodes-in-n8n).

### 2. Register an OAuth service (Google Sheets)

OAuth services need an OAuth client (created in the upstream provider's developer console) before NyxID can run the consent flow.

**Cloud Console steps (required for both web UI and CLI paths):**

1. In Google Cloud Console, create an `OAuth 2.0 Client ID` (Web application) and add `https://<your-nyxid-host>/api/v1/providers/callback` to `Authorized redirect URIs`.
2. Enable the `Google Sheets API` and `Google Drive API`.
3. On the `OAuth consent screen`, open `Data access` → `Add or remove scopes` and add `https://www.googleapis.com/auth/spreadsheets`. NyxID's scope flag only takes effect for scopes already declared on the consent screen.
4. Add your Google account to `Test users`.
5. Note the **Client ID** and **Client Secret** for the OAuth 2.0 Client. You'll paste both into NyxID in the next step.

**Web UI.**

1. In the NyxID web console, open `AI Services` and click `Add Service`.
2. Search for `Google` and click `Connect` on the Google catalog entry. NyxID detects the service uses OAuth and opens the OAuth client form.
3. Paste the `Client ID` and `Client Secret` from the Cloud Console step above and click `Continue to Authentication`.
4. On the next screen, paste `https://www.googleapis.com/auth/spreadsheets` into the `Additional scopes` field, then click `Connect with Google`.
5. Approve the Google consent screen. NyxID redirects back and lands on the service detail page.
6. The catalog default for `api-google` is `www.googleapis.com`, but Google Sheets lives on `sheets.googleapis.com`. On the service detail page, edit the `Endpoint URL` field to `https://sheets.googleapis.com` and save.

**CLI.** Save the Client ID to `~/.gc_id` and the Client Secret to `~/.gc_secret` (`chmod 600` both), then:

```bash
GC_ID="$(cat ~/.gc_id)" GC_SECRET="$(cat ~/.gc_secret)" \
  nyxid service credentials api-google \
  --client-id-env GC_ID \
  --client-secret-env GC_SECRET

nyxid service add api-google \
  --oauth \
  --scope "https://www.googleapis.com/auth/spreadsheets" \
  --label "Google Sheets"

# CLI prints an authorization URL. Open it, sign in, and approve.
# After approval, override the endpoint URL (sheets, not www):
nyxid service update <ID> --endpoint-url "https://sheets.googleapis.com"
```

NyxID stores the resulting refresh token and refreshes the access token automatically on every proxied call. Replace `<ID>` with the user-side service ID printed by `nyxid service add` (or run `nyxid service list` to find it).

### 3. Paste the Agent Key into an n8n Header Auth credential

The Agent Key from [Step 0](#step-0--get-nyxid-running-and-create-an-agent-key) is what n8n uses to authenticate to NyxID on every proxied request.

In n8n, open `Credentials` → `New` → `Header Auth` and set:

| Field | Value |
|---|---|
| `Name` | `NyxID API Key` |
| `Header Name` | `X-API-Key` |
| `Header Value` | The `nyx_…` Agent Key from Step 0 |

Click `Save`.

> If you saved the Agent Key to a local file, copy it into your clipboard with `cat ~/.nyx_key \| pbcopy` (macOS) or `cat ~/.nyx_key \| xclip -selection clipboard` (Linux), paste into the `Header Value` field, then securely delete the file with `rm -P ~/.nyx_key` (macOS) or `shred -u ~/.nyx_key` (Linux).

For per-workflow blast-radius isolation (so a leaked key can only call the services this workflow uses), open `AI Services` → `Agent Keys` → `[your key]` in the web console, locate the `Service Scope` card, uncheck `Allow all services`, select the registered services, and save.

### 4. Configure HTTP Request nodes in n8n

For each upstream service the workflow needs to call, add an `HTTP Request` node and configure four fields:

1. **`Method`** — set to the HTTP method the upstream API expects (`GET`, `POST`, etc.).
2. **`URL`** — use the proxy pattern below, substituting the slug NyxID returned in [Step 1](#1-register-each-upstream-service-in-nyxid) (or [Step 2](#2-register-an-oauth-service-google-sheets) for Google Sheets) and the downstream API path you would normally use without NyxID:

   ```
   https://<nyxid-host>/api/v1/proxy/s/<service-slug>/<downstream-api-path>
   ```

3. **`Authentication`** — select `Generic Credential Type`. n8n then shows a **`Generic Auth Type`** dropdown — select `Header Auth`.
4. **`Credential for Header Auth`** — select the `NyxID API Key` credential you saved in [Step 3](#3-paste-the-agent-key-into-an-n8n-header-auth-credential).

Reference URLs for the four services in this guide:

| Upstream | Method + Path |
|---|---|
| Gemini | `POST /api/v1/proxy/s/llm-google-ai/models/gemini-2.5-flash:generateContent` |
| TwitterAPI.io | `GET  /api/v1/proxy/s/twitterapi-io/twitter/user/last_tweets?userName=…` |
| Google Sheets | `POST /api/v1/proxy/s/api-google/v4/spreadsheets/<sheet-id>/values/Sheet1!A:H:append` |
| Telegram Bot | `POST /api/v1/proxy/s/api-telegram-bot/sendMessage` |

NyxID injects the correct authentication for each service: `x-goog-api-key` for Gemini, `x-api-key` for TwitterAPI.io, refreshed `Authorization: Bearer …` for Google Sheets, and the path-based `/bot<token>/` prefix for Telegram. The n8n node never sees any of these.

## Verification

Send one test request per service from your terminal to confirm the proxy is wired correctly before invoking it from n8n. Set the shell variables first:

```bash
NYX_API_KEY="nyx_…"                     # the Agent Key from Step 0
NYXID_BASE="https://<your-nyxid-host>"  # use http://localhost:3001 for self-host
```

Then run each test. Each one passes when NyxID injects the correct upstream credential:

```bash
# Telegram — getMe returns the bot identity when the path-based token is injected
curl -sf "$NYXID_BASE/api/v1/proxy/s/api-telegram-bot/getMe" \
  -H "X-API-Key: $NYX_API_KEY" | jq .ok
# → true

# Gemini — list models verifies the x-goog-api-key header is injected
curl -sf "$NYXID_BASE/api/v1/proxy/s/llm-google-ai/v1beta/models" \
  -H "X-API-Key: $NYX_API_KEY" | jq '.models | length'
# → integer > 0

# TwitterAPI.io — last_tweets verifies the x-api-key header is injected
# (using elonmusk as a public account; substitute any handle you have access to)
curl -sf "$NYXID_BASE/api/v1/proxy/s/twitterapi-io/twitter/user/last_tweets?userName=elonmusk" \
  -H "X-API-Key: $NYX_API_KEY" | jq '.tweets | length'
# → integer ≥ 0

# Google Sheets — verifies the OAuth bearer is refreshed and injected.
# Replace <SHEET_ID> with a spreadsheet ID your account can read.
curl -sf "$NYXID_BASE/api/v1/proxy/s/api-google/v4/spreadsheets/<SHEET_ID>?fields=spreadsheetId" \
  -H "X-API-Key: $NYX_API_KEY" | jq -r .spreadsheetId
# → "<SHEET_ID>"
```

Any `200 OK` with the expected upstream body confirms NyxID injected the right credential. If a request returns `401`, `403`, or a 5xx, see [Troubleshooting](#troubleshooting).

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `401` from NyxID, body says `Missing API key` / `Invalid API key` | The `X-API-Key` header in n8n is empty or wrong | Re-paste the Agent Key into the `Header Auth` credential |
| `403` from NyxID | The Agent Key is missing the `proxy` scope, or the `Service Scope` card excludes this service | Add `proxy` to the key's scopes, or open the key's `Service Scope` card and either include the service or re-enable `Allow all services` |
| `401` from the upstream API | The downstream credential stored in NyxID is wrong or revoked | `nyxid service rotate-credential <ID>` and re-paste the upstream credential |
| Google Sheets `ACCESS_TOKEN_SCOPE_INSUFFICIENT` | The Sheets scope was passed to the CLI but not declared on the consent screen | Add `https://www.googleapis.com/auth/spreadsheets` to the OAuth consent screen, then redo `nyxid service add api-google --oauth` |
| Google Sheets returns `404 was not found on this server` | Service points at `www.googleapis.com` instead of `sheets.googleapis.com` | `nyxid service update <ID> --endpoint-url "https://sheets.googleapis.com"` |
| Google Sheets returns `429 Too Many Requests` | n8n's default `1 req/500ms` exceeds Google's `60 writes/min/user` quota around row 60 | Set the `HTTP Request` node's `Batch Interval` to ≥ 1200 ms and enable `Retry On Fail` (max 3, wait 30 s) |
| Gemini returns truncated JSON | `thinkingConfig` is consuming `maxOutputTokens` | Set `maxOutputTokens: 65536` and `thinkingConfig.thinkingBudget: 1024` |

## Operational notes

- **Credential rotation.** Run `nyxid service rotate-credential <ID>` to replace any single upstream credential. Workflows continue to use the same proxy URL with no n8n change.
- **Reuse outside n8n.** The same Agent Key works as `X-API-Key` from `curl`, Claude Code, Cursor, or any HTTP client.
- **Audit attribution.** Every proxied request is recorded against the Agent Key in NyxID's audit log. Use one Agent Key per workflow when you need per-workflow attribution.

## Reference

- **Reference walkthrough for the proxy URL** (Web UI / CLI / AI-driven / Direct API): [docs/connecting-services/](../connecting-services/)
- **Per-agent isolation** (one scoped key per agent): [Claude Code & Codex per-agent quickstart](claude-code.md)
- **Reach localhost APIs from a cloud-hosted n8n**: [Node Proxy quickstart](node-proxy.md)
- **Wrap any REST API as MCP tools**: [MCP wrapping quickstart](mcp-wrapping.md)
- **NyxID architecture**: [docs/ARCHITECTURE.md](../ARCHITECTURE.md)
