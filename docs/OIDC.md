# NyxID OIDC Integration Guide

NyxID is a full OpenID Connect (OIDC) 1.0 identity provider. This guide covers
how to configure NyxID as an OIDC provider and integrate it with relying parties.

## Table of Contents

- [Issuer URL](#issuer-url)
- [Discovery Endpoints](#discovery-endpoints)
- [Authorization Code Flow with PKCE](#authorization-code-flow-with-pkce)
- [Token Types](#token-types)
- [Scopes and Claims](#scopes-and-claims)
- [JWKS and Signature Verification](#jwks-and-signature-verification)
- [Token Introspection](#token-introspection)
- [Token Revocation](#token-revocation)
- [Dynamic Client Registration](#dynamic-client-registration)
- [Configuration Examples](#configuration-examples)
- [MCP Client Integration](#mcp-client-integration)
- [Troubleshooting](#troubleshooting)
- [Environment Variables](#environment-variables)

## Supported Specifications

| Spec | Description |
|------|-------------|
| OpenID Connect Core 1.0 | ID tokens, UserInfo, standard claims |
| OpenID Connect Discovery 1.0 | `/.well-known/openid-configuration` |
| RFC 8414 | OAuth 2.0 Authorization Server Metadata |
| RFC 7636 | PKCE (Proof Key for Code Exchange) -- **required** |
| RFC 7662 | Token Introspection |
| RFC 7009 | Token Revocation |
| RFC 7591 | Dynamic Client Registration |
| RFC 8693 | Token Exchange (delegated access) |
| RFC 9728 | OAuth 2.0 Protected Resource Metadata |

---

## Issuer URL

The OIDC issuer is your NyxID `BASE_URL`:

```
# Development
http://localhost:3001

# Production
https://auth.example.com
```

The issuer appears in:

- The `issuer` field of `/.well-known/openid-configuration`
- The `iss` claim of all tokens (access, refresh, ID)
- The discovery URL pattern: `{issuer}/.well-known/openid-configuration`

By default, `JWT_ISSUER` is set to `BASE_URL`. You can override it via the
`JWT_ISSUER` environment variable, but this is usually unnecessary.

When configuring a relying party, set the **issuer** or **provider URL** to
your NyxID `BASE_URL`. The relying party will auto-discover all endpoints via
the standard discovery document.

---

## Discovery Endpoints

### OpenID Connect Discovery

```
GET /.well-known/openid-configuration
```

Returns the OIDC provider metadata. Example response:

```json
{
  "issuer": "https://auth.example.com",
  "authorization_endpoint": "https://auth.example.com/oauth/authorize",
  "token_endpoint": "https://auth.example.com/oauth/token",
  "userinfo_endpoint": "https://auth.example.com/oauth/userinfo",
  "jwks_uri": "https://auth.example.com/.well-known/jwks.json",
  "introspection_endpoint": "https://auth.example.com/oauth/introspect",
  "revocation_endpoint": "https://auth.example.com/oauth/revoke",
  "response_types_supported": ["code"],
  "grant_types_supported": ["authorization_code", "refresh_token"],
  "subject_types_supported": ["public"],
  "id_token_signing_alg_values_supported": ["RS256"],
  "scopes_supported": ["openid", "profile", "email", "roles", "groups"],
  "claims_supported": [
    "sub", "iss", "aud", "exp", "iat", "email", "email_verified",
    "name", "picture", "nonce", "at_hash", "roles", "groups",
    "permissions", "acr", "amr", "auth_time", "sid"
  ],
  "code_challenge_methods_supported": ["S256"],
  "token_endpoint_auth_methods_supported": ["client_secret_post", "none"]
}
```

### OAuth Authorization Server Metadata (RFC 8414)

```
GET /.well-known/oauth-authorization-server
```

Returns the same metadata plus a `registration_endpoint` for dynamic client
registration. MCP clients check this endpoint first before falling back to
the OIDC discovery endpoint.

### Protected Resource Metadata (RFC 9728)

```
GET /.well-known/oauth-protected-resource
```

Used by MCP clients to discover where to authenticate before connecting to
the NyxID MCP proxy.

### JWKS

```
GET /.well-known/jwks.json
```

Returns the public key(s) used to sign JWTs. See
[JWKS and Signature Verification](#jwks-and-signature-verification).

---

## Authorization Code Flow with PKCE

NyxID supports the Authorization Code flow with PKCE (S256). PKCE is
**required** for all flows -- there is no implicit or password grant.

### Step 1: Register a Client

Register an OAuth client through the admin API or via
[Dynamic Client Registration](#dynamic-client-registration).

You'll need:
- `client_id` -- assigned during registration
- `client_secret` -- for confidential clients (optional for public clients)
- `redirect_uris` -- one or more callback URLs

### Step 2: Generate PKCE Parameters

```bash
# Generate code_verifier (43-128 chars, unreserved URI chars)
CODE_VERIFIER=$(openssl rand -base64 32 | tr -d '=' | tr '+/' '-_')

# Generate code_challenge (S256)
CODE_CHALLENGE=$(echo -n "$CODE_VERIFIER" | openssl dgst -sha256 -binary | openssl base64 | tr -d '=' | tr '+/' '-_')
```

### Step 3: Authorization Request

Redirect the user to:

```
GET /oauth/authorize
  ?response_type=code
  &client_id=YOUR_CLIENT_ID
  &redirect_uri=https://app.example.com/callback
  &scope=openid profile email
  &code_challenge=CODE_CHALLENGE
  &code_challenge_method=S256
  &state=RANDOM_STATE
  &nonce=RANDOM_NONCE
```

| Parameter | Required | Description |
|-----------|----------|-------------|
| `response_type` | Yes | Must be `code` |
| `client_id` | Yes | Your OAuth client ID |
| `redirect_uri` | Yes | Must match a registered redirect URI |
| `code_challenge` | Yes | PKCE S256 challenge |
| `code_challenge_method` | Yes | Must be `S256` |
| `scope` | No | Space-separated scopes (default: `openid`) |
| `state` | Recommended | CSRF protection token |
| `nonce` | Recommended | Replay protection (included in ID token) |

The user authenticates on the NyxID login page. After authentication, NyxID
redirects back to `redirect_uri` with an authorization code:

```
https://app.example.com/callback?code=AUTH_CODE&state=RANDOM_STATE
```

**Supported redirect URI types:**
- Standard HTTPS URLs
- Loopback redirects (RFC 8252 s7.3): `http://127.0.0.1:*`, `http://localhost:*`, `http://[::1]:*`
- Private-use URI schemes (RFC 8252 s7.1): e.g., `cursor://`, `vscode://`

### Step 4: Exchange Code for Tokens

```bash
curl -X POST https://auth.example.com/oauth/token \
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

The `id_token` is only returned when the `openid` scope is requested.

### Step 5: Refresh Tokens

```bash
curl -X POST https://auth.example.com/oauth/token \
  -H "Content-Type: application/x-www-form-urlencoded" \
  -d "grant_type=refresh_token" \
  -d "refresh_token=REFRESH_TOKEN"
```

NyxID uses **refresh token rotation**: each refresh returns a new refresh
token and invalidates the old one. A 120-second grace period handles network
retries. Reuse of a revoked refresh token outside the grace period triggers
revocation of the entire token family.

---

## Token Types

All tokens are RS256-signed JWTs.

| Token | Default TTL | Audience (`aud`) | Key Claims |
|-------|-------------|-------------------|------------|
| Access Token | 15 min | `BASE_URL` | `scope`, `token_type: "access"`, optional RBAC |
| Refresh Token | 7 days | `BASE_URL` | `token_type: "refresh"` |
| ID Token | 1 hour | `client_id` | `email`, `name`, `picture`, `nonce`, `at_hash` |
| Service Account Token | 1 hour | `BASE_URL` | `sa: true` |
| Delegated Token | 5 min | `BASE_URL` | `act.sub`, `delegated: true` |

### Access Token Claims

```json
{
  "sub": "user-uuid",
  "iss": "https://auth.example.com",
  "aud": "https://auth.example.com",
  "exp": 1700000000,
  "iat": 1699999100,
  "jti": "unique-token-id",
  "scope": "openid profile email roles",
  "token_type": "access",
  "sid": "session-uuid",
  "roles": ["admin", "user"],
  "groups": ["engineering"],
  "permissions": ["users:read", "users:write"]
}
```

RBAC claims (`roles`, `groups`, `permissions`) are only included when the
corresponding scopes are requested.

### ID Token Claims

```json
{
  "sub": "user-uuid",
  "iss": "https://auth.example.com",
  "aud": "your-client-id",
  "exp": 1700003600,
  "iat": 1700000000,
  "email": "user@example.com",
  "email_verified": true,
  "name": "Jane Doe",
  "picture": "https://...",
  "nonce": "your-nonce-value",
  "at_hash": "base64url-hash",
  "acr": "urn:mace:incommon:iap:silver",
  "amr": ["pwd", "mfa"],
  "auth_time": 1700000000,
  "sid": "session-uuid"
}
```

---

## Scopes and Claims

| Scope | Description | Claims Added |
|-------|-------------|--------------|
| `openid` | Required for OIDC. Triggers ID token issuance. | `sub`, `iss`, `aud` |
| `profile` | User profile information | `name`, `picture` |
| `email` | User email address | `email`, `email_verified` |
| `roles` | RBAC roles and permissions | `roles`, `permissions` |
| `groups` | RBAC group membership | `groups` |

---

## JWKS and Signature Verification

All tokens are signed with RS256. To verify tokens:

1. Fetch the JWKS from `/.well-known/jwks.json`
2. Match the `kid` (key ID) from the token's JWT header to a key in the JWKS
3. Use the matching RSA public key to verify the RS256 signature

```
GET /.well-known/jwks.json
```

```json
{
  "keys": [
    {
      "kty": "RSA",
      "kid": "abc123def456",
      "use": "sig",
      "alg": "RS256",
      "n": "...",
      "e": "AQAB"
    }
  ]
}
```

The JWKS should be cached by relying parties and refreshed periodically
(e.g., every 24 hours or when a `kid` mismatch is encountered).

---

## Token Introspection

RFC 7662 token introspection allows resource servers to validate tokens
server-side.

```bash
curl -X POST https://auth.example.com/oauth/introspect \
  -H "Content-Type: application/x-www-form-urlencoded" \
  -d "token=ACCESS_TOKEN" \
  -d "client_id=YOUR_CLIENT_ID" \
  -d "client_secret=YOUR_CLIENT_SECRET"
```

Response (active token):

```json
{
  "active": true,
  "sub": "user-uuid",
  "iss": "https://auth.example.com",
  "scope": "openid profile email",
  "token_type": "access",
  "exp": 1700000000,
  "iat": 1699999100
}
```

Response (inactive/invalid token):

```json
{
  "active": false
}
```

---

## Token Revocation

RFC 7009 token revocation. Always returns `200 OK` regardless of outcome.

```bash
curl -X POST https://auth.example.com/oauth/revoke \
  -H "Content-Type: application/x-www-form-urlencoded" \
  -d "token=REFRESH_TOKEN"
```

Access tokens are stateless and cannot be revoked individually (they expire
after their TTL). Revoke the associated refresh token to prevent further
access token issuance.

---

## Dynamic Client Registration

RFC 7591 dynamic client registration is available for public clients. This is
primarily used by MCP clients and native apps that perform automatic
registration.

```bash
curl -X POST https://auth.example.com/oauth/register \
  -H "Content-Type: application/json" \
  -d '{
    "client_name": "My App",
    "redirect_uris": ["https://app.example.com/callback"],
    "grant_types": ["authorization_code", "refresh_token"],
    "response_types": ["code"],
    "token_endpoint_auth_method": "none"
  }'
```

Response:

```json
{
  "client_id": "generated-client-id",
  "client_name": "My App",
  "redirect_uris": ["https://app.example.com/callback"],
  "grant_types": ["authorization_code", "refresh_token"],
  "response_types": ["code"],
  "token_endpoint_auth_method": "none"
}
```

---

## Configuration Examples

### Generic OIDC Relying Party

Use these values to configure any OIDC-compatible library or service:

| Setting | Value |
|---------|-------|
| Issuer / Provider URL | `https://auth.example.com` (your `BASE_URL`) |
| Authorization Endpoint | `https://auth.example.com/oauth/authorize` |
| Token Endpoint | `https://auth.example.com/oauth/token` |
| UserInfo Endpoint | `https://auth.example.com/oauth/userinfo` |
| JWKS URI | `https://auth.example.com/.well-known/jwks.json` |
| Scopes | `openid profile email` |
| Response Type | `code` |
| PKCE | Required, S256 |
| Token Auth Method | `client_secret_post` or `none` (public) |

Most OIDC libraries only need the **Issuer URL** and will auto-discover
everything else from `/.well-known/openid-configuration`.

### NextAuth.js

```javascript
import NextAuth from "next-auth";

export default NextAuth({
  providers: [
    {
      id: "nyxid",
      name: "NyxID",
      type: "oidc",
      issuer: process.env.NYXID_ISSUER, // e.g. https://auth.example.com
      clientId: process.env.NYXID_CLIENT_ID,
      clientSecret: process.env.NYXID_CLIENT_SECRET,
    },
  ],
});
```

### openid-client (Node.js)

```javascript
import { Issuer } from "openid-client";

const nyxid = await Issuer.discover("https://auth.example.com");
const client = new nyxid.Client({
  client_id: "YOUR_CLIENT_ID",
  client_secret: "YOUR_CLIENT_SECRET",
  redirect_uris: ["https://app.example.com/callback"],
  response_types: ["code"],
});

// Generate authorization URL with PKCE
const code_verifier = generators.codeVerifier();
const code_challenge = generators.codeChallenge(code_verifier);

const authUrl = client.authorizationUrl({
  scope: "openid profile email",
  code_challenge,
  code_challenge_method: "S256",
  state: generators.state(),
  nonce: generators.nonce(),
});
```

### curl Walkthrough

```bash
# 1. Discover endpoints
curl -s https://auth.example.com/.well-known/openid-configuration | jq .

# 2. Generate PKCE
CODE_VERIFIER=$(openssl rand -base64 32 | tr -d '=' | tr '+/' '-_')
CODE_CHALLENGE=$(echo -n "$CODE_VERIFIER" \
  | openssl dgst -sha256 -binary \
  | openssl base64 \
  | tr -d '=' | tr '+/' '-_')

# 3. Open authorization URL in browser
open "https://auth.example.com/oauth/authorize?\
response_type=code&\
client_id=YOUR_CLIENT_ID&\
redirect_uri=http://localhost:8080/callback&\
scope=openid+profile+email&\
code_challenge=${CODE_CHALLENGE}&\
code_challenge_method=S256&\
state=random123"

# 4. After callback, exchange code for tokens
curl -X POST https://auth.example.com/oauth/token \
  -d "grant_type=authorization_code" \
  -d "code=RECEIVED_CODE" \
  -d "redirect_uri=http://localhost:8080/callback" \
  -d "client_id=YOUR_CLIENT_ID" \
  -d "code_verifier=${CODE_VERIFIER}"

# 5. Get user info
curl -H "Authorization: Bearer ACCESS_TOKEN" \
  https://auth.example.com/oauth/userinfo
```

---

## MCP Client Integration

MCP clients (such as Claude Code or Cursor) integrate with NyxID using
standard OAuth discovery:

1. **Resource discovery**: The client fetches
   `GET /.well-known/oauth-protected-resource` from the MCP endpoint to find
   the authorization server URL.

2. **Server metadata**: The client fetches
   `GET /.well-known/oauth-authorization-server` to discover endpoints,
   including the `registration_endpoint`.

3. **Dynamic registration**: The client registers itself via
   `POST /oauth/register` (RFC 7591) as a public client.

4. **Authorization**: Standard Authorization Code flow with PKCE follows.

5. **Token usage**: The client sends the access token as a Bearer token when
   connecting to the NyxID MCP proxy at `/mcp`.

No manual client configuration is needed -- MCP clients handle discovery and
registration automatically.

---

## Troubleshooting

### "Issuer mismatch" errors

Verify that your `BASE_URL` matches exactly the URL your relying party uses
to reach NyxID. Common causes:
- Trailing slash mismatch (`https://auth.example.com` vs `https://auth.example.com/`)
- HTTP vs HTTPS mismatch
- Port mismatch (e.g., behind a reverse proxy)

Check the discovery document:
```bash
curl -s https://auth.example.com/.well-known/openid-configuration | jq .issuer
```

The returned `issuer` must match the `iss` claim in tokens and the URL the RP
uses for discovery.

### "Invalid token" errors

- Verify you're using the correct JWKS endpoint to get the signing key
- Check that the token hasn't expired (`exp` claim)
- Confirm the `aud` claim matches what your resource server expects
  - Access/refresh tokens use `BASE_URL` as audience
  - ID tokens use `client_id` as audience

### PKCE errors

NyxID requires PKCE with S256 for all authorization code flows. Ensure:
- `code_challenge_method` is explicitly set to `S256`
- `code_verifier` is sent in the token exchange request
- The verifier matches the challenge from the authorization request

### CORS issues

If your frontend makes direct requests to NyxID's OAuth endpoints, ensure
`FRONTEND_URL` is set to your frontend's origin. NyxID only allows CORS
from the configured `FRONTEND_URL`.

---

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `BASE_URL` | `http://localhost:3001` | Backend URL, used as the OIDC issuer |
| `JWT_ISSUER` | Same as `BASE_URL` | Override the OIDC issuer (usually leave unset) |
| `JWT_PRIVATE_KEY_PATH` | `keys/private.pem` | RSA private key for signing JWTs |
| `JWT_PUBLIC_KEY_PATH` | `keys/public.pem` | RSA public key for JWKS endpoint |
| `JWT_ACCESS_TTL_SECS` | `900` (15 min) | Access token lifetime |
| `JWT_REFRESH_TTL_SECS` | `604800` (7 days) | Refresh token lifetime |
| `SA_TOKEN_TTL_SECS` | `3600` (1 hour) | Service account token lifetime |
