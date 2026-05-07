# Direct API — Connect a Service via curl / HTTP

For automation environments where a CLI dependency is awkward: n8n, Zapier, CI/CD, custom scripts. Same endpoints `nyxid service add` and `nyxid proxy request` call under the hood.

For Web UI / CLI / AI-driven, see the [hub](README.md).

## Get a NyxID Agent Key

The recommended auth method for unattended automation is `X-API-Key`. Generate one in the web console:

1. Sign in.
2. Open `AI Services`, switch to the `Agent Keys` tab.
3. Click `Create API Key`, give it a name.
4. Under `Scopes`, select `proxy` (required for `/api/v1/proxy/...` — without it the proxy returns 403). If your script will also create / update / delete services via `POST /api/v1/keys`, add `write` as well — the management routes gate on `write` or `admin` only (the `services:write` badge is valid as a scope but is **not** honored by the management write check, so don't use it on its own).
5. Click `Create` and copy the raw key (starts with `nyx_...`). It's shown once.

Set it in your shell. `<BASE_URL>` is `https://nyx-api.chrono-ai.fun` for hosted, `http://localhost:3001` for self-host:

```bash
export NYX_API_KEY=nyx_...
export NYXID_BASE=<BASE_URL>
```

> **Windows users:** Run these from a WSL Ubuntu shell — see [Windows setup](../../README.md#windows-setup) in the main README. Native PowerShell and CMD are not supported.

## Connect and verify

`<EXTERNAL_CREDENTIAL>` below is the **provider's** key (e.g. an OpenAI `sk-...` key), **not** your `NYX_API_KEY`.

Connect a service from the catalog:

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

Verify the proxy. It should return a real OpenAI models response:

```bash
curl -X GET "$NYXID_BASE/api/v1/proxy/s/llm-openai/models" \
  -H "X-API-Key: $NYX_API_KEY"
```

If step 2 returns a real downstream response (not a 401 / 403), the service is connected.

## Bearer token alternative

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

curl -X GET "$NYXID_BASE/api/v1/proxy/s/llm-openai/models" \
  -H "Authorization: Bearer $NYX_TOKEN"
```

Bearer tokens expire (default 15 min). Prefer `X-API-Key` for anything unattended.

## Listing the catalog

List connectable catalog entries:

```bash
curl "$NYXID_BASE/api/v1/catalog" -H "X-API-Key: $NYX_API_KEY"
```

Include system services too:

```bash
curl "$NYXID_BASE/api/v1/catalog?include_all=true" -H "X-API-Key: $NYX_API_KEY"
```

List parsed OpenAPI endpoints for a slug:

```bash
curl "$NYXID_BASE/api/v1/catalog/llm-openai/endpoints" -H "X-API-Key: $NYX_API_KEY"
```

## Next

- **Click-through equivalent:** [Web UI](web-ui.md).
- **Same flow with the `nyxid` CLI:** [CLI](cli.md).
- **Full endpoint reference:** [docs/API.md](../API.md).
