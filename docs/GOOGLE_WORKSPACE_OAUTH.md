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
request `openid email profile`. Workspace bundles Drive, Calendar, Gmail,
and, after multi-origin activation, Docs, Sheets, and Slides; Google does not have a single Workspace OAuth scope. Workspace and
Gmail require `gmail.send` for sending and replying. It is selected and locked in
the permission picker. Both custom and managed OAuth requests must include it,
and Google must return it in the granted scopes before authorization completes.
Native Docs/Sheets/Slides editing is available through the three separate services
and through both Drive and Workspace after the multi-origin activation described below.
Drive includes the editor operations under its existing Drive permissions; Workspace
includes that expanded Drive bundle together with Calendar and Gmail.
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
   product addition creates Docs, Sheets, and Slides catalog entries. Drive and Workspace
   receive the same editor operations when the shared temporary activation gate is enabled. Existing
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

Each separate editor service has one origin. Drive and Workspace also expose these
operations after multi-origin activation. Existing `api-google`, Drive, Calendar,
Gmail, and Workspace operations retain their original paths and request contracts;
none are retargeted to these hosts. Connect Drive, Workspace, or an individual editor
product according to the required scope. An existing connection with full `drive`
access needs no additional OAuth scope for these editor operations; the Google Cloud
project must have the corresponding APIs enabled.
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
Durable grants retain whole-segment path matching. Custom-method (`:verb`) operations
cannot yet receive durable grants; creation is rejected up front.

Sheets' `range` parameters carry `x-nyxid-path-constraint: sheets_a1_range` on the
OpenAPI **parameter**, outside its JSON Schema. The backend enforces this grammar
in REST, generic MCP, and typed MCP proxy matching. It accepts A1 cells,
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
listing, moving, exporting, and deleting are available alongside the editor operations
on Drive and Workspace.

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
  `/drive/v3/files/{fileId}/export?mimeType=...`. After activation, use the same
  Drive connection for the Docs, Sheets, and Slides paths listed above.
- Gmail: list `/gmail/v1/users/me/messages?maxResults=1`, then read a returned
  message with `GET /gmail/v1/users/me/messages/{id}?format=full`. With the
  required `gmail.send` permission and the user's intent to send, submit a
  base64url-encoded RFC 2822 MIME message as `{"raw":"..."}` to
  `POST /gmail/v1/users/me/messages/send`. To reply, also set `threadId` in the
  JSON body and include matching `Subject`, `In-Reply-To`, and `References`
  MIME headers. Sending creates the outgoing message in Gmail's Sent folder.
- Workspace: the same Drive, Calendar, Gmail, and activated Docs/Sheets/Slides
  operations work through the Workspace service. Its hosted OpenAPI document
  combines their definitions and publishes the editor origins at path level.
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

## Drive and Workspace multi-origin routing

`api-google-drive` includes its nine file operations plus the same 13 editor operations as the separate Docs, Sheets, and Slides services. Its public spec at `/api/v1/catalog-specs/google-drive/openapi.json` contains 22 operations. `api-google-workspace` composes that expanded Drive spec with 13 Calendar and three Gmail operations, for 38 operations at `/api/v1/catalog-specs/google-workspace/openapi.json`. The separate editor services remain available. Both combined specs keep `https://www.googleapis.com` as their root server; editor path items copy the product overlays' root servers. OpenAPI operation > path > root precedence identifies each destination without a vendor extension. Composition leaves the raw source overlays and all existing operation definitions unchanged.

### Multi-origin design and decisions

Platform keys cannot be combined with a destination map, including disabled platform-key configurations and legacy catalog master credentials. A bearer injection method alone does not establish authority to send a platform-held key to additional recipients. Admin writes reject this combination; runtime catalog loading, target selection, and master-credential authorization also reject it before dispatch. Drive and Workspace use user-owned Google OAuth credentials; each provider requirement supplies effective bearer injection.

The catalog owns `destination_targets`, mapping stable IDs (`docs`, `sheets`, `slides`) to exact
normalized HTTPS origins. Google recipients are explicitly limited to `docs.googleapis.com`,
`sheets.googleapis.com`, and `slides.googleapis.com`, on port 443. Endpoint and policy `target_id`
fields select only entries in the parent's map. Only in-tree overlays can supply routing origins;
remote and instance specs cannot expand or select destinations. HTTPS is required for all new
destination maps, including custom admin maps. Services without a map retain their existing behavior.

The endpoint selector is a top-level field because older endpoint writers replace `parameters`
wholesale. Those writers leave the independent target field intact. Absent selectors and empty maps
are omitted from serialization; endpoint contract and catalog digests include a target only when
present. Existing operation contracts, positive generations, identifiers, grants, resource URIs,
connections, keys, agent bindings, node configuration, billing attribution, and audit identities are
preserved.

REST, generic MCP, typed MCP, and every exact-approval resolution mode select the operation's origin
before dispatch or execution-authority hashing. The existing destination URL field binds the origin;
execution consumes and revalidates that resolved target without retargeting it. The real v1 and v2
policy projections retain their original shapes. The whole-catalog and exact-view fences have no
special compatibility variant for the added operations: adding operations changes the catalog view
that a human approved. Durable grants bind the endpoint contract, so existing operation grants remain
valid. Nested execution futures are boxed to bound the size of enclosing async state machines
without changing execution order or stack settings.

Selected operations require effective bearer injection. Google keeps its stored service
`auth_method: none` and provider requirement `injection_method: bearer`; changing those stored values
would change existing connection contracts. Agent credential overrides are checked against the same
provider and map recipients at binding creation and before selected execution. Token-exchange
services cannot have destination maps because their exchange configuration can use the destination
URL when minting a credential.

Selected HTTP requests use dedicated backend and node clients that do not follow redirects. This
prevents an injected custom header, query credential, or body from reaching a redirect recipient
without authorization for that hop. Existing services keep their redirect behavior. Selected routes
reject WebSocket upgrades on direct and node paths; Docs, Sheets, and Slides targets are HTTP-only.
Specialized transports, including the ChatGPT transport, cannot replace a selected destination.

Nodes must advertise `http_signature_v2`. The versioned HTTP signature binds service ID/slug, target
ID, normalized origin, timestamp, nonce, method, path, query, and body. Capability checks run before
dispatch markers and again against the owning live socket. Cross-replica checks use capability
metadata in the existing ephemeral connection-owner record, without migrating node configuration or
credential identity. An incompatible node fails closed with code 8013,
`node_http_signature_unsupported`. The node executor rejects a missing or empty selected origin with
HTTP 502 and reason `target_base_url_missing`, before any fallback to a locally configured URL; the
v2 verifier independently requires a normalized HTTPS origin. Non-target calls retain the legacy
wire format and signature behavior.

Served instance specs rewrite the proxy root and remove nested server overrides. A cycle-safe queue
follows local Path Item, Callback, Response, and Link references without expansion or external
fetches. External references to those routing objects are omitted in place with
`x-nyxid-omitted-external-ref` and a reason; the rest of the document keeps serving. Links with an
external `operationRef` are also omitted. Local and `operationId` links keep their other metadata
with the `server` override stripped. Omitted inline Responses retain a fixed description, as required
by OpenAPI. Ordinary schema references, including external schemas, remain untouched.

Omission closes routing pointers that could direct a rich client past the proxy. Rejecting the whole
spec would instead change instance/template fallback behavior. These external routing references
never produced NyxID endpoint rows or tools. An operation whose response is omitted remains in the
served spec, and its template tools remain available. Serving the rewritten document changes neither
the cached parser input nor instance/template catalog precedence.

Cache inputs receive the resolved origin so different targets cannot collide. Billing remains
service-keyed. Target dispatch, completion, and denial events use the existing chained audit append
path and record the stable target ID and sanitized origin as metadata.

The hosted specs always publish 22 Drive operations and 38 Workspace operations. A temporary, off-by-default writer gate orders
readers before activation writes while keeping spec composition independent of database access.
Before activation, editor calls return actionable HTTP 503/code 12300,
`workspace_destinations_not_activated`; this rollout state is excluded from proxy-fault telemetry.
Activation uses a known-default compare-and-set so administrator changes are preserved. A skipped
activation with an empty map logs the first failed precondition at warning level; skipped metadata
updates log at debug level. The activation order and gate removal condition are below.

This design retains the separate product services and source overlays, and requires no credential or
identity migration or billing redesign. Token-exchange targets, selected WebSocket support, and
redirect-hop reauthorization are outside its scope.

### Activation order and approval window

1. Verify the deployed commit on every replica; the version label alone is insufficient (the original `v0.29.0` tag predates target routing). Deploy the new backend readers everywhere with `GOOGLE_WORKSPACE_MULTI_ORIGIN_ENABLED=false` (the default). No editor endpoints activate just by deploying. The hosted specs already list 22 Drive and 38 Workspace operations; editor calls return actionable HTTP 503/code 12300, `workspace_destinations_not_activated`, during this short operator-controlled window.
2. Upgrade every node used by Drive or Workspace, including failover candidates, and verify it advertises HTTP signature v2. Old nodes continue handling non-target operations.
3. Set `GOOGLE_WORKSPACE_MULTI_ORIGIN_ENABLED=true` and restart a backend writer. Startup compare-and-sets each known default Drive/Workspace policy plus absent/empty map, then additively inserts 13 endpoint rows per service. Check the materialized catalogs have 22 Drive endpoints, 38 Workspace endpoints, and the three targets on each. An environment with Workspace already activated adds Drive's editor operations on the next enabled restart. An admin-edited policy/map requires an explicit administrator decision; startup never overwrites it. When activation is skipped with an empty map, startup warns with the first failed precondition name; skipped description/limitation updates are logged at debug level.
4. Leave the gate enabled. It is idempotent and safe on subsequent restarts; disabling it does not reverse persisted activation. Remove this temporary gate once all environments have activated.

Approvals pending at activation may require one re-approval. The exact-approval lifetime defaults to 30 seconds and API settings allow at most 300 seconds, so this affects at most five minutes of outstanding API-configured approvals. The whole-catalog fence remains unchanged: if a caller's visible catalog includes Drive or Workspace, the new operations can cause `catalog_drift` on a pending exact approval for any service in that catalog. Drive/Workspace policy changes also cause `execution_authority_drift` for pending approvals carrying an execution digest; catalog drift is checked first and takes precedence when both changed. Rows predating the execution digest still enforce the catalog fence. Observation reports live drift only after human approval; a request still awaiting the human decision remains pending. Redemption enforces the same fences before provider effects. Durable operation grants bind the endpoint contract, not the catalog or union policy, and existing endpoint grants remain valid. Ordinary connection-level approvals do not gain an exact-catalog fence. Direct database edits to approval timeouts outside the supported API limits can extend the window.

An old backend replica ignores top-level `target_id` and the policy's new target field. It matches a Docs path by method/template and sends it to the legacy `www.googleapis.com` base, where Google returns 404. This degraded request remains within the same Google provider; no credential crosses providers. That same-provider fact is the only reason this is tolerable as a rollback/mixed-version failure mode. A non-Google multi-target rollout requires a gate that excludes old readers before any target metadata is published. The supported order above upgrades readers first. Old startup endpoint writers do not remove the independent top-level target field when replacing `parameters`.
