# OpenClaw + NyxID

This repository ships an OpenClaw integration at [`integrations/openclaw`](../integrations/openclaw).

## Included Assets

- `openclaw-plugin-nyxid`: TypeScript auth plugin for OpenClaw
- `skills/nyxid`: ClawHub skill bundle with helper shell scripts

## Configuration

```json
{
  "plugins": {
    "nyxid": {
      "enabled": true,
      "baseUrl": "https://auth.nyxid.dev",
      "clientId": "your-client-id",
      "clientSecret": "your-client-secret",
      "defaultScopes": "openid profile email",
      "delegationScopes": "proxy:*"
    }
  }
}
```

## Flow Summary

1. OpenClaw authenticates the user with NyxID using OAuth 2.0 Authorization Code + PKCE.
2. The plugin stores the NyxID access token and refresh token in the OpenClaw auth profile.
3. `nyxid_list_services` calls `GET /api/v1/proxy/services` with the user access token.
4. `nyxid_proxy` exchanges the user access token for a short-lived delegated token using RFC 8693.
5. NyxID injects the user's downstream credentials when the proxy request is forwarded.

## Current Backend Constraints

- RFC 8693 token exchange requires a confidential NyxID OAuth client, so `clientSecret` is required for delegated proxy usage.
- Delegated NyxID tokens cannot call `GET /api/v1/proxy/services`; only the user access token can.
- Approval-gated proxy calls are blocking and end in success or `403 Forbidden`.
