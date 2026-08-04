---
title: Register a developer app (OAuth client)
description: Register an OAuth 2.0 / OIDC client in NyxID so your application can use NyxID as an identity provider for "Sign in with NyxID" flows.
---

NyxID is a full OpenID Connect 1.0 identity provider. You can register a developer app (OAuth client) to add "Sign in with NyxID" to any web app, mobile app, or service. This is the same mechanism used by social login flows — your app gets a `client_id` and optionally a `client_secret`, and users authenticate through NyxID's hosted login page.

For the full OIDC specification support and token details, see [OAuth / OIDC concepts](/docs/shared/concepts/oauth-oidc).

## When to register a developer app

Register a developer app when your application needs to:

- Authenticate users with NyxID as the identity provider
- Receive an `id_token` (OpenID Connect) with user profile claims
- Issue short-lived access tokens your backend can verify
- Use the NyxID MCP proxy on behalf of a user (delegated access)

If you only need server-to-server access with no user login, use a service account instead (Service Accounts in the admin panel).

## Register an app

1. Go to **Developer** in the left sidebar.
2. Click **New application**.
3. Fill in the fields:

| Field | Description |
|---|---|
| **App name** | Display name shown on the consent screen. |
| **Redirect URIs** | One or more callback URLs where NyxID will send the authorization code after login. Must be exact matches. |
| **Allowed scopes** | Space-separated scopes this client can request: `openid`, `profile`, `email`, `roles`, `groups`. |
| **Client type** | **Confidential** (server-side app that can keep a secret) or **Public** (SPA, mobile, CLI). |

4. Click **Create**.

NyxID generates a `client_id`. If you selected **Confidential**, it also shows a `client_secret` **once** — copy it immediately.

:::warning
The `client_secret` is shown only at creation time. If you lose it, rotate the app from its detail page to get a new secret.
:::

## The authorization code flow (PKCE required)

NyxID requires PKCE for all flows. There is no implicit grant and no password grant.

### 1. Generate PKCE parameters

```bash
CODE_VERIFIER=$(openssl rand -base64 32 | tr -d '=' | tr '+/' '-_')
CODE_CHALLENGE=$(echo -n "$CODE_VERIFIER" | openssl dgst -sha256 -binary | openssl base64 | tr -d '=' | tr '+/' '-_')
```

### 2. Redirect the user

```
GET https://nyx.chrono-ai.fun/oauth/authorize
  ?response_type=code
  &client_id=YOUR_CLIENT_ID
  &redirect_uri=https://app.example.com/callback
  &scope=openid profile email
  &code_challenge=CODE_CHALLENGE
  &code_challenge_method=S256
  &state=RANDOM_STATE
  &nonce=RANDOM_NONCE
```

The user authenticates on NyxID's login page. NyxID redirects to your `redirect_uri` with an authorization `code`.

### 3. Exchange code for tokens

```bash
curl -X POST https://nyx.chrono-ai.fun/oauth/token \
  -H "Content-Type: application/x-www-form-urlencoded" \
  -d "grant_type=authorization_code" \
  -d "code=AUTH_CODE" \
  -d "redirect_uri=https://app.example.com/callback" \
  -d "client_id=YOUR_CLIENT_ID" \
  -d "client_secret=YOUR_CLIENT_SECRET" \
  -d "code_verifier=CODE_VERIFIER"
```

Response:

```json
{
  "access_token": "eyJhbGciOiJSUzI1NiIs...",
  "refresh_token": "eyJhbGciOiJSUzI1NiIs...",
  "id_token": "eyJhbGciOiJSUzI1NiIs...",
  "token_type": "Bearer",
  "expires_in": 900,
  "scope": "openid profile email"
}
```

The `id_token` is an RS256-signed JWT containing user claims (`sub`, `email`, `name`, `picture`). The access token expires after 15 minutes; use the refresh token to get new ones.

### 4. Verify tokens

All tokens are RS256-signed. Fetch the public keys from:

```
GET https://nyx.chrono-ai.fun/.well-known/jwks.json
```

Match the `kid` in the token header to a key in the response, then verify the RS256 signature. Most OIDC libraries do this automatically when you provide the issuer URL.

## Auto-discovery

NyxID publishes a standard OIDC discovery document. Most OIDC libraries only need the issuer URL:

```
https://nyx.chrono-ai.fun
```

The library fetches `/.well-known/openid-configuration` and discovers all endpoints automatically.

## Redirect URI types

NyxID accepts the following redirect URI forms:

- Standard HTTPS URLs: `https://app.example.com/callback`
- Loopback (RFC 8252): `http://127.0.0.1:*`, `http://localhost:*`
- Private-use URI schemes (native apps, CLI tools): `cursor://`, `vscode://`, your app's custom scheme

## Configure a framework

### NextAuth.js

```javascript
import NextAuth from "next-auth";

export default NextAuth({
  providers: [
    {
      id: "nyxid",
      name: "NyxID",
      type: "oidc",
      issuer: process.env.NYXID_ISSUER, // https://nyx.chrono-ai.fun
      clientId: process.env.NYXID_CLIENT_ID,
      clientSecret: process.env.NYXID_CLIENT_SECRET,
    },
  ],
});
```

### openid-client (Node.js)

```javascript
import { Issuer } from "openid-client";

const nyxid = await Issuer.discover("https://nyx.chrono-ai.fun");
const client = new nyxid.Client({
  client_id: process.env.NYXID_CLIENT_ID,
  client_secret: process.env.NYXID_CLIENT_SECRET,
  redirect_uris: ["https://app.example.com/callback"],
  response_types: ["code"],
});
```

## Manage an existing app

From **Developer**, click an app to open its detail page. You can:

- **Add or remove redirect URIs** — changes take effect immediately
- **Rotate the client secret** — the old secret is invalidated; update all deployments before rotating
- **Delete the app** — revokes all tokens issued to this client

## Org-owned apps

A developer app can be owned by an org so all org admins can manage it. When creating the app, select the org in the **Owner** field. See [Organizations](/docs/web/guides/organizations) for the org model.
