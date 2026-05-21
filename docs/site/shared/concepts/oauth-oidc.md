---
title: OAuth & OIDC identity
description: How NyxID functions as a full OpenID Connect identity provider, what tokens it issues, and how relying parties and MCP clients authenticate against it.
---

NyxID is a full OpenID Connect 1.0 identity provider. Applications can use NyxID as their auth backend — handling login, user management, MFA, and session tokens — while NyxID also brokers access to downstream APIs. This dual role (identity provider and credential broker) is what lets NyxID issue delegation tokens that a downstream service can exchange for a scoped call to the LLM gateway without ever seeing the user's API keys.

## Discovery

NyxID follows standard OIDC and OAuth 2.0 discovery. A relying party only needs the issuer URL (`BASE_URL`); all endpoint URLs are discovered from the well-known documents:

| Endpoint | Purpose |
|----------|---------|
| `GET /.well-known/openid-configuration` | OIDC provider metadata |
| `GET /.well-known/oauth-authorization-server` | RFC 8414 AS metadata (checked first by MCP clients) |
| `GET /.well-known/oauth-protected-resource` | RFC 9728 resource metadata (used by MCP clients to find the AS) |
| `GET /.well-known/jwks.json` | Public keys for verifying token signatures |

## Supported specifications

| Spec | Description |
|------|-------------|
| OpenID Connect Core 1.0 | ID tokens, UserInfo endpoint, standard claims |
| OpenID Connect Discovery 1.0 | `/.well-known/openid-configuration` |
| RFC 8414 | OAuth 2.0 Authorization Server Metadata |
| RFC 7636 | PKCE — required for all authorization code flows |
| RFC 7662 | Token Introspection |
| RFC 7009 | Token Revocation |
| RFC 7591 | Dynamic Client Registration |
| RFC 8693 | Token Exchange (delegated access) |
| RFC 9728 | OAuth 2.0 Protected Resource Metadata |

## The authorization code flow

NyxID supports only the Authorization Code flow with PKCE (S256). Implicit grant and Resource Owner Password Credentials grant are not supported. PKCE is required for all flows regardless of client type.

The flow:

1. The relying party generates a `code_verifier` (random string) and a `code_challenge` (`SHA256(code_verifier)`, base64url-encoded).
2. The user is redirected to `/oauth/authorize` with `response_type=code`, `code_challenge`, `code_challenge_method=S256`, and any requested scopes.
3. NyxID authenticates the user (username/password, MFA, social login), validates the client and redirect URI, and returns an authorization code.
4. The relying party exchanges the code for tokens at `/oauth/token`, presenting the `code_verifier` to prove it originated the flow.
5. NyxID verifies `SHA256(code_verifier) == code_challenge` before issuing tokens.

Redirect URI types supported: standard HTTPS URLs, loopback redirects (`http://127.0.0.1:*`, `http://localhost:*`), and private-use URI schemes (e.g. `cursor://`, `vscode://`).

## Token types

All tokens are RS256-signed JWTs using a 4096-bit RSA key pair.

| Token | Default TTL | Audience | Key claims |
|-------|-------------|----------|------------|
| Access token | 15 min | `BASE_URL` | `scope`, `token_type: "access"`, optional RBAC |
| Refresh token | 7 days | `BASE_URL` | `token_type: "refresh"` |
| ID token | 1 hour | `client_id` | `email`, `name`, `picture`, `nonce`, `at_hash` |
| Service account token | 1 hour | `BASE_URL` | `sa: true` |
| Delegation token | 5 min | `BASE_URL` | `act.sub`, `delegated: true` |

Access tokens and refresh tokens use `BASE_URL` as audience. ID tokens use the `client_id` of the requesting application. When validating a token, make sure the `aud` claim matches what your resource server expects.

### RBAC in access tokens

When the `roles`, `groups`, or `permissions` scopes are requested, the corresponding claims are included in the access token:

```json
{
  "sub": "user-uuid",
  "scope": "openid profile email roles",
  "roles": ["admin", "user"],
  "groups": ["engineering"],
  "permissions": ["users:read", "users:write"]
}
```

These claims are populated from NyxID's internal role and group model and can be used by resource servers for authorization decisions without a separate token introspection call.

## Refresh token rotation

Each refresh returns a new refresh token and invalidates the old one. A 120-second grace period handles concurrent requests and network retries. Reuse of a revoked refresh token outside the grace period triggers revocation of the entire token family (the old token and its successor).

## Dynamic client registration

MCP clients and native apps use RFC 7591 dynamic client registration to self-register without admin intervention:

```http
POST /oauth/register
Content-Type: application/json

{
  "client_name": "My App",
  "redirect_uris": ["https://app.example.com/callback"],
  "grant_types": ["authorization_code", "refresh_token"],
  "response_types": ["code"],
  "token_endpoint_auth_method": "none"
}
```

Dynamically registered clients are public clients. Confidential clients (with a `client_secret`) are registered through the admin API or the developer apps section of the web console.

## Token introspection and revocation

Resource servers can validate tokens server-side via RFC 7662 introspection (`POST /oauth/introspect`). This is useful when the resource server cannot or does not want to maintain the JWKS and verify signatures locally.

Access tokens are stateless and cannot be revoked individually — they expire after their TTL. Revoking a refresh token (`POST /oauth/revoke`) prevents further access token issuance but does not invalidate access tokens already issued.

## How MCP clients use OIDC

MCP clients check `/.well-known/oauth-protected-resource` first to find the authorization server, then `/.well-known/oauth-authorization-server` to discover the full endpoint list and the `registration_endpoint`. The client self-registers, completes the Authorization Code + PKCE flow, and uses the resulting access token to connect to `/mcp`. No manual configuration is needed on the NyxID side.

## Delegation tokens and token exchange

Beyond standard OIDC, NyxID supports RFC 8693 Token Exchange for issuing scoped delegation tokens. A downstream service that is registered as an NyxID OAuth client and holds a user's access token can exchange it for a 5-minute delegation token scoped to a specific capability (e.g. `llm:proxy`). This is the mechanism that lets NyxID-integrated services call the LLM gateway on a user's behalf without holding the user's upstream API keys.

See [MCP proxy](/docs/shared/concepts/mcp-proxy) for how delegation tokens flow in the MCP context.

## Related guides

- [Developer apps](/docs/web/guides/developer-apps)
- [Account security](/docs/web/guides/account-security)
- [MCP proxy](/docs/shared/concepts/mcp-proxy)
