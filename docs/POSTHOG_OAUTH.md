# PostHog OAuth compatibility

For the registered confidential-client connector, follow
[PostHog OAuth connections](POSTHOG_CONNECTIONS.md). The CIMD approach below
is the separate public-client implementation plan.

Checked against PostHog's current documentation and live authorization-server
metadata on 2026-10-07. PostHog supports OAuth 2.0 authorization-code connections
for third-party apps, S256 PKCE, refresh tokens, and token revocation.

The recommended registration mechanism is a **Client ID Metadata Document
(CIMD)**. Publish JSON on a domain controlled by the app and use that HTTPS URL
as the OAuth client ID. The document declares the app name, logo, redirect
URIs, and a ceiling of required and optional scopes. This flow has no manual
registration prerequisite. Publisher verification is optional and removes
the unverified-app notice from the consent screen.

Publish an explicit nonempty `com.posthog.scopes` list and an explicit
`optional_scopes` list. Both lists form the scope ceiling and each permission
still requires user consent. Optional scopes require a nonempty required list;
PostHog filters permissions people cannot grant to CIMD clients and rejects a
document if every required scope is filtered out. On metadata refresh, omitting
either scope field preserves its stored value; `optional_scopes: []` explicitly
clears optional scopes. Check the actual token response before enabling features
that need optional permissions.

PostHog caches client metadata according to the document's `Cache-Control:
max-age`, so redirect and permission changes need a planned cache window.
Protocol redirect URIs may use a different domain when listed exactly, but
optional publisher verification asks for HTTPS redirects on the same production
domain as the client metadata. Verification is granted separately for US and EU.

| Purpose | Endpoint |
| --- | --- |
| Discovery | `https://oauth.posthog.com/.well-known/oauth-authorization-server` |
| Authorization | `https://oauth.posthog.com/oauth/authorize/` |
| Token and refresh | `https://oauth.posthog.com/oauth/token/` |
| Revocation | `https://oauth.posthog.com/oauth/revoke/` |
| US private API | `https://us.posthog.com` |
| EU private API | `https://eu.posthog.com` |

The region-agnostic domain routes OAuth between US and EU Cloud. Private API
calls still need the correct regional API domain. Public event-ingestion hosts
(`us.i.posthog.com` and `eu.i.posthog.com`) serve a different API surface.

OAuth scopes mirror PostHog's personal API key scopes. Useful initial read
scopes include `project:read`, `insight:read`, `dashboard:read`, `query:read`,
and `feature_flag:read`. Optional write scopes can permit dashboard, insight,
feature flag, survey, and other resource management after user consent.
The metadata advertises many more scopes; support in discovery does not
guarantee a CIMD client or a particular user can grant every scope.

NyxID already supplies authorization-code flows, PKCE, encrypted tokens,
scope selection, and refresh. Its current provider credential checks require
both a client ID and secret for OAuth2, and its configurable token
authentication methods are Basic and client-secret POST. Supporting PostHog's
recommended public CIMD client therefore needs explicit support for
`token_endpoint_auth_method: "none"` across configuration validation, credential
readiness/resolution, authorization, and refresh. The connection UI must also
accept a client ID without a secret. A managed client needs a published
metadata route and regional API destination selection.

PostHog's live metadata also advertises `client_secret_post` and
`private_key_jwt` authentication, but those capabilities do not make the
recommended CIMD flow a confidential client with a shared secret.

Sources:

- [PostHog OAuth integration](https://posthog.com/docs/api/oauth)
- [API authentication and regional hosts](https://posthog.com/docs/api/overview)
- [Live authorization-server metadata](https://oauth.posthog.com/.well-known/oauth-authorization-server)

The documented region-agnostic protected-resource metadata URL returned 404
when checked; use the working authorization-server discovery endpoint above
when implementing the connection.

The requested Fable 5.1 consultation is saved in the
[full advisory response](plans/posthog-oauth-fable-review.md). The primary agent's
[implementation plan](plans/posthog-oauth.md) incorporates that review, including
public-client validation, hosted metadata, CLI/BYO parity, regional destination
binding, rollout order, and the remaining real-vendor checks.
