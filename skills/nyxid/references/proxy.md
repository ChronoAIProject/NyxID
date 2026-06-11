# Make Proxy Requests

## Table of contents

- [How to find the right API paths](#how-to-find-the-right-api-paths)
- [Important: paths are relative to the service's base URL](#important-paths-are-relative-to-the-services-base-url)
- [Making the request](#making-the-request)
- [Calling NyxID from raw HTTP (no CLI)](#calling-nyxid-from-raw-http-no-cli)
- [Common service examples](#common-service-examples)
- [WebSocket-authenticated services](#websocket-authenticated-services)

NyxID proxies requests to downstream services -- it handles authentication, but you need to
know the correct API paths, methods, and body formats for each service.

## How to find the right API paths

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

## Important: paths are relative to the service's base URL

Each service in NyxID has a configured `endpoint_url` (base URL) that may already include
a version prefix. For example, `api-twitter` uses `https://api.x.com/2` as its base URL.
When making a proxy request, the path you provide is appended to that base URL:

- Service base URL: `https://api.x.com/2`
- Your path: `/tweets`
- Actual request: `https://api.x.com/2/tweets`

So do NOT duplicate the version prefix in your path. Check `nyxid service show <id> --output json`
to see the `endpoint_url` if you're unsure.

## Making the request

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

## Calling NyxID from raw HTTP (no CLI)

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

## Anonymous public proxy endpoints

Admins can expose selected catalog-service methods and paths through explicit anonymous endpoint rules. These routes are separate from the authenticated proxy:

| Path | Auth | Notes |
|---|---|---|
| `GET/POST/... /public/s/{slug}/{path}` | None | Only matches enabled `DownstreamService.anonymous_endpoints` rules for the same method and path pattern. |
| `POST /public/mcp` | None | Returns an MCP `tools/list` projection containing only enabled anonymous rules. |

Enabled anonymous rules are valid only when the catalog service has `identity_propagation_mode = "none"`, `forward_access_token = false`, and `inject_delegation_token = false`. Disabled rules are drafts and may be stored before the service is made compatible. Runtime public execution still force-strips identity propagation, access-token forwarding, delegation-token injection, downstream auth defaults, WebSocket upgrades, and NyxID auth/session response headers.

```bash
nyxid public request <slug> /public/a -X GET

curl "https://nyx-api.chrono-ai.fun/public/s/<slug>/public/a"

curl -X POST "https://nyx-api.chrono-ai.fun/public/mcp" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}'
```

Anonymous public routes have their own per-IP limits and smaller request body cap. Configure `PUBLIC_PROXY_RATE_LIMIT_PER_MINUTE`, `PUBLIC_MCP_RATE_LIMIT_PER_MINUTE`, and `PUBLIC_PROXY_MAX_BODY_SIZE` in the backend environment when the defaults are not appropriate.

## Common service examples

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

# AWS Cost Explorer (base: https://ce.us-east-1.amazonaws.com) -- JSON-RPC
# POST to / with X-Amz-Target picking the operation. NyxID injects SigV4
# from the stored access-key JSON. See `nyxid catalog show aws-cost-explorer`
# for IAM requirements (must be a management-account credential).
nyxid proxy request aws-cost-explorer / -m POST \
  -H "Content-Type: application/x-amz-json-1.1" \
  -H "X-Amz-Target: AWSInsightsServiceV20210101.GetCostAndUsage" \
  -d '{"TimePeriod":{"Start":"2026-04-13","End":"2026-05-13"},"Granularity":"DAILY","Metrics":["UnblendedCost"],"GroupBy":[{"Type":"DIMENSION","Key":"LINKED_ACCOUNT"},{"Type":"TAG","Key":"project"}]}'

# Google Cloud APIs use the generic api-google-cloud catalog entry.
# Add one UserService per Google API host; the default OAuth scope is
# https://www.googleapis.com/auth/cloud-platform.read-only.
# NOTE: for BigQuery / Cloud Billing, user OAuth refresh dies every ~16h
# with invalid_rapt (Google session reauth on Cloud scopes). For unattended
# access register a service account instead -- see services.md
# "Cloud-billing services" (nyxid external-key add-gcp-service-account).
nyxid service add api-google-cloud --oauth \
  --endpoint-url https://cloudbilling.googleapis.com \
  --slug google-cloud-billing

# Cloud Billing (base: https://cloudbilling.googleapis.com) -- list
# billing accounts. The API covers billing metadata, not historical
# spend totals.
nyxid proxy request google-cloud-billing /v1/billingAccounts -m GET

nyxid service add api-google-cloud --oauth \
  --endpoint-url https://bigquery.googleapis.com \
  --slug google-bigquery

# BigQuery billing export (base: https://bigquery.googleapis.com) --
# query the billing export dataset for spend-by-project. Replace
# `<PROJECT>` / `<DATASET>` / `<BILLING_ACCOUNT_ID>` with the actual
# values from Google Cloud's billing-export setup.
nyxid proxy request google-bigquery /bigquery/v2/projects/<PROJECT>/queries -m POST \
  -d '{"useLegacySql":false,"query":"SELECT DATE(usage_start_time) AS day, project.id AS project, SUM(cost) - SUM((SELECT IFNULL(SUM(c.amount),0) FROM UNNEST(credits) c)) AS net_cost FROM `<PROJECT>.<DATASET>.gcp_billing_export_v1_<BILLING_ACCOUNT_ID>` WHERE DATE(usage_start_time) >= DATE_SUB(CURRENT_DATE(), INTERVAL 30 DAY) GROUP BY day, project ORDER BY day DESC, net_cost DESC"}'

# Discover all available proxy services
nyxid proxy discover --output json
```

NyxID injects the user's credentials automatically. Do not ask for or log raw downstream credentials.

For AWS Cost Explorer above, identical proxy bodies can be served from
an in-process response cache when operators enable
`CLOUD_RESPONSE_CACHE_TTL_SECS` (default disabled). AWS Cost Explorer
charges $0.01 per paginated request, so the cache can save budget and
latency for polling-style dashboards. Successful (2xx) responses only
-- error responses are passed through so permission-misconfig errors
aren't masked.

## WebSocket-authenticated services

Some protocols (Home Assistant, Discord gateway, MQTT-over-WS, Slack RTM) do not accept the credential on the HTTP upgrade. They complete the upgrade unauthenticated, send a challenge frame, then expect a response frame carrying the credential. NyxID can inject a held credential into that response frame on the client's behalf -- the client never sees the challenge.

From an agent's point of view, this is a normal WS proxy call with only the NyxID bearer:

```python
import asyncio, json, os, websockets

async def main():
    async with websockets.connect(
        "wss://nyx-api.chrono-ai.fun/api/v1/proxy/s/home-assistant/websocket",
        additional_headers={"Authorization": f"Bearer {os.environ['NYXID_ACCESS_TOKEN']}"},
    ) as ws:
        # First visible frame is auth_ok -- the auth_required challenge was
        # consumed by NyxID and never reached this client.
        print(await ws.recv())
        await ws.send(json.dumps({"id": 1, "type": "get_states"}))
        print(await ws.recv())

asyncio.run(main())
```

Or with `websocat`:

```bash
websocat -H="Authorization: Bearer $NYXID_ACCESS_TOKEN" \
  "wss://nyx-api.chrono-ai.fun/api/v1/proxy/s/home-assistant/websocket"
```

**Home Assistant preset.** The admin service-edit dashboard has a one-click Home Assistant preset in the "WebSocket auth frames" section. It installs this rule on the catalog entry so every user whose `UserService` is catalog-backed inherits it automatically:

```json
{
  "trigger": {"json_field_equals": {"path": "$.type", "value": "auth_required"}},
  "template": "{\"type\":\"auth\",\"access_token\":\"${credential}\"}",
  "frame_kind": "text",
  "consume_trigger": true,
  "direction": "downstream"
}
```

Expected on-wire behavior:

```text
Downstream -> NyxID:    {"type":"auth_required","ha_version":"..."}
NyxID -> Downstream:    {"type":"auth","access_token":"<held credential>"}
Downstream -> Client:   {"type":"auth_ok"}
```

With `consume_trigger: true` the client's first visible frame is `auth_ok`, not `auth_required`. The credential substitution for `${credential}` uses the service's held LLAT (or bearer); the client only ever sends its NyxID bearer.

**User-owned custom services: configure via CLI.** Home Assistant is normally
added as a custom endpoint today, so configure the WebSocket auth-frame preset
when creating the user-owned service:

```bash
nyxid service add --custom \
  --slug my-ha \
  --label "Home Assistant" \
  --endpoint-url "https://ha.local:8123/api" \
  --auth-method bearer \
  --auth-key-name Authorization \
  --credential-env HA_TOKEN \
  --ws-frame-preset home-assistant
```

To add or clear the preset on an existing user service:

```bash
nyxid service update "$USER_SERVICE_ID" --ws-frame-preset home-assistant
nyxid service update "$USER_SERVICE_ID" --ws-frame-clear
```

Raw REST uses the user-service update endpoint (the route is `PUT`, not
`PATCH`):

```bash
curl -X PUT "https://nyx-api.chrono-ai.fun/api/v1/user-services/$USER_SERVICE_ID" \
  -H "Authorization: Bearer $ACCESS_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"ws_frame_injections":[{
    "trigger":{"json_field_equals":{"path":"$.type","value":"auth_required"}},
    "template":"{\"type\":\"auth\",\"access_token\":\"${credential}\"}",
    "frame_kind":"text",
    "consume_trigger":true,
    "direction":"downstream"
  }]}'
```

**Platform operators:** configure catalog defaults via the admin dashboard or
`PUT /api/v1/services/{service_id}`. A non-empty user-owned
`UserService.ws_frame_injections` list overrides catalog rules; an empty user
list falls back to catalog rules for catalog-backed services.

**Other WS protocols.** The same pattern covers any challenge/response post-upgrade auth. Only Home Assistant has a built-in preset today; others need the rule hand-written.

| Protocol | Challenge shape | Response template |
|----------|-----------------|-------------------|
| Home Assistant | `{"type":"auth_required"}` text frame | `{"type":"auth","access_token":"${credential}"}` |
| Discord gateway | op:10 Hello text frame | op:2 IDENTIFY with the bot token |
| MQTT-over-WS CONNECT | binary CONNECT packet | binary CONNECT with username/password |

Binary frame triggers are supported via `"frame_kind": "binary"`, but `json_field_equals` only evaluates text frames -- use `first_frame_from_downstream` or `frame_index_from_downstream` for binary protocols.

**Limits.** Max 4 rules per service, 4 KB per template, 8 injections per WS connection. `${credential}` is the only supported template interpolation. Credentials never appear in logs, errors, or audit payloads -- the proxy only records a 12-hex-char SHA-256 prefix for correlation. Successful injection emits the metadata-only audit event `ws_frame_auth_injected`. See `docs/WS_FRAME_INJECTION.md` for the full schema.
