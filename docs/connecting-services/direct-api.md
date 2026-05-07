# Direct API — Connect a Service via curl / HTTP

Four numbered steps. End state: an `HTTP/1.1 200` response from your first proxied call. For automation environments where a CLI dependency is awkward: n8n, Zapier, CI/CD, custom scripts. Same endpoints `nyxid service add` and `nyxid proxy request` call under the hood.

For Web UI / CLI / AI-driven, see the [hub](README.md).

## Get a NyxID Agent Key

The recommended auth method for unattended automation is `X-API-Key`. Generate one in the web console:

1. Sign in.
2. Open `AI Services`, switch to the `Agent Keys` tab.
3. Click `Create API Key`, give it a name.
4. Under `Scopes`, select `proxy` (required for `/api/v1/proxy/...` — without it the proxy returns 403). If your script will also create / update / delete services via `POST /api/v1/keys`, add `write` as well — the management routes gate on `write` or `admin` only (the `services:write` badge is valid as a scope but is **not** honored by the management write check, so don't use it on its own).
5. Click `Create` and copy the raw key (starts with `nyx_...`). It's shown once.

`<BASE_URL>` in the steps below is `https://nyx-api.chrono-ai.fun` for hosted, `http://localhost:3001` for self-host.

> **Windows users:** Run every command on this page from a Unix-compatible shell — WSL Ubuntu (recommended) or Git Bash both work. See [docs/WINDOWS_SETUP.md](../WINDOWS_SETUP.md) for the one-time setup.

## Connect and verify

`<EXTERNAL_CREDENTIAL>` below is the **provider's** key (e.g. an OpenAI `sk-...` key), **not** your `NYX_API_KEY`.

### Step 1 — Set shell variables

```bash
export NYX_API_KEY=nyx_...
export NYXID_BASE=<BASE_URL>
```

### Step 2 — Connect the service

```bash
curl -X POST "$NYXID_BASE/api/v1/keys" \
  -H "X-API-Key: $NYX_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "service_slug": "llm-openai",
    "credential": "<EXTERNAL_CREDENTIAL>",
    "label": "production-openai"
  }'
```

### Step 3 — Copy the returned slug

The response includes a top-level `slug` field. If `llm-openai` already existed on your account, the new entry may be suffixed (e.g. `llm-openai-2`, `llm-openai-3`, or a random-suffixed variant once the numeric range is exhausted). **Use that exact value in Step 4** — it is the only handle that addresses your specific service instance.

Example response excerpt:

```json
{
  "id": "...",
  "label": "production-openai",
  "slug": "llm-openai-2",
  "status": "connected",
  ...
}
```

### Step 4 — Verify with a proxied request

Substitute `<RETURNED_SERVICE_SLUG>` with the `slug` value you copied in Step 3.

```bash
curl -X GET "$NYXID_BASE/api/v1/proxy/s/<RETURNED_SERVICE_SLUG>/models" \
  -H "X-API-Key: $NYX_API_KEY"
```

Success looks like a real downstream response (for OpenAI's `models` endpoint, a JSON list of models). If you see `401`, `403`, `5xx`, or an HTML error page instead, see [Did it work?](README.md#did-it-work) in the hub.

You're done with the required path. The sections below are **optional** — skip them unless you need them.

## Optional — Bearer token alternative

If you specifically need `Authorization: Bearer ...` (short-lived user-session token, not recommended for unattended automation):

```bash
export NYX_TOKEN="$(
  curl -sS -X POST "$NYXID_BASE/api/v1/auth/login" \
    -H "Content-Type: application/json" \
    -d '{
      "email": "you@example.com",
      "password": "your-password",
      "client": "token"
    }' \
  | jq -r '.access_token'
)"

curl -X GET "$NYXID_BASE/api/v1/proxy/s/<RETURNED_SERVICE_SLUG>/models" \
  -H "Authorization: Bearer $NYX_TOKEN"
```

Bearer tokens expire (default 15 min). Prefer `X-API-Key` for anything unattended.

## Optional — Browse the catalog

List connectable catalog entries:

```bash
curl "$NYXID_BASE/api/v1/catalog" -H "X-API-Key: $NYX_API_KEY"
```

Include system services too:

```bash
curl "$NYXID_BASE/api/v1/catalog?include_all=true" -H "X-API-Key: $NYX_API_KEY"
```

List parsed OpenAPI endpoints for a slug. This returns an empty list when the catalog entry has no parsed OpenAPI spec — which can happen for `llm-openai`. Try a slug whose catalog entry advertises an `openapi_spec_url` (visible via `GET /api/v1/catalog/<SLUG>`) if you want to see structured endpoint output:

```bash
curl "$NYXID_BASE/api/v1/catalog/<SLUG>/endpoints" -H "X-API-Key: $NYX_API_KEY"
```

## Next

- **Click-through equivalent:** [Web UI](web-ui.md).
- **Same flow with the `nyxid` CLI:** [CLI](cli.md).
- **Full endpoint reference:** [docs/API.md](../API.md).
