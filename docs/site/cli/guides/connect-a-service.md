---
title: Connect an AI service
description: The full nyxid service flow — catalog, custom endpoints, OAuth and device-code providers, inspecting, rotating, and routing a connection.
---

The [Get Started flow](/docs/cli/getting-started/first-connection) connects one catalog service in four steps. This guide covers the rest of the surface: discovering what's available, wiring up custom and OAuth services, and maintaining a connection over time.

Every connection provisions three records — an [endpoint, a key, and a service](/docs/shared/concepts/endpoints-keys-services) — in a single `nyxid service add`. You rarely touch them individually.

## Browse the catalog

The catalog is the set of services NyxID already knows how to reach (base URL, auth method, OpenAPI spec).

```bash
nyxid catalog list              # connectable services only
nyxid catalog list --all        # also includes system services
nyxid catalog show llm-openai   # base URL, auth, capabilities for one entry
nyxid catalog endpoints llm-openai  # operations parsed from its OpenAPI spec
```

## Add from the catalog

Pass the catalog slug and point NyxID at the credential via an environment variable so the secret never lands in your shell history:

```bash
export OPENAI_API_KEY=sk-...
nyxid service add llm-openai --credential-env OPENAI_API_KEY
```

The CLI prints a `Slug:` line — that's the handle you proxy through. If you already had an `llm-openai`, the new one is suffixed (e.g. `llm-openai-2`).

:::tip
For multi-field credentials like `aws_sigv4`, use `--credential-file <path>` (or `-` for stdin) instead of `--credential-env`.
:::

## Add a custom (non-catalog) endpoint

For a private or in-house API the catalog doesn't list, use `--custom` with an explicit endpoint and auth method:

```bash
nyxid service add --custom \
  --slug my-internal-api \
  --endpoint-url https://internal.example.com \
  --auth-method bearer \
  --auth-key-name Authorization \
  --credential-env MY_API_KEY
```

`--auth-method` accepts `bearer`, `bot_bearer` (Discord-style `Bot ` prefix), `header`, `query`, `path`, `basic`, `body` (inject into the JSON body), or `none` (no credential). `--auth-key-name` is the header/query/field name the credential goes into.

:::note
If the API lives behind a firewall or on localhost, route it through a [credential node](/docs/cli/guides/credential-node) instead — add `--via-node <node-id>` and hold the credential on the node.
:::

## Connect an OAuth or device-code provider

For providers that use OAuth instead of a static key, swap `--credential-env` for a flow flag. The CLI walks you through authorization in the browser:

```bash
nyxid service add api-lark --oauth
nyxid service add llm-openai --device-code   # for headless / no-browser hosts
```

Request extra scopes on top of the provider's defaults with `--scope` (repeatable, comma- or space-separated). For providers that require your own OAuth app (Lark / Feishu / X), supply the client credentials so per-connection token refresh keeps working:

```bash
export LARK_APP_SECRET=...
nyxid service add api-lark --oauth \
  --oauth-client-id cli_xxx \
  --oauth-client-secret-env LARK_APP_SECRET \
  --scope "contact:contact.base:readonly"
```

`--scope` is **additive** — it adds to the provider's default scopes. To change the scopes of a connection you already have (including removing one), use `service scopes` below.

### Lark / Feishu Approval forwarding

The user OAuth services `api-lark` and `api-feishu` expose native Lark Approval task listing (`approval_tasks_query`) and forwarding (`approval_task_forward`) through the catalog and MCP. The server's startup catalog sync publishes these tools after upgrading; refresh the MCP tool list afterward. Enable the user permissions `approval:task:read` and `approval:task:write` in your Lark/Feishu app, then connect with those additional scopes:

```bash
nyxid service add api-lark --oauth \
  --oauth-client-id cli_your_app_id \
  --oauth-client-secret-env LARK_APP_SECRET \
  --scope "approval:task:read,approval:task:write"
```

For an existing connection, run `nyxid service scopes api-lark` and retain your current scopes while adding both Approval permissions. Reauthorization preserves the same connection. Substitute `api-feishu` for the China region. OAuth does not grant permissions that the app or tenant has not enabled.

List pending tasks with `GET /api/v1/proxy/<SERVICE_ID>/approval/v4/tasks?topic=1`. Take `instance_code` and `task_id` from the same task, check `support_api_operate`, and forward it with:

```http
POST /api/v1/proxy/<SERVICE_ID>/approval/v4/tasks/forward?user_id_type=open_id
Content-Type: application/json

{"instance_code":"<INSTANCE_CODE>","task_id":"<TASK_ID>","transfer_user_id":"ou_recipient","comment":"Please review this request"}
```

Authenticate these proxy requests with your NyxID credentials. The recipient ID must match `user_id_type`; the forwarding tool requires that parameter explicitly. Forwarding changes the current approver and is marked as a destructive write in the catalog; configured NyxID approval policies still apply. The connected Lark user must be allowed to act on the task. Check the response's `code`: Lark can return a provider error with HTTP 200. Avoid blindly retrying if the task has already been processed or its state changed.

This uses the user OAuth `/tasks/forward` API. The older `/tasks/transfer` operation uses a tenant token and a different request body. No bot credentials are substituted for user OAuth. See the [Lark CLI's published API contract](https://github.com/larksuite/cli/blob/7beffb086d7fa3c5b843d8affa7c089f49cfc65e/internal/registry/catalog/services/approval.json) for the user routes and permissions.

## Lark Base

NyxID provides `bitable_fields_list`, standalone `bitable_media_upload`, and
managed `bitable_record_create_with_attachments` for Lark/Feishu Base workflows.
Discover the tools on the selected connection. Attachment operations require
the live owner-chat authority and feature flag described below; an absent tool
is not permission to recreate it with raw uploads and record-create calls.

### Connect the account and grant Base access

Use `api-lark` for a user's OAuth authority, or `api-lark-bot` for a bot app's
tenant authority. China-region equivalents are `api-feishu` and
`api-feishu-bot`. A tenant connection exchanges the app's credentials for a
tenant access token; it does not borrow a user's OAuth token or the inbound
channel bot's stored credential. Choose the exact connection that has access
to the destination Base.

For user OAuth, enable the permissions in the app's developer console and
request them when connecting:

```bash
nyxid service add api-lark --oauth \
  --oauth-client-id cli_your_app_id \
  --oauth-client-secret-env LARK_APP_SECRET \
  --scope "base:field:read,base:record:create,docs:document.media:upload"
```

For tenant access, run `nyxid connect api-lark-bot` and have the human enter the
app credentials in the hosted page. Enable app permissions in the developer
console rather than requesting user OAuth consent for a tenant connection.
For an existing OAuth connection, use `nyxid service scopes <slug>` and retain
its current scopes while adding the required ones. Seed defaults remain
read-only; connecting a service does not automatically grant write permissions.

| Permission | Purpose |
| --- | --- |
| `base:field:read` | Read field definitions and current select options |
| `base:record:create` | Create Base records |
| `docs:document.media:upload` | Upload images or files for Base attachment fields |
| `bitable:app` | Coarse Base read/write alternative listed by these APIs |

API permissions and document access are separate. The connected user needs
access to the Base; for tenant calls, add the exact app through the Base's
**More → Add Applications** menu and grant the needed access. Read-only access
is enough for a field probe, but receipt creation needs write access. An
application scope alone cannot repair the Base document ACL.

The inbound channel bot is configured separately. To download message
resources, its app needs any one of `im:message`, `im:message:readonly`, or
`im:message.history:readonly` in the Lark/Feishu developer console, as listed
by the official [Obtain resource files in messages](https://open.larksuite.com/document/uAjLw4CM/ukTMukTMukTM/reference/im-v1/message-resource/get)
API (`/reference/im-v1/message-resource/get`). These permissions do not grant
the service connection access to write a Base.

### Enable only the required capabilities

Both flags are per person and default off. Upgrade every replica and drain
pre-release workers before enabling either flag.

| `assistant:channel-media` | `assistant:attachment-writes` | Behavior |
| --- | --- | --- |
| Off | Off | Existing behavior; new attachment operations refuse locally |
| On | Off | Owner channel photos/files reach the assistant; attachment-write tools stay unavailable |
| Off | On | Attachment-write tools can use browser uploads; new channel media is not admitted |
| On | On | Owner channel media can feed managed Base receipt writes |

Turning media admission off does not discard previously accepted work. For
queue conversion, admin drain, and rollback completion queries, see
[Assistant uploads](https://github.com/ChronoAIProject/NyxID/blob/main/docs/chat/10-uploads.md#channel-rollout-and-rollback).

### Choose the operation

Operation IDs below belong to the selected connection. Discover their actual
MCP names and schemas instead of guessing a service prefix.

| Operation | Contract |
| --- | --- |
| `bitable_fields_list` | Read field definitions and select options before choosing values. Managed create also validates referenced fields against bounded schema pagination. |
| `bitable_media_upload` | In a live owner chat, upload one authorized `attachment_id` and return a usable `file_token`. NyxID derives size, defaults the safe filename, and supplies `extra.drive_route_token` for `parent_node`. It creates no record and has no managed record-replay ledger. |
| `bitable_record_create_with_attachments` | Validate fields and source completeness, upload the attachments, then create one record under the managed guard and durable replay rules. One composite decision covers the expansion. |

The composite's JSON body is `{fields: object, attachments: {<field name>:
[attachment_id, ...]}}`. It requires a nonempty attachment map and nonempty
arrays, at most ten unique IDs, 20 MiB per file and 50 MiB total, and complete
photo sets from each inbound message. Missing or omitted source photos refuse
the write with `N of M photos from that message could not be read; ask the
owner to resend`. Repeated IDs or identical content under different IDs are
refused before provider calls.

The server reads at most five pages of 100 field definitions before uploading.
Every referenced field must exist. Set attachment fields through `attachments`;
they must have Base attachment type 17 and cannot also appear in `fields`.
Number fields require finite JSON numbers; single-select strings and
multi-select arrays must use existing option names. Unknown options are
refused with `Base select option does not exist; options will not be created`.
Receipt-specific required values, amount bounds and decimal precision come
from the operation scope in the recipe below. The managed route is
`POST /nyxid/managed/bitable/apps/{app_token}/tables/{table_id}/records` inside
NyxID's trusted MCP execution, not a public Lark HTTP endpoint to call directly.

Only live, non-guest owner chat keys can use attachment-write operations.
Ordinary Agent Keys and OAuth plugin connections cannot use them. They are
excluded from `/mcp/config` and exact-operation descriptors, and exact, durable,
and async execution refuse them. Plain `bitable_record_create` keeps its
existing discovery and execution behavior. A custom or mounted copy of the
OpenAPI document cannot grant access to chat attachments.

### Handle duplicates and uncertain outcomes

Managed create identifies photos by content within the same owner, Base app,
and table, even when they are uploaded again in another conversation. An
authorized identical successful replay returns the stored record even after
credential rotation, without uploading or creating again. A changed receipt or
overlapping batch containing a succeeded photo is refused with `Photo already
belongs to record <record_id>; use bitable_record_update to change its fields`.
Use that record for corrections; there is no duplicate-photo override. Add a
deliberate duplicate manually in Lark if needed.

| Result or recorded state | Meaning and next action |
| --- | --- |
| `{record_id, already_created:false}` | One new record was created. |
| `{record_id, already_created:true}` | An identical successful write already created this record, including after credential rotation. |
| `uploading` / `dispatching` before the deadline | `A Base write for this photo is in progress; try again shortly`. |
| `uncertain` | `Base write outcome unknown; check the Base; NyxID will not create it again`. Check manually and stop. |
| `failed_pre_dispatch` | No record create was dispatched; correcting the cause permits a new call. |
| `failed_definite` | A documented provider refusal proves no record was created. Fix the cause before a new call. |
| `failed_definite` with `last_code: not_sent` | The record-create request did not leave NyxID. Earlier reads/uploads may have completed; after fixing the local rejection, a new call may create once. |

Permission failures for managed field reads, uploads and provider create
refusals name the **outbound service slug and connection label**, with the
numeric provider code. Fix that connection's scopes and Base access; inbound
bot permissions do not supply outbound write authority.

Identical-photo resend protection has a **400-day horizon**, independent of
upload retention. Terminal write metadata and photo reservations remain for
400 days; active writes are never deleted by retention. Owner purge removes
both. These rules do not apply to records created outside the managed tool.

After the create dispatch barrier, NyxID never automatically resends a create.
For `uncertain`, **outcome unknown; check the Base; NyxID will not create it
again**. Do not re-call the composite, use plain create, change connections, or
split the batch to work around that result. Failed pre-dispatch and definite
attempts release their photo reservations; succeeded and uncertain attempts
retain them. A retry reuses uploaded tokens only under the same connection,
credential generation and operation contracts; rotation cannot reuse those
tokens, while an already-succeeded identical replay still returns its record.

### Restrict a Receipt-Bot specialist

Grant the chosen Lark UserService to the specialist, then allow only
`bitable_fields_list` and `bitable_record_create_with_attachments`. Pin the
Base/table and require the receipt's dropdown values, date, amount, and photo
field with the [Receipt-Bot operation-scope recipe](https://github.com/ChronoAIProject/NyxID/blob/main/docs/AGENT_OPERATION_SCOPES.md#receipt-bot-recipe).
That worked example is the single source for the complete JSON and the real
specialist/API-key management routes. The grants UI selects operations and
preserves saved value limits; author the recipe's nested limits through the
API. Scoping an ordinary Agent Key does not make it eligible for live-chat
attachment writes.

## Change an OAuth connection's scopes

To re-scope an existing OAuth connection, declare the exact set you want with `service scopes <id|slug> --set`:

```bash
# Replace this connection's scopes with exactly this set (adds and/or removes)
nyxid service scopes api-twitter --set "tweet.read,media.write"

# No --set: opens the picker seeded with the connection's current scopes
nyxid service scopes api-twitter

# Headless host: print a URL + code to approve on another device, then exit
nyxid service scopes api-twitter --set "tweet.read,media.write" --no-wait
```

This re-authorizes the **same** connection in place — its agent bindings, node routing, and org sharing are preserved (it never creates a duplicate). Because changing scopes means re-consenting, the provider's authorization screen still opens in a browser, exactly as when you first add the connection; `--no-wait` lets that browser be on a different device.

What you can remove depends on the provider:

| Provider behavior | Removal |
|---|---|
| Has a token-revocation endpoint (X, Google, Discord, Slack, Twitch, Reddit, TikTok) | Removal applies; the old grant is revoked. |
| No revocation endpoint (Facebook, Spotify, LinkedIn, Microsoft, Lark/Feishu) | NyxID uses only the new scopes, but the old grant lingers until you revoke it in the provider's own settings. |
| Grant-level scope union (GitHub) | Granted scopes are locked — re-auth can't narrow them. You can still add scopes. |

> **Note:** dropping `offline.access` ("Stay connected") removes the refresh token, so the connection stops auto-refreshing and will need reconnecting when the access token expires. Keep it unless you mean to.

## Inspect and verify

```bash
nyxid service list                  # all your services + their slugs and IDs
nyxid service show <id>             # one service's full config
nyxid proxy request <slug> models   # a real proxied call — expect HTTP 200
```

The proxy injects the stored credential server-side; it never travels from your terminal. See [the proxy](/docs/shared/concepts/the-proxy) for the full request path.

## Maintain a connection

Find the service ID with `nyxid service list`, then:

```bash
# Rotate the upstream credential
nyxid service rotate-credential <id> --credential-env NEW_KEY_VAR

# Switch routing between a node and direct
nyxid service route <id> --node <node-id>
nyxid service route <id> --direct

# Retarget or relabel
nyxid service update <id> --endpoint-url https://new.example.com --label "New label"

# Remove it
nyxid service delete <id>
```

## Share a service across an org

Add `--org <id|slug|name>` to `service add` (you must be an org admin). Every member then sees the service in their own `nyxid service list` and proxies through it with their own account — see [Create scoped agent keys](/docs/cli/guides/scoped-agent-keys) and the organizations model.

## Next

- [Your first agent call](/docs/ai/getting-started/first-agent-call) — let an AI agent reach this service over MCP.
- [Set up a credential node](/docs/cli/guides/credential-node) — proxy to localhost / firewalled APIs.
- [Command reference: `service`](/docs/cli/reference/service) — every subcommand and flag, in one place.
