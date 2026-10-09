# PostHog OAuth connections

NyxID seeds two private-API connections. Choose the cloud region where the
PostHog account lives; an EU token must not be sent to the US API.

| Region | Provider | Catalog service | API base URL |
| --- | --- | --- | --- |
| US | `posthog` | `api-posthog` | `https://us.posthog.com` |
| EU | `posthog-eu` | `api-posthog-eu` | `https://eu.posthog.com` |

These are user-owned OAuth connections, independent of NyxID's PostHog
telemetry configuration. They use authorization code + S256 PKCE, form-encoded
`client_secret_post` token exchange, encrypted token storage, refresh, and
best-effort RFC 7009 revocation through the existing provider flow.

## Register the OAuth client

PostHog recommends Client ID Metadata Documents (CIMD) for new integrations.
NyxID's managed OAuth setup currently requires a client ID and secret, so this
connector uses PostHog's confidential dynamic client registration instead.
Do not paste a CIMD URL into this connector or invent a client secret.

Register once per region and NyxID deployment. For US Cloud:

```sh
curl --fail-with-body https://us.posthog.com/oauth/register/ \
  -H 'Content-Type: application/json' \
  --data '{
    "client_name": "NyxID",
    "redirect_uris": ["https://YOUR-NYXID-BACKEND/api/v1/providers/callback"],
    "grant_types": ["authorization_code", "refresh_token"],
    "response_types": ["code"],
    "token_endpoint_auth_method": "client_secret_post",
    "scope": "project:read insight:read dashboard:read feature_flag:read query:read"
  }'
```

Replace the callback with the backend's `BASE_URL` plus
`/api/v1/providers/callback`. The redirect must match exactly. For EU Cloud,
use `https://eu.posthog.com/oauth/register/` and configure `posthog-eu`.
Local development can register `http://localhost:3001/api/v1/providers/callback`.
The response contains `client_id` and `client_secret`; save both securely.
Registration creates an external OAuth application and is an operator setup
step, not something NyxID repeats at startup.

## Configure and connect

1. Deploy the change and restart NyxID to seed the providers and services.
2. In **Providers > Manage Providers**, edit **PostHog (US)** or **PostHog (EU)**.
   Enter the registered client ID and secret. Keep PKCE enabled, form token
   encoding, and `client_secret_post`. Use **Admin Only** for managed-only
   access, or the default **Admin or User** to also permit custom clients.
3. In **AI Services > Connect Service**, choose the matching PostHog region,
   select **NyxID managed**, and authorize a PostHog account and its projects.
4. Use the connection's NyxID proxy URL and NyxID key to call PostHog paths.
   NyxID injects the user's PostHog access token as a Bearer credential.

The managed client permits `project:read`, `insight:read`, `dashboard:read`,
`feature_flag:read`, and `query:read`. It requests no identity scopes or write
permissions. A user may register their own client with additional scopes and
use the custom-app flow; reauthorize to change permissions. The registration's
scope ceiling must include the requested scopes.

## Verify

Using the connection's proxy URL as the base:

- `GET /api/projects/` lists accessible projects.
- `GET /api/projects/{project_id}/insights/` reads saved insights.
- `GET /api/projects/{project_id}/dashboards/` reads dashboards.
- `GET /api/projects/{project_id}/feature_flags/` reads flag configuration.
- `POST /api/projects/{project_id}/query/` with
  `{"query":{"kind":"HogQLQuery","query":"SELECT event, count() FROM events GROUP BY event LIMIT 10"}}`
  runs an analytics query. This POST reads analytics; it needs `query:read`.

Access depends on the projects and scopes granted at consent. A 403 can mean
missing scope or project access. This connector does not send ingestion events
and does not provision concrete MCP operations from a curated OpenAPI overlay;
it uses NyxID's existing generic proxy discovery.

## References

Protocol configuration checked on 2026-10-07:

- [PostHog OAuth integration](https://posthog.com/docs/api/oauth)
- [US authorization server metadata](https://us.posthog.com/.well-known/oauth-authorization-server)
- [EU authorization server metadata](https://eu.posthog.com/.well-known/oauth-authorization-server)
- [PostHog confidential dynamic registration implementation](https://github.com/PostHog/posthog/blob/master/posthog/api/oauth/dcr.py)
- [PostHog API overview](https://posthog.com/docs/api)
