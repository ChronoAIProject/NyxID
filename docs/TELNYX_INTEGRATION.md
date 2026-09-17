# Telnyx integration

NyxID seeds a **Telnyx** catalog service (`api-telnyx`) that supports user-owned
API keys and an admin-managed shared platform key. It uses
`https://api.telnyx.com/v2` and `Authorization: Bearer <key>`.
Configure this service through **Services** or **Providers → Telnyx → Linked services**.
Both entry points edit the same catalog service.

Choose the setup that matches who owns the key and how users will call Telnyx:

| Setup | Where to configure it | Current behavior |
| --- | --- | --- |
| Personal or organization key | **AI Services**, Telnyx catalog connection | Users call Telnyx with their own account's credential. |
| Shared SaaS/platform key | **Services**, edit the `api-telnyx` catalog entry | Authorized users call Telnyx with the server-held key through their NyxID connection. |

## Admin storage

The shared key lives on the admin-managed catalog row in
`downstream_services.credential_encrypted`. Only an administrator can replace it
or edit `platform_key`, which controls availability and owner grants. New
platform-bound user connections reference that catalog row without creating a
user-owned credential. NyxID checks current access before decrypting the key and
sends it only to the catalog-controlled destination through server transport.
Rotation updates the catalog credential once for its platform-bound connections.

This path uses the ordinary catalog, key provisioning, proxy, MCP discovery, and
billing services.

Other admin and ownership surfaces have different purposes:

| Location | Suitability for Telnyx |
| --- | --- |
| **Services → Telnyx → Edit → Platform key** | Existing admin storage and execution path; recommended for the shared catalog service. |
| A separate admin-created internal HTTP service, such as `telnyx-shared` | Existing alternative when the shared service needs its own name, pricing, or endpoint policy. Configure its credential, explicit platform-key audience, and spec URL. |
| **Admin → Platform Credentials** | Currently populated by registered channel adapters for managed onboarding and webhook credentials. Telnyx is not registered there, and the Telnyx proxy does not read this store. Exposing catalog keys here would require UI/API integration. |
| **Providers → Telnyx → Linked services** | Select `api-telnyx` and choose **Configure service** to edit the same shared key, audience, endpoint policy, and prices. Provider OAuth app credentials remain separate. |
| An organization-owned Telnyx connection | Existing option for sharing an organization's own account with authorized members. This remains an org credential, not a site-wide platform key. |

An internal service whose slug starts with `platform-` is reserved for retired vendor
credential stores and cannot execute or appear in catalog discovery. Use `api-telnyx` or a different slug such as `telnyx-shared`
for the independent path. Separate catalog services currently store separate
credentials; they do not share a central secret reference.

## Offer a shared key through the catalog

This uses NyxID's existing
[platform-key access controls](PLATFORM_KEYS_AND_INFERENCE.md). Requests run on
the shared Telnyx account. Grants control who can use its key; they do not create
separate Telnyx accounts or isolate assistants and conversations per customer.
Use this setup for callers authorized to access that account. A customer-facing
feature needing per-customer resources or restricted call/message behavior needs
server-side validation and ownership checks. Those can live in ordinary
application handlers and services.

1. Create an API key in
   [Telnyx Mission Control](https://portal.telnyx.com/#/app/api-keys).
2. Sign in to NyxID as an administrator. Open **Services** (`/services`), find
   **Telnyx** with slug `api-telnyx`, and choose **Edit**. If it is missing, start
   a backend version containing the Telnyx seed first. Alternatively, open
   **Providers → Telnyx → Linked services**, select `api-telnyx`, and choose
   **Configure service**. Creating a provider offers this same follow-on setup;
   linking never transfers a key from another service.
3. In **Platform key**, paste the raw key into **Replace platform credential**
   and turn on **Enable platform key**. NyxID adds the `Bearer` prefix.
4. Set **Audience** to **Selected people and organizations** and select the
   owners who should have access. **All authenticated users** makes the key
   available to every authenticated user. Neither option grants anonymous access.
5. If NyxID should charge users, configure **NyxID platform key** under
   **Billing lanes** and verify its synchronization status after saving. Storing
   a key does not itself configure a price; existing legacy billing may still
   apply when no lane prices are configured.
6. Click **Save Changes**, review the proposed changes, then click **Confirm changes**.
   An authorized user can open **AI Services**, add
   **Telnyx**, choose **Use NyxID's key**, and click **Connect**. Use server
   routing for a platform key; node routing is unavailable for this binding.
   An existing connection can switch its credential binding on its detail page.
7. Create or select a NyxID API key with access to that connection. Verify the
   setup with the read-only model listing below before sending messages or
   placing calls.

For an unconfigured service, you can save only the credential with **Enable
platform key** off; NyxID stores it encrypted with disabled, restricted access
until you explicitly enable it and configure its audience or grants.

For endpoint restrictions, use **Endpoint policy → Restrict allowed endpoints**
in the shared service editor. Add exact HTTP methods and path templates, such as
`GET /ai/openai/models`. The API uses `proxy_operation_policy`. If
`PLATFORM_REQUIRE_OPERATION_POLICY=true`, a missing policy blocks calls using the
platform credential. An explicitly empty policy denies every operation. Policies
apply to the catalog service, including connections using their own keys, so use
a separate service when the platform offering needs different endpoint access.
These rules do not validate message destinations, request bodies, or ownership of
Telnyx resource IDs. The hosted OpenAPI overlay describes discoverable operations;
it is not an execution allowlist.

Use the returned connection slug, which may differ from `api-telnyx`. Set
`NYXID_BASE_URL`, `NYXID_API_KEY`, and `TELNYX_SERVICE_SLUG` in the calling
environment, then run:

```sh
curl --fail-with-body \
  "${NYXID_BASE_URL}/api/v1/proxy/s/${TELNYX_SERVICE_SLUG}/ai/openai/models" \
  -H "Authorization: Bearer ${NYXID_API_KEY}"
```

The caller supplies its **NyxID** API key. NyxID injects the stored Telnyx key
upstream. A model-list response verifies the proxy and credential path; it does
not verify voice connections, messaging senders, or resource ownership.

## Connect a personal or organization account

Create a key in [Telnyx Mission Control](https://portal.telnyx.com/#/app/api-keys).
In **AI Services**, add **Telnyx** from the catalog and paste the raw key. NyxID
adds the `Bearer` prefix. The existing organization ownership and node credential
flows apply to this connection.

The CLI supports the same catalog entry:

```sh
nyxid catalog show api-telnyx
nyxid service add api-telnyx
nyxid catalog endpoints api-telnyx
```

The add command prompts for the credential. To use an existing environment
variable instead, pass `--credential-env TELNYX_API_KEY`.

For OpenAI-compatible clients, set the base URL to
`{NYXID_BASE_URL}/api/v1/proxy/s/{USER_SERVICE_SLUG}/ai/openai` and authenticate
with a NyxID API key that can access that user service. The connected service's
slug is returned when it is created; it may differ from `api-telnyx`.

## Discovered operations

The hosted overlay at `/api/v1/catalog-specs/telnyx/openapi.json` exposes 16
operations. Startup sync materializes them as catalog endpoints for MCP discovery.

| Capability | Operations |
| --- | --- |
| Model inference | List models, chat completions, embeddings, Responses API |
| AI assistants | List, create, get, chat |
| Conversations | Create a conversation, get its messages |
| Speech | List voices, generate speech |
| Messaging | Send a message, get delivery status |
| Calling | Dial, get call status |

Chat, Responses, and assistant chat expose SSE response metadata. Speech exposes
binary audio metadata. Creating an assistant, chatting with an assistant that may
execute tools, sending a message, and dialing a call carry approval annotations.
These discovery annotations work with NyxID's existing authorization and approval
policies.

Telnyx's current OpenAI-compatible endpoints live under `/ai/openai`; the older
`/ai/chat/completions` and `/ai/models` paths are deprecated. Its full upstream
OpenAPI document exceeds NyxID's 5 MB fetch limit, so the hosted overlay uses a
focused, reference-free schema. The catalog drift check compares all overlay
methods and paths with Telnyx's official specification.

Text-to-speech WebSockets use `/text-to-speech/speech` through the existing proxy
with Telnyx's documented handshake and frame protocol. Telnyx Ultra speech is
HTTP-only. Incoming call media streams and webhooks need a separate application
receiver. Outbound calls require a configured Telnyx voice connection and an
authorized caller ID; messages require a configured sender and any applicable
messaging registration.

## Retired vendor credentials

The former `platform-telnyx` internal vendor store is retired. Startup disables
legacy vendor rows and their bindings; runtime guards also reject rows written by
older replicas after startup. Historical credentials, audit, and usage remain
stored. No credential is automatically copied to `api-telnyx`; an administrator
must explicitly supply the desired credential in the shared service editor.
See [service configuration and retirement](SERVICE_CONFIGURATION.md).

## Upstream references

- [Authentication](https://developers.telnyx.com/docs/development/api-fundamentals/authentication)
- [Developer documentation](https://developers.telnyx.com/llms.txt)
- [Official OpenAPI specification](https://raw.githubusercontent.com/team-telnyx/openapi/master/openapi/spec3.json)
