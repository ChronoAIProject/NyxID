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

## Reviewing a broker binding's service access

Broker-capable applications can let a signed-in user review the service grant behind an existing binding without replacing its opaque binding credential. The application starts a normal Authorization Code + PKCE request with `prompt=consent`, the required RFC 8707 `resource` values, the exact external subject, and `binding_grant_id=SHA-256(binding_id)`. The raw `binding_id` must never be placed in a browser-visible URL or form.

NyxID accepts the review only when the binding belongs to the authenticated user, OAuth client, and exact external subject. The consent page then distinguishes services already authorized by the binding, services required by the application, and optional services the user can add or remove. Required resources cannot be deselected individually; the user can deny the whole request instead.

When the user confirms the review, NyxID rotates the refresh grant behind the same binding with optimistic concurrency. The token response contains `binding_updated: true` and no replacement `binding_id`. This keeps the application's stored binding handle stable while NyxID remains the source of truth for service authorization.

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

## Adding service access incrementally

Use `include_granted_scopes=true` on `/oauth/authorize` (or `/oauth/par`) when an application needs more OAuth permissions. Put the new permissions in `scope`, as in Google's incremental authorization flow. NyxID preserves previously granted scopes and service access. A live consent for this user and client is required; if it has expired or been revoked, restart ordinary authorization. NyxID also accepts `service_access_mode=incremental` for existing callers; it has the same add-only semantics.

| Parameter | Meaning |
|-----------|---------|
| `include_granted_scopes=true` | Request additive OAuth scopes and show a confirmation page. Omission keeps ordinary authorization/review behavior. |
| `scope` | Only the newly needed OAuth scopes are required. Omission preserves the current scope grant without adding the client's other configured scopes. |
| `service_access_mode=incremental` | NyxID-specific alias for the same add-only review, retained for callers using the original service-access contract. |
| Repeated `requested_service_ids` | Exact **UserService UUIDs** the application needs. At most 100 entries; duplicates are deduplicated. These services are required for this request. Catalog IDs, display names, and slugs are not substitutes. |
| Repeated `resource` | Optional RFC 8707 resources. Resolved services are also required, but these resources narrow the initial **access token**, not the accumulated refresh/binding grant. `/oauth/token` can request a different subset within the accumulated grant. |
| `binding_grant_id` | Optional SHA-256 of the existing binding handle, when updating that specific binding. Its exact external subject must match, including an absent external subject for ordinary account bindings. Never put the raw `binding_id` in the URL. |

For example, an app with A and B already granted can request C and D:

```text
/oauth/authorize?response_type=code
  &client_id=CLIENT_ID
  &redirect_uri=REGISTERED_CALLBACK
  &scope=openid%20proxy%20offline_access
  &code_challenge=PKCE_CHALLENGE
  &code_challenge_method=S256
  &state=CSRF_AND_RETURN_CONTEXT
  &include_granted_scopes=true
  &requested_service_ids=USER_SERVICE_C_UUID
  &requested_service_ids=USER_SERVICE_D_UUID
```

The consent page shows C and D under **Allow 2 additional services**, with A and B in a read-only **Already authorized** section. Newly requested OAuth scopes appear as **additional permissions** in the main review and Allow button. Users may explicitly select other optional services. An existing unrestricted grant remains unrestricted; zero new services and permissions produces a **Continue** action. A previously authorized service that is now unavailable remains in the stored grant but is identified as unavailable and cannot be used through the proxy; only unavailable new services block approval. App-supplied free-text permission descriptions are not accepted.

The approved authorization code stores the complete accumulated service boundary. Refresh tokens and broker bindings inherit that boundary; an optional `resource=C` only narrows the access token. Binding updates keep the existing handle and return `binding_updated: true`, without a replacement handle. Without `binding_grant_id`, the flow uses the user's client-wide consent and may issue a new binding through the ordinary broker flow. It does not update other existing bindings automatically.

NyxID signs the grant snapshot, mode, IDs, scopes, app identity, and request context. The browser receives a short, user-bound review handle and loads the signed request from NyxID; the server verifies its signature, expiry, authenticated user, client, and live ownership on submission. URL display hints are not authority. All-services escalation, unknown or inaccessible new IDs, disabled new services, and mismatched external subjects fail closed. A consent revision/fingerprint and binding grant version fence stale decisions and codes. Routine broker token rotation does not change the grant version and can continue during review. Conflicts return an OAuth error callback with `state`, allowing the caller to restore its draft and retry. A later revocation cannot recreate the deleted consent. Binding replacement and the final consent fence commit atomically. Previously issued stateless access tokens retain their normal expiry semantics.

Deploy the backend and consent UI together before enabling this mode in callers. The dedicated incremental decision endpoint rejects requests on old backend replicas instead of silently replacing the grant. Old authorization servers that do not implement the mode must not be used as a fallback.

### Aevatar Channel integration boundary

Aevatar's Channel editor can use repeated `required_service_ids` on its own edit URL to carry the exact NyxID UserService IDs and focus `#services`. **That editor parameter is an Aevatar contract**, not a NyxID authorization request parameter. The caller must:

1. Resolve and validate the IDs, and compare them with its effective grant and current channel selection. Already authorized services only need local selection.
2. Save the unsaved editor draft and return location behind an opaque `state`, then start Authorization Code + PKCE with `include_granted_scopes=true`, the newly needed OAuth scopes in `scope`, and the missing IDs as `requested_service_ids`. Include the binding hash when the existing handle should be updated.
3. Validate state, exchange the code, and re-read actual access before marking the missing services available. A redirect alone is not proof that access was granted. Refresh or replace locally cached credentials as appropriate.
4. Restore the draft and preselect newly authorized required services. Persist Channel changes only when the user presses **Save changes**. Cancel, expiry, and conflict must preserve the draft and offer a fresh authorization attempt.

This NyxID contract does not add the Services button, callback handling, or draft persistence to the separate Aevatar console repository.
