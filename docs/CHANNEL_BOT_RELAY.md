# Channel Bot Relay Design

> **ADR-013 update (2026-04-09):** Channel Bot Relay is now a **pure passthrough gateway** per ADR-013. NyxID no longer stores message bodies, attachments, or raw webhook payloads in `channel_messages` — only routing metadata. Synchronous agent replies (HTTP 200 + body) are no longer supported; agents must return 202 and post replies via `POST /api/v1/channel-relay/reply`. See NyxID#221, ADR-013, and [`docs/CHANNEL_EVENT_GATEWAY.md`](./CHANNEL_EVENT_GATEWAY.md) for the full rationale. The earlier deprecation-in-favor-of-service-connections direction (NyxID#191) has been recalled: channel relay and the new HTTP Event Gateway are the first-class inbound paths.

## Overview

NyxID Channel Bot Relay turns NyxID into a **multi-platform messaging gateway**. Users register bots or connect accounts (Telegram, Discord, Lark, Feishu, Slack, WhatsApp, X). Adapter-owned webhooks or polling normalize inbound messages into a common format, route each message to the correct AI agent's callback URL, and relay the agent's asynchronous response back to the chat.

Combined with [Agent Isolation](./AGENT_ISOLATION.md), the same NyxID user can wire different messaging platforms (or even different conversations on the same platform) to different AI agents -- each with independent credentials, rate limits, and audit trails.

---

## X DM Accounts

X is managed-only: users connect their own X account with OAuth and provide no developer credentials. NyxID uses the account's user-context token to poll incoming one-to-one and group DMs and deliver the routed agent's asynchronous reply. App-only bearer tokens cannot access private DMs. Although X also supports legacy OAuth 1.0a user-context credentials, a BYO path would require users to supply and maintain developer credentials, so this channel deliberately does not expose one.

### Platform setup and pricing

Admin > Platform Credentials lists every registered adapter's provider descriptor. Meta uses encrypted `platform_credentials`; X's `ProviderOAuth { provider_slug: "twitter" }` backing reads and writes the **same encrypted Client ID and Client Secret** used by the existing `twitter` OAuth provider. There is no second copy of X's app credentials. Clearing either X credential clears a shared provider field, retaining the provider record, and stops all of the `twitter` provider's OAuth connections and logins until restored. Both provider-wide and per-field clear buttons require this impact confirmation; CLI clears require `--confirm-shared-provider`. Update and delete audits record `shared_provider_slug`. Rotating credentials affects all connections using the shared app. Secret values never appear in admin responses. The descriptor supplies fields, setup checklist, backing information and callback URL for the generic API, page and CLI.

Configure a confidential Web App with OAuth 2.0 user authentication and PKCE in the X Developer Console. Set the callback to `{BASE_URL}/api/v1/providers/callback`, exactly as displayed in Platform Credentials. Enable `tweet.read users.read dm.read dm.write offline.access`. Seed migration additively includes these scopes in `twitter.default_scopes`; it does not grant new permissions to old connections. Completion rejects connections without every required scope and requires fresh consent.

**Pricing checked 2026-09-08:** [X's current pricing documentation](https://docs.x.com/x-api/getting-started/pricing) specifies pay-per-usage credits with no subscriptions, replacing the older Basic/Pro tier assumption. Published DM Event reads cost $0.010 per resource and DM Interaction creates cost $0.015 per request. Fund and monitor **NyxID's shared app** in the [Developer Console/pricing page](https://developer.x.com/#pricing): all customers' DM traffic consumes that app's credits and caps. Prices and access entitlements can change; the console is authoritative. Repeated polling can retrieve already-seen resources, so budget from provider usage reports rather than treating every lookup as free.

[Current rate limits](https://docs.x.com/x-api/fundamentals/rate-limits): `/2/dm_events` permits 15 reads per 15 minutes per user. Existing-conversation message creation permits 15 per 15 minutes and 1,440 per day per user, plus 1,440 per day per app. X therefore declares a 60-second minimum poll interval. The generic sweep defaults to 30 seconds (`CHANNEL_POLL_INTERVAL_SECS`; `0` disables). A quiet, healthy account normally receives new DMs within roughly 60-90 seconds, plus callback processing; pagination, provider backoff and sweep load can increase latency.

### Connect, route and recover

Use Add Channel Bot > X (Twitter), or `nyxid channel-bot register --platform x --managed`. The CLI checks the server's managed bootstrap and prints `/channel-bots?connect=x`; optional label and org scope are preserved. An unconfigured app shows an explicit unavailable state.

`GET /api/v1/channel-bots/managed-onboarding/x` returns `available`, `flow: "oauth_connection"`, `provider_slug`, `required_scopes` and `authorize_start_url`. `POST` to its `/start` endpoint accepts `{label, target_org_id?}` and creates an owner-bound OAuth connection through existing OAuth/PKCE machinery. Channel-started connection rows are tagged `source: "channel_onboarding"` with a unique `source_id`. The existing OAuth refresh sweep deletes pending rows older than one hour, retaining completed connections. Cleanup is disabled when `OAUTH_REFRESH_SWEEP_INTERVAL_SECS=0`. The existing popup completion protocol carries only a nonce and completion status. `POST` to `/complete` accepts `{connection_id, label, target_org_id?}`; this ID is the `user_api_keys` row UUID, not the OAuth protocol's internal connection UUID. Completion validates ownership, platform credential provenance and scopes, verifies `/2/users/me`, initializes the cursor and applies the ordinary duplicate-account and bot-limit checks.

All managed bootstrap/start/complete and reconnect routes require an authenticated human. API keys, delegated tokens, service accounts and relay tokens are rejected by the existing human-only middleware. Writes share the existing per-user managed-onboarding rate limit. Org creation and reconnect require the same owning-org admin access as other channel bots; an actor's personal connection cannot back an org bot.

The bot starts `active` with `credential_source: "connection"` and `webhook_registered: false`. A polling descriptor suppresses webhook setup UI; no callback or webhook verification secret is needed. Configure conversation/default-agent routing exactly as for other channels. Detail and `channel-bot show` include the connected handle, connection UUID, cursor, last/next poll, error count and locally authored failure cause.

Polling, Verify Bot and async replies resolve the live owner-bound connection and reuse `refresh_user_api_key_in_place`. A deleted/revoked connection, insufficient scopes or permanent refresh failure marks the bot `failed` with a metadata-only audit event; there is no stale-token fallback. Transient polling errors back off exponentially, and five consecutive failures stop polling with an audit. Rate limits defer polling without counting as an error. Reconnect runs OAuth again and posts `{connection_id}` to `POST /api/v1/channel-bots/{id}/reconnect`; it requires the same X account, swaps the connection and preserves the committed cursor. The next sweep processes DMs received during the failure through the normal bounded page walk and deduplication. Only a missing cursor is initialized to the newest event. Delete removes the bot and routes but **retains the OAuth connection**; revoke that connection separately when appropriate.

### Polling and message semantics

The adapter's `Ingestion::Poll` and `CredentialResolution::OAuthConnection` hooks drive generic sweeps. Up to eight due bots per adapter are processed concurrently, and a per-bot failure does not stop the other bots in that tick. Each due bot is atomically claimed by `find_one_and_update`, with a 90-second lease renewed every 20 seconds and a 15-minute work deadline. Claims, renewals and cursor commits are fenced; reconnect invalidates the previous claim. All outcomes attempt to release the lease. An abrupt process exit recovers at lease expiry. Backoff, cursors, timestamps and error counts are persisted with BSON datetime compatibility for legacy rows.

[The DM events endpoint](https://docs.x.com/x-api/direct-messages/get-dm-events) does **not** support `since_id`. The adapter requests 100 `MessageCreate` events per page, follows `pagination_token` newest-first until the saved event ID (or an older ID) is reached, and emits the batch chronologically. Own-account messages are skipped. At onboarding it saves the newest existing ID without emitting history; empty accounts use an explicit empty baseline. At most ten pages (about 1,000 events) are walked. When that limit is reached before the cursor, the fetched batch is emitted chronologically and the cursor advances to its newest event after processing. Older DMs are skipped, with a persistent, locally authored `last_poll_notice` on the detail page and CLI; this does not count as an error or fail the bot. A processing error or a 429 before the bounded window completes retains the old cursor. X exposes only the last 30 days of events.

The shared pipeline retains the existing webhook behavior, metadata-only message storage, routing, callback telemetry and touch semantics. X opts into the existing bot/platform/message-ID dedup hook. Overlapping completed batches are suppressed. Callback delivery retains the existing relay semantics: a stored message is not automatically redelivered after callback failure. This is not an exactly-once end-to-end delivery guarantee, and an interrupted provider reply may already have sent an earlier chunk.

One-to-one IDs have `{smaller_user_id}-{larger_user_id}` form; group DMs have their own conversation ID. Sender display names and media are normalized from expansions; raw event data is forwarded to the agent and never logged or persisted as a message body. Media URLs are private and may need the connected user's bearer token; the callback never exposes that token. `referenced_tweets` contains shared posts, **not DM reply targets**. Current docs expose no DM reply-reference field; optional `referenced_events` reply references are preserved when supplied, without inventing references from shared posts.

Replies call only `POST /2/dm_conversations/{dm_conversation_id}/messages`, supporting existing private/group conversations. Text is split into chunks of at most 10,000 Unicode characters. `metadata.attachments` accepts one `{media_id}` uploaded by that user; it is attached to the first chunk only. Editing is unsupported. 401, 403 and 429 errors use local messages and rate-limit retry delay, never upstream error prose. The current v2 schema does not publish a text maximum; the established 10,000-character DM limit could not be independently reconfirmed from the Help Center during implementation (HTTP 403).

### Automation policy and verified limits

Automated replies to inbound DMs are allowed under [X's automation rules](https://help.x.com/en/rules-and-policies/x-automation) subject to user intent, consent and opt-out requirements. Unsolicited automated outbound DMs are not permitted. NyxID's channel reply authorization binds an existing inbound message/conversation; this adapter never initiates a conversation. Operators remain responsible for consent, opt-out handling and compliant agent responses. The policy page returned HTTP 403 during this implementation, so its current exact wording could not be rechecked. The current pricing page lists DM webhook charges; Enterprise-only webhook entitlement was not independently confirmed. This implementation deliberately uses the requested polling model and needs no webhook subscription.

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
        RL->>AG: POST callback_url<br/>X-NyxID-Callback-Token: JWT<br/>X-NyxID-Signature: HMAC<br/>X-NyxID-Message-Id: uuid

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
        bytes lark_verification_token_encrypted "Lark/Feishu only"
        bytes lark_encrypt_key_encrypted "Lark/Feishu only, optional"
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
        +registration() RegistrationDescriptor
        +registration_token(fields) Secret
        +updated_token(current, fields) OptionalSecret
        +build_verify_secrets(keys, bot) PlatformVerifySecrets
        +webhook_policy(body) WebhookPolicy
        +subscription_handshake(bot, query) String
        +prepare_webhook(bot, secrets, headers, body) PreparedWebhook
        +verify_webhook(bot, secrets, headers, body) Result
        +parse_inbound(body) Result~Vec~InboundMessage~~
        +reply_context(thread_id, created_at, metadata)
        +supports_reply_metadata(metadata) bool
        +send_reply(http, credentials, conversation_id, reply) Result~String~
        +register_webhook(http, bot, url, secret) Result
        +verify_bot_token(http, credentials) Result~BotIdentity~
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
        -Verification Token check
        -Optional Encrypt Key signature + AES-256-CBC decrypt
        -url_verification handled after bot lookup
        -App access token caching
    }

    PlatformAdapter <|.. TelegramAdapter
    PlatformAdapter <|.. DiscordAdapter
    PlatformAdapter <|.. LarkFamilyAdapter
    PlatformAdapter <|.. SlackAdapter
    PlatformAdapter <|.. WhatsAppAdapter

    class WhatsAppAdapter {
        +platform_id() "whatsapp"
        -Meta App Secret HMAC-SHA256
        -GET Verify Token handshake
        -Phone Number ID event filter
        -Immediate POST acknowledgment
        -Cloud API text, template, interactive replies
    }

    note for LarkFamilyAdapter "Single implementation,\nregistered twice:\nlark = larksuite.com\nfeishu = feishu.cn"
```

### Platform-Specific Notes

| Platform | Auth Model | Webhook Verification | Challenge | Send Reply API |
|---|---|---|---|---|
| **Telegram** | Bot token (`123:ABC...`) | `X-Telegram-Bot-Api-Secret-Token` header (constant-time) | None | `POST /bot{token}/sendMessage` |
| **Discord** | Bot token + Application ID | Ed25519 signature (`X-Signature-Ed25519` + `X-Signature-Timestamp`) | `PING` -> `PONG` interaction response | `POST /channels/{id}/messages` with `Authorization: Bot {token}` |
| **Lark** | App ID + App Secret for tenant access token; Verification Token for inbound webhook auth; optional Encrypt Key for signed/encrypted delivery | Always verify Verification Token. If Encrypt Key is configured, also require `X-Lark-Signature` with `hex(SHA256(timestamp + nonce + encrypt_key + raw_body))`, then decrypt `{\"encrypt\":\"...\"}` using AES-256-CBC, PKCS7, IV = first 16 bytes, key = `SHA256(encrypt_key)` | `url_verification` -> verify token first, then echo `challenge` | `POST /im/v1/messages` with tenant access token |
| **Feishu** | Same as Lark | Same as Lark | Same as Lark | Same as Lark, different base URL (`open.feishu.cn`) |
| **Slack** | Bot user OAuth token (`xoxb-`) + Signing Secret | HMAC-SHA256 over `v0:timestamp:body`, five-minute replay window | `url_verification` | `POST /api/chat.postMessage` |
| **WhatsApp** | Permanent System User access token + Phone Number ID + Meta App Secret; optional WABA ID | `X-Hub-Signature-256: sha256=<HMAC-SHA256(app_secret, raw_body)>` | GET subscription: constant-time SHA-256 Verify Token check, raw challenge as `text/plain` | `POST /{version}/{phone_number_id}/messages` with Bearer auth |

For the Lark/Feishu platform family, `register_webhook()` remains a no-op. Configure the webhook URL and subscribe to both `im.message.receive_v1` and `card.action.trigger` only in the Lark/Feishu Developer Console. The console inputs map to NyxID fields as follows:

- **App ID** -> `ChannelBot.app_id`
- **App Secret** -> `ChannelBot.app_secret_encrypted`
- **Verification Token** -> `ChannelBot.lark_verification_token_encrypted`
- **Encrypt Key** -> `ChannelBot.lark_encrypt_key_encrypted` (optional)

### Adding New Platforms

Managed registration is optional. Adapters declare `platform_credentials()` (provider key, fields, masking, setup checklist), `managed_onboarding()` (safe bootstrap and accepted completion fields), and implement completion/setup/re-registration hooks. `platform_webhook()` opts into the shared callback; handshake and target extraction remain adapter-owned. The admin inventory comes from `registered_adapters()` in the same registry used by `resolve_adapter`. Generic handlers never branch on an IM name. Registration fields can declare `platform_fallback`; managed verification resolves missing bot secrets from encrypted platform credentials on demand. Existing adapters keep their default hooks.

Implement `PlatformAdapter` in `backend/src/services/channel_adapters/<platform>.rs`, then register its module and add it to `services/channel_adapters/mod.rs::resolve_adapter`, the single runtime registry. The handler module re-exports it for existing callers.

- `registration()` declares field names, human labels, required/secret/patchable/clearable flags, storage columns, and which secrets verify webhooks. It also declares fields that rebuild outbound tokens, automatic versus dashboard webhook setup, the one-time secret label, and setup instructions. Handler and service validation, encrypted persistence, safe configuration responses, and secret decryption follow this descriptor.
- `BotCredentials` carries the outbound token and platform bot identity separately. `registration_token` and `updated_token` own token construction; only the existing Lark/Feishu adapter retains its legacy composite format. Token rotation verifies the replacement before storing it. Label-only updates never decrypt credentials.
- `PlatformVerifySecrets` is a generic map of zeroizing secret values, with redacted Debug output. Its default builder decrypts only fields marked as webhook secrets; an adapter may override `build_verify_secrets`.
- `webhook_policy` selects inline processing, immediate acknowledgment with background processing, or the existing challenge-only response. Lark/Feishu challenges remain inside authenticated `prepare_webhook`. `subscription_handshake` defaults to denial and handles GET protocols when supported.
- `prepare_webhook` verifies and preprocesses raw payloads before normalization. WhatsApp filters app-wide events by the bot's Phone Number ID here. `reply_context` translates stored thread context into outbound metadata; `supports_reply_metadata` admits platform-native content without text.
- `dedup_inbound_by_platform_message_id` defaults to false. WhatsApp opts in to check existing inbound metadata by bot, platform, and platform message ID before routing/dispatch. Keep it off for platforms such as Telegram where edited messages legitimately reuse an ID.
- The public `/api/v1/webhooks/channel/{platform}/{bot_id}` GET/POST handlers use these hooks. No new per-platform handler or generic pipeline branch is needed. Unknown platforms and OpenClaw return 404 on both methods; OpenClaw has its own integration path.
- Mirror the registration fields in `frontend/src/lib/channel-platforms.ts`, extend the frontend platform type/schema and CLI options, and add normalization, verification, lifecycle, and reply tests. Callback payloads stay platform-independent.

### WhatsApp Cloud API Setup

This integration supports **WhatsApp Business Platform through Meta's Cloud API**. The consumer **WhatsApp Business App has no API** and is not supported by this adapter. Twilio-hosted WhatsApp uses a different API and is out of scope.

1. In Meta Business Settings, create a System User, assign the WhatsApp Business Account, and generate a permanent access token with `whatsapp_business_messaging` and `whatsapp_business_management` permissions. Obtain the **Phone Number ID** from WhatsApp > API Setup and the **Meta App Secret** from the app's Basic settings. The Phone Number ID is neither the display phone number nor the Meta App ID.
2. Register the bot:

   ```bash
   nyxid channel-bot register --platform whatsapp --label "WhatsApp Support" \
     --token-env WHATSAPP_ACCESS_TOKEN --phone-number-id 123456789 \
     --app-secret-env META_APP_SECRET --waba-id 987654321
   ```

   `--waba-id` is optional. API fields are `bot_token`, `phone_number_id`, `app_secret`, and optional `waba_id`. The Phone Number ID is stored in `platform_bot_id`, the encrypted access token in `bot_token_encrypted`, the encrypted App Secret in `app_secret_encrypted`, and WABA ID in the optional generalized `app_id` column. No new model fields or collections are needed. API responses expose the identifier as `waba_id`, never as an App ID.
3. Copy the returned **Callback URL** and **Verify Token** into Meta App Dashboard > WhatsApp > Configuration, then verify and save. The CLI prints both, and the web creation dialog shows them before navigation. The generated `webhook_secret` is shown once; only its SHA-256 hash is stored. Detail/list responses never return it. Keep it in a password manager; if lost, re-register the bot. The GET handshake returns the unquoted challenge with 200, or 403 for missing/invalid parameters or token.
4. Subscribe to the **messages** webhook field. Separately subscribe the app to the WABA with `POST /{version}/{WABA_ID}/subscribed_apps` and Bearer authentication, following Meta's dashboard/subscription instructions. NyxID's `register_webhook` is a no-op; supplying `waba_id` records it for display but does not perform this API call.
5. Add a conversation route to an agent key with a callback URL, then send the business phone a message. `conversation_id` is the sender's WhatsApp ID (`wa_id`/`from`, digits without `+`), and conversation type is always `private`. Use these digits when creating a route. Outbound recipients also accept one leading `+` and spaces/dashes, which the adapter removes before sending. The bot remains `pending_webhook` until a verified POST arrives.

Meta subscriptions are app-wide: every bot URL filters out messages whose `metadata.phone_number_id` differs from its registered identity. A shared app callback does not relay the app's other numbers. Use separate apps or Meta's supported [webhook overrides](https://developers.facebook.com/documentation/business-messaging/whatsapp/webhooks/override) to route each number to its matching NyxID bot URL.

Rotate credentials with `nyxid channel-bot update <BOT_ID> --token-env WHATSAPP_ACCESS_TOKEN --app-secret-env META_APP_SECRET`, or PATCH `bot_token`/`app_secret`. Rotation preserves the Verify Token and phone identity. The Verify Bot action preserves WhatsApp's subscription secret; existing platforms retain their original verification lifecycle and response statuses.

Inbound text, button replies, and interactive button/list selections normalize to text (title preferred, ID fallback). Image and sticker attachments use the `image` category; audio, video, and documents use `audio`, `video`, and `file`. Captions become text. Each attachment carries its media ID as `file_key` and a versioned Graph lookup URL. Fetch that URL with the bot's Bearer token to obtain the short-lived download `url`, then fetch the download URL with the same authentication. NyxID does not expose bot tokens to agents; agents needing media must have a separately authorized credential/proxy connection. Location and contact cards normalize to readable text; full details remain in the individual message's `raw_platform_data`. Orders use `unknown`; reaction, system, unsupported, unknown, and unrecognized types are skipped. Status-only and error-only deliveries are acknowledged without dispatch. Every message in a batch is processed independently.

Replies support plain text, `metadata.template` objects, or `metadata.interactive` objects. Template and interactive content cannot be combined. Metadata cannot override the recipient, `recipient_type`, or `messaging_product`. `context.message_id` carries the original inbound message ID. Text splits into sequential messages of at most 4096 Unicode characters and returns the last `wamid`. A later chunk failure can leave earlier chunks delivered; there is no automatic outbound retry. Editing is unsupported. Outside the 24-hour customer service window, send an approved template; Graph code 131047 identifies that condition. Rate-limit and delivery errors use locally authored messages and numeric codes, never upstream text that could echo credentials.

WhatsApp POST webhooks return 200 immediately and process in the background; verification/parse failures are suppressed while logging diagnostic errors with bot/platform identifiers, never bodies or tokens. This is best-effort delivery: an acknowledged event can be lost on process failure. Meta can retry for seven days. WhatsApp skips a delivery when an inbound `channel_messages` row already exists for the same bot, platform, and platform message ID (`wamid`), using the existing sparse platform-message index. Other platforms retain their existing behavior, including Telegram edits that reuse IDs. This lookup is not an atomic concurrent-delivery claim; simultaneous arrivals can still race. Suppression lasts while metadata is retained and also applies to rows whose callback failed. No new replay collection, index, or age cutoff is introduced.

The adapter pins its Graph API version in the single `GRAPH_API_VERSION` constant. The `whatsapp-business` provider and `api-whatsapp-business` catalog counterpart inject a bearer token at the Graph root for separately authorized direct API calls; callers choose their versioned paths. There is no hosted WhatsApp OpenAPI overlay in the current registry.

### Managed WhatsApp onboarding (Embedded Signup)

NyxID can operate one Meta app as a Tech Provider. Users choose **Connect with Meta** in Channel Bots, enter a label and personal/organization scope, and complete Meta's popup. The popup offers an ordinary Cloud API number or an existing WhatsApp Business App number through coexistence. No user-entered access token, phone ID, or app secret is needed. **Advanced: use your own Meta app** preserves the manual flow above. With no platform credentials configured, only the existing manual form is offered.

#### Administrator setup

1. Open **Admin > Platform Credentials** (`/admin/platform-credentials`). Create or choose a Meta Business app; the existing NyxID `facebook` provider app can be reused by adding the WhatsApp product. Enroll as a Tech Provider.
2. Add WhatsApp and Facebook Login for Business. Create a new Embedded Signup configuration, select the Cloud API product (this selects stable v4), use a business integration system-user token, and allow the NyxID frontend domain and JavaScript SDK login. Enter App ID, App Secret, and the new Embedded Signup Configuration ID in NyxID. An old v2 configuration must be replaced.
3. Complete Business Verification and App Review; request advanced access for `whatsapp_business_management` and `whatsapp_business_messaging`.
4. Copy the platform Callback URL and generated Verify Token from NyxID into Meta's app-level Webhooks settings. Subscribe to `messages`. For coexistence also enable Meta's Business App onboarding flow and subscribe to `smb_app_state_sync`, `smb_message_echoes`, and `history`.
5. Customers supply their own payment method and pay Meta directly for WhatsApp messaging. Sharing a credit line requires Solution Partner status. Meta's current pricing is message-based; consult its pricing page for applicable categories and rates.

The admin API lists adapter-declared providers even before configuration. Plain fields and encrypted secret fields live in `platform_credentials`, one UUID document per unique provider, with `updated_by` and BSON `updated_at`. PATCH/PUT `{ "fields": { "app_secret": "...", "app_id": "..." } }` sets or rotates fields; `null` clears one. DELETE removes the provider. App secrets are never returned. The generated encrypted platform Verify Token is intentionally readable by admins and can be regenerated with `{ "regenerate_verify_token": true }`; update Meta's dashboard after regeneration. Ordinary app-secret rotation preserves the Verify Token. Removing/clearing the app secret disables managed onboarding and stops managed signature verification/replies until restored. There is no decrypted credential cache in `AppState`, so changes apply on the next request across replicas. Audit events contain field names and provider identifiers only.

#### Protocol and recovery

`GET /api/v1/channel-bots/managed-onboarding/whatsapp` returns availability, public app/config IDs, the pinned Graph version, `signup_version`, supported feature types, and adapter-owned `signup_extras` keyed by feature type. Completion is `POST /api/v1/channel-bots/managed-onboarding/whatsapp/complete` with `{code, phone_number_id?, waba_id, business_id?, label, target_org_id?}`. Onboarding, re-registration, and repair use the same human-only router middleware as connect-link completion, rejecting API-key, delegated, service-account, and relay credentials. Completion, re-registration, and repair share five attempts per user per minute using the database limiter. JSON completion returns 201; `Accept: text/event-stream` returns progress (`exchanging`, `subscribing`, `registering`) followed by a result or the normal client-safe `ErrorResponse` (`error`, `message`, `error_code`). The browser uses the runtime API origin and the shared authenticated transport.

The server exchanges the one-use code without `redirect_uri`, debugs the token with the app token, and requires both granular permission target sets to include the WABA. It also verifies phone membership in that WABA and the exact phone identity. A WABA-only finish succeeds only when there is one unambiguous number. IDs are validated; phone-list pagination uses a bounded cursor walk on the pinned Graph host. App-secret proof (HMAC-SHA256 of the bearer token keyed by the app secret) is attached to managed Graph calls, including later verification and replies.

The ordinary registration persistence path performs duplicate checks, token encryption, and webhook-secret generation. The bot is saved as `credential_source: "platform"`, `pending_webhook` before webhook subscription so Meta's immediate verification callback can find it. Legacy rows default to `"user"`. Subscription, per-WABA webhook override, number registration, optional business ID, and coexistence sync outcomes are retained in `managed_setup`. A generated six-digit PIN is envelope-encrypted on the bot and never returned. The detail page's **Re-register number** calls `POST /api/v1/channel-bots/{id}/reregister`, reusing that PIN. A failed register request is accepted as already registered only after a separate Graph identity/status query confirms `CONNECTED`; upstream error prose is never interpreted or exposed.

Subscription/register failures remain visible on the saved bot. **Repair setup** on the detail page, or `nyxid channel-bot repair <id>`, calls owner-authorized `POST /api/v1/channel-bots/{id}/managed-setup/repair`. It repeats subscription, override, and registration/coexistence checks, retries failed sync requests, persists and returns the new setup state. Successful one-shot sync requests are preserved. Repair reuses the encrypted PIN and per-bot Verify Token (`webhook_secret_encrypted`); earlier managed rows with only a verify hash receive one atomically installed encrypted token and matching hash before the callback handshake. BYO rows keep their existing hash-only storage. A lost browser connection can leave a saved bot, so inspect its setup state before retrying signup. No Graph/database distributed transaction or durable setup job is claimed.

Deleting a managed bot best-effort removes the WABA callback override only when no other active managed bot shares that WABA. As specified by Meta, this is a bodyless `POST /{version}/{waba_id}/subscribed_apps`, which restores delivery to the platform callback. The number stays subscribed to the app in Meta. If another managed bot shares the WABA, the override is retained; a soft-deleted managed callback URL still forwards verified deliveries to active siblings. Cleanup outcomes (`removed`, `retained_shared`, `failed`, or `not_applicable`) are recorded in `channel_bot_deleted` audit metadata. Cleanup failure does not prevent local deletion.

The platform callback is `GET/POST /api/v1/webhooks/channel/whatsapp/platform`. GET rejects malformed subscription queries before accessing credentials, then compares the generated platform Verify Token in constant time. POST acknowledges immediately, then verifies the raw `X-Hub-Signature-256` with the platform app secret and resolves each `entry[].changes[].value.metadata.phone_number_id` to an active or pending active bot with `credential_source: "platform"`. Each target uses the existing per-bot verification, phone filtering, dedup, routing, and dispatch path; unknown and BYO-only numbers are dropped. Per-WABA `override_callback_uri` and `verify_token` are attempted after app subscription, but override failure is non-fatal because the platform callback is sufficient. Overrides apply to every phone in a WABA, so a managed per-bot POST URL dispatches sibling numbers too. BYO per-bot routes remain unchanged. Background acknowledgment and concurrent-dedup limitations are the same as the existing WhatsApp receiver.

Coexistence is verified server-side using `is_on_biz_app` and `platform_type = CLOUD_API`. Meta explicitly requires skipping `/register` for these numbers; re-registration checks that state without submitting a PIN. NyxID initiates Meta's one-shot contact/history sync requests immediately after subscription, within the documented 24-hour window, and records requested/failed states. These events do not import historical conversations or app-sent messages into agent conversations. A failed one-shot sync may require repeating Embedded Signup; follow Meta's recovery guidance.

Business integration system-user tokens default to **never expire** for offline server-to-server use, but the Facebook Login for Business configuration can choose an expiry and access can be revoked. NyxID rejects already-expired tokens at onboarding and stores the returned token encrypted; it does not exchange it for a short-lived user token or claim an automatic refresh flow. Reconnect through Embedded Signup after revocation/expiry. Managed token/app-secret rotation fields are hidden and rejected by the ordinary update API.

#### Browser hosting and Meta documentation

The SDK loads from `https://connect.facebook.net/en_US/sdk.js` only while the managed panel is open. Messages are accepted only from the exact origin `https://www.facebook.com`, with validated `WA_EMBEDDED_SIGNUP` finish/cancel/error shapes. SDK failures, cancellation, blocked popups, and timeouts have explicit UI states. The repository's `frontend/nginx.conf.template` does not set a CSP; the backend security-header CSP applies to API responses. Deployments that add a frontend CSP must permit the SDK in `script-src`, Facebook login frames in `frame-src`, and required Facebook/Graph requests in `connect-src`. Keep those allowances scoped to Meta origins and test against the deployed policy; do not weaken the API CSP.

Documentation checked September 8, 2026: [implementation](https://developers.facebook.com/documentation/business-messaging/whatsapp/embedded-signup/implementation), [Tech Provider onboarding](https://developers.facebook.com/documentation/business-messaging/whatsapp/embedded-signup/onboarding-customers-as-a-tech-provider), [coexistence](https://developers.facebook.com/documentation/business-messaging/whatsapp/embedded-signup/onboarding-business-app-users), [webhook overrides](https://developers.facebook.com/documentation/business-messaging/whatsapp/webhooks/override), [number registration](https://developers.facebook.com/documentation/business-messaging/whatsapp/reference/whatsapp-business-phone-number/register-api), [business token lifetime](https://developers.facebook.com/documentation/facebook-login/facebook-login-for-business), [secure requests](https://developers.facebook.com/docs/graph-api/guides/secure-requests/), and [debug token](https://developers.facebook.com/docs/graph-api/reference/debug_token/).

NyxID uses stable **Embedded Signup v4**, as recommended by [Meta's versions page](https://developers.facebook.com/documentation/business-messaging/whatsapp/embedded-signup/versions) and [v4 implementation](https://developers.facebook.com/documentation/business-messaging/whatsapp/embedded-signup/version-4). Meta's implementation page explicitly supports Graph `v25.0`, the existing `GRAPH_API_VERSION`. Stable v4 is selected by the product-enabled Facebook Login configuration, not an `extras.version` override. The default launch sends `extras: {}`; coexistence adds only `featureType: "whatsapp_business_app_onboarding"`, which Meta's Business App customization instructions retain for v4. No legacy `sessionInfoVersion` is sent. The adapter descriptor supplies the signup version and launch extras; the browser has no hardcoded version contract. Events support Cloud API, WABA-only, and Business App finishes (including a missing phone ID), optional v4 asset arrays, and cancel/error payloads. Multiple selected WABAs and unsupported migration/grant-only flows produce a local selection error. Validate the product-enabled configuration and both flows on a real Meta tenant before production.

Meta documents granular targets as strings/numeric IDs and permits absent targets for all-assets grants; NyxID intentionally requires explicit WABA targets in both scopes. No stable numeric "already registered" error was documented, hence the independent connected-state check.

Meta references checked for this implementation: [webhook overview](https://developers.facebook.com/documentation/business-messaging/whatsapp/webhooks/overview/), [endpoint verification](https://developers.facebook.com/documentation/business-messaging/whatsapp/webhooks/create-webhook-endpoint), [phone numbers](https://developers.facebook.com/documentation/business-messaging/whatsapp/business-phone-numbers/phone-numbers), [text messages](https://developers.facebook.com/documentation/business-messaging/whatsapp/messages/text-messages), [media](https://developers.facebook.com/documentation/business-messaging/whatsapp/business-phone-numbers/media), and [error codes](https://developers.facebook.com/documentation/business-messaging/whatsapp/support/error-codes).

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
  "correlation_id": "77a1b4f1-f7a4-421e-9f89-7d0b1de9e0c6",
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
  "reply_token": "eyJhbGciOiJSUzI1NiIsImtpZCI6...",
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
| `correlation_id` | UUID | No | Per-delivery correlation ID. This equals the callback JWT `jti`; retries mint a new value. |
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
| `content.attachments` | array | No | Non-text attachments: `{ content_type, url, platform_message_id, file_key, image_key, filename, mime_type, size_bytes }`. Omitted when empty. `platform_message_id`, `file_key`, and `image_key` are provider-scoped opaque handles, present only when the platform exposes them. For Lark/Feishu, `url` is the authenticated message-resource endpoint; NyxID forwards the reference and does not download or store the attachment body. |
| `reply_to_message_id` | UUID | Yes | NyxID `message_id` of the message being replied to. `null` for standalone messages. |
| `thread_id` | string | Yes | Platform-native thread ID (Discord threads, Lark threads). `null` if not in a thread. |
| `timestamp` | ISO 8601 | No | When the message was sent on the platform (not when NyxID received it). |
| `reply_token` | string | Yes | Per-callback RS256 JWT the agent can send back as `Authorization: Bearer <reply_token>` on `POST /api/v1/channel-relay/reply` instead of its full API key. See [Reply Token](#reply-token). `null` only if token generation failed on NyxID — agents that receive `null` must fall back to API-key auth on the reply call. |
| `raw_platform_data` | object | Yes | The full original webhook payload from the platform (Telegram Update, Discord Interaction, Lark Event, etc.). Use this for platform-specific features like inline keyboards, embeds, interactive cards, or any data not captured by the normalized fields. `null` only if the raw data could not be preserved. |

**Headers:**

| Header | Description |
|---|---|
| `Content-Type` | `application/json` |
| `X-NyxID-Callback-Token` | RS256 JWT proving the callback came from NyxID. Verify with the JWKS at `/.well-known/jwks.json`. |
| `X-NyxID-Signature` | Transitional HMAC-SHA256 of request body, signed with the API key's hash. Dual-emitted during migration and will be removed later. |
| `X-NyxID-Message-Id` | UUID of the `channel_message` record |
| `X-NyxID-Timestamp` | ISO 8601 timestamp (for replay protection) |
| `X-NyxID-Platform` | Platform identifier (`telegram`, `discord`, `lark`, `feishu`) |
| `X-NyxID-User-Token` | Short-lived access token for the bot owner (`JWT_RELAY_ACCESS_TTL_SECS`, default `300`s). The agent uses it as `Authorization: Bearer <token>` to call NyxID's **proxy / LLM gateway / MCP** (and approval *status* polling) on behalf of the user. It is a relay token: it is **rejected** on account, admin, key-management, session, channel-reply, and other non-proxy endpoints; it inherits the originating agent key's service/node allowlist; on MCP it is stateless (no session is minted, so it re-authenticates every call); and it stops working the moment that agent key is revoked/deactivated. To *reply* to a conversation, use the agent API key or the per-callback `reply_token`, not this token. Absent if token generation fails. |

#### Callback Authentication (JWT)

NyxID signs every callback delivery with a dedicated RS256 JWT in `X-NyxID-Callback-Token`. The JWT header carries `kid`; fetch NyxID's public keys from `/.well-known/jwks.json` and select the matching key.

Claims:

| Claim | Value |
|---|---|
| `iss` | NyxID issuer (`JWT_ISSUER`) |
| `aud` | `channel-relay/callback` |
| `exp` | Expiration timestamp |
| `iat` | Issued-at timestamp |
| `jti` | Per-delivery callback token ID |
| `token_type` | `relay_callback` |
| `api_key_id` | Agent API key ID bound to the callback |
| `message_id` | Callback payload `message_id` |
| `platform` | Callback payload `platform` |
| `body_sha256` | Lowercase hex SHA-256 of the exact request body bytes |

Callback tokens have a 5-minute TTL (`JWT_RELAY_CALLBACK_TTL_SECS`, default `300`) and validation allows 60 seconds of clock skew. Compute `body_sha256` over the exact wire bytes received from the HTTP request; reformatting JSON, normalizing field order, changing whitespace, or adding a trailing newline changes the hash and must fail verification.

`payload.correlation_id` always equals the token `jti`. Each retry mints a fresh `jti` and `correlation_id`; idempotency is by `message_id`, not by `jti`.

During the transition, NyxID also dual-emits the legacy `X-NyxID-Signature` HMAC header over the same body bytes. That header remains for compatibility in this release and will be removed later.

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

If the agent needs more time (LLM inference, tool calls, etc.), it returns `202 Accepted` with an empty body, then calls back when ready. The reply endpoint accepts either the agent's API key or the per-callback reply token minted with the inbound payload:

```
POST /api/v1/channel-relay/reply
Authorization: Bearer <nyxid_ag_xxxxx OR reply_token from callback>
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

<a id="reply-token"></a>
#### Reply Token

Each inbound callback carries a short-lived `reply_token` (RS256 JWT) that lets the agent post its async reply without holding the full agent API key. Intended for downstream runtimes (e.g. Aevatar) that would otherwise need to persist agent credentials and take on the associated secret-management burden.

| Property | Value |
|---|---|
| `aud` | `channel-relay/reply` (rejected everywhere else) |
| `token_type` | `relay_reply` |
| TTL | `JWT_RELAY_REPLY_TTL_SECS` (default 1800 = 30 min) |
| Max uses | `1` (duplicate `jti` → `401 "Reply token already used"`) |
| Claim bindings | `api_key_id`, `conversation_id`, `inbound_message_id`, `platform` — all four must match the reply request; body's `message_id` must equal `inbound_message_id` |
| Revocation coupling | At reply time NyxID re-checks that the bound `api_key_id` is still active; a revoked key invalidates all outstanding tokens immediately |
| Replay store | MongoDB `reply_token_uses` collection, TTL-indexed on `exp_at` |
| Clock skew | 60s tolerance on both `iat` and `exp` |

The reply-token path skips the API-key branch's "caller must be the assigned agent for this conversation" check: the token was minted for a specific inbound message, so allowing the original callback recipient to complete its reply even after the conversation is reassigned is intentional. Narrowness is enforced by the four claim bindings above.

A leaked reply token's blast radius is a single reply to a single message for at most 30 minutes. Contrast with a leaked `full_key`, which grants unbounded access to everything the agent key can do until manually revoked.

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
| `PATCH` | `/api/v1/channel-bots/{id}` | Update bot label or platform verification material |
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

### Relay (agent-authenticated)

| Method | Path | Auth | Description |
|---|---|---|---|
| `POST` | `/api/v1/channel-relay/reply` | API key **or** reply token | Agent sends async reply to a message. See [Reply Token](#reply-token). |
| `GET` | `/api/v1/channel-relay/messages/{conversation_id}` | API key | Get conversation message history |
| `GET` | `/api/v1/channel-relay/resolve-sender` | API key | Resolve a platform sender to a NyxID user (query params: `platform`, `platform_id`) |

### Platform Webhooks (unauthenticated, signature-verified)

| Method | Path | Description |
|---|---|---|
| `POST` | `/api/v1/webhooks/channel/telegram/{bot_id}` | Telegram bot webhook |
| `POST` | `/api/v1/webhooks/channel/discord/{bot_id}` | Discord interaction webhook |
| `POST` | `/api/v1/webhooks/channel/lark/{bot_id}` | Lark event webhook |
| `POST` | `/api/v1/webhooks/channel/feishu/{bot_id}` | Feishu event webhook |
| `POST` | `/api/v1/webhooks/channel/slack/{bot_id}` | Slack Events API webhook |
| `GET`, `POST` | `/api/v1/webhooks/channel/whatsapp/{bot_id}` | Meta subscription verification and message webhook |

These share the generic adapter-driven route outside JWT authentication, with the same global rate limiting and body-limit posture as the existing channel webhooks. `delegated_read_denied_path` already denies the entire `webhooks` route class; the public subscription handler never extracts `AuthUser` or relies on delegated authorization.

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
| **Webhook forgery** | Per-platform verification: Telegram secret header, Discord Ed25519, Lark / Feishu Verification Token checks plus optional Encrypt Key signature verification and AES decryption. All comparisons use constant-time equality where applicable. |
| **WhatsApp verification** | POST HMAC verifies the exact raw body with the Meta App Secret before phone-number filtering. GET subscription checks SHA-256 of the one-time Verify Token in constant time. Bodies, secrets, and upstream free-form errors are never logged. |
| **Replay attacks** | Callbacks include `X-NyxID-Timestamp`; agents should reject messages older than 5 minutes. Callback JWTs also expire after 5 minutes with 60s skew tolerance. |
| **Callback authentication** | `X-NyxID-Callback-Token` is an RS256 JWT verifiable through `/.well-known/jwks.json`; its `body_sha256` claim binds the exact request bytes. `X-NyxID-Signature` HMAC is dual-emitted during transition and will be removed later. |
| **Agent impersonation** | Async reply endpoint accepts two auth paths: (a) the agent API key, which must match the conversation's `agent_api_key_id`; or (b) a per-callback reply token bound to a specific `inbound_message_id`, `conversation_id`, `api_key_id`, and `platform`, single-use, 30-min TTL, and revalidated against live `api_key.is_active` on every call. See [Reply Token](#reply-token). |
| **Rate limiting** | Per-bot rate limiting on inbound webhooks. Per-agent rate limiting on callback dispatch (reuses `PerAgentRateLimiter` from agent isolation). |

### Migrating a stuck Lark / Feishu bot

Bots created before issue #455 may have the right App ID / App Secret but still remain in `pending_webhook` because the inbound Verification Token and optional Encrypt Key were never stored separately.

To self-heal an existing bot:

1. Read the bot's Event Subscriptions settings in the Lark / Feishu console.
2. Update the bot with its Verification Token and, if enabled in the console, its Encrypt Key.
3. Wait for the next verified inbound webhook or `url_verification` challenge. NyxID will auto-promote the bot from `pending_webhook` to `active` after successful verification.

You can update the bot either via the API or the CLI:

- `PATCH /api/v1/channel-bots/{id}` with `{ "verification_token": "...", "encrypt_key": "..." }`
- `nyxid channel-bot update <BOT_ID> --verification-token ... [--encrypt-key ...]`

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
| `JWT_RELAY_CALLBACK_TTL_SECS` | `300` | Lifetime for `X-NyxID-Callback-Token` JWTs |
| `CHANNEL_RELAY_CALLBACK_TIMEOUT_SECS` | `30` | HTTP timeout for agent callback requests |
| `CHANNEL_RELAY_MAX_BOTS_PER_USER` | `5` | Maximum bots a user can register |
| `CHANNEL_RELAY_MESSAGE_TTL_DAYS` | `30` | TTL for `channel_messages` auto-cleanup |
| `CHANNEL_POLL_INTERVAL_SECS` | `30` | Generic polling sweep interval; `0` disables. Each adapter's minimum interval, backoff and lease also apply. |

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
