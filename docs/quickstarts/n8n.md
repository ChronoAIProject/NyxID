# n8n: One Credential for Every Upstream API

Configure n8n to call any number of upstream APIs through a single NyxID **Header Auth** credential. NyxID injects the per-service authentication (API keys, OAuth tokens, path-based tokens) at proxy time; the n8n credential never carries a downstream secret.

```
n8n  ─┐
curl ─┼──(X-API-Key: nyx_…)──►  NyxID Proxy  ──►  Gemini · TwitterAPI.io · Google Sheets · Telegram · …
agent─┘                                            (NyxID injects the right auth per service)
```

This guide uses four upstream APIs as worked examples — Gemini (header auth), TwitterAPI.io (custom header), Google Sheets (OAuth2), Telegram Bot (path-based auth). The same flow works for any HTTP API.

## Prerequisites

- A NyxID account and an Agent Key with the `proxy` scope. If you don't have these yet, complete **Step 0** below; otherwise jump to the [Procedure](#procedure).
- An n8n instance (cloud or self-hosted) where you can add a credential and HTTP Request nodes.
- The downstream API tokens you intend to register (Gemini key, Telegram bot token, etc.) saved locally with `chmod 600` permissions.

### Step 0 — Get NyxID running and create an Agent Key

**Hosted (recommended).** Sign up at [nyx.chrono-ai.fun/register](https://nyx.chrono-ai.fun/register) using the invite code in the [README Setup](../../README.md#setup). After signing in, open **AI Services → Agent Keys → Create API Key**, select the `proxy` scope, click **Create**, and save the displayed `nyx_…` value (shown once).

**Self-host.** Follow [docs/SETUP.md](../SETUP.md) to bring up the Docker stack, register at `http://localhost:3000`, then create the Agent Key via the same web console flow.

Save the Agent Key to `~/.nyx_key` with `chmod 600`. The procedure below assumes it lives there.

## Procedure

### 1. Register each upstream service in NyxID

For services in NyxID's catalog, pass the catalog slug. For services not in the catalog, register a custom service with explicit auth metadata.

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

Each `nyxid service add` prints the user-side slug NyxID landed on. On a fresh account this matches the catalog or `--slug` value; if you already have a service with that slug, NyxID appends `-2`, `-3`, or a random suffix to keep slugs unique. Record the printed slugs — you need them in the n8n HTTP Request URLs.

### 2. Register an OAuth service (Google Sheets)

OAuth services need an OAuth client (created in the upstream provider's developer console) before NyxID can run the consent flow.

1. In Google Cloud Console, create an **OAuth 2.0 Client ID** (Web application) and add `https://<your-nyxid-host>/api/v1/providers/callback` to **Authorized redirect URIs**.
2. Enable the **Google Sheets API** and **Google Drive API**.
3. On the **OAuth consent screen**, open **Data access → Add or remove scopes** and add `https://www.googleapis.com/auth/spreadsheets`. NyxID's `--scope` flag only takes effect for scopes already declared on the consent screen.
4. Add your Google account to **Test users**.
5. Save the Client ID to `~/.gc_id` and the Client Secret to `~/.gc_secret` (`chmod 600` both).

Configure the OAuth client on the NyxID `api-google` provider, then run the OAuth flow:

```bash
GC_ID="$(cat ~/.gc_id)" GC_SECRET="$(cat ~/.gc_secret)" \
  nyxid service credentials api-google \
  --client-id-env GC_ID \
  --client-secret-env GC_SECRET

nyxid service add api-google \
  --oauth \
  --scope "https://www.googleapis.com/auth/spreadsheets" \
  --label "Google Sheets"
```

The CLI prints an authorization URL. Open it, sign in to Google, and approve the consent screen. NyxID stores the resulting refresh token and refreshes the access token automatically on every proxied call.

Google Sheets lives on `sheets.googleapis.com`, but the catalog default for `api-google` is `www.googleapis.com`. Override the endpoint URL on the registered service:

```bash
# Replace <ID> with the value printed by `nyxid service add` (or `nyxid service list`).
nyxid service update <ID> --endpoint-url "https://sheets.googleapis.com"
```

### 3. Create the n8n credential

Generate an Agent Key reserved for n8n and write it to a temp file. The Agent Key is then pasted into n8n's Header Auth credential.

```bash
nyxid api-key create \
  --name "n8n" \
  --scopes "proxy" \
  --output json \
| jq -r '.full_key' > ~/.nyx_key && chmod 600 ~/.nyx_key
```

> The JSON field for the key value is `full_key` (not `key`, `api_key`, `value`, or `token`).

In n8n, open **Credentials → New → Header Auth** and set:

| Field | Value |
|---|---|
| Name | `NyxID API Key` |
| Header Name | `X-API-Key` |
| Header Value | The `nyx_…` key (paste from clipboard) |

After saving the credential, securely delete the temp file:

```bash
shred -u ~/.nyx_key   # Linux
rm -P ~/.nyx_key      # macOS
```

For per-workflow blast-radius isolation, scope the key to only the four services it should reach: open **AI Services → Agent Keys → \[your key\]** in the web console, locate the **Service Scope** card, uncheck **Allow all services**, select the registered services, and save.

### 4. Configure HTTP Request nodes in n8n

Every HTTP Request node uses the same credential (`NyxID API Key`) and the same proxy URL pattern:

```
https://<nyxid-host>/api/v1/proxy/s/<service-slug>/<downstream-api-path>
```

Substitute the slug NyxID returned in Step 1 and the downstream API path you would normally use without NyxID.

| Upstream | Method + Path |
|---|---|
| Gemini | `POST /api/v1/proxy/s/llm-google-ai/models/gemini-2.5-flash:generateContent` |
| TwitterAPI.io | `GET  /api/v1/proxy/s/twitterapi-io/twitter/user/last_tweets?userName=…` |
| Google Sheets | `POST /api/v1/proxy/s/api-google/v4/spreadsheets/<sheet-id>/values/Sheet1!A:H:append` |
| Telegram Bot | `POST /api/v1/proxy/s/api-telegram-bot/sendMessage` |

NyxID injects the correct authentication for each service: `x-goog-api-key` for Gemini, `x-api-key` for TwitterAPI.io, refreshed `Authorization: Bearer …` for Google Sheets, and the path-based `/bot<token>/` prefix for Telegram. The n8n node never sees any of these.

## Verification

From your terminal, send one request through each service to confirm the proxy is wired correctly before invoking it from n8n:

```bash
NYX_API_KEY="$(cat ~/.nyx_key 2>/dev/null || echo nyx_…)"
NYXID_BASE="https://<your-nyxid-host>"   # or http://localhost:3001 for self-host

# Telegram getMe — succeeds when the bot token is injected correctly
curl -sf "$NYXID_BASE/api/v1/proxy/s/api-telegram-bot/getMe" \
  -H "X-API-Key: $NYX_API_KEY" | jq .ok
# → true
```

Any `200 OK` with the expected upstream body confirms NyxID injected the right credential. If you see `401`, `403`, or a 5xx, see [Troubleshooting](#troubleshooting).

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `401` from NyxID, body says `Missing API key` / `Invalid API key` | The `X-API-Key` header in n8n is empty or wrong | Re-paste the Agent Key into the Header Auth credential |
| `403` from NyxID | The Agent Key is missing the `proxy` scope, or the **Service Scope** card excludes this service | Add `proxy` to the key's scopes, or open the key's **Service Scope** card and either include the service or re-enable **Allow all services** |
| `401` from the upstream API | The downstream credential stored in NyxID is wrong or revoked | `nyxid service rotate-credential <ID>` and re-paste the upstream credential |
| Google Sheets `ACCESS_TOKEN_SCOPE_INSUFFICIENT` | The Sheets scope was passed to the CLI but not declared on the consent screen | Add `https://www.googleapis.com/auth/spreadsheets` to the OAuth consent screen, then redo `nyxid service add api-google --oauth` |
| Google Sheets returns `404 was not found on this server` | Service points at `www.googleapis.com` instead of `sheets.googleapis.com` | `nyxid service update <ID> --endpoint-url "https://sheets.googleapis.com"` |
| Google Sheets returns `429 Too Many Requests` | n8n's default `1 req/500ms` exceeds Google's `60 writes/min/user` quota around row 60 | Set the HTTP node's **Batch Interval** to ≥ 1200 ms and enable **Retry On Fail** (max 3, wait 30 s) |
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
