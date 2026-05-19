# Channels: bots, conversation routing, and the HTTP event gateway

## Table of contents

- [Bot-Capable Service Connections](#bot-capable-service-connections)
  - [If Lark/Feishu bot calls fail, recreate the binding](#if-larkfeishu-bot-calls-fail-recreate-the-binding)
  - [Picking the right service for the job](#picking-the-right-service-for-the-job)
- [Channel Bot Relay](#channel-bot-relay)
  - [Register a bot](#register-a-bot)
  - [Manage bots](#manage-bots)
  - [Fix a stuck Lark / Feishu bot](#fix-a-stuck-lark--feishu-bot)
  - [Configure conversation routing](#configure-conversation-routing)
  - [How it works](#how-it-works)
  - [Agent-facing endpoints](#agent-facing-endpoints)
- [HTTP Event Gateway — device/analyzer events](#http-event-gateway--deviceanalyzer-events)
  - [Create a device channel](#create-a-device-channel)
  - [Envelope](#envelope)
  - [Push events](#push-events)
  - [Response codes](#response-codes)

## Bot-Capable Service Connections

NyxID treats messaging platform bots as standard service connections. The credentials live in the same place as any other service (encrypted, scoped, audited) and outbound bot API calls go through the regular `/api/v1/proxy/s/{slug}/{path}` proxy. Inbound webhook handling is the responsibility of the calling agent runtime (Aevatar, custom backend, etc.) -- NyxID does not own chat runtime.

```bash
# Telegram bot (path-injected token)
nyxid service add api-telegram-bot
# CLI prompts for the bot token (from @BotFather)

# Then call any Bot API method directly -- pass only the method name in the
# proxy path. NyxID prepends `bot<token>/` automatically, so the forwarded
# URL becomes https://api.telegram.org/bot<token>/<method>.
nyxid proxy request api-telegram-bot sendMessage \
  -m POST -d '{"chat_id":12345,"text":"hello"}'

nyxid proxy request api-telegram-bot setWebhook \
  -m POST -d '{"url":"https://aevatar-host/api/channels/telegram/callback/abc"}'

nyxid proxy request api-telegram-bot getWebhookInfo -m POST -d '{}'

# IMPORTANT: do NOT include `/bot/` or `/bot{token}/` in the proxy path --
# NyxID adds it for you. `setWebhook` is correct; `bot/setWebhook` would
# forward as `bot<token>/bot/setWebhook` and Telegram returns 404.

# Lark bot (tenant token exchange is fully automatic)
nyxid service add api-lark-bot
# CLI prompts for app_id AND app_secret. NyxID stores both encrypted and
# handles the tenant_access_token exchange transparently on every call.
# After register, the CLI prints a `Configure Permissions:` block with a
# deep link into the Lark developer console's Permissions & Scopes page,
# scopes pre-selected from the catalog's `required_permissions`. The
# same link is surfaced by `nyxid service show api-lark-bot-<id>` and on
# the web UI key detail page. Use it instead of telling the user to
# search for scope keys manually.
# Just hit the Lark API path directly -- no manual token management:
nyxid proxy request api-lark-bot /open-apis/im/v1/chats -m GET

nyxid proxy request api-lark-bot /open-apis/im/v1/messages \
  -m POST \
  -H "Content-Type: application/json; charset=utf-8" \
  -d '{"receive_id":"oc_xxx","msg_type":"text","content":"{\"text\":\"hello\"}"}'

# NyxID caches the tenant_access_token in-process (~2h TTL) and single-
# flights refreshes per app, so concurrent requests never produce
# duplicate exchanges. Your app_secret never leaves NyxID.

# Feishu bot (China region — same flow, same automatic token exchange)
nyxid service add api-feishu-bot

# Discord bot (Bot prefix in Authorization header, persistent token)
nyxid service add api-discord-bot
# CLI prompts for the bot token. Then call:
nyxid proxy request api-discord-bot /channels/{channel_id}/messages \
  -m POST -d '{"content":"hello"}'
# NyxID adds `Authorization: Bot <your_token>` automatically.

# Slack bot (persistent xoxb- token, standard Bearer auth)
nyxid service add api-slack-bot
# CLI prompts for the Bot User OAuth Token (xoxb-...) from your Slack app's
# OAuth & Permissions page. NyxID injects `Authorization: Bearer xoxb-...`
# on every outbound call.
nyxid proxy request api-slack-bot /chat.postMessage \
  -m POST \
  -H "Content-Type: application/json; charset=utf-8" \
  -d '{"channel":"C1234567890","text":"hello"}'

nyxid proxy request api-slack-bot /conversations.list -m GET
```

### If Lark/Feishu bot calls fail, recreate the binding

If `nyxid proxy request api-lark-bot ...` (or `api-feishu-bot`) returns
errors like **"Missing access token for authorization"**, **"token_exchange
auth method requires token_exchange_config"**, or any `99991xxx` Lark
error that shouldn't happen given your setup, your binding is probably
stuck on the **old body-injection shape** from an earlier NyxID version.

**In both the old and new flows, your `app_secret` is stored encrypted
on NyxID and never leaves the server after registration.** The only
thing that changed is how NyxID uses it:

- **Old flow:** NyxID stored only `app_secret`. The *caller* had to
  explicitly hit `/open-apis/auth/v3/tenant_access_token/internal`; the
  proxy merged `app_secret` into that request body server-side, handed
  back a `tenant_access_token`, and the caller was then responsible for
  caching it and attaching `Authorization: Bearer ...` to every
  subsequent Lark call.
- **New flow:** NyxID stores `app_id` **and** `app_secret` together
  (JSON blob, same AES-256 encryption). NyxID calls the exchange
  endpoint itself server-to-server, caches the `tenant_access_token`
  in-process with single-flight dedup, and injects the Bearer header on
  every outbound Lark request. Callers just hit the real API path.

Older bindings only contain `app_secret` without `app_id`, so the new
transparent-exchange path can't use them. Fix by deleting the binding
and re-adding -- this prompts for both fields and stores them in the
new shape:

```bash
# List your bindings and find the stale one (grab its id)
nyxid service list --output json | jq '.keys[] | select(.slug == "api-lark-bot") | {id, label}'

# Delete it (replace <id> with the id from the previous command; --yes
# skips the confirmation prompt so this works in agent contexts)
nyxid service delete <id> --yes

# Re-add -- the new prompt asks for BOTH app_id and app_secret
nyxid service add api-lark-bot

# Verify the new binding works (should return chats, not a missing-token error)
nyxid proxy request api-lark-bot /open-apis/im/v1/chats -m GET
```

You're just re-registering the same secret you already gave NyxID the
first time -- it travels once from your terminal to NyxID over HTTPS,
gets re-encrypted at rest, and then stays there. The same recreation
steps apply to `api-feishu-bot`.

### Picking the right service for the job

| Slug | Purpose |
|---|---|
| `api-lark` | Lark API as a logged-in user (OAuth) |
| `api-lark-bot` | Lark API as a bot (automatic tenant token exchange) |
| `api-feishu` | Feishu API as a logged-in user (OAuth) |
| `api-feishu-bot` | Feishu API as a bot (automatic tenant token exchange) |
| `api-telegram-bot` | Telegram Bot API |
| `api-discord` | Discord API as a logged-in user (OAuth) |
| `api-discord-bot` | Discord API as a bot (persistent bot token) |
| `api-slack` | Slack Web API as a logged-in user (OAuth) |
| `api-slack-bot` | Slack Web API as a bot (persistent `xoxb-` token) |

## Channel Bot Relay

NyxID can bridge messaging platforms (Telegram, Discord, Lark, Feishu, Slack) to AI agent callback URLs. Users register their own bots, configure conversation-to-agent routing, and NyxID handles webhook reception, message normalization, and delivery to the agent.

NyxID is a **pure passthrough gateway** (ADR-013): it never stores message bodies or attachments. Only routing metadata lives in NyxID; the full conversation history belongs to the downstream agent.

### Register a bot

```bash
# Telegram
nyxid channel-bot register --platform telegram --label "My Support Bot" --token-env TELEGRAM_BOT_TOKEN

# Discord (requires public key for signature verification)
nyxid channel-bot register --platform discord --label "My Discord Bot" --token-env DISCORD_BOT_TOKEN --public-key "ed25519_public_key_hex"

# Lark (requires app credentials + verification token; optional encrypt key)
nyxid channel-bot register --platform lark --label "My Lark Bot" --token-env LARK_BOT_TOKEN --app-id "cli_xxx" --app-secret-env LARK_APP_SECRET --verification-token "vtoken_xxx" --encrypt-key "encrypt_key_xxx"

# Feishu (same flags as Lark)
nyxid channel-bot register --platform feishu --label "My Feishu Bot" --token-env FEISHU_BOT_TOKEN --app-id "cli_xxx" --app-secret-env FEISHU_APP_SECRET --verification-token "vtoken_xxx" --encrypt-key "encrypt_key_xxx"

# Slack (pass the xoxb- bot user token and the app's signing secret)
nyxid channel-bot register --platform slack --label "My Slack Bot" --token-env SLACK_BOT_TOKEN --app-secret-env SLACK_SIGNING_SECRET
```

For Telegram, NyxID auto-registers the webhook. For Discord/Lark/Feishu/Slack, configure the webhook URL in the platform's developer console: `https://<your-nyxid>/api/v1/webhooks/channel/<platform>/<bot-id>`. Telegram/Discord/Slack bots auto-activate on first successful webhook delivery. Lark/Feishu bots promote from `pending_webhook` to `active` only after inbound webhook verification passes, which requires the bot's Verification Token to be set correctly. Encrypt Key is optional, but if it is enabled in the Lark/Feishu console it must also be set on the bot. The CLI falls back to `NYXID_LARK_VERIFICATION_TOKEN` and `NYXID_LARK_ENCRYPT_KEY` when `--verification-token` or `--encrypt-key` are omitted. For Slack, paste the URL into the app's **Event Subscriptions** page — Slack's `url_verification` handshake is answered automatically.

**Lark/Feishu permission setup link (NyxID#167).** For Lark/Feishu bots, every response that includes the bot's `app_id` also carries a `permission_setup_url` and `permission_setup_scopes` field. The URL deep-links into the developer console's Permissions & Scopes page with the scopes NyxID's adapter needs (`im:message`, `im:message:send_as_bot`) already pre-checked, ready for "Bulk Enable". The CLI prints it as a `Configure Permissions:` block after `nyxid channel-bot register`, `nyxid channel-bot show`, and `nyxid channel-bot update` (table mode); the web UI renders it as a "Configure Permissions" section on the bot detail page. When helping a user set up a Lark/Feishu bot, point them at this link instead of asking them to manually search for scope keys in the developer console.

### Manage bots

```bash
nyxid channel-bot list                          # list registered bots
nyxid channel-bot show <ID>                     # bot details + conversation count
nyxid channel-bot update <ID> --label "New Label" --verification-token "vtoken_xxx" --encrypt-key "encrypt_key_xxx" --app-id "cli_xxx" --app-secret "secret_xxx"
nyxid channel-bot verify <ID>                   # re-verify token and webhook
nyxid channel-bot delete <ID> --yes             # deregister bot
```

### Fix a stuck Lark / Feishu bot

If an existing Lark / Feishu bot is stuck in `pending_webhook`, the owner should update the bot with the correct Verification Token and, if the Lark / Feishu console has encryption enabled, the matching Encrypt Key:

```bash
nyxid channel-bot update <ID> --verification-token "vtoken_xxx" --encrypt-key "encrypt_key_xxx"
```

The same fix is available in the web UI bot detail page, which uses `PATCH /api/v1/channel-bots/{id}` under the hood. After the next verified inbound webhook is received, NyxID auto-promotes the bot to `active`.

If the bot is also missing required scopes (a common parallel symptom), surface the `permission_setup_url` from `nyxid channel-bot show <ID>` — clicking it opens the developer console with NyxID's required scopes pre-selected so the owner can grant them in one click.

### Configure conversation routing

Each conversation route maps a platform chat to an AI agent (via API key with `callback_url`):

```bash
# Set up an API key with a callback URL first
nyxid api-key create --name "my-agent" --platform claude-code --callback-url "https://my-agent.example.com/webhook"

# Route all messages from a bot to this agent (default/catch-all)
nyxid channel-bot route create --bot <BOT_ID> --agent <API_KEY_ID_OR_NAME>

# Route a specific DM or group chat to a specific agent
nyxid channel-bot route create --bot <BOT_ID> --conversation-id "<chat_id>" --agent <API_KEY_ID_OR_NAME>

# Route a specific group chat with conversation type hint
nyxid channel-bot route create --bot <BOT_ID> --conversation-id "<group_chat_id>" --conversation-type group --agent <API_KEY_ID_OR_NAME>

# Per-user routing in a group (different agents for different users)
nyxid channel-bot route create --bot <BOT_ID> --conversation-id "<group_chat_id>" --sender-id "<user_id>" --agent <AGENT_A>
nyxid channel-bot route create --bot <BOT_ID> --conversation-id "<group_chat_id>" --sender-id "<user_id_2>" --agent <AGENT_B>

# List and manage routes
nyxid channel-bot route list --bot-id <BOT_ID>
nyxid channel-bot route update <ROUTE_ID> --agent <NEW_KEY>
nyxid channel-bot route delete <ROUTE_ID> --yes
```

Routing priority: sender-specific match > exact conversation match > default catch-all.

For Telegram, `conversation_id` is the `chat.id` (a number like `-1001234567890` for groups). For Discord, it's the `channel_id`. For Slack, it's the channel id (`C...` public channel, `G...` private group / mpim, `D...` direct message). The bot must be added to the group/channel on the platform side.

### How it works

1. User sends message on Telegram/Discord/Lark/Feishu/Slack
2. Platform webhook delivers to NyxID
3. NyxID verifies signature, resolves route, writes a metadata-only record (per ADR-013, no text or attachments are persisted)
4. NyxID POSTs the normalized payload to the agent's `callback_url` signed with a per-delivery RS256 JWT (`X-NyxID-Callback-Token`); a transitional `X-NyxID-Signature` HMAC header is dual-emitted and will be removed once downstreams flip over to JWT verification
5. **Agent must return 202.** Sync replies (200 + body) are no longer supported — any body on a 200 is silently discarded
6. Agent processes asynchronously, then POSTs the reply to `/channel-relay/reply`
7. NyxID delivers the reply to the platform chat

Slack specifics: inbound events land on `/api/v1/webhooks/channel/slack/{bot_id}` and are HMAC-verified against the app's signing secret (`v0:{ts}:{body}` with a 5-minute replay window). NyxID ACKs with HTTP 200 inside Slack's 3-second window and processes in a background task. Outbound replies go through `chat.postMessage`; threaded replies anchor on the thread root via `metadata.thread_ts`. Rate-limit signals (HTTP 429 with `Retry-After`, or `{"ok":false,"error":"ratelimited"}`) surface as a clearly-labeled error so the agent can decide when to retry.

The callback payload includes normalized fields (`content.text`, `sender`, etc.), the full `raw_platform_data` (original Telegram/Discord/Lark/Slack JSON), a per-callback `reply_token` (RS256 JWT) the agent can use to post its async reply without holding the agent API key, and a top-level `correlation_id` that equals the callback JWT's `jti` (one-turn runtime key). The callback is the **only** place the message body exists inside NyxID — it's built in-memory from the live webhook parse and once the callback returns, NyxID retains nothing but metadata.

### Agent-facing endpoints

```bash
# Async reply — this is the only way for an agent to respond.
# Authorization: Bearer <agent API key> OR <reply_token from the callback payload>.
POST /api/v1/channel-relay/reply
{ "message_id": "<inbound-msg-id>", "reply": { "text": "..." } }

# Edit a previously-sent reply (Lark/Feishu only in v1).
# Addresses the upstream platform message returned by a prior /reply call
# (e.g. Lark `om_xxx`). Same dual auth as /reply.
POST /api/v1/channel-relay/reply/update
{ "message_id": "<upstream_platform_message_id>", "reply": { "text": "..." } }

# Message history (metadata only — `text` and `attachments` are NOT returned per ADR-013)
GET /api/v1/channel-relay/messages/<conversation_id>?page=1&per_page=50

# Resolve platform sender to NyxID user
GET /api/v1/channel-relay/resolve-sender?platform=telegram&platform_id=12345
```

#### Editing a sent reply (progressive / streaming renders)

`POST /channel-relay/reply/update` lets an agent PATCH the text of a reply it already sent, which is how you implement progressive / streaming reply rendering on Lark/Feishu without flooding the chat with one message per token chunk.

- **Body:** `{ "message_id": "<upstream_platform_message_id>", "reply": { "text": "...", "metadata": {...} } }`. `message_id` is the platform message id (e.g. Lark `om_xxx`) returned by the prior `/reply` call — **not** the inbound message id.
- **Auth:** Same as `/reply`: agent API key OR the original per-callback reply token. The reply token is reusable for edits — see the reply-token section below for the JTI semantics.
- **Platform support in v1:**
  - Lark / Feishu: text edits via `PUT /im/v1/messages/{id}`, card edits via `PATCH /im/v1/messages/{id}` (pass the new card in `reply.metadata.card`).
  - Telegram / Discord / Slack / OpenClaw: `501` with `code="edit_unsupported"`. Degrade to a final `/reply` at turn end.
  - Device channels: `400 device_channel_reply_not_allowed` (device conversations have no reply surface).
- **Throttling is the caller's job.** NyxID only protects against abuse — per-upstream-message rate limit (default `10/s` burst `20`, configurable via `CHANNEL_RELAY_EDIT_RATE_LIMIT_PER_SECOND` / `..._BURST`). `429 rate_limited` on exceed.
- **Error classification:** Lark frequency-limit errors surface as `429`; "message not editable / wrong state" errors as `409`; malformed content as `400`. Anything else falls through to `502`.

> **ADR-013 note:** `GET /channel-relay/messages/...` returns only routing metadata (direction, platform, sender ids, delivery status, timestamps). Agents that need conversation bodies must retain their own history.

#### Callback token (inbound request authentication)

Every callback delivery carries an RS256 JWT in `X-NyxID-Callback-Token` that downstreams verify via the public JWKS at `/.well-known/jwks.json` (no shared secret needed — the JWT header's `kid` selects the active key). This is the preferred production auth path and replaces the legacy HMAC signature.

- **Shape:** RS256 JWT. `aud = "channel-relay/callback"`. `token_type = "relay_callback"`.
- **Claim bindings:** `api_key_id`, `message_id`, `platform`, `jti`, and `body_sha256` (lowercase hex SHA-256 of the exact wire body bytes). Plus standard `iss`, `iat`, `exp`. `payload.correlation_id == jti`.
- **Body hash is byte-exact.** Reformatting JSON, reordering fields, adding whitespace, or a trailing newline changes the hash and MUST fail verification. Compute SHA-256 over the raw request body you received — never over a re-serialized value.
- **TTL:** `JWT_RELAY_CALLBACK_TTL_SECS` (default `300` = 5 min). 60s clock-skew tolerance on both `iat` and `exp`.
- **Retries mint fresh tokens.** NyxID does not persist `jti` — idempotency is by `message_id`, not by `jti`. Each retry attempt carries a new `jti`, `correlation_id`, and `exp`.
- **Transitional HMAC:** `X-NyxID-Signature` (HMAC-SHA256 of the same body using the API key's hash) is dual-emitted during migration. Downstreams should switch to JWT verification; HMAC will be removed in a future release.

#### Reply token (dual-auth on `/channel-relay/reply` and `/channel-relay/reply/update`)

The callback payload includes a short-lived `reply_token` (RS256 JWT) the agent can present as `Authorization: Bearer <reply_token>` instead of the agent API key. Intended for runtimes that don't want to persist agent credentials (e.g. Aevatar).

- **Shape:** RS256 JWT. `aud = "channel-relay/reply"` (rejected everywhere else). `token_type = "relay_reply"`.
- **Claim bindings:** `api_key_id`, `conversation_id`, `inbound_message_id`, `platform` — all four must match the reply request. For `/reply`, the body's `message_id` must equal `inbound_message_id`. For `/reply/update`, NyxID looks up the outbound row by the body's `message_id` (platform id) and verifies its stored `reply_to_message_id` equals the token's `inbound_message_id`.
- **TTL:** `JWT_RELAY_REPLY_TTL_SECS` (default `1800` = 30 min). 60s clock-skew tolerance on both `iat` and `exp`.
- **JTI semantics:** `jti` is consumed on the first successful `/reply`. Reuse on `/reply` returns `401 "Reply token already used"`. `/reply/update` uses the same token without consuming a new JTI — it requires the JTI to already exist in `reply_token_uses` (i.e. proof the token was used to send), so bare-minted tokens cannot edit-flood. The same token can therefore drive one send + many edits within the TTL.
- **Revocation coupling:** On every call NyxID re-checks that the bound `api_key_id` (and the channel bot) is still active — revoking the key invalidates all outstanding tokens immediately.
- **Null tokens:** If NyxID failed to mint a token, `reply_token` is `null` in the callback; fall back to the agent API key on the reply call.

Agents that already hold the API key can ignore `reply_token` entirely and keep using `Authorization: Bearer nyxid_ag_...`.

## HTTP Event Gateway — device/analyzer events

NyxID accepts push-mode events from external devices and analyzers on the same channel relay infrastructure. The envelope is converted to a `CallbackPayload` with `platform = "device"` and forwarded through the agent's `callback_url` just like a chat message.

Device channels are **first-class** and **not backed by a bot** — no Telegram/Discord/Lark/Feishu registration is required. A device channel is a `ChannelConversation` row with `platform = "device"` and `channel_bot_id = null`.

**Endpoint:** `POST /api/v1/channel-events/{conversation_id}`
**Auth:** Bearer API key (`nyxid_ag_...`) bound to the target conversation
**Storage:** Metadata only. Event payloads are never persisted (ADR-013).
**Retry:** None. NyxID is a pure passthrough — the client decides what to do on failure.
**Rate limit:** 100 events/second per conversation (default, configurable via `CHANNEL_EVENT_RATE_LIMIT_PER_SECOND`).
**Idempotency:** Best-effort — same `event_id` within 5 minutes is deduplicated.
**One-way:** Device conversations do not support agent replies. `POST /channel-relay/reply` returns `400 device_channel_reply_not_allowed` against a device channel.

### Create a device channel

Before pushing events, create a device channel (once) and bind it to an agent API key with a `callback_url`:

```bash
# Create the agent key first if you don't have one
nyxid api-key create --name "household-agent" --platform custom \
  --callback-url "https://my-agent.example.com/webhook"

# Create the device channel
nyxid channel-event channel create \
  --conversation-id household-camera \
  --agent-key-id <API_KEY_ID> \
  --conversation-type camera     # optional; defaults to "device"

# List device channels
nyxid channel-event channel list

# Delete a device channel (takes the NyxID-assigned _id, not the logical name)
nyxid channel-event channel delete <CONVERSATION_ROW_ID> --yes
```

You can also create the channel through `POST /api/v1/channel-conversations` directly:

```json
{
  "platform": "device",
  "platform_conversation_id": "household-camera",
  "agent_api_key_id": "<api-key-uuid>"
}
```

Validation rules for device channels:

- `channel_bot_id` MUST be omitted or null.
- `platform_conversation_id` is **required** and must be non-empty and not `"*"` (no catch-all routes).
- `platform_sender_id` and `default_agent: true` are rejected — devices have no group/sender or fan-out concept.
- Uniqueness is per `(user_id, platform_conversation_id)` — each owner gets one active device channel per logical name.

### Envelope

```json
{
  "event_id": "550e8400-e29b-41d4-a716-446655440000",
  "source": "camera-analyzer",
  "type": "person_detected",
  "timestamp": "2026-04-08T12:00:00Z",
  "payload": { "room": "living_room", "confidence": 0.95 },
  "metadata": { "analyzer_version": "1.0" }
}
```

### Push events

The `conversation_id` in the path is the NyxID-assigned `_id` returned by `channel create` (not the logical `platform_conversation_id`).

```bash
# Push from the CLI
nyxid channel-event push \
  --conversation-id <CONVERSATION_ROW_ID> \
  --source camera-analyzer \
  --type person_detected \
  --payload-json '{"room":"living_room","confidence":0.95}'
```

```bash
# Push via curl
curl -X POST https://<your-nyxid>/api/v1/channel-events/<CONVERSATION_ROW_ID> \
  -H "Authorization: Bearer nyxid_ag_..." \
  -H "Content-Type: application/json" \
  -d '{
    "event_id": "550e8400-e29b-41d4-a716-446655440000",
    "source": "camera-analyzer",
    "type": "person_detected",
    "timestamp": "2026-04-08T12:00:00Z",
    "payload": {"room": "living_room", "confidence": 0.95}
  }'
```

### Response codes

| Status | Meaning |
|---|---|
| 200 | Accepted (delivered) or deduplicated |
| 400 | Invalid envelope shape |
| 401 | Missing/invalid bearer, **or** conversation not found, **or** API key is not bound to the conversation (collapsed into one opaque error to prevent existence-probing) |
| 429 | Per-channel rate limit exceeded |
| 502 | Downstream agent unreachable or returned non-2xx |
