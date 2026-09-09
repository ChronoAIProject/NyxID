# Managed Notion Connections

NyxID exposes one catalog service, `api-notion`, backed by a seeded `notion`
OAuth provider. Configure one Notion public integration on that provider and
users connect from **AI Services > Connect Service** with **NyxID managed**,
exactly like GitHub and the Google products. Each connection has its own
encrypted token.

| Service | Catalog slug | Base URL |
| --- | --- | --- |
| Notion | `api-notion` | `https://api.notion.com` |

## How Notion differs from Google and GitHub

Read this before assuming the Google runbook transfers.

- **There are no OAuth scopes.** Notion's authorize request takes no `scope`
  parameter. What the integration may do is fixed in Notion's developer
  settings (its *capabilities*), and which pages and databases it can reach is
  chosen by the user in Notion's own page picker during authorization. The
  provider is seeded with `supports_oauth_scopes: false` and
  `default_scopes: None`. The server rejects non-empty `additional_scopes`
  and `scope_override`, omits the authorize `scope` parameter even if defaults
  are configured, and the connection dialog hides the scope picker. Other
  providers with absent defaults can still accept optional scopes. There is
  no Notion entry in `platform_scope_allowlist`.
- **`owner=user` is mandatory** on the authorize URL. It is seeded in
  `extra_auth_params`; removing it breaks the flow.
- **Token exchange and refresh use JSON bodies and HTTP Basic**
  (`token_request_encoding: "json"`, `client_secret_basic`). Revocation uses
  `revocation.request_encoding: "json"` with a `{"token":"..."}` body and
  the same Basic authentication. OAuth headers come from the provider's
  `oauth_request_headers`, independently of proxy headers.
- **No PKCE.** Notion's OAuth implementation does not support it.
- **Refresh tokens are supported.** Notion documents
  `POST /v1/oauth/token` with `grant_type: "refresh_token"` and a
  `refresh_token`, returning a new access token and refresh token. NyxID stores
  and rotates both in the legacy and multi-connection paths. The documented
  response has no `expires_in`; NyxID does not invent a lifetime. An absent
  expiry remains absent, so proactive expiry-based refresh is not scheduled
  for that token. Explicit refresh remains available when a refresh token
  was returned. Optional response fields may be absent or null.
- **Requests need a `Notion-Version` header.** The seed pins `2022-06-28`
  separately on the provider's OAuth requests and the service's overridable
  proxy defaults. Callers can override the proxy version; changing it does
  not change OAuth requests. This version matches the hosted overlay's
  database API. Notion's current version is `2026-03-11`; database queries
  moved to data sources starting with `2025-09-03`, so do not change the
  version for the overlay's database operations without updating the spec.

## Notion Setup

1. Open <https://www.notion.so/profile/integrations> and click **New
   integration**.
2. Choose **Public** integration. Internal integrations issue a single static
   token and have no OAuth flow, so they cannot back a managed connector.
3. Fill in the public-integration fields Notion requires: integration name,
   logo, company name, website, privacy policy URL, terms of use URL, and a
   support email.
4. Under **Capabilities**, select what the integration may do. For a general
   read/write connector: **Read content**, **Update content**, **Insert
   content**, and the comment capabilities you need. For user identity, pick
   **Read user information without email addresses** unless you specifically
   need addresses — this is the closest thing Notion has to scope minimisation,
   and it is set here rather than per authorization.
5. Under **OAuth Domain & URIs**, add the NyxID backend callback to **Redirect
   URIs**:

   ```text
   https://YOUR-NYXID-BACKEND/api/v1/providers/callback
   ```

   Local development may additionally use:

   ```text
   http://localhost:3001/api/v1/providers/callback
   ```

   This must match the backend's `BASE_URL` exactly — scheme, host, port, and
   path. It is the same generic callback every NyxID OAuth provider uses.
6. Copy the **OAuth client ID** and **OAuth client secret** from the
   integration's secrets section. The secret is shown once; store it before
   leaving the page.

Confirm Notion's current requirements for distributing a public integration to
users outside your own workspace before launch — that policy is Notion's and
changes independently of NyxID.

## NyxID Setup

1. Deploy this change. Startup seeds the `notion` provider, the `api-notion`
   service, its `ServiceProviderRequirement` (bearer / `Authorization`), and
   the `Notion-Version` headers, and the hosted OpenAPI spec URL. Startup
   repairs missing OAuth fields from the initial Notion seed and replaces
   its exact outdated limitations/upload metadata. Explicit OAuth settings,
   customized service metadata, and admin-set spec URLs are preserved.
   Catalog spec sync then creates the concrete operation rows used by MCP.
2. As a NyxID administrator, open **Providers > Manage Providers**, edit
   **Notion** (slug `notion`), and enter the **Client ID** and **Client
   Secret**. Set **Credential Mode** to **Admin or User** (`both`) for managed
   plus custom-app options, or **Admin Only** (`admin`) for managed only. Keep
   **Active** enabled and save.
3. Leave the seeded OAuth configuration alone: the authorize and token URLs,
   `client_secret_basic`, JSON token/revocation encoding, OAuth version header,
   PKCE off, `owner=user`, and `supports_oauth_scopes: false`.
4. Open **AI Services > Connect Service**, pick **Notion**, choose **NyxID
   managed**, and authorize with a test account. The managed option appears
   only when the provider holds a usable client ID *and* secret.

During authorization Notion asks the user to select pages. Anything not
selected is invisible to the API, so a connection that returns empty results
usually means nothing was shared rather than that auth failed.

## Verify the Connection

Prefix these with the connection's NyxID proxy URL from its service details.

- `POST /v1/search` with an empty body — lists the pages and databases the user
  shared with the integration.
- `GET /v1/users/me` — returns the bot user, confirming the token and the
  `Notion-Version` header.
- `GET /v1/pages/{page_id}` then `PATCH /v1/pages/{page_id}` — read and write a
  shared page.
- `POST /v1/pages` with a shared parent — create a page.

## Limitations

- Only content the user explicitly shared reaches the API. Newly created pages
  elsewhere in the workspace must be shared separately.
- Notion rate limits at roughly three requests per second per integration,
  with burst allowance; expect 429s under parallel agent traffic.
- File contents can be proxied to `POST /v1/file_uploads/{file_upload_id}/send`
  on `api.notion.com`. Set `Content-Type: multipart/form-data; boundary=...`
  and send the file in the `file` part. The caller's multipart content type
  overrides the seeded JSON default; `supports_proxy_binary_upload` is true.
- Deleting a connection schedules best-effort upstream revocation. Inspect
  the revocation audit outcome for delivery failures; local deletion alone
  is not proof that an upstream token was revoked.

## MCP Operations

The hosted spec at `/api/v1/catalog-specs/notion/openapi.json` covers search,
page retrieval/creation/update, database retrieval/query, block children
listing/appending, user retrieval/listing/current bot, and comment
listing/creation. Startup sync additively materializes 13 `ServiceEndpoint`
rows, so a connected Notion service publishes concrete operations in
`/api/v1/mcp/config` and MCP `tools/list`. Repeated syncs preserve operation IDs.
The overlay is a curated subset; uploads remain available through HTTP proxy.

## Protocol References

Checked against Notion's current documentation on 2026-09-09:

- [Create a token](https://developers.notion.com/reference/create-a-token)
- [Refresh a token](https://developers.notion.com/reference/refresh-a-token)
- [Revoke a token](https://developers.notion.com/reference/revoke-token)
- [Send a file upload](https://developers.notion.com/reference/send-a-file-upload)
- [Database query, API 2022-06-28](https://developers.notion.com/reference/post-database-query)

Existing providers retain form token encoding when no encoding is configured,
except code exchange and multi-connection refresh for legacy Lark/Feishu rows
(including custom rows using their recognized token URLs), which retain JSON.
Legacy `UserProviderToken` refresh always retains its original form fallback,
including Lark/Feishu. An explicit, validated `token_request_encoding` overrides
each fallback; startup does not opt existing Lark/Feishu connections into JSON.
Revocation encoding defaults independently to form, preserving existing
RFC 7009 requests and their `token_type_hint`. JSON revocation omits that
optional hint. Updating only `revocation_url` preserves the structured encoding,
authentication, style, and grant-revocation settings.

OAuth headers default to empty and scope support defaults to true. The only
accepted OAuth headers are `Notion-Version` and `Anthropic-Version`, each with
a valid `YYYY-MM-DD` date. Arbitrary names and values are rejected on writes;
old unsafe entries are suppressed on reads and removed at startup. Provider
debug output redacts the map and encrypted client credentials.

## Node-Native OAuth

`nyxid node credentials add-oauth --service api-notion --from-catalog` fetches token encoding,
OAuth version headers, scope support, HTTP Basic authentication, and `owner=user`
from the catalog. Supply your public integration's client credentials locally
and register the loopback callback with Notion. The node exchanges the code as
JSON, saves tokens encrypted locally, and retains protocol options with the
credential so later refreshes use the same encoding and version. Scopeless
providers omit `scope` from the authorize URL and reject explicit `--scope` or
`--scopes` selections locally. Existing node credential files
without protocol options retain form encoding and empty OAuth headers.

Node OAuth credential endpoints require HTTPS, including device authorization,
token polling, code exchange, refresh, and catalog revocation URLs. Local
development permits HTTP only on `localhost`, `127.0.0.1`, and `[::1]`. These
local requests bypass proxies, and `localhost` is pinned to `127.0.0.1`.
Credential requests never follow redirects. Invalid endpoint configurations
fail before credentials are sent.

## Overlay Drift Verification

The weekly guard checks Notion against its official machine-readable spec at
<https://developers.notion.com/openapi.json>. Twelve overlay operations are
present in the current spec. `POST /v1/databases/{database_id}/query` is versioned:
the guard requires this overlay's exact `2022-06-28` pin and checks the live
[official legacy reference](https://developers.notion.com/reference/post-database-query.md).
Missing operations, changed legacy documentation, and failed upstream fetches
fail the guard. Run `python3 scripts/check-catalog-spec-drift.py --overlay
notion.openapi.json` to check just this overlay.
