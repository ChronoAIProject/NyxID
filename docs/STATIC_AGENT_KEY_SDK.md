# Static browser Agent Key SDK

A static site can use an approved child Agent Key without a site backend. The NyxID operator opts in each exact site origin with `STATIC_AGENT_KEY_ORIGINS`. The browser stores the delivered child credential locally and sends it as an explicit Bearer credential.

## Configuration

Set a comma-separated list of exact HTTP(S) origins on every NyxID replica. Do not include the NyxID frontend, API, domain aliases, or another origin already listed in `CORS_ALLOWED_ORIGINS` or `CSRF_TRUSTED_ORIGINS`. Entries with paths, credentials, queries, fragments, wildcards, or unsupported schemes are ignored.

```dotenv
STATIC_AGENT_KEY_ORIGINS=https://static.example
```

The allowlist is credential-free. Browsers cannot use cookies on this surface, and responses never include `Access-Control-Allow-Credentials`.

## Browser routes

The static origin receives CORS for these exact route classes.

- `POST /api/v1/auth/agent-key/request`
- `POST /api/v1/auth/agent-key/poll`
- `GET /api/v1/auth/agent-key/self`
- `DELETE /api/v1/auth/agent-key/self`
- `POST /api/v1/proxy/s/llm-nyx/v1/responses`
- `GET /api/v1/proxy/s/llm-nyx/v1/responses/{response_id}`

The request and poll exchange retains its existing one-time code and rate limits. Self metadata and revocation retain the live child credential check. Nyx Responses retains the normal proxy service scope, limits, idempotency, streaming, and recovery behavior. `Idempotency-Key` is allowed for send.

Approval remains a first-party human flow. The static site cannot approve, deny, mint, or manage another credential, and it cannot use CORS to reach other private API or proxy routes.

`OPTIONS` preflights must request one of the route methods and only the `Accept`, `Authorization`, `Content-Type`, and `Idempotency-Key` headers.

## Rollout

Deploy the configuration before publishing the static site. Verify one approved origin and one lookalike origin with preflight and actual requests. Verify that a reload can use the locally stored credential, self DELETE revokes it, and a subsequent self or Responses request fails through the existing authorization path. Public deployment and browser integration remain operator-owned checks.
