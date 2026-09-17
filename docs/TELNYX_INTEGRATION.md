# Telnyx integration

NyxID seeds a **Telnyx** connection (`api-telnyx`) for user-owned API keys and a
**Telnyx platform vendor template** (`platform-telnyx`) for storing an
administrator's shared API key. Both use `https://api.telnyx.com/v2` and
`Authorization: Bearer <key>`.

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

## Store platform credentials

After the updated backend starts, open **Admin → Platform Operations → Add
platform vendor**, choose **Telnyx**, and enter the raw API key. This provisions
the provider-less internal `platform-telnyx` credential row through the existing
encrypted, write-only credential flow.

The template has no bound platform operation. The UI reports **No operation
shipped yet**: saving the key prepares it for a future operation and does not
enable platform-funded AI, calls, or messaging. Existing `speak` and
`call_and_say` operations retain their ElevenLabs and Twilio contracts.

To make Telnyx available using the shared key, implement and bind a named,
server-constructed operation with the chosen request limits and account scope,
following [Platform Operations V1](assistant/PLATFORM_OPS_V1_SPEC.md). In
particular, assistant and conversation identifiers on a shared Telnyx account
need ownership enforcement before they can become user-facing operations.

## Upstream references

- [Authentication](https://developers.telnyx.com/docs/development/api-fundamentals/authentication)
- [Developer documentation](https://developers.telnyx.com/llms.txt)
- [Official OpenAPI specification](https://raw.githubusercontent.com/team-telnyx/openapi/master/openapi/spec3.json)
