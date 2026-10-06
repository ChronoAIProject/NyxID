# Cloudflare, Supabase Management, and Railway OAuth

NyxID seeds three cloud management connectors. Each supports a shared NyxID
OAuth app or a user's own OAuth app (`credential_mode: "both"`). The backend
handles authorization-code exchange with S256 PKCE, HTTP Basic client
authentication, form-encoded token requests, encrypted token storage, and refresh.

| Service | Provider slug | Catalog slug | API base URL |
| --- | --- | --- | --- |
| Cloudflare | `cloudflare` | `api-cloudflare` | `https://api.cloudflare.com/client/v4` |
| Supabase Management | `supabase-management` | `api-supabase-management` | `https://api.supabase.com/v1` |
| Railway | `railway` | `api-railway` | `https://backboard.railway.com` |

These are downstream resource connections. Register each vendor OAuth app with
the backend callback matching `BASE_URL`:

```text
https://YOUR-NYXID-BACKEND/api/v1/providers/callback
```

After deployment, open **Providers > Manage Providers**, edit the relevant
provider, and save its client ID and secret. The shared app becomes available
in **AI Services > Connect Service** once both credentials are configured.
Users may also provide their own app credentials. Credentials are supplied
through the existing write-only provider fields; none are embedded in the seed.
Seeding is additive and preserves operator settings on subsequent restarts.

## Cloudflare

Create a confidential OAuth client under **Manage Account > OAuth clients**.
Use the authorization-code grant, `client_secret_basic` authentication, and
the NyxID callback. Select the API permissions your integration needs, such
as account, zone, DNS, and Workers permissions. Required and optional API
permissions are configured on the client; users choose accounts and optional
permissions in Cloudflare's consent screen.

New Cloudflare clients are private and only account members can authorize
them. A shared NyxID app intended for customers in other Cloudflare accounts
needs public visibility and publisher-domain verification. Cloudflare makes
the change to public visibility permanent. Account administrators can block
new public OAuth authorizations.

NyxID requests `openid offline_access`. Cloudflare's discovery document
advertises S256 PKCE, refresh tokens, Basic client authentication, and the
revocation endpoint. API permissions still depend on the registered client
and the user's consent; identity and offline scopes alone do not describe
the client's infrastructure permissions. The scope picker permits additional
provider-supported scopes for clients that require them.

The seeded endpoints are:

- Authorization: `https://dash.cloudflare.com/oauth2/auth`
- Token: `https://dash.cloudflare.com/oauth2/token`
- Revocation: `https://dash.cloudflare.com/oauth2/revoke`

The hosted overlay exposes account, zone, and DNS record listing. Other
Cloudflare API paths are available through the normal NyxID proxy, subject
to the granted permissions.

Sources: [Cloudflare OAuth](https://developers.cloudflare.com/fundamentals/oauth/),
[client registration](https://developers.cloudflare.com/fundamentals/oauth/create-an-oauth-client/),
[OAuth endpoints](https://developers.cloudflare.com/fundamentals/oauth/integrate-with-cloudflare/),
[live discovery metadata](https://dash.cloudflare.com/.well-known/openid-configuration).

## Supabase Management

Create and publish an OAuth app in **Organization settings > OAuth Apps**.
Configure its permissions there; Organizations Read and Projects Read support
the hosted discovery operations. Add write permissions only for the management
operations the app needs.

Supabase configures permissions on the app and deprecates the authorization
`scope` parameter. The seeded provider has `supports_oauth_scopes: false`,
omits that parameter, and rejects nonempty scope overrides. Permission changes
require existing users to authorize the app again.

The seeded endpoints are:

- Authorization: `https://api.supabase.com/v1/oauth/authorize`
- Token and refresh: `https://api.supabase.com/v1/oauth/token`

Management OAuth tokens grant access to the Management API. The existing
`api-supabase` **Supabase Data API** connector continues to use a project URL
and an API key for PostgREST table access. Neither connector obtains a
database password or an end-user Supabase Auth JWT.

No programmatic revocation endpoint is seeded. Disconnect removes the local
connection; remove the app authorization in Supabase to revoke remote access.

Sources: [build a Supabase integration](https://supabase.com/docs/guides/integrations/build-a-supabase-integration),
[app permissions](https://supabase.com/docs/guides/integrations/build-a-supabase-oauth-integration/oauth-scopes),
[Management API](https://supabase.com/docs/reference/api/introduction).

## Railway

A workspace administrator creates a **Web** OAuth app in **Workspace settings
> Developer > New OAuth App**. Use Basic client authentication and register the
NyxID callback. Native clients use a different authentication method and are
not the configuration of this seeded server-side connector.

The default scopes are `openid email profile offline_access project:viewer`.
`openid` is mandatory. `offline_access` and `prompt=consent` are needed to
receive refresh tokens. NyxID always sends the consent prompt so returning
users can also select different projects. Access tokens last one hour;
refresh tokens rotate and NyxID retains the latest returned token.

Users choose which resources to share. Add `project:member` to manage selected
projects, or choose workspace viewer/member scopes for workspace access.
Workspace discovery through `me.workspaces` also needs `email` and `profile`.
The shared-app scope allowlist includes identity, offline access, and project
or workspace viewer/member roles. `workspace:admin` requires a user's own app.
Railway caps granted access at the user's actual role.

The seeded endpoints are:

- Authorization: `https://backboard.railway.com/oauth/auth`
- Token and refresh: `https://backboard.railway.com/oauth/token`
- API: JSON `POST /graphql/v2` at `https://backboard.railway.com`

For example, this query lists projects selected during consent:

```graphql
query {
  externalWorkspaces {
    id
    name
    projects { id name }
  }
}
```

The hosted GraphQL operation accepts queries and mutations. Its tool metadata
always marks it as destructive and requiring approval because arbitrary
GraphQL bodies may change infrastructure. There is no seeded programmatic
revocation endpoint; revoke remote authorization under **Account settings > Apps**.

Sources: [Railway OAuth](https://docs.railway.com/integrations/oauth),
[login and tokens](https://docs.railway.com/integrations/oauth/login-and-tokens),
[scopes](https://docs.railway.com/integrations/oauth/scopes-and-user-consent),
[project discovery](https://docs.railway.com/integrations/oauth/fetching-workspaces-or-projects).

## Verify before rollout

Configure each shared client in a test environment, authorize a test account,
and run account/project discovery through its NyxID proxy URL. Verify a token
refresh and reconnect. Client-side registration, callback matching, Cloudflare
visibility/domain verification, Supabase app permissions, and Railway resource
selection must be checked against the real vendor; the local mocked protocol
tests cannot prove those external settings are correct.

The [Fable 5.1 OAuth review](plans/posthog-oauth-fable-review.md) identified
these specific checks before activating the shared apps:

- **Cloudflare permissions:** authorize with the seeded `openid offline_access`
  string and confirm that the token can list the intended accounts/zones using
  the registered client's permissions. Cloudflare's discovery lists identity
  and offline scopes, while its client documentation also refers to API
  permission scope names. If the real flow requires explicit permission names,
  configure them in the provider's editable `default_scopes`; validate them
  against the registered client and consent screen.
- **Supabase scope omission:** authorize without `scope`. Current documentation
  deprecates the parameter, but its example still includes `scope=all`. If the
  real flow requires it, an administrator can set `supports_oauth_scopes: true`
  and `default_scopes: ["all"]` through the provider update API. The deprecated
  parameter is deliberately absent in the initial seed.
- **Supabase expiry:** record the real token response's `expires_in` and refresh
  behavior. Without `expires_in`, NyxID stores no expiry and cannot schedule a
  proactive refresh. Confirm the token lifetime before relying on unattended
  access, and address any expiring token that omits its expiry information.

For Cloudflare, also verify that RFC 7009 revocation disables the tested tokens.
For Railway, verify resource selection, refresh-token rotation, and rejection
of the previous refresh token. These outcomes have not yet been checked with
registered vendor apps in this worktree.

## Local validation (2026-10-07)

The focused backend run passed 42 tests: the two new cloud protocol/seeding
tests, scope catalog and embedded OpenAPI contracts, existing provider/service
seeding, legacy and modern refresh encoding, and remote revocation. The cloud
protocol test exercises all three providers with both shared and BYO clients,
S256 verifier matching, Basic/form token authentication, encrypted storage,
refresh-token rotation without a credential epoch bump, Supabase scope omission,
and Railway's shared-versus-BYO scope policy. Database tests ran against an
isolated local MongoDB replica set because OAuth state writes use transactions.

The full server test target exceeded the practical local memory budget. The
focused executable was built from a temporary source copy with unrelated test
functions excluded from compilation. Production code and all six selected
service/test files were retained; the selected files were verified byte-for-byte
against the worktree. This is focused validation, not a full backend suite pass.

Frontend provider-branding and add-key dialog checks passed 67 tests. TypeScript
build, lint of the changed frontend file, Rust formatting, overlay JSON
validation, and whitespace checks passed. A broader frontend run reported six
failures in the unchanged `nyxbot-agent-details.test.tsx`; an isolated run of that
file reported three failures. Those failures remain unresolved and are not
claimed to be pre-existing. No real-vendor authorization or deployment was
performed.
