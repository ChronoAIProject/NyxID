# Channel Bot Relay Design

> **ADR-013 update (2026-04-09):** Channel Bot Relay is now a **pure passthrough gateway** per ADR-013. NyxID no longer stores message bodies, attachments, or raw webhook payloads in `channel_messages` — only routing metadata. Synchronous agent replies (HTTP 200 + body) are no longer supported; agents must return 202 and post replies via `POST /api/v1/channel-relay/reply`. See NyxID#221, ADR-013, and [`docs/CHANNEL_EVENT_GATEWAY.md`](./CHANNEL_EVENT_GATEWAY.md) for the full rationale. The earlier deprecation-in-favor-of-service-connections direction (NyxID#191) has been recalled: channel relay and the new HTTP Event Gateway are the first-class inbound paths.

## Overview

NyxID Channel Bot Relay turns NyxID into a **multi-platform messaging gateway**. Users register their own bots (Telegram, Discord, Lark, Feishu), NyxID receives messages via platform webhooks, normalizes them into a common format, routes each message to the correct AI agent's callback URL, and relays the agent's response back to the chat.

Combined with [Agent Isolation](./AGENT_ISOLATION.md), the same NyxID user can wire different messaging platforms (or even different conversations on the same platform) to different AI agents -- each with independent credentials, rate limits, and audit trails.

---

## Problem Statement

Today, connecting an AI agent to a messaging platform requires:

1. **Per-platform bot infrastructure** -- each agent team builds and hosts their own Telegram/Discord/Lark bot
2. **Platform-specific code** -- webhook verification, message parsing, reply formatting differs per platform
3. **No centralized credential management** -- bot tokens scattered across agent configs
4. **No unified audit trail** -- no visibility into which agent handled which message
5. **No agent routing** -- can't send Telegram DMs to Claude and Discord messages to GPT without separate bots

NyxID already solves the equivalent problem for API credentials (proxy gateway). Channel Bot Relay extends this to messaging.

---

## High-Level Architecture

```mermaid
graph TB
    subgraph Messaging Platforms
        TG[Telegram]
        DC[Discord]
        LK[Lark]
        FS[Feishu]
    end

    subgraph NyxID
        WH[Webhook Handlers<br/>per-platform endpoints]
        PA[Platform Adapters<br/>normalize + verify]
        RS[Routing Service<br/>conversation -> agent]
        RL[Relay Service<br/>callback + reply]
        DB[(MongoDB<br/>channel_bots<br/>channel_conversations<br/>channel_messages)]
    end

    subgraph AI Agents
        A1[Claude Code<br/>callback URL A]
        A2[GPT Agent<br/>callback URL B]
        A3[Custom Agent<br/>callback URL C]
    end

    TG -->|webhook| WH
    DC -->|webhook| WH
    LK -->|webhook| WH
    FS -->|webhook| WH

    WH --> PA
    PA --> RS
    RS --> RL
    RL -->|lookup/store| DB

    RL -->|POST normalized msg| A1
    RL -->|POST normalized msg| A2
    RL -->|POST normalized msg| A3

    A1 -->|reply body| RL
    A2 -->|reply body| RL
    A3 -->|reply body| RL

    RL -->|send_reply| PA
    PA -->|platform API| TG
    PA -->|platform API| DC
    PA -->|platform API| LK
    PA -->|platform API| FS
```

---

## Message Flow

### Inbound: Platform -> Agent

```mermaid
sequenceDiagram
    participant U as User (Telegram/Discord/etc.)
    participant P as Platform API
    participant W as NyxID Webhook Handler
    participant A as Platform Adapter
    participant R as Routing Service
    participant RL as Relay Service
    participant DB as MongoDB
    participant AG as AI Agent (callback URL)

    U->>P: Send message
    P->>W: POST /api/v1/webhooks/channel/{platform}
    W->>A: verify_webhook(headers, body)
    A-->>W: OK (signature valid)
    W->>A: parse_inbound(body)
    A-->>W: Vec<InboundMessage>

    loop For each InboundMessage
        W->>R: resolve_agent(bot_id, conversation_id, sender_id)
        R->>DB: Lookup channel_conversations
        DB-->>R: (agent_api_key_id, callback_url)
        R-->>W: AgentRoute

        W->>DB: Insert channel_message (direction: inbound)

        W->>RL: forward_to_agent(message, callback_url)
        RL->>AG: POST callback_url<br/>X-NyxID-Signature: HMAC<br/>X-NyxID-Message-Id: uuid

        alt Sync Reply (200 + body)
            AG-->>RL: { reply: { text: "..." } }
            RL->>A: send_reply(bot, conversation_id, reply)
            A->>P: Platform send message API
            P->>U: Display reply
            RL->>DB: Insert channel_message (direction: outbound)
        else Async Ack (202)
            AG-->>RL: 202 Accepted
            Note over RL: Agent will call /channel-relay/reply later
        else Error/Timeout
            RL->>DB: Update callback_status = "failed"
        end
    end

    W-->>P: 200 OK (always, to prevent platform retries)
```

### Async Reply: Agent -> Platform

```mermaid
sequenceDiagram
    participant AG as AI Agent
    participant H as NyxID Reply Handler
    participant DB as MongoDB
    participant A as Platform Adapter
    participant P as Platform API
    participant U as User

    AG->>H: POST /api/v1/channel-relay/reply<br/>Authorization: Bearer {api_key}
    H->>DB: Lookup channel_message by message_id
    H->>DB: Verify api_key_id matches conversation's agent
    H->>DB: Lookup channel_bot (get encrypted token)
    H->>A: send_reply(bot, conversation_id, reply)
    A->>P: Platform send message API
    P->>U: Display reply
    H->>DB: Insert channel_message (direction: outbound)
    H-->>AG: 200 OK { platform_message_id: "..." }
```

### Bot Registration

```mermaid
sequenceDiagram
    participant U as User (authenticated)
    participant H as NyxID Bot Handler
    participant A as Platform Adapter
    participant P as Platform API
    participant DB as MongoDB

    U->>H: POST /api/v1/channel-bots<br/>{ platform: "telegram", bot_token: "123:ABC" }
    H->>A: verify_bot_token(bot_token)
    A->>P: GET /getMe (or equivalent)
    P-->>A: { id: "bot123", username: "MyBot" }
    A-->>H: BotIdentity

    H->>DB: Check max_bots_per_user limit
    H->>H: Encrypt bot_token (AES-256)
    H->>H: Generate webhook_secret (32 bytes)
    H->>DB: Insert channel_bot (status: pending_verification)

    H->>A: register_webhook(bot, webhook_url, secret)
    A->>P: POST /setWebhook (or equivalent)
    P-->>A: OK

    H->>DB: Update channel_bot (status: active, webhook_registered: true)
    H-->>U: 201 Created { id, platform, bot_username, status }
```

---

## Agent Routing & Isolation

### How Conversations Map to Agents

```mermaid
graph TD
    MSG[Inbound Message] --> R{Routing Service}

    R -->|Step 1| EC{Exact conversation<br/>match?}
    EC -->|Yes| AGENT[Route to bound agent]
    EC -->|No| SS{Step 2: Sender-specific<br/>match in group?}
    SS -->|Yes| AGENT
    SS -->|No| DA{Step 3: Default agent<br/>for this bot?}
    DA -->|Yes| AGENT
    DA -->|No| UNROUTED[Log as unrouted<br/>Optional: send 'not configured' reply]

    AGENT --> CB[POST agent callback_url]
```

### Integration with Agent Isolation (PR #132)

The callback URL lives on the **ApiKey** (the agent), not on individual conversation routes. When a user sets up an agent on NyxID (`nyxid ai-setup agent create --platform claude-code`), they register the agent's callback URL as part of the agent configuration. Conversation routes then just say "send to this agent" -- NyxID already knows how to reach it.

This means:
- **`ApiKey.callback_url`** (new field) -- where NyxID sends channel messages for this agent
- **`ChannelConversation.agent_api_key_id`** -- which agent handles this conversation (callback URL resolved from the API key)
- No `agent_callback_url` on the conversation route -- the URL is a property of the agent, not the conversation

```mermaid
graph LR
    subgraph Channel Relay Layer
        BOT[Channel Bot<br/>Telegram / Discord / Lark]
        CONV[Channel Conversation<br/>agent_api_key_id]
    end

    subgraph Agent Isolation Layer
        AK[ApiKey<br/>platform, callback_url<br/>rate limits, scopes]
        ASB[AgentServiceBinding<br/>per-agent credential override]
    end

    subgraph Proxy Layer
        PS[Proxy Service<br/>credential injection<br/>scope enforcement]
    end

    CONV -->|references| AK
    AK -->|scopes| ASB
    ASB -->|overrides credentials at| PS

    BOT -->|receives messages for| CONV
    CONV -->|forwards to agent via| AK
```

The relay and proxy are **parallel paths, not nested**:

- **Relay path**: Platform -> NyxID webhook -> agent callback URL (message forwarding)
- **Proxy path**: Agent -> NyxID proxy -> downstream API (credential injection)

An agent receiving a message via the relay can then call external APIs through NyxID's proxy using its scoped API key. The agent isolation scope enforcement applies to the proxy call, not the relay.

---

## Data Model

### Entity Relationship

```mermaid
erDiagram
    User ||--o{ ChannelBot : registers
    User ||--o{ ApiKey : owns
    ChannelBot ||--o{ ChannelConversation : has
    ApiKey ||--o{ ChannelConversation : "routes to"
    ApiKey ||--o{ AgentServiceBinding : "binds credentials via"
    ChannelConversation ||--o{ ChannelMessage : contains
    ChannelBot ||--o{ ChannelMessage : "sent/received via"

    ApiKey {
        string id PK "existing model -- new field added"
        string name "human-readable agent name"
        string platform "claude-code | codex | openclaw | generic"
        string callback_url "NEW: where to POST channel messages"
        int rate_limit_per_second "per-agent rate limit"
        int rate_limit_burst "per-agent burst"
        array allowed_service_ids "proxy scope"
        array allowed_node_ids "proxy scope"
    }

    ChannelBot {
        string id PK
        string user_id FK
        string platform "telegram | discord | lark | feishu"
        string label
        bytes bot_token_encrypted
        string platform_bot_id
        string platform_bot_username
        bool webhook_registered
        string webhook_secret_hash
        string status "pending | active | failed | invalid"
        string app_id "Lark/Feishu only"
        bytes app_secret_encrypted "Lark/Feishu only"
        string public_key "Discord only"
        bool is_active
        datetime created_at
        datetime updated_at
    }

    ChannelConversation {
        string id PK
        string user_id FK
        string channel_bot_id FK
        string platform
        string platform_conversation_id
        string platform_conversation_type "private | group | channel"
        string platform_sender_id "optional: per-sender routing in groups"
        string agent_api_key_id FK "which agent handles this"
        bool default_agent "fallback route for unmatched conversations"
        bool is_active
        datetime last_message_at
        datetime created_at
        datetime updated_at
    }

    ChannelMessage {
        string id PK
        string channel_bot_id FK
        string conversation_id FK
        string user_id FK
        string direction "inbound | outbound"
        string platform
        string platform_message_id
        string sender_platform_id
        string sender_display_name
        string content_type "text | image | file | audio | video"
        string text
        array attachments "MessageAttachment[]"
        object raw_platform_data "original JSON for debugging"
        string agent_api_key_id FK
        string callback_status "pending | delivered | failed | timeout"
        string reply_to_message_id FK "for outbound: which inbound this replies to"
        string platform_reply_message_id
        datetime created_at "TTL: 30 days"
    }
```

### MongoDB Indexes

| Collection | Index | Type | Purpose |
|---|---|---|---|
| `channel_bots` | `{ user_id: 1, platform: 1 }` | Unique | One bot per platform per user |
| `channel_bots` | `{ platform: 1, platform_bot_id: 1 }` | Standard | Webhook bot lookup |
| `channel_conversations` | `{ channel_bot_id: 1, platform_conversation_id: 1 }` | Unique | One mapping per conversation |
| `channel_conversations` | `{ user_id: 1, platform: 1 }` | Standard | List user's routes |
| `channel_conversations` | `{ agent_api_key_id: 1 }` | Standard | Find routes for an agent |
| `channel_messages` | `{ conversation_id: 1, created_at: -1 }` | Standard | Conversation history |
| `channel_messages` | `{ created_at: 1 }` | TTL (30d) | Auto-cleanup |

---

## Platform Adapter Trait

```mermaid
classDiagram
    class PlatformAdapter {
        <<trait>>
        +platform_id() str
        +verify_webhook(bot, headers, body) Result
        +parse_inbound(body) Result~Vec~InboundMessage~~
        +send_reply(http, bot, conversation_id, reply) Result~String~
        +register_webhook(http, bot, url, secret) Result
        +verify_bot_token(http, token) Result~BotIdentity~
        +handle_challenge(body) Option~JSON~
    }

    class TelegramAdapter {
        +platform_id() "telegram"
        -Secret header verification
        -Reuses telegram_service.rs
        -No challenge needed
    }

    class DiscordAdapter {
        +platform_id() "discord"
        -Ed25519 signature verification
        -PING/PONG challenge
        -Interaction-based model
    }

    class LarkFamilyAdapter {
        -base_url: String
        +platform_id() "lark" or "feishu"
        -HMAC-SHA256 verification
        -url_verification challenge
        -App access token caching
    }

    PlatformAdapter <|.. TelegramAdapter
    PlatformAdapter <|.. DiscordAdapter
    PlatformAdapter <|.. LarkFamilyAdapter

    note for LarkFamilyAdapter "Single implementation,\nregistered twice:\nlark = larksuite.com\nfeishu = feishu.cn"
```

### Platform-Specific Notes

| Platform | Auth Model | Webhook Verification | Challenge | Send Reply API |
|---|---|---|---|---|
| **Telegram** | Bot token (`123:ABC...`) | `X-Telegram-Bot-Api-Secret-Token` header (constant-time) | None | `POST /bot{token}/sendMessage` |
| **Discord** | Bot token + Application ID | Ed25519 signature (`X-Signature-Ed25519` + `X-Signature-Timestamp`) | `PING` -> `PONG` interaction response | `POST /channels/{id}/messages` with `Authorization: Bot {token}` |
| **Lark** | App ID + App Secret | HMAC-SHA256 on `X-Lark-Signature` header | `url_verification` event -> echo `challenge` | `POST /im/v1/messages` with tenant access token |
| **Feishu** | App ID + App Secret (same as Lark) | Same as Lark | Same as Lark | Same as Lark, different base URL (`open.feishu.cn`) |

### Adding New Platforms

To add a new platform (e.g. WhatsApp, Slack, LINE), create a single adapter file implementing `PlatformAdapter`:

1. `backend/src/services/channel_adapters/whatsapp.rs` (~300 lines)
2. Register in `channel_adapters/mod.rs` and `resolve_adapter()`
3. Add a webhook route in `routes.rs`

No model changes, no callback payload changes, no frontend changes needed. The agent receives the same normalized payload regardless of platform.

### Why no generic/passthrough adapter

A generic adapter that skips normalization and forwards raw webhooks was considered and rejected:

- **Verification**: Each platform has a unique webhook signature scheme (HMAC, Ed25519, secret header). A generic adapter can't verify unknown platforms, so webhooks would either be unauthenticated (security risk) or require the user to implement verification logic somewhere.
- **Normalization**: With `content.text: null` and everything in `raw_platform_data`, the agent must parse every platform's JSON format itself -- defeating the purpose of the relay.
- **Replies**: NyxID needs to know how to call each platform's send API (different URLs, auth schemes, body formats). A generic adapter can't send replies, so the agent would need direct platform API access anyway.
- **User experience**: Setting up a "generic" bot would require the user to understand webhook verification, raw payload formats, and reply mechanics for their platform. At that point they're better off receiving webhooks directly without NyxID in the middle.

The `raw_platform_data` field on the callback payload serves the advanced use case: agents that need platform-specific features (Telegram inline keyboards, Discord embeds, Lark cards) can read it alongside the normalized fields. But the adapter still handles verification, parsing, and reply delivery.

**Bottom line**: Each supported platform gets a dedicated adapter (~300 lines). The cost is low, the user experience is complete (register bot, configure route, done), and agents get both normalized and raw data.

---

## Callback Contract

### NyxID -> Agent (Webhook POST)

NyxID sends a normalized message to the agent's callback URL.

```json
{
  "message_id": "550e8400-e29b-41d4-a716-446655440000",
  "platform": "telegram",
  "agent": {
    "api_key_id": "880e8400-e29b-41d4-a716-446655440000",
    "name": "claude-support-bot"
  },
  "conversation": {
    "id": "660e8400-e29b-41d4-a716-446655440000",
    "platform_id": "12345678",
    "type": "private"
  },
  "sender": {
    "platform_id": "87654321",
    "display_name": "John Doe"
  },
  "content": {
    "type": "text",
    "text": "What is the weather in Tokyo?",
    "attachments": []
  },
  "reply_to_message_id": null,
  "thread_id": null,
  "timestamp": "2026-03-31T12:00:00Z",
  "raw_platform_data": { "update_id": 123, "message": { "...": "full Telegram/Discord/Lark JSON" } }
}
```

**Design rationale:**

The payload normalizes messages into a common format so agents can handle all platforms with one code path. For agents that need platform-specific features (Telegram inline keyboards, Discord embeds/components, Lark interactive cards), the full original webhook JSON is included in `raw_platform_data`. Most agents ignore it; advanced integrations read it directly.

- **`agent.api_key_id`** is the primary agent identifier. Same `ApiKey._id` from agent isolation (PR #132). A shared callback endpoint dispatches based on this value. Use for routing/authorization.
- **`agent.name`** is the human-readable label from key creation (e.g., `"claude-support-bot"`). For logging and display only -- never use for authorization.
- **`sender.platform_id`** is the platform-native user ID. The agent is responsible for mapping this to its own users. If the agent uses NyxID OAuth, it already has `nyxid_user_id` in its user table from login time -- it can match `sender.platform_id` against platform identities it collected during onboarding. NyxID doesn't need to do this lookup because the agent already has the data.
- **No NyxID user IDs for senders** -- the agent knows the bot owner (from its API key), and knows the sender (from its own user table or the optional resolve-sender API). NyxID's job is message transport, not identity resolution.
- **No PII** -- no emails, no NyxID-stored names. `sender.display_name` is platform-provided (Telegram `first_name`, Discord `username`).

**Field Reference:**

| Field | Type | Nullable | Description |
|---|---|---|---|
| `message_id` | UUID | No | NyxID's internal ID for this message record (stored in `channel_messages`). The agent uses this to send async replies via `POST /channel-relay/reply`. |
| `platform` | string | No | Which messaging platform the message came from: `telegram`, `discord`, `lark`, or `feishu`. |
| `agent.api_key_id` | UUID | No | The `ApiKey._id` assigned to this conversation route. This is the agent's identity from agent isolation. A shared callback endpoint dispatches based on this. |
| `agent.name` | string | No | Human-readable name of the API key (e.g., `"claude-support-bot"`). For logging and display only. |
| `conversation.id` | UUID | No | NyxID's internal ID for the conversation route (from `channel_conversations`). Stable across all messages in the same chat. |
| `conversation.platform_id` | string | No | The platform's native conversation identifier (Telegram `chat_id`, Discord `channel_id`, Lark `chat_id`). |
| `conversation.type` | string | No | Conversation kind: `private` (1:1 DM), `group` (multi-user chat), or `channel` (broadcast). |
| `sender.platform_id` | string | No | The message author's ID on the platform. The agent maps this to its own users. |
| `sender.display_name` | string | Yes | Display name from the platform (Telegram `first_name`, Discord `username`). `null` if not provided. |
| `content.type` | string | No | Content kind: `text`, `image`, `file`, `audio`, `video`, `location`, `sticker`, or `unknown`. |
| `content.text` | string | Yes | Text body. Present for `text`; may contain caption for media. `null` for non-text without caption. |
| `content.attachments` | array | No | Non-text attachments: `{ type, url, filename, mime_type, size_bytes }`. Empty `[]` for plain text. |
| `reply_to_message_id` | UUID | Yes | NyxID `message_id` of the message being replied to. `null` for standalone messages. |
| `thread_id` | string | Yes | Platform-native thread ID (Discord threads, Lark threads). `null` if not in a thread. |
| `timestamp` | ISO 8601 | No | When the message was sent on the platform (not when NyxID received it). |
| `raw_platform_data` | object | Yes | The full original webhook payload from the platform (Telegram Update, Discord Interaction, Lark Event, etc.). Use this for platform-specific features like inline keyboards, embeds, interactive cards, or any data not captured by the normalized fields. `null` only if the raw data could not be preserved. |

**Headers:**

| Header | Description |
|---|---|
| `Content-Type` | `application/json` |
| `X-NyxID-Signature` | HMAC-SHA256 of request body, signed with the API key's hash |
| `X-NyxID-Message-Id` | UUID of the `channel_message` record |
| `X-NyxID-Timestamp` | ISO 8601 timestamp (for replay protection) |
| `X-NyxID-Platform` | Platform identifier (`telegram`, `discord`, `lark`, `feishu`) |
| `X-NyxID-User-Token` | Short-lived access token for the bot owner. The agent can use this as `Authorization: Bearer <token>` to call NyxID APIs (proxy, approvals, etc.) on behalf of the user. Scoped to `proxy read`. Absent if token generation fails. |

### Identity Resolution (optional convenience API)

For agents that don't maintain their own user-to-platform mapping, NyxID provides a lookup endpoint. This is a convenience -- most agents integrated with NyxID OAuth already have this data from user onboarding.

```
GET /api/v1/channel-relay/resolve-sender?platform=telegram&platform_id=87654321
Authorization: Bearer nyxid_ag_xxxxx
```

**Response (linked):**
```json
{
  "platform": "telegram",
  "platform_id": "87654321",
  "nyxid_user_id": "770e8400-e29b-41d4-a716-446655440000",
  "linked": true
}
```

**Response (not linked):**
```json
{
  "platform": "telegram",
  "platform_id": "87654321",
  "nyxid_user_id": null,
  "linked": false
}
```

**Resolution checks** (in order):
1. `notification_channels` -- Telegram `telegram_chat_id` matched against `platform_id`
2. `user_provider_tokens` -- Telegram identity tokens with `telegram_user_id` metadata
3. Future: dedicated `channel_identity_links` collection for explicit cross-platform mapping

Scoped to the bot owner's account -- only resolves identities linked to the user who registered the bot.

### Agent -> NyxID (Sync Reply, HTTP 200)

Agent returns a reply in the callback response body:

```json
{
  "reply": {
    "text": "The weather in Tokyo is 22C and sunny.",
    "reply_to_platform_message_id": "optional, for threading",
    "metadata": null
  }
}
```

**Reply Field Reference:**

| Field | Type | Nullable | Description |
|---|---|---|---|
| `reply.text` | string | Yes | The text response to send back to the chat. Required for text replies. |
| `reply.reply_to_platform_message_id` | string | Yes | Platform-native message ID to reply to (for threading). If set, the reply will appear as a threaded response on platforms that support it (Telegram reply, Discord thread, Lark thread). |
| `reply.metadata` | object | Yes | Platform-specific extras (e.g., Telegram `parse_mode`, Discord embed objects). Passed through to the platform adapter. `null` for plain text replies. |

### Agent -> NyxID (Async, HTTP 202 then POST later)

If the agent needs more time (LLM inference, tool calls, etc.), it returns `202 Accepted` with an empty body, then calls back when ready:

```
POST /api/v1/channel-relay/reply
Authorization: Bearer nyxid_ag_xxxxx
Content-Type: application/json

{
  "message_id": "550e8400-e29b-41d4-a716-446655440000",
  "reply": {
    "text": "After checking multiple sources, the weather in Tokyo is 22C and sunny with 60% humidity.",
    "metadata": null
  }
}
```

**Async Reply Field Reference:**

| Field | Type | Nullable | Description |
|---|---|---|---|
| `message_id` | UUID | No | The `message_id` from the original inbound callback payload. Identifies which message this reply is for, so NyxID can resolve the correct conversation and platform to send the reply to. |
| `reply.text` | string | Yes | The text response to send back to the chat. |
| `reply.metadata` | object | Yes | Platform-specific extras, same as sync reply. |

### Callback Flow Decision

```mermaid
flowchart TD
    CB[Agent Callback POST] --> STATUS{Response Status?}

    STATUS -->|200 + body| SYNC[Parse reply JSON]
    SYNC --> SEND[send_reply via adapter]
    SEND --> LOG_OUT[Log outbound message]

    STATUS -->|202 no body| ASYNC[Mark callback_status = delivered]
    ASYNC --> WAIT[Agent calls /channel-relay/reply later]
    WAIT --> SEND

    STATUS -->|4xx / 5xx| ERR[Mark callback_status = failed]
    ERR --> OPT{Send error<br/>msg to chat?}
    OPT -->|configurable| ERRMSG[Platform: 'Agent unavailable']
    OPT -->|no| DONE[Done]

    STATUS -->|timeout| TO[Mark callback_status = timeout]
    TO --> OPT
```

---

## API Endpoints

### Bot Management (authenticated, human-only)

| Method | Path | Description |
|---|---|---|
| `POST` | `/api/v1/channel-bots` | Register a new bot |
| `GET` | `/api/v1/channel-bots` | List user's bots |
| `GET` | `/api/v1/channel-bots/{id}` | Get bot details |
| `DELETE` | `/api/v1/channel-bots/{id}` | Delete bot (deregisters webhook) |
| `POST` | `/api/v1/channel-bots/{id}/verify` | Re-verify bot token and webhook |

### Conversation Routes (authenticated, human-only)

| Method | Path | Description |
|---|---|---|
| `POST` | `/api/v1/channel-conversations` | Create conversation -> agent route (callback URL resolved from `ApiKey.callback_url`) |
| `GET` | `/api/v1/channel-conversations` | List user's routes (filterable by bot, platform, agent) |
| `GET` | `/api/v1/channel-conversations/{id}` | Get route details |
| `PUT` | `/api/v1/channel-conversations/{id}` | Update route (change agent) |
| `DELETE` | `/api/v1/channel-conversations/{id}` | Delete route |

### Relay (API-key authenticated)

| Method | Path | Description |
|---|---|---|
| `POST` | `/api/v1/channel-relay/reply` | Agent sends async reply to a message |
| `GET` | `/api/v1/channel-relay/messages/{conversation_id}` | Get conversation message history |
| `GET` | `/api/v1/channel-relay/resolve-sender` | Resolve a platform sender to a NyxID user (query params: `platform`, `platform_id`) |

### Platform Webhooks (unauthenticated, signature-verified)

| Method | Path | Description |
|---|---|---|
| `POST` | `/api/v1/webhooks/channel/telegram` | Telegram bot webhook |
| `POST` | `/api/v1/webhooks/channel/discord` | Discord interaction webhook |
| `POST` | `/api/v1/webhooks/channel/lark` | Lark event webhook |
| `POST` | `/api/v1/webhooks/channel/feishu` | Feishu event webhook |

---

## Security

### Threat Model

```mermaid
graph TD
    subgraph Threats
        T1[SSRF via callback URL]
        T2[Bot token leakage]
        T3[Webhook forgery]
        T4[Replay attacks]
        T5[Message injection in group chats]
        T6[Agent impersonation on async reply]
    end

    subgraph Mitigations
        M1[Block private IPs, HTTPS-only in prod]
        M2[AES-256 at rest, never in API responses]
        M3[Per-platform signature verification]
        M4[X-NyxID-Timestamp + replay window]
        M5[platform_sender_id scoping on routes]
        M6[api_key_id must match conversation agent]
    end

    T1 --> M1
    T2 --> M2
    T3 --> M3
    T4 --> M4
    T5 --> M5
    T6 --> M6
```

| Concern | Mitigation |
|---|---|
| **SSRF** | Callback URLs validated: HTTPS-only in production, block RFC 1918/loopback ranges, optional domain allowlist |
| **Bot token storage** | AES-256 encrypted at rest (same pattern as `UserApiKey.credential_encrypted`). Never returned in API responses. Only `platform_bot_username` is exposed. |
| **Webhook forgery** | Per-platform verification: Telegram secret header, Discord Ed25519, Lark HMAC-SHA256. All constant-time comparison. |
| **Replay attacks** | Callbacks include `X-NyxID-Timestamp`. Agents should reject messages older than 5 minutes. |
| **Callback authentication** | `X-NyxID-Signature` is HMAC-SHA256 of the request body, keyed with the API key's hash. Agents verify this to confirm the request came from NyxID. |
| **Agent impersonation** | Async reply endpoint requires the calling API key to match the conversation's `agent_api_key_id`. |
| **Rate limiting** | Per-bot rate limiting on inbound webhooks. Per-agent rate limiting on callback dispatch (reuses `PerAgentRateLimiter` from agent isolation). |

---

## Tool Approval API (Aevatar Integration)

Aevatar agents use a **tool approval middleware** that pauses destructive tool calls and asks the user for permission before executing. The approval can be handled locally (in the chat UI) or remotely via NyxID. NyxID's `NyxIdToolApprovalHandler` creates an approval request on NyxID, then polls until the user decides (via Telegram push, mobile app, or web UI).

This is a **prerequisite** for the channel relay: when an agent receives a forwarded message and invokes tools on behalf of the user, destructive tools must go through the approval flow.

### Flow

```mermaid
sequenceDiagram
    participant A as Aevatar Agent
    participant M as Tool Approval Middleware
    participant H as NyxIdToolApprovalHandler
    participant N as NyxID API
    participant U as User (Telegram / Web / Mobile)

    A->>M: invoke tool (e.g. invoke_service)
    M->>M: Check approval mode + destructive flag
    M->>H: request_approval(tool_name, args, is_destructive)

    H->>N: POST /api/v1/approvals/requests
    Note over H,N: { tool_name, tool_call_id, arguments, is_destructive, approval_mode }
    N-->>H: { id, status: "pending", expires_at }

    N->>U: Push notification / Telegram message

    loop Poll (every 2s, up to 45s)
        H->>N: GET /api/v1/approvals/requests/{id}
        N-->>H: { status: "pending" }
    end

    U->>N: POST /api/v1/approvals/requests/{id}/decide { approved: true }

    H->>N: GET /api/v1/approvals/requests/{id}
    N-->>H: { status: "approved" }

    H-->>M: Approved
    M-->>A: Execute tool
```

### New Endpoint: Create Tool Approval Request

```
POST /api/v1/approvals/requests
Authorization: Bearer <user_access_token or api_key>
Content-Type: application/json

{
  "tool_name": "invoke_service",
  "tool_call_id": "call_abc123",
  "arguments": "{\"service_id\":\"...\",\"endpoint_id\":\"...\"}",
  "is_destructive": true,
  "approval_mode": "alwaysrequire"
}
```

**Response (201 Created):**
```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "status": "pending",
  "expires_at": "2026-04-01T12:05:00Z"
}
```

**Field Reference:**

| Field | Type | Required | Description |
|---|---|---|---|
| `tool_name` | string | Yes | Name of the tool requesting approval (max 256 chars) |
| `tool_call_id` | string | No | LLM-generated tool call ID for correlation |
| `arguments` | string | No | Serialized JSON of tool arguments (max 65536 chars) |
| `is_destructive` | bool | No | Whether the tool performs irreversible operations (default: false) |
| `approval_mode` | string | No | Aevatar's approval mode: `"alwaysrequire"`, `"auto"`, `"neverrequire"` (informational, stored but not enforced -- NyxID always creates the request) |

### Polling

Aevatar polls the existing `GET /api/v1/approvals/requests/{id}` endpoint. No dedicated tool-specific polling endpoint is needed — the existing endpoint returns the full `ApprovalRequestItem` which includes `status` and all tool fields.

NyxID uses `"rejected"` as the status value (not `"denied"`). Aevatar should check for `"rejected"` when determining if a request was denied.

### Rollback Compatibility

This design is fully rollback-safe:

| Concern | Design Decision | On Rollback |
|---|---|---|
| **New model fields** (`tool_name`, etc.) | Optional with `#[serde(default)]` | Old code ignores extra fields in MongoDB documents (no `deny_unknown_fields`) |
| **New endpoint** (`POST /requests`) | Additive only, no existing endpoint behavior changes | Old code returns 404; Aevatar times out (expected for new feature) |
| **Status values** | `"rejected"` used consistently everywhere -- no normalization layer | No change needed |
| **Sentinel `service_id: "tool_approval"`** | Tool approval documents appear in `list_requests` with `service_name` = tool name | Old frontend renders them as normal approval entries (odd `service_name` but functional) |
| **Frontend** | Additive: new tool approval rendering in approval list/detail pages. Guards on `tool_name != null` to distinguish tool vs proxy approvals | Old frontend ignores `tool_name` field (not in its type), renders tool approvals as normal entries with tool name as service name -- functional, just not pretty |

**Key rule**: existing endpoints (`GET /requests/{id}`, `GET /requests/{id}/status`, `POST /{id}/decide`) return the same response shapes and status values as before. All new behavior is on new paths.

### Model Changes

Four optional fields added to `ApprovalRequest` (existing collection, no migration needed):

```rust
// On ApprovalRequest model
pub tool_name: Option<String>,        // e.g. "invoke_service"
pub tool_call_id: Option<String>,     // LLM correlation ID
pub tool_arguments: Option<String>,   // serialized JSON
pub is_destructive: Option<bool>,     // destructive flag
```

Tool approval requests use sentinel values for proxy-oriented fields:
- `service_id: "tool_approval"`, `service_name: <tool_name>`, `service_slug: "tool"`
- `requester_type: "api_key"` or `"access_token"` (from auth context)
- `approval_mode: PerRequest` (tool approvals have no grant semantics)

### Relationship to Existing Approval System

The tool approval flow reuses the existing approval infrastructure:
- Same `approval_requests` MongoDB collection
- Same `process_decision` flow (web UI, Telegram, mobile push)
- Same notification pipeline (Telegram bot, FCM, APNs)
- Same `POST /{id}/decide` endpoint for user decisions

The only new surface is `POST /requests` for external creation and the response format alignment.

---

## Implementation Phases

### Phase 0: Tool Approval Endpoint

Prerequisite for agent tool execution. Backend + frontend changes to properly render tool approvals.

**Backend (modified):**
- `backend/src/models/approval_request.rs` -- add optional tool fields (`tool_name`, `tool_call_id`, `tool_arguments`, `is_destructive`), all `Option` with `#[serde(default)]`
- `backend/src/services/approval_service.rs` -- add `create_tool_approval_request()` function
- `backend/src/handlers/approvals.rs` -- add `create_request` handler (`POST /requests`), new request/response types (`CreateToolApprovalRequest`, `CreateApprovalResponse`). Existing `get_request_by_id` adds optional `tool_name`, `tool_call_id`, `tool_arguments`, `is_destructive` fields to `ApprovalRequestItem` (additive, no behavior change -- `null` for existing proxy approvals). Aevatar polls the existing `GET /requests/{id}` endpoint and checks for `"rejected"` status.
- `backend/src/routes.rs` -- add `POST /requests` to approval routes

**Frontend (modified):**
- `frontend/src/types/approvals.ts` -- add optional `tool_name`, `tool_call_id`, `tool_arguments`, `is_destructive` fields to approval request type
- `frontend/src/pages/approvals.tsx` -- distinguish tool vs proxy approvals in list: show "Tool: {tool_name}" badge when `tool_name` is present, skip service link for `service_id === "tool_approval"`
- `frontend/src/pages/approval-detail.tsx` (or equivalent decide page) -- show tool context: tool name, truncated arguments preview, destructive badge. Fall back to existing proxy display when `tool_name` is null

**Rollback**: old frontend ignores new fields, renders tool approvals as normal entries with tool name as service name.

### Phase 1: Foundation

Models, platform adapter trait, error codes, config. Frontend types and schemas (no UI yet).

```mermaid
gantt
    title Phase 1 - Foundation
    dateFormat  X
    axisFormat %s

    section Models
    channel_bot.rs           :a1, 0, 1
    channel_conversation.rs  :a2, 0, 1
    channel_message.rs       :a3, 0, 1
    Register in mod.rs       :a4, after a1, 1

    section Infrastructure
    Error variants (10000-10005)  :b1, 0, 1
    Config env vars               :b2, 0, 1
    DB indexes                    :b3, after a1, 1

    section Trait
    PlatformAdapter trait     :c1, 0, 1
    Normalized types          :c2, 0, 1
```

**Backend (new):**
- `backend/src/models/channel_bot.rs`
- `backend/src/models/channel_conversation.rs`
- `backend/src/models/channel_message.rs`
- `backend/src/services/channel_platform.rs` (trait + types)

**Backend (modified):**
- `backend/src/models/mod.rs` -- register modules
- `backend/src/services/mod.rs` -- register module
- `backend/src/errors/mod.rs` -- new error variants (10000-10005)
- `backend/src/config.rs` -- new env vars
- `backend/src/db.rs` -- new indexes

**Frontend (new):**
- `frontend/src/types/channels.ts` -- TypeScript types for `ChannelBot`, `ChannelConversation`, `ChannelMessage`, platform enum
- `frontend/src/schemas/channels.ts` -- Zod schemas for bot creation/update, conversation route creation/update

**Rollback**: no UI references these types yet; unused exports are harmless.

### Phase 2: Telegram Adapter

First platform adapter, reuses existing `telegram_service.rs`. No frontend changes (adapter internals).

**Backend (new):**
- `backend/src/services/channel_adapters/mod.rs`
- `backend/src/services/channel_adapters/telegram.rs`

### Phase 3: Core Services

Bot CRUD, conversation routing, relay orchestration. No frontend changes (service internals).

**Backend (new):**
- `backend/src/services/channel_bot_service.rs`
- `backend/src/services/channel_routing_service.rs`
- `backend/src/services/channel_relay_service.rs`

### Phase 4: Handlers, Routes & Bot Management UI

Wire up all backend endpoints. Ship the core frontend: bot management page, conversation route editor.

**Backend (new):**
- `backend/src/handlers/channel_bots.rs`
- `backend/src/handlers/channel_webhooks.rs`
- `backend/src/handlers/channel_relay.rs`

**Backend (modified):**
- `backend/src/handlers/mod.rs`
- `backend/src/routes.rs`
- `backend/src/main.rs` (webhook health check background task)

**Frontend (new):**
- `frontend/src/hooks/use-channel-bots.ts` -- TanStack Query hooks for bot CRUD
- `frontend/src/hooks/use-channel-conversations.ts` -- TanStack Query hooks for conversation routes
- `frontend/src/pages/channel-bots.tsx` -- bot list page: table with platform icon, bot username, status, active conversations count
- `frontend/src/pages/channel-bot-detail.tsx` -- bot detail: status, webhook health, conversation routes list, delete action
- `frontend/src/components/dashboard/add-channel-bot-dialog.tsx` -- create bot dialog: platform selector (Telegram only in Phase 4), bot token input, validation feedback
- `frontend/src/components/dashboard/channel-route-editor.tsx` -- conversation route CRUD: agent selector (from user's API keys), default agent toggle

**Frontend (modified):**
- `frontend/src/router.tsx` -- add `/channel-bots` and `/channel-bots/:id` routes
- `frontend/src/components/dashboard/sidebar.tsx` -- add "Channel Bots" nav item

**Rollback**: pages return 404 in router (removed routes), sidebar item disappears. No data loss -- bot records stay in MongoDB, webhook stays registered on Telegram (can be cleaned up manually or on re-deploy).

### Phase 5: Multi-Platform Adapters & Frontend

Discord, Lark, Feishu adapters. Frontend platform selector updated to support all platforms.

**Backend (new):**
- `backend/src/services/channel_adapters/discord.rs`
- `backend/src/services/channel_adapters/lark.rs`

**Backend (new dependency):**
- `ed25519-dalek` (Discord signature verification)

**Frontend (modified):**
- `frontend/src/components/dashboard/add-channel-bot-dialog.tsx` -- platform selector expands from Telegram-only to include Discord, Lark, Feishu. Each platform shows platform-specific setup instructions (e.g., Discord: Application ID + Bot Token; Lark: App ID + App Secret)
- `frontend/src/pages/channel-bot-detail.tsx` -- platform-specific status indicators and webhook health display
- `frontend/src/types/channels.ts` -- add Discord/Lark/Feishu-specific fields to `ChannelBot` type

**Rollback**: old frontend only shows Telegram in platform selector. Discord/Lark/Feishu bots created before rollback still appear in list (platform field renders as raw string) but can't be managed via the old create dialog.

### Phase 6: OpenClaw Bridge Migration

Migrate existing `openclaw_channel_mappings` to the generic relay. Backward-compatible dual-path lookup.

**Backend (new):**
- `backend/src/services/channel_adapters/openclaw.rs`

**Backend (modified):**
- `backend/src/handlers/openclaw_channel.rs` (dual-path lookup: check new `channel_conversations` first, fall back to legacy `openclaw_channel_mappings`)

**Frontend (modified):**
- `frontend/src/pages/channel-bots.tsx` -- show migrated OpenClaw bots alongside user-registered bots (read-only badge for auto-migrated entries)
- `frontend/src/hooks/use-channel-bots.ts` -- filter/display logic for `platform: "openclaw"` entries

**Rollback**: dual-path lookup means old code still reads from `openclaw_channel_mappings` directly. New entries written to `channel_conversations` won't be visible to old code, but old entries continue to work. No data loss.

### Phase 7: Message Log & Polish

Conversation message history viewer, delivery status dashboard, and UX polish.

**Frontend (new):**
- `frontend/src/pages/channel-conversation-detail.tsx` -- message log: inbound/outbound messages with timestamps, delivery status, sender info, content preview
- `frontend/src/hooks/use-channel-messages.ts` -- TanStack Query hooks for message history (paginated)
- `frontend/src/components/dashboard/channel-message-list.tsx` -- chat-style message list component with direction indicators

**Frontend (modified):**
- `frontend/src/pages/channel-bot-detail.tsx` -- add "View Messages" link per conversation route, delivery stats summary (delivered/failed/timeout counts)
- `frontend/src/router.tsx` -- add `/channel-conversations/:id/messages` route

**Rollback**: message log pages return 404. Bot management from Phase 4 continues to work.

---

## Phase Dependency Graph

```mermaid
graph LR
    P0[Phase 0<br/>Tool Approval<br/>BE + FE] --> P1[Phase 1<br/>Foundation<br/>BE + FE types]
    P1 --> P2[Phase 2<br/>Telegram Adapter<br/>BE only]
    P1 --> P3[Phase 3<br/>Core Services<br/>BE only]
    P2 --> P3
    P3 --> P4[Phase 4<br/>Handlers + Bot UI<br/>BE + FE]
    P4 --> P5[Phase 5<br/>Multi-Platform<br/>BE + FE]
    P4 --> P6[Phase 6<br/>OpenClaw Migration<br/>BE + FE]
    P4 --> P7[Phase 7<br/>Message Log<br/>FE only]

    style P0 fill:#fce4ec
    style P1 fill:#e1f5fe
    style P2 fill:#e1f5fe
    style P3 fill:#fff3e0
    style P4 fill:#fff3e0
    style P5 fill:#f3e5f5
    style P6 fill:#f3e5f5
    style P7 fill:#e8f5e9
```

---

## Environment Variables

| Variable | Default | Description |
|---|---|---|
| `CHANNEL_RELAY_CALLBACK_TIMEOUT_SECS` | `30` | HTTP timeout for agent callback requests |
| `CHANNEL_RELAY_MAX_BOTS_PER_USER` | `5` | Maximum bots a user can register |
| `CHANNEL_RELAY_MESSAGE_TTL_DAYS` | `30` | TTL for `channel_messages` auto-cleanup |

---

## Relationship to Existing Systems

| Existing System | Relationship | Migration Path |
|---|---|---|
| **Telegram approval bot** (system-level) | Completely separate. The admin's global bot for approval notifications is untouched. | None needed |
| **Telegram Login Widget** (identity provider) | Separate. Uses Telegram for authentication, not messaging. | None needed |
| **OpenClaw channel bridge** | Superseded. The new relay is a generalized version. | Phase 6: dual-path lookup, gradual migration |
| **Agent isolation** (PR #132) | Complementary. `ApiKey.id` is the `agent_api_key_id` reference. Proxy scope enforcement applies when agents make proxy calls. | Already integrated via shared `ApiKey` model |
| **Proxy gateway** | Parallel path. Relay forwards messages; proxy forwards API calls. Agents may use both. | None needed |
| **Approval system** | Extended. Tool approval creation endpoint (`POST /requests`) added for Aevatar agent tool execution. Reuses existing approval infrastructure. | Phase 0: add creation endpoint + response format alignment |

---

## Example: End-to-End Scenario

### Setup (one-time)

```mermaid
sequenceDiagram
    participant U as User
    participant N as NyxID
    participant TG as Telegram API

    Note over U,N: Step 1: Register agent with callback URL
    U->>N: nyxid ai-setup agent create<br/>--name claude-support<br/>--platform claude-code<br/>--callback-url https://my-claude.example.com/webhook
    N-->>U: API key: nyxid_ag_xxxxx (api_key_id: 880e...)

    Note over U,N: Step 2: Register Telegram bot
    U->>N: POST /api/v1/channel-bots<br/>{ platform: "telegram", bot_token: "123:ABC" }
    N->>TG: getMe (verify token)
    TG-->>N: { username: "MySupportBot" }
    N->>TG: setWebhook (register NyxID webhook URL)
    N-->>U: Bot registered (id: 660e...)

    Note over U,N: Step 3: Route conversations to agent
    U->>N: POST /api/v1/channel-conversations<br/>{ channel_bot_id: "660e...",<br/>  agent_api_key_id: "880e...",<br/>  default_agent: true,<br/>  resolve_sender_identity: true }
    N-->>U: Route created -- all DMs to MySupportBot go to claude-support
```

The callback URL is on the **agent** (API key), not the conversation route. If the user later creates a second agent ("gpt-research") with a different callback URL and routes a Discord bot to it, the same pattern applies.

### Runtime

```mermaid
sequenceDiagram
    participant Alice as Alice (Telegram)
    participant TG as Telegram API
    participant N as NyxID
    participant C as Claude Agent

    Alice->>TG: "Summarize my emails"
    TG->>N: Webhook POST (message from Alice in DM)
    N->>N: Verify Telegram signature
    N->>N: Parse message, resolve route -> claude-support (api_key_id: 880e...)
    N->>N: Resolve callback_url from ApiKey: https://my-claude.example.com/webhook
    N->>C: POST https://my-claude.example.com/webhook<br/>{ agent: { api_key_id: "880e...", name: "claude-support" },<br/>  sender: { platform_id: "87654321", nyxid_user_id: "770e..." },<br/>  content: { text: "Summarize my emails" } }
    C->>C: Match nyxid_user_id to internal user (from NyxID OAuth)
    C->>C: Process with LLM + tools
    C-->>N: 200 { reply: { text: "You have 3 unread..." } }
    N->>TG: sendMessage(chat_id, "You have 3 unread...")
    TG->>Alice: "You have 3 unread..."
```

**Meanwhile, on Discord (same user, different agent):**

```mermaid
sequenceDiagram
    participant Bob as Bob (Discord)
    participant DC as Discord API
    participant N as NyxID
    participant G as GPT Agent

    Bob->>DC: "Generate a report"
    DC->>N: Webhook POST (interaction from Bob)
    N->>N: Verify Ed25519 signature
    N->>N: Parse message, resolve route -> gpt-research (api_key_id: 990e...)
    N->>N: Resolve callback_url from ApiKey: https://my-gpt.example.com/webhook
    N->>G: POST https://my-gpt.example.com/webhook<br/>{ agent: { api_key_id: "990e...", name: "gpt-research" },<br/>  content: { text: "Generate a report" } }
    G-->>N: 202 Accepted (async, needs time)
    N-->>DC: Ack interaction

    Note over G: Agent processes for 30 seconds...

    G->>N: POST /channel-relay/reply<br/>Authorization: Bearer nyxid_ag_yyyyy<br/>{ message_id: "...", reply: { text: "Report: ..." } }
    N->>DC: Create message in channel
    DC->>Bob: "Report: ..."
```
