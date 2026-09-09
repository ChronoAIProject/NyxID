# Managed Google Workspace Connections

NyxID exposes four catalog services backed by the existing `google` OAuth
provider. Configure one Google web client on that provider; users choose a
product in **AI Services > Connect Service**, select **NyxID managed**, and approve
Google's consent screen. Each connection has its own encrypted tokens and
refresh lifecycle. Adding Workspace does not automatically create Calendar,
Drive, or Gmail connections.

| Service | Catalog slug | Default API scopes |
| --- | --- | --- |
| Google Workspace | `api-google-workspace` | `drive`, `calendar`, and `gmail.readonly` |
| Google Calendar | `api-google-calendar` | `calendar` |
| Google Drive | `api-google-drive` | `drive` |
| Gmail | `api-google-gmail` | `gmail.readonly` |

The full API scope prefix is `https://www.googleapis.com/auth/`. All four also
request `openid email profile`. Workspace bundles Drive, Calendar,
and Gmail; Google does not have a single Workspace OAuth scope. Workspace and
Gmail offer `gmail.send` as an optional permission for sending and replying.
Native Docs/Sheets editing and Workspace administration are outside this bundle.

Gmail access is limited to `gmail.readonly` and `gmail.send`. NyxID does not
request `gmail.modify`, `gmail.compose`, or `https://mail.google.com/` for these
products. The published Gmail operations support searching/listing messages,
reading message headers and bodies, and sending MIME messages. Deletion, trash,
archive, marking messages read, label changes, and Gmail draft management are
not exposed. An application can keep deletion recommendations in its own data
and let the user review and delete messages in Gmail.

## Google Cloud Setup

Use the project containing the OAuth client you want NyxID to own. Google
sign-in for the NyxID platform is configured separately from this downstream
provider; creating a sign-in client alone does not provision managed services.

1. Open [Google Cloud Console](https://console.cloud.google.com/) and select
   the intended project in the top project selector. Check its **Project ID**
   in **IAM & Admin > Settings**.
2. Go to **APIs & Services > Library**. Search for **Google Drive API**, open
   it, and click **Enable**. Repeat for **Google Calendar API** and **Gmail API**.
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
   - `https://www.googleapis.com/auth/gmail.readonly`
   - `https://www.googleapis.com/auth/gmail.send`

   The Drive and Calendar variants support the narrower selections in NyxID's
   permission picker. Full `drive` is required to manage arbitrary existing files;
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
   All four products use the same callback. This server authorization-code
   flow does not require an Authorized JavaScript origin. Add frontend
   origins only if you separately use Google's browser JavaScript SDK.
7. Retain the client ID and secret for the NyxID provider configuration.
   Google production publishing and verification are separate from enabling
   the APIs. Full Drive and Gmail read access are restricted; server access to
   restricted data may require a security assessment unless an exception applies.
   Testing-mode refresh tokens normally expire after seven days when these
   API scopes are requested.

## NyxID Setup

1. Deploy the backend and frontend changes. Startup creates the four service
   rows and their operation catalogs. Existing Google services and credentials
   remain in place, and repeated startup does not duplicate the new entries.
   Startup replaces the unique service-provider index with a nonunique lookup
   index. Rolling back to an older backend requires removing the new product
   rows first, because the older version recreates the one-service constraint.
2. As a NyxID administrator, open **Providers > Manage Providers**, edit the
   existing **Google** provider (slug `google`), and enter the Google
   **Client ID** and **Client Secret**. Set **Credential Mode** to
   **Admin or User** (`both`) for managed and custom-app options, or
   **Admin Only** (`admin`) for managed credentials only. Keep **Active**
   enabled and click **Save Changes**. All four catalog entries use these
   credentials; users supply only their consent through Google.
3. Retain the seeded OAuth configuration: Google's v2 authorization endpoint,
   `https://oauth2.googleapis.com/token`, PKCE enabled, and extra authorization
   parameters `access_type=offline` and `prompt=consent`. Product scopes are
   resolved per connection; the generic Google provider can keep its identity
   defaults. Keep a single Google provider; all four products resolve its
   current client credentials, including secret rotations.
4. Open **AI Services > Connect Service** and connect each desired product with
   a Google test account. The managed option appears only when the provider
   has usable client credentials. Custom apps continue to use their own
   connection-pinned credentials.

The authorization endpoint derives the product from the connection's catalog
link, including renamed services, reconnects, and org-owned connections.
Calendar, Drive, and Gmail each reject scopes from the other products. Both
the REST and MCP proxy enforce the product's published operation policy, even if Google returns
a token carrying broader permissions from an existing grant. Batch APIs are
not exposed. A NyxID API key must also be authorized for the chosen service.

Existing Workspace catalog defaults are upgraded at startup to publish Gmail
operations and offer Gmail scopes. The migration updates the original seeded
policy, metadata, and provider requirement scopes; administrator customizations
are preserved. Existing tokens keep their grants: reconnect the Workspace
connection and approve the added Gmail permissions before using mail operations.
Choose `gmail.send` when sending or replying is needed. Adding scopes to Google
Cloud's consent configuration alone does not upgrade an existing token.

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
- Gmail: list `/gmail/v1/users/me/messages?maxResults=1`, then read a returned
  message with `GET /gmail/v1/users/me/messages/{id}?format=full`. With the
  optional `gmail.send` permission and the user's intent to send, submit a
  base64url-encoded RFC 2822 MIME message as `{"raw":"..."}` to
  `POST /gmail/v1/users/me/messages/send`. To reply, also set `threadId` in the
  JSON body and include matching `Subject`, `In-Reply-To`, and `References`
  MIME headers. Sending creates the outgoing message in Gmail's Sent folder.
- Workspace: the same Drive, Calendar, and Gmail operations work through the
  Workspace service. Its hosted OpenAPI document combines their definitions.
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
[Calendar scopes](https://developers.google.com/workspace/calendar/api/auth),
[Gmail scopes](https://developers.google.com/workspace/gmail/api/auth/scopes),
[Gmail threads and replies](https://developers.google.com/workspace/gmail/api/guides/threads).
