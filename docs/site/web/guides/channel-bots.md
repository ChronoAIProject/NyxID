---
title: Connect a channel bot
description: Register a Telegram, Discord, or Lark/Feishu bot in NyxID so inbound messages are routed to your AI agent's callback URL.
---

NyxID Channel Bot Relay turns NyxID into a multi-platform messaging gateway. You register your own bot, NyxID receives messages via platform webhooks, normalizes them into a common format, and routes each message to the AI agent you designate for that conversation. The agent's reply is sent back to the platform on its behalf.

This page covers the web console steps. For the full design and callback contract, see the Channel Bot Relay architecture doc.

## How it works

1. You register a bot in the NyxID console (bot token, credentials, platform-specific fields).
2. NyxID automatically registers a webhook with the platform (Telegram, Discord) or you register it manually (Lark / Feishu).
3. You create a **Conversation route** that maps a specific chat conversation to an Agent Key with a callback URL.
4. When a message arrives, NyxID verifies the webhook signature, normalizes the message, and `POST`s a structured payload to the agent's callback URL.
5. The agent posts its reply via `POST /api/v1/channel-relay/reply` — agents must return 202 and reply asynchronously; synchronous replies are not supported.

## Prerequisites

Before registering a bot, you need an Agent Key with a `callback_url` configured. Create one from **AI Services → Agent Keys → Create API Key** and set the **Callback URL** field to the HTTPS endpoint where your agent receives messages.

## Register a Telegram bot

1. Create a bot with `@BotFather` on Telegram. Copy the bot token (`123456:ABCdef...`).
2. In NyxID, go to **Channel Bots → Register bot**.
3. Select **Platform: Telegram**.
4. Enter a label (e.g. `support-bot`).
5. Paste the bot token.
6. Click **Register**.

NyxID calls `/getMe` to verify the token, registers the webhook automatically, and sets the bot status to **active**.

:::tip
If the bot status stays at `pending_webhook`, check that the bot token is correct and that your NyxID instance is reachable from the public internet (or that Telegram can reach it). Self-hosted instances behind NAT need a public URL.
:::

## Register a Discord bot

1. Create an application and bot in the [Discord Developer Portal](https://discord.com/developers/applications). Copy the **Bot Token** and **Application ID** (= **Public Key** is the Ed25519 public key, separate from the token).
2. In NyxID, go to **Channel Bots → Register bot**.
3. Select **Platform: Discord**.
4. Enter the label, bot token, and application public key.
5. Click **Register**.

After registration, set your Discord application's **Interactions Endpoint URL** to:

```
https://nyx.chrono-ai.fun/api/v1/webhooks/channel/discord/<BOT_ID>
```

Discord sends a `PING` challenge on first setup. NyxID handles it automatically (responds with `PONG`).

## Register a Lark or Feishu bot

Lark and Feishu use the same adapter (different base URLs). Webhook registration is manual — you configure the URL in the developer console, not through NyxID.

1. In the [Lark Developer Console](https://open.larksuite.com/app) (or Feishu equivalent), create an app and enable bot capabilities.
2. Note the **App ID**, **App Secret**, **Verification Token**, and optionally the **Encrypt Key**.
3. In NyxID, go to **Channel Bots → Register bot**, select **Platform: Lark** (or **Feishu**).
4. Enter:

| Field | Source |
|---|---|
| **Label** | Your name for this bot |
| **App ID** | Lark Developer Console → App ID |
| **App Secret** | Lark Developer Console → App Secret |
| **Verification Token** | Event Subscriptions → Verification Token |
| **Encrypt Key** | Event Subscriptions → Encrypt Key (optional) |

5. Click **Register**. NyxID stores the credentials; the bot status starts as `pending_webhook`.

6. In the Lark Developer Console, under **Event Subscriptions**, set the webhook URL to:

```
https://nyx.chrono-ai.fun/api/v1/webhooks/channel/lark/<BOT_ID>
```

   Subscribe to at least `im.message.receive_v1`. The bot promotes to **active** automatically after the first verified inbound event.

:::note
If an existing Lark bot is stuck in `pending_webhook`, check that the Verification Token in NyxID matches the one in the Lark console. Patch the bot via the console or CLI and wait for the next inbound to promote it.
:::

## Field reference: Lark / Feishu

| Developer console field | NyxID field | Purpose |
|---|---|---|
| App ID | `app_id` | Authenticate outbound calls to Lark APIs (send replies, fetch tenant access tokens) |
| App Secret | `app_secret_encrypted` | Same — used with App ID for access token requests |
| Verification Token | `lark_verification_token_encrypted` | Verify inbound webhook authenticity |
| Encrypt Key | `lark_encrypt_key_encrypted` (optional) | Decrypt AES-256-CBC-encrypted event payloads; also used for `X-Lark-Signature` verification |

Do not swap these fields. App ID + App Secret are for outbound; Verification Token is for inbound. They serve different purposes and will cause silent failures if mixed up.

## Create a conversation route

After your bot is active, map a conversation to an agent:

1. Go to **Channel Bots**, open the bot, click **Conversations → Add route**.
2. Choose the route type:
   - **Default agent** — all unmatched conversations on this bot go to this agent
   - **Specific conversation** — enter the platform's native conversation ID (Telegram `chat_id`, Discord `channel_id`, Lark `chat_id`)
   - **Sender in group** — messages from a specific sender ID within a group
3. Select the **Agent Key** that handles this route. The key's `callback_url` receives the messages.
4. Click **Save**.

## Async reply

Agents must return HTTP 202 to the callback immediately and post their reply separately:

```bash
POST https://nyx.chrono-ai.fun/api/v1/channel-relay/reply
Authorization: Bearer nyxid_ag_YOUR_KEY
Content-Type: application/json

{
  "message_id": "<message_id from callback payload>",
  "reply": {
    "text": "Here is your answer..."
  }
}
```

The `reply_token` field in the callback payload can be used as an alternative to the agent API key for the reply call — it is a short-lived JWT bound to that specific message.

## View conversation history

From the bot's detail page, open a conversation route to see recent messages and delivery status. Message records are retained for 30 days.

## Update bot credentials

Bot credentials can be updated without re-registering. From the bot's detail page, click **Edit** and update any field. For Lark bots that need a Verification Token or Encrypt Key added after the fact:

```bash
# CLI
nyxid channel-bot update <BOT_ID> --verification-token vtoken_xxx --encrypt-key key_xxx
```

## Delete a bot

From the bot's detail page, click **Delete**. NyxID deregisters the webhook on the platform side before deleting the record. All associated conversation routes are deleted.

:::warning
If the org owns the bot, org deletion is blocked until the bot is deleted first.
:::
