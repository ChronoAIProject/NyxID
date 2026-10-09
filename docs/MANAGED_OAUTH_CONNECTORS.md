# Managed OAuth Connectors

NyxID seeds 27 OAuth 2.0 providers with fixed authorization and token
endpoints, each paired with an `api-{slug}` catalog service that injects the
connection's access token as a bearer header. The registry is
`MANAGED_OAUTH_PROVIDER_SEEDS` in `backend/src/services/provider_service.rs`.
Seeding is idempotent and never overwrites an existing provider or service.

"Simple" here means a user clicks Connect, authorizes on the provider's site,
and returns to NyxID without entering a tenant URL, region, or API key. A live
connection and token refresh have **not** been tested for any of these
providers.

## Connecting

Every provider is seeded with `credential_mode=both` and no client
credentials. A connection needs one of:

- **A NyxID platform app.** An operator registers an OAuth app with the
  provider, using callback `{BASE_URL}/api/v1/providers/callback`, and stores
  its client ID and secret on the NyxID provider.
- **The user's own app.** The user supplies their app's client ID and secret
  when adding the service (`POST /keys` or `PUT /providers/{id}/credentials`).
  The app must use the same callback. NyxID runs the flow, then stores and
  refreshes the token.
- **A credential node.** `nyxid node credentials add-oauth --service
  api-{slug} --from-catalog` runs the flow on the node and keeps the token
  there. The node listens on `http://127.0.0.1:{random port}/callback`, so
  this only works with providers that accept a loopback redirect on any port.
  Stripe requires the hosted flow; see [Stripe setup](STRIPE_OAUTH.md).

Without a platform app or user-supplied credentials, the OAuth start fails
with "requires either admin-configured OAuth app credentials or your own OAuth
app credentials".

NyxID's managed-app scope allowlist covers only selected seeded providers. Add
reviewed entries for these providers before exposing a shared app, and keep
the scope picker aligned with the scopes registered upstream.

## Providers

| Provider | Slug | Provider | Slug |
| --- | --- | --- | --- |
| Airtable | `airtable` | Asana | `asana` |
| Attio | `attio` | Bitbucket | `bitbucket` |
| Box | `box` | Calendly | `calendly` |
| Capsule CRM | `capsule-crm` | ClickUp | `clickup` |
| Crowdin | `crowdin` | Dialpad | `dialpad` |
| Dropbox | `dropbox` | Eventbrite | `eventbrite` |
| Figma | `figma` | GitLab | `gitlab` |
| HubSpot | `hubspot` | Intercom | `intercom` |
| Jira Cloud | `jira` | Linear | `linear` |
| Miro | `miro` | PagerDuty | `pagerduty` |
| Productboard | `productboard` | Sentry | `sentry` |
| Shippo | `shippo` | Square | `square` |
| Todoist | `todoist` | Zoom | `zoom` |
| Stripe | `stripe` | | |

The seed adds connection metadata and a generic proxy service only. Stripe has a curated read-only OpenAPI overlay. The other services in this
registry do not yet have curated overlays. Curated read operations, scope
menus, refresh and revocation behaviour, and provider-specific adapters should
be added and tested separately for each service.

Each provider still needs app registration and end-to-end tests for callback,
refresh rotation, reconnection, and deletion before a deployment offers it
broadly.

## Provider notes

### Stripe

Stripe uses Stripe Apps OAuth, with permissions configured in the app manifest.
The client-secret field holds the app developer API key. Its curated overlay
provides eight read-only operations. See [Stripe setup](STRIPE_OAUTH.md) for
registration, publication, test/live configuration, and disconnect behavior.

### Todoist

Authorization `https://app.todoist.com/oauth/authorize`, token
`https://api.todoist.com/oauth/access_token`, `client_secret_post`. Seeded
with the single `data:read` scope. Todoist requires **comma-separated** scopes
when requesting more than one, while NyxID joins scopes with spaces. Add a
provider-specific separator and matching allowlist parsing before exposing a
multi-scope picker or write operations. Test the registered app's actual
expiry and refresh behaviour, since Todoist supports both long-lived and
refresh-enabled apps.

### Dropbox

Authorization `https://www.dropbox.com/oauth2/authorize`, token
`https://api.dropboxapi.com/oauth2/token`; space-separated scopes.
`token_access_type=offline` is set in `extra_auth_params` so the initial
exchange returns a refresh token. Pick the app's content-access model (App
Folder or Full Dropbox) and enable only the scopes the first operations need.
File-content operations use a separate Dropbox API host and need a separate
service design.

### Airtable

| Field | Value |
| --- | --- |
| Authorization URL | `https://airtable.com/oauth2/v1/authorize` |
| Token URL | `https://airtable.com/oauth2/v1/token` |
| PKCE / token request | S256 / form-encoded, `client_secret_basic` |
| Initial scopes | `schema.bases:read`, `data.records:read` |
| Service base URL | `https://api.airtable.com/v0` |

Airtable requires at least one scope and S256 PKCE. The user chooses which
bases and workspaces the integration may access, so a valid token does not
imply access to every base. Access tokens last about one hour; refresh tokens
rotate and expire after 60 days of non-use. Do not configure remote
revocation until its current upstream contract is verified.

### Asana

| Field | Value |
| --- | --- |
| Authorization URL | `https://app.asana.com/-/oauth_authorize` |
| Token URL | `https://app.asana.com/-/oauth_token` |
| PKCE / token request | S256 / form-encoded, `client_secret_post` |
| Initial scopes | `workspaces:read`, `projects:read`, `tasks:read` |
| Service base URL | `https://app.asana.com/api/1.0` |

The app must register its scopes before authorization; omitting scopes can
select Asana's broad `default` permission. The app's distribution settings
must permit the connecting user's workspace. Asana's revoke endpoint accepts
only a refresh token, while NyxID's generic RFC 7009 path tries both tokens
and would report a failure. Leave remote revocation unconfigured until that
is handled.

### Linear

| Field | Value |
| --- | --- |
| Authorization URL | `https://linear.app/oauth/authorize` |
| Token URL | `https://api.linear.app/oauth/token` |
| PKCE / token request | S256 / form-encoded, `client_secret_post` |
| Initial scopes | `read` |
| Service base URL | `https://api.linear.app` |

Linear requires a **comma-separated** scope list, the same limitation as
Todoist. It returns rotating refresh tokens and roughly 24-hour access tokens.
Its API is GraphQL: reads and mutations share `POST /graphql`, which NyxID's
method/path operation policy cannot distinguish. Publish named, bounded read
operations before presenting a read-only agent tool.

### Calendly and ClickUp

Calendly's refresh tokens are single-use, so two successive refreshes are part
of its release check. ClickUp's token exchange uses a JSON body and its
authorization page lets users select workspaces; confirm the exact request
against a test app before enabling it in production.

## Deferred providers

These providers are not seeded because NyxID's current OAuth contract cannot
collect the required input, or because the flow is not a personal-account
connection:

| Provider | Why it is deferred |
| --- | --- |
| Zendesk | The OAuth host is the customer's Zendesk subdomain; the connection form needs a tenant field. |
| Mailchimp | The API host includes the account's data-centre suffix; the service needs a dynamic base URL. |
| Zoho Books, Zoho Invoice | Regional accounts and product-specific API hosts require region selection. |
| Salesforce | OAuth succeeds against a login host, then API calls use the selected org instance. |
| QuickBooks | Company (realm) selection is required after OAuth and must be bound to the API host. |
| Cal, Wrike, Greenhouse | Each needs a per-deployment base URL. |
| Stripe Connect | Platform authorization, not a personal connection. |
| Discord Bot, Slackbot | Bot installation and bot credentials are separate from a user OAuth connection. |
| TikTok Ads, Reddit Ads | Ads account review and non-personal scopes need a dedicated design. |
| Basecamp | Launchpad requires `type=web_server` on both the authorize and token requests, and `extra_auth_params` only reach the authorize request. API paths also need the account ID. |
| Contentful | Contentful's OAuth apps use the implicit grant, which NyxID's authorization-code flow does not support. |
| Stack Exchange | The API takes `access_token` and the application `key` as query parameters, not a bearer header. |

Existing Google, Microsoft, Slack, Twitch, Reddit, and Google Drive catalog
families keep their current NyxID providers and are not duplicated here.

## Sources

- [Todoist OAuth reference](https://developer.todoist.com/api/v1/#tag/Authorization/OAuth)
  and [Dropbox OAuth guide](https://developers.dropbox.com/oauth-guide).
- [Airtable OAuth reference](https://airtable.com/developers/web/api/oauth-reference),
  [list bases](https://airtable.com/developers/web/api/list-bases), and
  [list records](https://airtable.com/developers/web/api/list-records).
- [Asana OAuth](https://developers.asana.com/docs/oauth),
  [OAuth scopes](https://developers.asana.com/docs/oauth-scopes), and
  [REST API base URL](https://developers.asana.com/reference/rest-api-reference).
- [Linear OAuth](https://linear.app/developers/oauth-2-0-authentication) and
  [GraphQL API](https://linear.app/developers/graphql).
- [Calendly OAuth app setup](https://developer.calendly.com/creating-an-oauth-app),
  [single-use refresh-token guide](https://developer.calendly.com/docs/authentication/refresh-token-rotation-guide),
  and [ClickUp OAuth authentication](https://developer.clickup.com/docs/authentication).
- NyxID implementation: `backend/src/models/provider_config.rs`,
  `backend/src/services/user_token_service.rs`,
  `backend/src/services/user_credentials_service.rs`,
  `backend/src/services/provider_service.rs`, and
  `cli/src/node/oauth.rs`.
