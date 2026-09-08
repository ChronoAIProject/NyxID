# Managed Google Workspace Connections

NyxID exposes three catalog services backed by the existing `google` OAuth
provider. Configure one Google web client on that provider; users choose a
product in **AI Services > Add Service**, select **NyxID managed**, and approve
Google's consent screen. Each connection has its own encrypted tokens and
refresh lifecycle. Adding Workspace does not automatically create Calendar
and Drive connections.

| Service | Catalog slug | Default API scopes |
| --- | --- | --- |
| Google Workspace | `api-google-workspace` | `drive` and `calendar` |
| Google Calendar | `api-google-calendar` | `calendar` |
| Google Drive | `api-google-drive` | `drive` |

The full API scope prefix is `https://www.googleapis.com/auth/`. All three also
request `openid email profile`. Workspace is NyxID's Drive + Calendar bundle;
Google does not have a single Workspace OAuth scope. Gmail, native Docs/Sheets
editing, and Workspace administration are outside this bundle.

## Google Cloud Setup

Use the project containing the OAuth client you want NyxID to own. Google
sign-in for the NyxID platform is configured separately from this downstream
provider; creating a sign-in client alone does not provision managed services.

1. Open [Google Cloud Console](https://console.cloud.google.com/) and select
   the intended project in the top project selector. Check its **Project ID**
   in **IAM & Admin > Settings**.
2. Go to **APIs & Services > Library**. Search for **Google Drive API**, open
   it, and click **Enable**. Repeat for **Google Calendar API**.
3. Open **Google Auth Platform > Branding**. Set the NyxID app name, support
   email, homepage, privacy policy, and terms URLs. Authorized domains are
   registrable domains you control (for example `example.com`), without a
   scheme, port, or path. Verify ownership as required. Do not add `localhost`
   as an authorized branding domain.
4. Under **Audience**, choose **External** for users outside your Google
   Workspace organization. While the app is in **Testing**, add the Google
   accounts used for testing. **Internal** limits sign-in to your organization.
5. Under **Data Access > Add or remove scopes**, add identity scopes and:
   - `https://www.googleapis.com/auth/drive`
   - `https://www.googleapis.com/auth/calendar`
   - `https://www.googleapis.com/auth/drive.readonly`
   - `https://www.googleapis.com/auth/drive.file`
   - `https://www.googleapis.com/auth/calendar.readonly`

   The last three support the narrower selections in NyxID's permission
   picker. Full `drive` is required to manage arbitrary existing files;
   `drive.file` limits access to app-created or explicitly app-authorized
   files. Full `calendar` covers calendar creation and event management.
   Listing scopes here declares the app's data access; the actual permissions
   requested from a user come from NyxID's authorization URL.
6. Under **Clients**, create or edit a **Web application** OAuth client. Add
   the exact NyxID backend callback to **Authorized redirect URIs**:

   ```text
   https://YOUR-NYXID-BACKEND/api/v1/providers/callback
   ```

   Local development may additionally use:

   ```text
   http://localhost:3001/api/v1/providers/callback
   ```

   The host, scheme, port, and path must match the backend's `BASE_URL`.
   All three products use the same callback. This server authorization-code
   flow does not require an Authorized JavaScript origin. Add frontend
   origins only if you separately use Google's browser JavaScript SDK.
7. Retain the client ID and secret for the NyxID provider configuration.
   Google production publishing and verification are separate from enabling
   the APIs. Full Drive access is restricted; server access to restricted
   data may require a security assessment unless an exception applies.
   Testing-mode refresh tokens normally expire after seven days when these
   API scopes are requested.

## NyxID Setup

1. Deploy the backend and frontend changes. Startup creates the three service
   rows and their operation catalogs. Existing Google services and credentials
   remain in place, and repeated startup does not duplicate the new entries.
   Startup replaces the unique service-provider index with a nonunique lookup
   index. Rolling back to an older backend requires removing the new product
   rows first, because the older version recreates the one-service constraint.
2. As a NyxID administrator, open **Providers**, edit the existing **Google**
   provider (slug `google`), and save the Google **Client ID** and **Client
   Secret**. Use credential mode `both` for managed and custom-app options,
   or `admin` for managed credentials only. The provider must be active.
3. Retain the seeded OAuth configuration: Google's v2 authorization endpoint,
   `https://oauth2.googleapis.com/token`, PKCE enabled, and extra authorization
   parameters `access_type=offline` and `prompt=consent`. Product scopes are
   resolved per connection; the generic Google provider can keep its identity
   defaults. Do not create three copies of the OAuth client or provider.
4. Open **AI Services > Add Service** and connect each desired product with
   a Google test account. The managed option appears only when the provider
   has usable client credentials. Custom apps continue to use their own
   connection-pinned credentials.

The authorization endpoint derives the product from the connection's catalog
link, including renamed services, reconnects, and org-owned connections.
Calendar rejects Drive scope requests and vice versa. Both the REST and MCP
proxy enforce the product's published operation policy, even if Google returns
a token carrying broader permissions from an existing grant. Batch APIs are
not exposed. A NyxID API key must also be authorized for the chosen service.

## Verify the Connection

- Calendar: list `/calendar/v3/users/me/calendarList`, create a temporary
  secondary calendar with `POST /calendar/v3/calendars`, create/update/delete
  an event on its ID, then delete the temporary calendar. Check availability
  with `POST /calendar/v3/freeBusy`.
- Drive: create a temporary folder with `POST /drive/v3/files` and
  `mimeType=application/vnd.google-apps.folder`. Upload content with
  `POST /upload/drive/v3/files?uploadType=media`, rename/move it with
  `PATCH /drive/v3/files/{fileId}`, download it with `alt=media`, then delete
  the temporary files and folder. Export native Google documents through
  `/drive/v3/files/{fileId}/export?mimeType=...`.
- Workspace: perform both workflows through the Workspace service. Its hosted
  OpenAPI document combines the exact Drive and Calendar operation definitions.
- Verify token refresh after access-token expiry. Live Google consent and
  refresh require a configured client and test account; local tests use
  synthetic credentials and do not establish Google production readiness.

Prefix API paths above with the connection's NyxID proxy URL, available from
its service details. Binary uploads accept raw bytes over REST; MCP uses the
existing binary-body base64 convention.

## Shared Grant Lifecycle

**Disable/Enable** controls the selected NyxID connection without revoking
Google's grant. Deleting with upstream revocation can affect other connections:
Google documents revocation as removing the account's scopes and tokens for
all OAuth clients in the same Google project. NyxID marks Google as a provider
that revokes the entire grant and uses its existing cascade confirmation for
known sibling connections using that provider/client. Connections in other
providers or other applications sharing the Google project may also need to
reconnect. Use separate Google projects when independent revocation is required.

References: [Google web-server OAuth](https://developers.google.com/identity/protocols/oauth2/web-server),
[Drive scopes](https://developers.google.com/workspace/drive/api/guides/api-specific-auth),
[Calendar scopes](https://developers.google.com/workspace/calendar/api/auth).
