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

## View bots across organizations

The **Scope** picker defaults to **View all**, which combines your personal bots
and bots owned by organizations you administer. A divider separates **View all**
from **User** (your personal bots) and the individual organizations. Each bot shows its owner in
the combined view. Deleted bots are excluded.

Choose **User** or an organization to narrow the list. **Add Bot** defaults
to the selected organization, or to Personal when viewing all bots; you can choose
the owner in the creation dialog. Device channels have their own scope picker.

API clients can request the same combined list using an account access token:

```bash
curl "$NYXID_BASE_URL/api/v1/channel-bots?scope=all" \
  -H "Authorization: Bearer $NYXID_ACCESS_TOKEN"
```

The response contains `bots` and `total`; each bot's `user_id` identifies its
owner. The three selectors are:

| Selector | Bots returned |
|---|---|
| `?scope=all` | Your personal bots and bots from organizations you administer |
| `?scope=user` | Your personal bots only |
| `?org_id=<id>` | Bots from the selected organization you administer |

`scope=user` uses your authenticated identity, so you do not need to supply a
user ID. Omitting both parameters also returns personal bots for compatibility.
Combining either `scope` value with `org_id` returns HTTP 400.

To find an organization's ID, list your memberships:

```bash
curl "$NYXID_BASE_URL/api/v1/orgs" \
  -H "Authorization: Bearer $NYXID_ACCESS_TOKEN"
```

The `orgs` array includes `id`, `display_name`, `slug`, and `your_role` for each
organization. Choose the relevant entry where `your_role` is `admin`, then pass
its `id` as `org_id`. Organization discovery also accepts general Agent Keys;
channel-bot management retains its existing account authentication requirements.

## Register a Telegram bot

1. Create a bot with `@BotFather` on Telegram. Copy the bot token (`123456:ABCdef...`).
2. In NyxID, go to **Channel Bots → Register bot**.
3. Select **Platform: Telegram bot token**.
4. Enter a label (e.g. `support-bot`).
5. Paste the bot token.
6. Click **Register**.

NyxID calls `/getMe` to verify the token, registers the webhook automatically, and sets the bot status to **active**.

:::tip
If the bot status stays at `pending_webhook`, check that the bot token is correct and that your NyxID instance is reachable from the public internet (or that Telegram can reach it). Self-hosted instances behind NAT need a public URL.
:::

## Use the Telegram manager as a public bot

An existing manager configured in **Admin → Platform Credentials → Telegram — bot creation** can use the same **Telegram bot token** registration flow. NyxID keeps the manager webhook and shares its live token. Assign a dedicated Agent Key as the **default agent** to answer public messages, or an exact conversation route for selected chats.

Manager-channel callbacks receive a message-bound reply token, but no owner access token. Telegram senders are not signed in to the channel owner's NyxID account. Scope credentials held directly by the agent separately. A plain `/start` offers chat and bot creation; setup links, `/recover`, and callback buttons stay in the manager workflow.

**Verify Bot** checks the live manager webhook without replacing it. If configuration needs repair, save the manager in Platform Credentials and verify again. Token rotation there updates replies automatically. Deleting the channel disables its routes and preserves bot creation. The current limit is 32 concurrent background deliveries per backend process; excess messages are dropped, and restarts or failures may lose messages. Delivery can be duplicated or finish out of order.

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

NyxID exchanges your App ID and App Secret for a tenant access token automatically. There is no separate Bot token to enter.

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

## Transfer bot ownership

The bot owner, an organization admin, or a NyxID platform admin can transfer a
supported bot from its detail page. In the
**Ownership transfer** card, click **Transfer ownership**, choose **Person** or
**Organization**, and search for the destination in the owner dropdown. Click
**Review transfer** to check the current owner, destination, effects, and any
dependency blockers, then **Confirm transfer**.

The bot keeps its credentials and webhook configuration. Existing routes are
permanently retired, and conversation history stays with the previous owner.
The destination must create new routes using its own Agent Keys. After the
transfer, the console returns to Channel Bots. Organization admins need
unrestricted management access. No mobile approval is required. An owner’s
Agent Key with `write` or `admin` scope and unrestricted service management can
also use the transfer API; permission to send bot messages alone is insufficient.

X bots created through managed OAuth onboarding can move with their dedicated
OAuth connection. The connection must be active and used only by that bot.
Shared connections and authorization or refresh work in progress appear as
specific blockers during review. Managed Telegram and Aurinko email handovers
are not supported yet.

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

## Agent-initiated messages

A human owner can enable **Allow unprompted messages** on a specific conversation's message page, then use **Send test message**. This permission defaults off and lets the assigned agent send alerts or scheduled updates without an incoming message. Catch-all routes, device channels, and unsupported adapters cannot initiate.

Agents discover eligible routes through `GET /api/v1/channel-relay/conversations` and send with `POST /api/v1/channel-relay/send`:

```json
{ "conversation_id": "<route-uuid>", "message": { "text": "Your job finished" }, "idempotency_key": "job-123" }
```

Use the assigned API key; reply tokens are not accepted. Reusing a key with the same payload returns the original receipt, while changed payloads or in-flight sends return 409. Claims expire after 24 hours. A receipt means the platform accepted the message, not that someone read it. NyxID stores only routing metadata and never queues or automatically retries these messages.

```bash
nyxid channel-bot route update <ROUTE_ID> --allow-agent-initiated true
nyxid channel-bot send --conversation <ROUTE_ID> --text 'Test notification'
nyxid channel-bot route update <ROUTE_ID> --allow-agent-initiated false
```
