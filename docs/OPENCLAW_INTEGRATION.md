# OpenClaw + NyxID

This repository ships an OpenClaw integration at [`integrations/openclaw`](../integrations/openclaw).

## Included Assets

- `openclaw-plugin-nyxid`: TypeScript auth plugin for OpenClaw
- `skills/nyxid`: ClawHub skill bundle with helper shell scripts
- `openclaw.plugin.json`: OpenClaw auth-plugin manifest with bundled skill reference

## Default Hosted Configuration

The hosted NyxID base URL for the OpenClaw integration is:

`https://nyx-api.chrono-ai.fun`

Users can install the published skill from ClawHub with:

```bash
clawhub install nyxid
```

## Configuration

```json
{
  "plugins": {
    "nyxid": {
      "enabled": true,
      "baseUrl": "https://nyx-api.chrono-ai.fun",
      "clientId": "your-client-id",
      "clientSecret": "your-client-secret",
      "defaultScopes": "openid profile email",
      "delegationScopes": "proxy:*",
      "apiKey": "optional-nyxid-api-key"
    }
  }
}
```

## Auth Modes

### OAuth mode

Use `clientId` with `baseUrl` to let OpenClaw connect the user's NyxID account through OAuth 2.0 Authorization Code + PKCE.

Add `clientSecret` when the OpenClaw install should perform RFC 8693 token exchange for delegated proxy calls.

### API key mode

Use one of:

- `plugins.nyxid.apiKey`
- `NYXID_API_KEY`

This mode is intended for self-hosted or pre-provisioned installs that do not want an interactive OAuth flow. The plugin sends direct NyxID requests with `X-API-Key`.

You can also provide `NYXID_ACCESS_TOKEN` when an installation already has a NyxID bearer token.

## Flow Summary

### OAuth flow

1. OpenClaw authenticates the user with NyxID using OAuth 2.0 Authorization Code + PKCE.
2. The plugin stores the NyxID access token and refresh token in the OpenClaw auth profile.
3. `nyxid_list_services` calls `GET /api/v1/proxy/services` with the user access token.
4. `nyxid_proxy` exchanges the user access token for a short-lived delegated token using RFC 8693.
5. NyxID injects the user's downstream credentials when the proxy request is forwarded.

### API key flow

1. OpenClaw loads `NYXID_API_KEY` or `plugins.nyxid.apiKey`.
2. `nyxid_list_services` calls `GET /api/v1/proxy/services` with `X-API-Key`.
3. `nyxid_proxy` calls the slug-based proxy endpoint directly with the same API key.
4. NyxID injects the user's downstream credentials when the proxy request is forwarded.

## Current Backend Constraints

- RFC 8693 token exchange requires a confidential NyxID OAuth client, so `clientSecret` is required for delegated proxy usage.
- Delegated NyxID tokens cannot call `GET /api/v1/proxy/services`; service discovery must use the base user token or API key.
- Approval-gated proxy calls are blocking and end in success or `403 Forbidden`.
- The bundled skill helper scripts accept either `NYXID_ACCESS_TOKEN` or `NYXID_API_KEY`.
