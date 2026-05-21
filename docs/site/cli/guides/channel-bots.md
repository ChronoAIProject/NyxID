---
title: Connect a channel bot
description: Register a Telegram, Discord, Lark/Feishu, or Slack bot from the terminal and route its conversations to an AI agent.
---

A channel bot bridges a messaging platform to an AI agent: inbound messages are relayed to an agent's callback URL, and the agent replies back through NyxID. The CLI registers the bot, wires up webhook verification, and maps conversations to the agent key that should answer them. For the dashboard equivalent and platform-console screenshots, see the [web channel-bots guide](/docs/web/guides/channel-bots).

This assumes you are [logged in](/docs/cli/getting-started/authenticate). Pass secrets via `--*-env` flags so they never land in shell history.

## 1. Register the bot

```bash
export TELEGRAM_BOT_TOKEN=...        # from @BotFather
nyxid channel-bot register \
  --platform telegram \
  --label support \
  --token-env TELEGRAM_BOT_TOKEN
```

`--platform` is one of `telegram`, `discord`, `lark`, `feishu`, `slack`. Some platforms need extra material at registration:

- **Lark / Feishu** — `--app-id`, `--app-secret-env`, and `--verification-token` (used to verify inbound webhooks). `--encrypt-key` is optional, matching the Event Subscriptions console.
- **Discord** — `--public-key` for signature verification.
- **Slack** — pass the `xoxb-` bot token via `--token-env` and the app **signing secret** via `--app-secret-env`.

Add `--org <id|slug|name>` to register an org-owned bot.

## 2. Create the agent that will answer

Messages are relayed to an agent identified by a NyxID API key that carries a callback URL. Create one (or reuse an existing key) — see [Create scoped agent keys](/docs/cli/guides/scoped-agent-keys):

```bash
nyxid api-key create \
  --name "support-agent" \
  --platform generic \
  --callback-url https://my-agent.example.com/nyxid/callback
```

NyxID POSTs inbound messages to that callback URL; the agent returns `202` and posts its reply back through the channel relay.

## 3. Route conversations to the agent

Map the bot's conversations to the agent key. A catch-all default route is the simplest start:

```bash
nyxid channel-bot route create \
  --bot-id <bot-id> \
  --agent-key-id <api-key-id> \
  --default-agent
```

For finer control, target a specific conversation with `--conversation-id` (and `--conversation-type private|group|channel`) or filter by `--sender-id`. List and adjust routes:

```bash
nyxid channel-bot route list --bot-id <bot-id>
nyxid channel-bot route update <route-id> --active false
nyxid channel-bot route delete <route-id> --yes
```

## Verify and manage

```bash
nyxid channel-bot list                 # bots + their status
nyxid channel-bot show <bot-id>
nyxid channel-bot verify <bot-id>      # re-check the token and re-register the webhook
nyxid channel-bot delete <bot-id> --yes
```

:::note
A Lark / Feishu bot stuck in `pending_webhook` is usually missing its verification token. Patch it, then wait for the next inbound to auto-promote it to `active`:

```bash
nyxid channel-bot update <bot-id> --verification-token vtoken_xxx --encrypt-key key_xxx
```
:::

## Next

- [`api-key` command reference](/docs/cli/reference/api-key) — agent keys and callback URLs.
- [Connect a channel bot (web)](/docs/web/guides/channel-bots) — the dashboard flow with platform-console steps.
