# Catalog Manager role and catalog CLI

Adding an AI service to the platform catalog means creating a provider (how
users authorize), a catalog service (where requests go), and the NyxID-managed
OAuth client. Admins can do all of it. The **Catalog Manager** role lets an
Operator do it without full Admin rights, and the `nyxid admin provider` and
`nyxid admin catalog` commands run the whole flow from a terminal.

## The role

Catalog Manager is a seeded system role (`catalog_manager`, permission
`nyxid:catalog:services:write`). Like Credits Manager it only takes effect for
an **Operator**; Admins already hold every permission and plain users gain
nothing from it.

```bash
nyxid admin user set-role <user-id> --role operator
nyxid admin user grant-role ops@example.com catalog_manager
nyxid admin user roles ops@example.com
nyxid admin user revoke-role ops@example.com catalog_manager
```

The admin user page lists the role beside Credits Manager. `GET /users/me`
reports `capabilities.manage_catalog`.

A Catalog Manager may:

- create OAuth, device-code and API-key providers;
- edit every field of a provider they created, including its OAuth client;
- on any other provider, including seeded connectors:
  - edit the name, description, default scopes, whether the vendor uses OAuth
    scopes (`supports_oauth_scopes`), documentation URL, API key instructions
    and link, and icon (links must be absolute http(s) URLs without embedded
    credentials or cloud-metadata hosts);
  - set the first OAuth client on a provider that has none;
  - edit its authorization, token, revocation and device URLs, credential
    mode, PKCE and token request settings only **before anyone has connected
    through it**: no `user_provider_tokens`, `user_provider_credentials` or
    `user_api_keys` row references it, including org-owned and API-key
    connections, because those settings decide where users' clients and
    tokens are sent. Once a platform client exists they must also re-enter it in the same
    request (`nyxid admin provider update <slug> --file f.yaml --with-client`),
    proving they already hold the secret. Providers whose client has no secret
    (public device-code clients) cannot be proven this way and stay Admin-only
    for those settings. After people connect, these settings need an Admin;
- upload an SVG icon for any provider;
- create public HTTP catalog services and link them to providers they created;
- on services they created, and on every shared catalog row (public, unowned,
  HTTP, outside the `internal` category, not a Tool offering) including seeded
  ones: edit descriptive fields and the OpenAPI spec URL, add, update, delete,
  discover, pause and publish operation endpoints. The destination URL is
  fixed once an Admin attaches a shared credential, a platform key, or a
  provider someone else created, so those credentials cannot be redirected;
- deactivate or reactivate services they created (not other rows);
- list the services linked to a provider (public rows and their own, without
  credential inspection).

Only an Admin may:

- replace or clear an OAuth client on a provider someone else created, or
  deactivate such a provider;
- change client-bound provider settings once people have connected through a
  provider someone else created;
- delete a provider (this revokes every user's tokens);
- deactivate or delete a service someone else created;
- link a service to a provider someone else created, so a new service cannot
  borrow an existing OAuth client;
- set or change a service's shared credential, platform key, billing,
  inference, identity propagation, token forwarding or delegation, token
  exchange, default headers, WebSocket frame injection, anonymous (no-auth)
  endpoint rules (including through the `/services/{id}/anonymous-endpoints`
  routes), operation policy, developer apps, visibility, category, skills, or
  tool twins;
- write private or person-owned rows, and Tool offerings and their
  operations.

A Catalog Manager owns the providers they create, including the right to
change the token URL or clear the client. Admins should therefore not place
a platform-owned OAuth client on a manager-owned provider; create an
Admin-owned provider instead. Before setting a client on any provider a
Catalog Manager could have edited, an Admin should check its URLs; every
provider update records its `authority` in the audit log. A Catalog Manager's
provider write applies only to the version their permission check read, so a
concurrent Admin change returns a conflict instead of combining with it.

Operations on seeded services that come from a built-in spec are recreated at
the next startup if deleted; pause them instead. Operation metadata such as
`risk` drives assistant confirmation cards, so Catalog Managers are trusted
not to downgrade a destructive operation on shared rows.

A row a Catalog Manager creates is marked `catalog_manager_created`, and
service responses report this server-owned marker so clients can mirror
own-row field rules such as `auth_scheme`. On marked rows a non-admin may change the row or its operation endpoints only
while holding the role, so revoking it also revokes write access, and
anonymous rules stay Admin-only. Rows created before this role keep the
creator rule for creators without the role. Provider
and link audit events record
`"authority": "admin" | "catalog_manager"`; provider updates also record
whether a client was set, never its value.

Provider and catalog service creation/update requires a first-party human
session or access token, including for Admins. API keys, Agent Key login
credentials (`nyxid login --agent-key`), delegated tokens, service accounts and
OAuth-app access tokens cannot run these administration writes. Use an ordinary
human CLI profile. Catalog Editor/Curation service accounts retain their
separate scoped skill-management routes (`/keys/{catalog_uuid}` and
`/catalog-curation`); this guard does not change those non-Tool workflows.

Service create/update requests use a strict documented DTO. Unknown request
fields return a validation error naming the bounded field name, without its
value; invalid types return a generic error so credentials and header values
cannot appear in diagnostics. CLI-only `playbook`, `operations` and `smoke_test`
are removed before service API writes. Use the endpoint routes for operations.

The field rules live in `backend/src/handlers/catalog_authority.rs`. They
destructure every request field, so a new field does not compile until it is
classified.

## Manifests

A manifest lists providers and services. Secrets never go in it: `apply`
rejects `client_id`, `client_secret` and `credential`.

```yaml
variables:                 # optional; ${name} is replaced in every string
  zoho_domain:
    prompt: Zoho data center domain
    default: zoho.com
    allowed: [zoho.com, zoho.eu, zoho.in]
providers:
  - slug: zoho-mail        # any POST /api/v1/providers field
    name: Zoho Mail
    provider_type: oauth2
    authorization_url: https://accounts.${zoho_domain}/oauth/v2/auth
    token_url: https://accounts.${zoho_domain}/oauth/v2/token
    default_scopes: [...]
services:
  - slug: api-zoho-mail    # any POST /api/v1/services field
    name: Zoho Mail
    provider: zoho-mail    # provider slug, resolved to provider_config_id
    base_url: https://mail.${zoho_domain}/api
    auth_method: bearer
```

Variable values come from `--set name=value`, an interactive answer, or the
default, and may contain only letters, digits, `.`, `-` and `_`.

## Commands

All commands accept the usual `--base-url`, `--profile` and `--output json`.

- `nyxid admin catalog add [--template zoho-mail | --file m.yaml]` runs the
  wizard. Without a file or template it asks for each field. It shows the plan,
  applies it after confirmation, prints the redirect URI to register with the
  vendor, asks for the client ID and the secret (hidden), then validates. Use
  `--client-id-env/--client-secret-env`, `--yes` and `--set` to run it without
  a terminal, and `--save` to keep the manifest built by the wizard.
- `nyxid admin catalog apply --file m.yaml [--dry-run]` creates missing rows
  and updates drifted fields by slug. Rerunning it reports `unchanged`. It
  refuses to change `provider_type`, `service_type`, `auth_method` or
  `auth_key_name` on existing rows. It reads scoped inactive inventory and reports
  `inactive` for a matching deactivated row, without creating a replacement.
  Ask an Admin to reactivate a service with
  `nyxid service update <id> --catalog-admin --active`; provider reactivation
  uses `nyxid admin provider update <id> --file reactivate-provider.json` with
  `{"is_active":true}`. Admin sees all inactive rows; a Catalog Manager sees
  their own. A foreign inactive slug is hidden and cannot be detected by a
  manager; ask an Admin before recreating a previously used slug. Duplicate
  same-slug inventory fails closed; explicit UUID targets remain available for
  single-resource icon/provider/endpoint commands.
- `nyxid admin catalog validate --file m.yaml` reports each service as
  `ready`, `pending_credentials` or `failed`, with the scopes, redirect URI,
  type-specific connect and safe GET smoke commands, C1–C6 playbook items,
  operation count/unmanaged rows, and Lark configuration fields. `ready` means
  configuration readiness; the separate playbook may still be incomplete, and
  verification is always `not_verified`.
- `nyxid admin provider create` and `nyxid admin provider update <provider>`
  run a provider wizard when given no `--file`: type, authorization, token
  and device URLs, whether the vendor uses OAuth scopes (`no` means NyxID
  never sends a `scope` parameter), the scopes (matching the vendor's CLI),
  extra authorization parameters, token endpoint authentication, PKCE, who
  supplies the OAuth app, revocation and documentation URLs. Update offers
  the stored values and sends only what changed. Both then offer to set the
  OAuth client.

A provider in `credential_mode: admin` (NyxID managed only, the default)
needs its OAuth client when it is created; `both` and `user` providers can be
created without one. The wizard asks for it before creating the provider, and
`provider create --file` takes `--client-id-env/--client-secret-env`.
`catalog apply` reads it from environment variables named after the provider
slug, for example `ZOHO_MAIL_CLIENT_ID` and `ZOHO_MAIL_CLIENT_SECRET` for
`zoho-mail`. Planning retains only those names: dry-run and validate neither
read their values nor refresh the saved session. Actual mutation reads values
just before sending. OAuth-client requests require HTTPS or exact loopback,
disable redirects, and redact error bodies while retaining HTTP status and
NyxID numeric error code. Device-code providers can use an ID without a secret
when their vendor permits it.
- `nyxid admin provider update <provider> --file f.yaml --with-client` sends
  the OAuth client with the change (prompted, or `--client-id-env` /
  `--client-secret-env`). A Catalog Manager needs it to change client-bound
  settings on a provider whose client someone else set.
- `nyxid catalog endpoint add | update | disable <service>` and
  `nyxid catalog discover <service>` manage a catalog row's operations;
  `nyxid catalog publish <service> --operation <name>` changes publication.
- `nyxid admin provider list | show | link | delete` manage single providers. `set-credentials <provider>` sets the NyxID-managed
  client; it is unrelated to `nyxid service credentials`, which stores a
  user's own OAuth app.

The callback to register with every vendor is
`{BASE_URL}/api/v1/providers/callback`.

## Example: Zoho Mail

```bash
nyxid admin catalog add --template zoho-mail
#   Zoho data center domain of the test mailbox (zoho.com, zoho.eu, ...) [zoho.com]:
#   Plan ... Apply these changes? [y/N]: y
#   zoho-mail: register this redirect URI in the vendor's OAuth app: https://.../api/v1/providers/callback
#   Client ID: ...
#   Client secret (hidden):
#   api-zoho-mail  zoho-mail  ready
nyxid service add api-zoho-mail --oauth        # connect a test mailbox as a user
nyxid proxy request api-zoho-mail accounts
```

The template pins one Zoho data center. Zoho expects comma-separated scopes,
so they are one entry. Its three read-only operations use the documented
`ZohoMail.accounts.READ,ZohoMail.folders.READ,ZohoMail.messages.READ` minimum.
`auth_scheme: Zoho-oauthtoken` makes NyxID send Zoho's documented authorization
header for HTTP, WebSocket and MCP calls. Upload a static SVG with
`nyxid admin catalog icon zoho-mail --file zoho-mail.svg` after checking its brand
terms. The template includes no vendor icon or account credentials.

Zoho documents refresh-token revocation as POST with the token in the URL
query. NyxID's provider revocation transport supports form and JSON payloads,
so the template records sourced `nyxid_incompatible` applicability and leaves
remote revocation unconfigured. Disconnect still removes the NyxID connection.
The source pages are the [Mail API index](https://www.zoho.com/mail/help/api/),
[accounts operation](https://www.zoho.com/mail/help/api/get-all-users-accounts.html)
and [OAuth revocation](https://www.zoho.com/mail/help/api/using-oauth-2.html#revoke).
Configuration readiness and loopback tests leave the real Zoho smoke and
outside-user confirmation gates open.

Bearer catalog services may set optional `auth_scheme` to a vendor's
documented RFC 9110 token of 1–64 ASCII bytes. Omission means `Bearer`, preserving
existing wire values. Spaces, CRLF and non-token characters are rejected. The
field belongs to the catalog row; a user connection cannot override it.
Catalog Managers may set it on creation and on their own marked rows;
changes on foreign or unmarked rows require an Admin. Tools Managers cannot
set it. Admin changes to a Tool's
scheme demote its operations. Clear an existing scheme with `auth_scheme: null`
in an update to restore `Bearer`.

## Uploading provider SVG icons

After creating the provider, upload a reviewed official vendor SVG:

```bash
nyxid admin catalog icon zoho-mail --file ./zoho-mail.svg --output json
nyxid admin provider show zoho-mail --output json
nyxid catalog show api-zoho-mail --output json
```

The command uploads the file to NyxID, then updates the provider's `icon_url`.
All catalog services linked to that provider inherit the icon. Admins may update
any provider; Catalog Managers may also upload icons for seeded and shared providers.

The backend must support `POST /api/v1/providers/{provider_id}/icon` before using
this command. Assets are stored in MongoDB and served publicly at
`/catalog-icons/{id}`; no separate frontend asset deployment is needed.
Each upload receives an immutable asset URL. If the provider update
fails after upload, the existing icon remains selected; correct the error and
rerun the command.

Files must be UTF-8 static SVGs, at most 128 KiB, with a `viewBox`. Scripts,
event handlers, styles, animation, embedded images and external resources are
rejected. Convert style declarations to SVG presentation attributes before
uploading. Shapes, groups, gradients, masks and clipping paths are supported.
Use an official vendor source and observe its brand-use terms. Verify the
resulting URL loads and inspect the catalog on light and dark backgrounds.

## Separate Tools Manager authority

Provider-less Tools use the seeded `tools_manager` role and
`nyxid:catalog:tools:write`, with the same Operator prerequisite. Catalog Manager
alone does not grant Tool writes, and Tools Manager alone does not grant provider
or AI-service administration. Admin publication is required. Both roles can be
granted/revoked through the existing role UI and CLI. See [Tools](TOOLS.md) for the
manifest CLI, wizard, icons, safe metadata and first credential boundaries.

## Six-item playbook and real-service gates

Catalog OAuth onboarding retains the evidence needed to assess each item:

| Item | Readiness evidence |
| --- | --- |
| C1 | Managed client ID and secret presence, without exposing values. Device-code needs an ID; the vendor may not require a secret. |
| C2 | Live default scopes match the manifest, with retained `vendor_cli` or `documented_minimum` HTTPS source. A documented `no_scopes` source plus `supports_oauth_scopes:false` and empty scopes is explicitly not applicable. |
| C3 | Authorization/token URLs, device-code/device-token URLs where applicable, and configured revocation or sourced `vendor_unsupported`/`nyxid_incompatible` applicability. |
| C4 | `credential_mode: admin` or `both`. `user` can remain connectable but fails this managed-client playbook item. `both` still needs C1. |
| C5 | An OpenAPI spec or explicit operations, with at least one active live endpoint. |
| C6 | A static provider SVG served from `/catalog-icons/{uuid}`. Catalog Managers may upload icons for seeded and shared providers. |

API-key providers have C1–C4 marked not applicable, retain C5/C6 and the API
smoke/outside-user gates, and print `service add <slug> --credential-env
VENDOR_API_KEY`. Device-code uses `--device-code`; OAuth2 uses `--oauth`.
Provider-less non-Tool services are outside this six-item OAuth playbook.
Provider-less Tools use their separate lane readiness in [Tools](TOOLS.md).

Evidence fields are CLI-only and never reach API writes or drift comparison:

```yaml
providers:
  - slug: example
    # normal provider fields...
    playbook:
      scope_source:
        kind: documented_minimum  # vendor_cli | documented_minimum | no_scopes
        url: https://vendor.example/docs/scopes
      revocation:
        kind: vendor_unsupported  # configured | vendor_unsupported | nyxid_incompatible
        source: https://vendor.example/docs/revocation
services:
  - slug: api-example
    provider: example
    # normal service fields...
    operations:
      - {name: list_accounts, method: GET, path: /accounts, description: List accounts}
    smoke_test: {operation: list_accounts}
```

Unknown evidence/operation/smoke keys are rejected. Evidence sources require
HTTPS without userinfo. Explicit operations and `openapi_spec_url` are mutually
exclusive. Apply matches operation names, creates missing rows and updates only
authored drift; omitted live rows are retained and reported `unmanaged`, never
deleted. An unchanged rerun performs no mutations. Operations permit only the
listed endpoint contract fields; publication is not a manifest field.

The smoke declaration either names a declared literal GET operation or provides
`{method: GET, path: /accounts}`. Paths accept letters/digits and `/._-`, without
query, headers or placeholders. Validate prints the existing `nyxid proxy
request <slug> <path> --method GET` command and does not execute it.

For provider-backed services, validate always reports these pending gates:
`api_smoke_test_real_vendor` and `outside_user_connection_admin_confirmed`.
An authorized human runs the real-vendor smoke with a test account, and an
Admin confirms an outside user's own vendor connection. Record a redacted date,
Admin, slug and HTTP status class in the verification report and update the
Lark row manually; do not record tokens, account identifiers or client values.
Lark configuration says `configured; not verified`. Local fake-vendor tests
prove the software flow only and never close these real-service gates.

The local acceptance harness is `bash scripts/tools/test-tools-manager-e2e.sh
catalog`. It builds the CLI and uses an owned temporary MongoDB replica set,
fake OAuth authorize/token/revoke endpoints and one loopback GET API. The test
runs manager apply and SVG upload, checks all six playbook items, connects a
second person through OAuth ingress, executes the printed smoke command and
disconnects. It also checks write-free reapply, unreadable credential environment
values during dry run/validate, and redacted CLI output, manifests and audits.
Requires local `mongod`, `mongosh`, Python 3 and Rust; uses no production account.
Recorded results and source snapshots belong in
[the implementation verification report](plans/2026-10-13-tools-manager-verification.md).
