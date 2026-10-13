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

A row a Catalog Manager creates is marked `catalog_manager_created`. On
marked rows a non-admin may change the row or its operation endpoints only
while holding the role, so revoking it also revokes write access, and
anonymous rules stay Admin-only. Rows created before this role keep the
creator rule for creators without the role. Provider
and link audit events record
`"authority": "admin" | "catalog_manager"`; provider updates also record
whether a client was set, never its value.

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
  `auth_key_name` on existing rows. It matches active rows only; a
  deactivated row with the same slug makes the create fail, so reactivate it
  first (`nyxid service update <id> --catalog-admin --active`).
- `nyxid admin catalog validate --file m.yaml` reports each service as
  `ready`, `pending_credentials` or `failed`, with the scopes, redirect URI,
  a test command, and values for the Lark Platform Accounts row.
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
`zoho-mail`; a dry run notes which providers still need one.
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
so they are one entry. NyxID sends `Authorization: Bearer`, while Zoho
documents `Zoho-oauthtoken`; a 401 with a fresh token means Zoho rejected the
scheme and NyxID needs a dedicated auth method.

## Uploading provider SVG icons

After creating the provider, upload a reviewed official vendor SVG:

```bash
nyxid admin catalog icon zoho-mail --file ./zoho-mail.svg --output json
nyxid admin provider show zoho-mail --output json
nyxid catalog show api-zoho-mail --output json
```

The command uploads the file to NyxID, then updates the provider's `icon_url`.
All catalog services linked to that provider inherit the icon. Admins may update
any provider; Catalog Managers may update only providers they created.

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
