# Managed Google Workspace Connections

NyxID exposes seven catalog services backed by the existing `google` OAuth
provider. Configure one Google web client on that provider; users choose a
product in **AI Services > Connect Service**, select **NyxID managed**, and approve
Google's consent screen. Each connection has its own encrypted tokens and
refresh lifecycle. Adding Workspace does not automatically create Calendar,
Drive, Gmail, Docs, Sheets, or Slides connections.

| Service | Catalog slug | Default API scopes |
| --- | --- | --- |
| Google Workspace | `api-google-workspace` | `drive`, `calendar`, `gmail.readonly`, and `gmail.send` |
| Google Calendar | `api-google-calendar` | `calendar` |
| Google Drive | `api-google-drive` | `drive` |
| Gmail | `api-google-gmail` | `gmail.readonly` and `gmail.send` |
| Google Docs | `api-google-docs` | `drive` |
| Google Sheets | `api-google-sheets` | `drive` |
| Google Slides | `api-google-slides` | `drive` |

The full API scope prefix is `https://www.googleapis.com/auth/`. All seven also
request `openid email profile`. Workspace bundles Drive, Calendar,
and Gmail; Google does not have a single Workspace OAuth scope. Workspace and
Gmail require `gmail.send` for sending and replying. It is selected and locked in
the permission picker. Both custom and managed OAuth requests must include it,
and Google must return it in the granted scopes before authorization completes.
Native Docs/Sheets/Slides editing uses the three separate services below.
Workspace administration remains outside this bundle.

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
   it, and click **Enable**. Repeat for **Google Calendar API**, **Gmail API**,
   **Google Docs API**, **Google Sheets API**, and **Google Slides API** as needed.
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
   All seven products use the same callback. This server authorization-code
   flow does not require an Authorized JavaScript origin. Add frontend
   origins only if you separately use Google's browser JavaScript SDK.
7. Retain the client ID and secret for the NyxID provider configuration.
   Google production publishing and verification are separate from enabling
   the APIs. Full Drive and Gmail read access are restricted; server access to
   restricted data may require a security assessment unless an exception applies.
   Testing-mode refresh tokens normally expire after seven days when these
   API scopes are requested.

## NyxID Setup

1. Startup seeds the seven service rows and their operation catalogs. This
   addition creates Docs, Sheets, and Slides catalog entries only. Existing
   Google service IDs, slugs, endpoints, credentials, agent bindings, node
   configuration, resource URIs, grants, and audit/billing identities stay in
   place. Repeated startup does not duplicate entries. Legacy Google token
   migration targets only `api-google`; it never chooses a product by the shared
   provider ID and waits if the original catalog target is absent.
   The earlier product rollout replaced the unique service-provider index with
   a nonunique lookup index. Versions predating that rollout recreate the
   one-service constraint and cannot start with multiple product rows present;
   account for that constraint when planning a rollback to those versions.
2. As a NyxID administrator, open **Providers > Manage Providers**, edit the
   existing **Google** provider (slug `google`), and enter the Google
   **Client ID** and **Client Secret**. Set **Credential Mode** to
   **Admin or User** (`both`) for managed and custom-app options, or
   **Admin Only** (`admin`) for managed credentials only. Keep **Active**
   enabled and click **Save Changes**. All seven catalog entries use these
   credentials; users supply only their consent through Google.
3. Retain the seeded OAuth configuration: Google's v2 authorization endpoint,
   `https://oauth2.googleapis.com/token`, PKCE enabled, and extra authorization
   parameters `access_type=offline` and `prompt=consent`. Product scopes are
   resolved per connection; the generic Google provider can keep its identity
   defaults. Keep a single Google provider; all seven products resolve its
   current client credentials, including secret rotations.
4. Open **AI Services > Connect Service** and connect each desired product with
   a Google test account. The managed option appears only when the provider
   has usable client credentials. Custom apps continue to use their own
   connection-pinned credentials.

The authorization endpoint derives the product from the connection's catalog
link, including renamed services, reconnects, and org-owned connections.
Calendar, Drive, and Gmail each reject scopes from the other products. Both
the REST and MCP proxy enforce the product's published operation policy, even if Google returns
a token carrying broader permissions from an existing grant. Google multipart
batch endpoints are not exposed. The new editor services expose their explicit
JSON `:batchUpdate` operations. A NyxID API key must also be authorized for the
chosen service.

Existing Workspace catalog defaults are upgraded at startup to publish Gmail
operations and offer Gmail scopes. The migration updates the original seeded
policy, metadata, and provider requirement scopes. Startup also adds the required
`gmail.send` scope to Workspace and Gmail requirements while retaining other
configured scopes and customized metadata. Existing tokens keep their grants:
reconnect existing Workspace and Gmail connections and approve Gmail sending
access. Adding scopes to Google Cloud's consent configuration alone does not
upgrade an existing token.

## Docs, Sheets, and Slides operations

Each service has one origin. Existing `api-google`, Drive, Calendar, Gmail, and
Workspace operations retain their original paths and request contracts; none
are retargeted to these hosts. Connect each desired editor product explicitly.
Native `documents`, `spreadsheets`, and `presentations` OAuth scopes are not added:
full `drive` already authorizes every published operation, subject to the user's
file permissions. The permission picker also allows `drive.file` (app-authorized
files) and `drive.readonly` (read access); narrower grants do not confer the
full write capability of the default `drive` scope.

| Service and origin | Published operations | Reason for coverage |
| --- | --- | --- |
| Docs — `https://docs.googleapis.com` | `POST /v1/documents`, `GET /v1/documents/{documentId}`, `POST /v1/documents/{documentId}:batchUpdate` | Create, inspect, and edit document content and formatting. Use `includeTabsContent=true` to read all tabs before editing them. |
| Sheets — `https://sheets.googleapis.com` | `POST /v4/spreadsheets`, `GET /v4/spreadsheets/{spreadsheetId}`, `POST /v4/spreadsheets/{spreadsheetId}:batchUpdate`; values `GET`/`PUT /v4/spreadsheets/{spreadsheetId}/values/{range}`, `POST .../{range}:append`, `POST .../{range}:clear` | Structural edits plus the four common cell-value workflows, without requiring agents to construct grid-update batches for simple reads or writes. |
| Slides — `https://slides.googleapis.com` | `POST /v1/presentations`, `GET /v1/presentations/{presentationId}`, `POST /v1/presentations/{presentationId}:batchUpdate` | Create, inspect object IDs, and edit slides, shapes, text, and formatting. |

Every operation's accepted scopes were checked against Google's live discovery
documents on **2026-09-14**, revisions **20260904** for Docs and Slides and
**20260909** for Sheets:
[Docs](https://docs.googleapis.com/$discovery/rest?version=v1),
[Sheets](https://sheets.googleapis.com/$discovery/rest?version=v4), and
[Slides](https://slides.googleapis.com/$discovery/rest?version=v1).
The per-operation scope evidence is committed in
`backend/specs/fixtures/google-editor-scope-acceptance.json`. Recheck it with
`python3 scripts/check-google-editor-scopes.py`. This reads public metadata and
does not perform account operations. The published schemas inline only the
Aevatar admission subset, with no references or union types.

For policy-controlled paths, REST decodes one URI layer; typed MCP encodes each
argument and decodes that layer once; generic MCP takes literal values. A
remaining `%` is rejected, so callers cannot request a second decoding pass.
Only a custom-method suffix declared by the matched template, such as
`:batchUpdate`, remains literal in the forwarded URL. Colons inside A1 values
are percent-encoded as data, matching Google's generated client behavior. A different
verb, empty ID, separator injection, or custom method supplied to an ordinary
wildcard is rejected. Typed calls also match their selected endpoint template
before approval, even if another operation is in the service's allowlist.
Services without an operation policy retain their existing passthrough behavior.

These canonicalization changes apply to **every service with an operation
policy**, including the existing Workspace, Drive, Calendar, and Gmail services.
Previously, REST allowed a percent sign remaining after Axum's URI decode; it
now rejects it. An ASCII space previously failed canonicalization immediately;
it now reaches parameter validation so an explicit constraint can permit quoted
Sheets titles. Spaces in ordinary wildcard IDs remain denied by policy matching.
Existing operation methods, paths, parameters, and bodies are unchanged, but
these shared REST input-validation rules change for existing policy-protected
connections as well as the new editor services.

Only the seven Google product services receive a seeded operation policy.
Administrators can also set `proxy_operation_policy` on other services through
the services API; those services inherit the same canonicalization rules.
Durable/scheduled operation grants use the shared path grammar on **all**
services, even without an operation policy. This includes newly supported
custom-method templates and stricter ordinary parameter validation. See
[Durable grant path compatibility](DURABLE_OPERATION_GRANTS.md#path-compatibility-and-rollout)
for the cross-service impact, including Discord custom-emoji reactions.

Sheets' `range` parameters carry `x-nyxid-path-constraint: sheets_a1_range` on the
OpenAPI **parameter**, outside its JSON Schema. The backend enforces this grammar
in REST, generic MCP, typed MCP, and durable-grant matching. It accepts A1 cells,
cell/row/column ranges, named ranges, and quoted sheet names, including
`Sheet1!A1:B2`, `'Quarter 1'!$A$1:$B$2`, `A:A`, and `1:10`. Columns stop at `ZZZ`,
Google Sheets' column limit. Spaces in quoted names are encoded on forwarding.
Range punctuation does not grant colon permission to `spreadsheetId` or any
other parameter. Append and clear suffixes match outside the captured range.

Literal percent signs in sheet titles are deliberately unsupported on the
policy-controlled values routes. For example, `'Q1 100%'!A1:B2` is rejected for
values get, update, append, and clear over REST and MCP, even though Google
allows that title. Encoding the percent sign as `%25` does not bypass this
restriction: after one decode it is rejected just like any other remaining
percent sign. This keeps range data from opening a second decoding pass. Use a
named range without `%`, or rename the sheet, to address those cells through
these routes.

The batch request arrays intentionally accept Google's individual request
objects without embedding the entire discovery schema. Google validates those
objects and applies its API-specific batch semantics. All mutations carry
approval annotations; batches and value writes that can replace/delete content
are marked destructive. The value-write MCP tools use a `body` object for the
ValueRange payload because it also has a `range` field; pass path `range` and
query `valueInputOption` separately. The clear tool accepts an empty `body: {}`.

These APIs have no account-level credential probe. The UI therefore hides the
automatic Test Agent Key action for the three editor products. Verify with a
read using an existing document ID, or create a temporary document with the
user's authorization and edit it through the published operations. Drive file
listing, moving, exporting, and deleting remain on the existing Drive service.

## Mixed-version rollout

Complete the backend rollout before exposing the new editor services to clients,
or route their connection/OAuth, catalog/discovery, REST, MCP, and approval
observe/redeem traffic only to updated replicas. A new replica seeds shared
catalog rows immediately; that does not make an older replica capable of
enforcing or executing them.

Traffic routing also does not upgrade startup migrations. Before restarting or
rolling back an older backend against the shared catalog, backport the explicit
`api-google` provider-token migration guard: the baseline migrator still chooses
a service using only the shared provider ID.

At baseline `28fd2c44`, older replicas ignore `path_parameter_constraints` when
deserializing policies. Their whole-segment wildcard matcher accepts colons,
so a Sheets values GET/PUT range such as `Sheet1!A1:B2` can still pass the old
policy and be forwarded with its colon encoded. The A1 grammar and the stricter
REST percent handling are not enforced there. The same older matcher does not
understand `{id}:batchUpdate` or `{range}:append`/`:clear`, so those operations
are denied before forwarding. Its forwarder also encodes every colon rather
than preserving declared custom-method suffixes. Replicas with the intermediate
colon grammar but without the A1 constraint instead deny colon-bearing ranges.
These custom-method/range denials fail closed, but the entire mixed-version
window must not be described as fail-closed: the baseline wildcard still accepts
values that updated replicas reject. Requests may therefore succeed or fail
depending on which replica receives them until traffic is confined to updated
replicas or the rollout completes. An exact approval issued against an older
policy projection can also fail revalidation on an updated replica; obtain a
fresh approval if that happens. No stored connections or grants should be
rewritten to complete the rollout.

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
  required `gmail.send` permission and the user's intent to send, submit a
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
