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
  provider is therefore seeded with `default_scopes: None`, and there is no
  entry in `platform_scope_allowlist` — that allowlist exists to stop a shared
  platform app requesting un-vetted scopes, and with no scope parameter there
  is nothing to gate.
- **`owner=user` is mandatory** on the authorize URL. It is seeded in
  `extra_auth_params`; removing it breaks the flow.
- **Token exchange uses HTTP Basic** (`client_secret_basic`), not form-body
  credentials.
- **No PKCE.** Notion's OAuth implementation does not support it.
- **Access tokens do not expire** and the token response carries no
  `refresh_token` or `expires_in`. Both are already optional on the callback
  path, so the token persists without an expiry and is never refreshed.
- **Every request needs a `Notion-Version` header.** It is seeded as an
  overridable default request header (`2022-06-28`), so callers that pin their
  own version still win.

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
   the `Notion-Version` default header. Existing rows are never overwritten.
2. As a NyxID administrator, open **Providers > Manage Providers**, edit
   **Notion** (slug `notion`), and enter the **Client ID** and **Client
   Secret**. Set **Credential Mode** to **Admin or User** (`both`) for managed
   plus custom-app options, or **Admin Only** (`admin`) for managed only. Keep
   **Active** enabled and save.
3. Leave the seeded OAuth configuration alone: the authorize and token URLs,
   `client_secret_basic`, PKCE off, `owner=user`, and the empty default scopes.
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
- File uploads go to a signed URL that Notion returns, not through the NyxID
  proxy, so `supports_proxy_binary_upload` is false for this service.
- Tokens do not expire, so there is no refresh path to monitor — but equally,
  a compromised token stays valid until revoked.

## Follow-ups not in this change

- No hosted OpenAPI overlay is registered for `api-notion`, so MCP exposes the
  generic proxy tool rather than named operations. Adding an overlay under
  `backend/specs/catalog/` and registering it in `catalog_spec_registry` would
  publish concrete operations.
- No brand glyph is registered in `frontend/src/components/service-icons/`.
