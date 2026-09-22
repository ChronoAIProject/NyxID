---
title: Connect a channel bot
description: Connect Telegram, Discord, Lark, Feishu, Slack, WhatsApp, X, or Aurinko Email through a dedicated onboarding page and route messages to your AI agent.
---

NyxID Channel Bot Relay turns NyxID into a multi-platform messaging gateway. You register your own bot, NyxID receives messages via platform webhooks, normalizes them into a common format, and routes each message to the AI agent you designate for that conversation. The agent's reply is sent back to the platform on its behalf.

This page covers the web console steps. For the full design and callback contract, see the Channel Bot Relay architecture doc.

## How it works

1. You open the platform's setup page and connect an account or enter the credentials requested for that platform.
2. NyxID creates the channel bot. Depending on the platform, NyxID configures delivery automatically or you copy its callback URL into the platform's developer console.
3. You create a **Conversation route** that maps a specific chat conversation to an Agent Key with a callback URL.
4. When a message arrives, NyxID verifies the webhook signature, normalizes the message, and `POST`s a structured payload to the agent's callback URL.
5. The agent posts its reply via `POST /api/v1/channel-relay/reply` — agents must return 202 and reply asynchronously; synchronous replies are not supported.

## Prerequisites

You need a NyxID account and the platform credentials or account access described below. To create an organization-owned bot, you must be an administrator of that organization.

Before routing messages, create an Agent Key with a `callback_url` configured. In **AI Services → NyxID API Keys**, create a key and set its **Callback URL** to the HTTPS endpoint where your agent receives messages. You can register the channel bot before assigning its agent.

## Share a channel setup page

Go to **Channel Bots → Setup links** to copy a dedicated onboarding link for any enabled channel. The list and each page use NyxID's live platform catalog, including credential fields, required/optional labels, secret masking, hints, and setup instructions. New platforms using credential forms or an existing managed connection flow appear without adding another page or route. A new kind of provider authorization flow also needs a frontend flow implementation.

Use your NyxID frontend origin with `/channel-bots/connect/{platform}`. For example:

| Channel | Setup path |
|---|---|
| Telegram bot creation | `/channel-bots/connect/telegram-new` |
| Existing Telegram bot token | `/channel-bots/connect/telegram` |
| Discord | `/channel-bots/connect/discord` |
| Lark | `/channel-bots/connect/lark` |
| Feishu | `/channel-bots/connect/feishu` |
| Slack | `/channel-bots/connect/slack` |
| WhatsApp | `/channel-bots/connect/whatsapp` |
| X | `/channel-bots/connect/x` |
| Aurinko Email | `/channel-bots/connect/aurinko` |

Each shared link opens a standalone connection wizard with the NyxID and platform logos, a platform-specific “Create your … channel bot” heading, and one primary connection action. Sign-in is required before the page is displayed. Recipients sign in to their own NyxID account and return to the selected channel. A default bot name is filled in, and personal ownership is automatic when it is the only available scope. The owner picker appears when there is an organization the user can administer. Users can edit the name and any prefilled fields, complete the platform's connection steps, then open the bot to finish setup and assign an agent. Channel setup and bot management remain accessible before the separate AI Services welcome wizard is complete; visiting them does not mark that wizard complete. Unknown or disabled channels cannot be registered.

### Complete a shared setup link

1. Open the channel link. If prompted, sign in to NyxID; the browser returns to the same channel and preserves the supplied form values.
2. Review **Bot name**. If **Create for** appears, choose your personal account or an organization you administer.
3. Complete the selected platform's flow below. Credential forms use **Create channel bot**; managed flows open Telegram, Meta, or the provider's authorization window. Provider consent and any required webhook setup still need to be completed.
4. On the completion screen, save any one-time verification secret and follow the returned setup instructions. Click **Open channel bot** to review its status and callback URL.
5. Complete the platform's webhook setup where required, then assign an agent using a default route or a specific conversation route. Send a test message from the platform and check its delivery in NyxID.

When every required credential is prefilled, step 3 requires just one **Create channel bot** click. A managed flow needs one click to open the provider and then the provider's own confirmation. The page never creates a bot merely because someone visits the link.

The **Setup links** listing's copy action copies only the platform URL, without credentials, organization choices, labels, or Telegram request IDs. Telegram setup preserves its draft and `request_id` in the current page URL for reloads. After creation, the wizard shows a completion screen with an **Open channel bot** action. One-time verification secrets stay on this screen until the user continues to the bot details.

The existing **Add Bot** dialog also has an **Open full setup page** link for the selected platform. The standalone page has a discreet **Channel setup guide** link in the top-right corner. Managed Telegram setup opens Telegram with one **Continue in Telegram** click; bot creation and any provider consent still happen in Telegram. Fully prefilled credential forms need only the **Create channel bot** click.

### Prefill an onboarding form

Append `label`, `target_org_id`, and any of the selected platform's `registration.fields[].name` values from `GET /api/v1/channel-platforms`. The page discovers these field names from the catalog; onboarding integrations do not need a separate frontend field mapping. For example:

```text
/channel-bots/connect/lark?label=Support&app_id=cli_example&app_secret=example-secret&verification_token=example-token
/channel-bots/connect/discord?label=Community&bot_token=example-token&public_key=example-public-key
```

Use `URLSearchParams` to encode values containing `+`, `&`, `=`, spaces, or other special characters. Numeric identifiers retain their exact digits. The platform comes from the path; query parameters cannot switch it. Only fields declared by that platform populate its credential form, and validation and ownership checks still apply. Prefilled manual credentials open the advanced form when a managed connection flow is also available.

| Channel | Credential query parameters |
|---|---|
| Telegram bot token | `bot_token` |
| Discord | `bot_token`, `public_key` |
| Lark / Feishu | `app_id`, `app_secret`, `verification_token`; optional `encrypt_key` |
| Slack | `bot_token`, `app_secret` (Slack's Signing Secret) |
| WhatsApp manual setup | `bot_token` (access token), `phone_number_id`, `app_secret`; optional `waba_id` |
| Aurinko Email | `bot_token` (mailbox account token), `app_secret` (application signing secret) |
| Telegram creation / X | No manual credential parameters; use the managed flow |

`label` and `target_org_id` apply across channels. `request_id` resumes a Telegram creation draft; it must be a valid UUID and the signed-in user must have access to the draft. Names such as `field1` and `field2` work when a platform declares those exact names; they are not positional aliases for its first and second fields. The live catalog is authoritative if it differs from this reference.

Use an organization UUID for `target_org_id` and share that link only with its administrators. An invalid or unauthorized organization is rejected by the server; NyxID does not silently create the bot in a different account. The recipient can explicitly choose **Personal** or another eligible organization instead. An omitted or blank `label` uses the default bot name.

For example, construct a Slack link without losing special characters:

```ts
const url = new URL("/channel-bots/connect/slack", frontendOrigin);
url.search = new URLSearchParams({
  label: "Support",
  bot_token: slackBotToken,
  app_secret: slackSigningSecret,
}).toString();
```

Prefilling never submits or connects automatically. Users can review and edit every populated field before submitting. Secret parameters are removed from the current URL with replacement navigation after the form consumes them and are kept only in the mounted form; reloading the cleaned URL requires entering those secrets again. Sign-in preserves the original onboarding parameters until the setup page opens.

Credential-bearing links grant their recipient access to those credentials. URL cleanup happens after the page loads; it does not erase copies from messages, server logs, or browser history recorded before cleanup. Use the credential-free **Setup links** URLs for broadly shared onboarding.

## Create a Telegram bot through Telegram

1. An administrator configures the Telegram manager in **Admin → Platform Credentials → Telegram — bot creation**.
2. Open `/channel-bots/connect/telegram-new` and review the bot name and owner.
3. Click **Continue in Telegram**. Complete Telegram's bot creation flow in the opened window or app.
4. Return to NyxID and wait for the connection to complete. **Reopen Telegram** returns you to the same draft if needed; the page URL retains its `request_id` for reloads.
5. Click **Open channel bot** and assign an agent.

If the page asks for manager configuration, an administrator must complete step 1. To connect a bot you already created, use the bot-token flow below.

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
2. Open `/channel-bots/connect/telegram`, or select **Telegram bot token** in **Channel Bots → Add Bot**.
3. Review the bot name (for example, `support-bot`) and paste the bot token.
4. Click **Create channel bot** on the standalone page, then **Open channel bot** and assign an agent.

NyxID calls `/getMe` to verify the token, registers the webhook automatically, and sets the bot status to **active**.

:::tip
If the bot status stays at `pending_webhook`, check that the bot token is correct and that your NyxID instance is reachable from the public internet (or that Telegram can reach it). Self-hosted instances behind NAT need a public URL.
:::

## Use the Telegram manager as a public bot

An existing manager configured in **Admin → Platform Credentials → Telegram — bot creation** can use the same **Telegram bot token** registration flow. NyxID keeps the manager webhook and shares its live token. Assign a dedicated Agent Key as the **default agent** to answer public messages, or an exact conversation route for selected chats.

Manager-channel callbacks receive a message-bound reply token, but no owner access token. Telegram senders are not signed in to the channel owner's NyxID account. Scope credentials held directly by the agent separately. A plain `/start` offers chat and bot creation; setup links, `/recover`, and callback buttons stay in the manager workflow.

**Verify Bot** checks the live manager webhook without replacing it. If configuration needs repair, save the manager in Platform Credentials and verify again. Token rotation there updates replies automatically. Deleting the channel disables its routes and preserves bot creation. The current limit is 32 concurrent background deliveries per backend process; excess messages are dropped, and restarts or failures may lose messages. Delivery can be duplicated or finish out of order.

## Register a Discord bot

1. Create an application and bot in the [Discord Developer Portal](https://discord.com/developers/applications). Copy the **Bot Token** from the Bot settings and the **Public Key** from General Information. The Public Key is an Ed25519 key, not the Application ID.
2. Open `/channel-bots/connect/discord`.
3. Review the bot name, enter the bot token and public key, and click **Create channel bot**.
4. Click **Open channel bot** and copy the callback URL into your Discord application's **Interactions Endpoint URL**.
5. Install the app in the intended server with the permissions needed for your interactions and assign an agent in NyxID.

The callback URL has this shape; use your NyxID instance's public API origin:

```
https://YOUR_NYXID_API_ORIGIN/api/v1/webhooks/channel/discord/<BOT_ID>
```

Discord sends a `PING` challenge on first setup. NyxID handles it automatically (responds with `PONG`).

## Register a Lark or Feishu bot

Lark and Feishu use the same adapter (different base URLs). Webhook registration is manual — you configure the URL in the developer console, not through NyxID.

NyxID exchanges your App ID and App Secret for a tenant access token automatically. There is no separate Bot token to enter.

1. In the [Lark Developer Console](https://open.larksuite.com/app) (or Feishu equivalent), create an app and enable bot capabilities.
2. Note the **App ID**, **App Secret**, **Verification Token**, and optionally the **Encrypt Key**.
3. Open `/channel-bots/connect/lark` or `/channel-bots/connect/feishu`.
4. Enter:

| Field | Source |
|---|---|
| **Bot name** | Your name for this bot |
| **App ID** | Lark Developer Console → App ID |
| **App Secret** | Lark Developer Console → App Secret |
| **Verification Token** | Event Subscriptions → Verification Token |
| **Encrypt Key** | Event Subscriptions → Encrypt Key (optional) |

5. Click **Create channel bot**, then **Open channel bot**. NyxID stores the credentials; the bot status starts as `pending_webhook`.

6. Copy the bot's callback URL into **Event Subscriptions** in the platform's developer console. For Lark it has this shape; Feishu uses `/feishu/` instead of `/lark/`:

```
https://YOUR_NYXID_API_ORIGIN/api/v1/webhooks/channel/lark/<BOT_ID>
```

7. Subscribe to at least `im.message.receive_v1`, enable the app's required message permissions, and publish/install the app for the intended workspace. Assign an agent in NyxID. The bot promotes to **active** automatically after the first verified inbound event.

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

## Register a Slack bot

1. Create a Slack app at [Your Apps](https://api.slack.com/apps). Configure the bot scopes required for the conversations it will read and for sending replies, then install the app in the workspace.
2. Copy the **Bot User OAuth Token** from **OAuth & Permissions** and the **Signing Secret** from **Basic Information → App Credentials**.
3. Open `/channel-bots/connect/slack`, review **Bot name**, and enter **Bot token** and **Signing Secret**. Click **Create channel bot**.
4. Click **Open channel bot** and copy its callback URL into the Slack app's **Event Subscriptions → Request URL**. Enable Events and let Slack verify the URL.
5. Subscribe to the relevant bot message events, save the app changes, and reinstall the app if Slack requests it after permissions change. Invite the bot to the channels it should handle.
6. Assign an agent in NyxID and send a test message in an allowed conversation.

Slack's Signing Secret verifies inbound requests. It is distinct from the bot token used to send replies. In prefilled URLs, the Signing Secret uses the catalog field name `app_secret`.

## Connect WhatsApp

Open `/channel-bots/connect/whatsapp`. The available form depends on whether an administrator has configured Meta Embedded Signup for the NyxID instance.

### Connect with Meta

1. Review the bot name and owner. Choose **New or existing Cloud API number**, or **I already use the WhatsApp Business app on this number** when that option is available.
2. Click **Connect with Meta** and complete Meta's login, business/account selection, and number authorization. Allow the popup if your browser blocks it.
3. Return to NyxID while it exchanges the authorization code, subscribes the account, and registers the number.
4. Click **Open channel bot**. Review the managed setup status and use **Repair setup** if NyxID shows an incomplete step. Assign an agent and send a test message.

The administrator must first configure Meta's app, Embedded Signup configuration, callback URL, and webhook subscriptions in **Admin → Platform Credentials**. For the exact operator steps, see the managed WhatsApp onboarding section in `docs/CHANNEL_BOT_RELAY.md`.

### Use your own WhatsApp credentials

1. If managed signup is available, expand **Advanced: use your own credentials**. Otherwise, the credential form is shown directly. A link containing manual credential parameters opens this form automatically.
2. Enter the **Access token**, **Phone Number ID**, and **Meta App Secret**. **WhatsApp Business Account ID** is optional. The Phone Number ID is Meta's identifier, not the display phone number or app ID.
3. Click **Create channel bot**. Copy the **Callback URL** and the one-time **Verify Token** from the completion screen before leaving it.
4. In **Meta App Dashboard → WhatsApp → Configuration**, enter that callback URL and Verify Token, verify and save, then subscribe to `messages`. Ensure the app is subscribed to the WhatsApp Business Account.
5. Click **Open channel bot**, assign an agent, and send a test message. Use a System User access token for ongoing operation; temporary API Setup tokens are for controlled tests with verified test recipients.

The Verify Token in step 3 is generated by NyxID. It is separate from both the Meta access token and Meta App Secret.

## Connect X (Twitter)

1. An administrator configures the X provider credentials and the required delivery settings for this NyxID instance. If these are missing, the page explains that account connection is unavailable.
2. Open `/channel-bots/connect/x` and review the bot name and owner.
3. Click **Connect X (Twitter) account**, sign in to the intended X account, and authorize the requested permissions in the popup.
4. Return to NyxID, click **Open channel bot**, review the connection/delivery status, and assign an agent.

X uses managed OAuth only. The page does not accept a developer token or other manually supplied X credentials. OAuth refusal leaves the page available for another attempt.

## Connect Aurinko Email

1. Connect the intended mailbox in Aurinko and obtain its **account access token** with `Mail.Read` and `Mail.Send`. Also obtain the separate **application signing secret** from the Aurinko dashboard.
2. Open `/channel-bots/connect/aurinko`, review **Bot name**, and enter **Account access token** and **Aurinko signing secret**.
3. Click **Create channel bot**. NyxID verifies the mailbox account and creates its email subscription automatically.
4. Click **Open channel bot**, configure a default agent with a callback URL, and send a new incoming email to the connected mailbox to check delivery.

Use a mailbox account token, not an application API key. Aurinko Email has no managed OAuth flow on this page. Its channel token and any Aurinko AI Service token are stored separately and must be rotated or deleted separately. This channel forwards new incoming personal mail; attachments are accessed through the AI Service.

## If setup does not complete

| What you see | Next step |
|---|---|
| Sign-in page | Sign in; NyxID returns to the selected channel with its supplied values. |
| Disabled **Create channel bot** | Fill every required field and correct the field errors. A valid prefilled form can be submitted without editing it. |
| Manager or account connection unavailable | Ask an administrator to configure that provider, then refresh the page. WhatsApp also offers manual credentials; Telegram creation and X require their managed flow. |
| Popup blocked or authorization cancelled | Allow the provider popup and retry the connection action. |
| X sign-in window was closed | Use **Retry connection**. If the original authorization window is still open, you can finish there instead. |
| Organization ownership is rejected | Confirm that the link contains the intended organization UUID and that you are its administrator, or explicitly choose **Personal** / another eligible organization. |
| Unsupported or disabled channel | Use **Choose another channel** to return to the current catalog. |
| `pending_webhook` after creation | Follow the platform's callback/subscription steps, check that the NyxID API URL is publicly reachable, and send a verified inbound event. |
| Credentials disappeared after reload | Secret query parameters were removed after prefilling. Enter the credentials again or reopen the original private link. |

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
