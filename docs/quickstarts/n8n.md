# n8n: Daily AI News Digest with One NyxID Credential

Build an n8n workflow that fetches an RSS feed, summarizes each article with Gemini, and posts the summary to Telegram. Two upstream APIs (Gemini, Telegram), one NyxID Agent Key in n8n, no upstream secrets stored in n8n.

```
Schedule  ─►  RSS Read  ─►  HTTP Request (Gemini)  ─►  HTTP Request (Telegram)
                              │                          │
                              └─►  NyxID Proxy  ◄─────────┘
                                   (injects Gemini key for one call,
                                    Telegram bot token for the other)
```

The Agent Key in n8n authenticates n8n to NyxID. NyxID stores both upstream credentials encrypted and injects the right one per request. Adding a third or fourth upstream service later is one more `Add Service` click — no n8n credential change required.

## What gets stored where

| Item | Stored in | Purpose |
|---|---|---|
| NyxID Agent Key (`nyx_…`) | n8n | Authenticates n8n to NyxID |
| Gemini API key | NyxID | Injected as `x-goog-api-key` when n8n calls Gemini through NyxID |
| Telegram bot token | NyxID | Injected into the Telegram Bot API path when n8n calls Telegram through NyxID |

n8n only stores the NyxID Agent Key. It never sees the Gemini or Telegram credentials.

## Prerequisites

- A NyxID account and an Agent Key with the `proxy` scope. If you don't have these yet, complete **Step 0** below.
- An n8n instance (cloud or self-hosted) where you can add a credential and build a workflow.
- A **Gemini API key**. Get one from [aistudio.google.com/apikey](https://aistudio.google.com/apikey) → `Create API key`.
- A **Telegram bot token** plus a **chat ID** to post to:
  - **Bot token**: chat with [@BotFather](https://t.me/BotFather), run `/newbot`, follow the prompts. BotFather replies with a token like `1234567890:ABCdef…`.
  - **Chat ID**: send any message to your new bot, then open `https://api.telegram.org/bot<TOKEN>/getUpdates` in a browser. Look for `"chat":{"id":<NUMBER>,…}` — that number is the chat ID.

### Step 0 — Get NyxID running and create an Agent Key

**Hosted (recommended).** Sign up at [nyx.chrono-ai.fun/register](https://nyx.chrono-ai.fun/register) using the invite code in the [README Getting Started](../../README.md#1-install-nyxid). After signing in, open `AI Services` → `Agent Keys` → `Create API Key`. In the dialog, name the key `n8n`, click the `proxy` badge under `Scopes` so it's highlighted, then click `Create key`. Copy the displayed `nyx_…` value (shown once).

![Create API Key dialog with proxy scope selected](../connecting-services/img/06-create-agent-key.png)

**Self-host.** Follow [docs/SETUP.md](../SETUP.md) to bring up the Docker stack, register at `http://localhost:3000`, then create the Agent Key via the same web console flow.

Save the Agent Key value somewhere safe — a password manager works, or a local file with `chmod 600`.

## Procedure

### 1. Register Gemini and Telegram in NyxID

> The screenshots below show OpenAI being registered (from the [Web UI walkthrough](../connecting-services/web-ui.md)). The dialog is identical for every catalog service — substitute the service name and token for the one you're registering.

Repeat the following for **each** service: first Gemini, then Telegram Bot.

1. In the web console, click `AI Services` in the left sidebar, then click `Add Service`.

   ![AI Services page](../connecting-services/img/01-ai-services.png)

2. Type the service name (`Gemini AI` for Gemini, `Telegram Bot` for Telegram) in the catalog search and click `Connect` on the matching entry.

   ![Add AI Service catalog with search](../connecting-services/img/02-add-service-catalog.png)

3. The `Configure Routing` step appears. Click `Direct` and then `Next: Enter Credentials`.

   ![Configure Routing](../connecting-services/img/03-routing-step.png)

4. Paste the upstream token (Gemini API key, then Telegram bot token) into the `API Key / Credential` field and click `Create Service`.

   ![Configure Service — credential entry](../connecting-services/img/04-credential-entry.png)

5. NyxID lands on the service detail page. Note the `Slug` at the top — typically `llm-google-ai` for Gemini and `api-telegram-bot` for Telegram, suffixed (`-2`, `-3`, …) if you already have a service with that slug. You'll use the slugs in [Step 3](#3-build-the-workflow).

   ![Service detail page with slug](../connecting-services/img/05-service-detail.png)

After both services are registered, your `AI Services` list shows two `External Services` rows — Gemini and Telegram.

### 2. Create the n8n Header Auth credential

The Agent Key from [Step 0](#step-0--get-nyxid-running-and-create-an-agent-key) is what n8n sends to NyxID on every proxied request. n8n stores it as a reusable `Header Auth` credential.

1. Open your n8n instance. In the left sidebar, click `Credentials`.

   <!-- TODO: screenshot of n8n Credentials page -->

2. Click `Add Credential` (or `Create New` on first use).
3. Type `Header Auth` in the search box and click the `Header Auth` result.
4. Fill the form:

   | Field | Value |
   |---|---|
   | `Name` | `NyxID API Key` |
   | `Header Name` | `X-API-Key` |
   | `Header Value` | The `nyx_…` Agent Key from Step 0 |

   <!-- TODO: screenshot of completed n8n Header Auth credential form -->

5. Click `Save`.

> **Pasting the key safely.** If you saved the Agent Key to a local file, copy it into your clipboard with `cat ~/.nyx_key | pbcopy` (macOS) or `cat ~/.nyx_key | xclip -selection clipboard` (Linux), paste into the `Header Value` field, then securely delete the file with `rm -P ~/.nyx_key` (macOS) or `shred -u ~/.nyx_key` (Linux).

### 3. Build the workflow

The workflow has four nodes wired in sequence:

```
Schedule Trigger ──► RSS Read ──► HTTP Request (Gemini) ──► HTTP Request (Telegram)
```

<!-- TODO: screenshot of the full workflow on the n8n canvas -->

Add each node from n8n's node panel and configure as described.

#### Node 1 — Schedule Trigger

Built-in trigger node, no auth.

| Field | Value |
|---|---|
| `Trigger Interval` | `Every Day` |
| `Trigger At Hour` | `8` (or whatever you want) |

#### Node 2 — RSS Read

Built-in node, no auth.

| Field | Value |
|---|---|
| `URL` | An RSS feed URL — e.g. `https://hnrss.org/frontpage` for Hacker News |
| `Limit` (under `Options`) | `3` (so the demo posts at most 3 items per run) |

The node outputs an array of items, each with `title`, `link`, `contentSnippet`, etc.

#### Node 3 — HTTP Request (Gemini)

This is the first proxied call. Configure these fields. The auth fields cascade — selecting one reveals the next.

| Field | Value |
|---|---|
| `Method` | `POST` |
| `URL` | `https://<nyxid-host>/api/v1/proxy/s/llm-google-ai/v1beta/models/gemini-2.5-flash:generateContent` |
| `Authentication` | `Generic Credential Type` |
| `Generic Auth Type` | `Header Auth` |
| `Credential for Header Auth` | `NyxID API Key` |
| `Send Body` | toggled on |
| `Body Content Type` | `JSON` |
| `Specify Body` | `Using JSON` |
| `JSON` | (see below) |

Body:

```json
{
  "contents": [{
    "parts": [{
      "text": "Summarize this in 2 sentences:\n\n{{ $json.title }}\n\n{{ $json.contentSnippet }}"
    }]
  }],
  "generationConfig": { "maxOutputTokens": 256 }
}
```

`{{ $json.title }}` and `{{ $json.contentSnippet }}` reference the previous node's output (the current RSS item).

<!-- TODO: screenshot of HTTP Request node with auth cascade filled -->

> **Important — pick `Generic Credential Type`, not `Predefined Credential Type`.**
>
> n8n's `Predefined Credential Type` list shows built-in OpenAI / Google / Telegram credentials that call those upstream APIs **directly**. Using one of those bypasses NyxID and defeats the purpose of this setup.
>
> `Generic Credential Type` → `Header Auth` is the path that sends the request through NyxID, where the upstream credential is injected.

> **Don't add upstream auth to the n8n node.** NyxID injects `x-goog-api-key` for Gemini and the path-based bot token for Telegram automatically. Adding `x-goog-api-key`, `Authorization: Bearer …`, or the Telegram token in the URL on the n8n side defeats credential isolation — your upstream secret ends up in n8n's database.

#### Node 4 — HTTP Request (Telegram)

The second proxied call.

| Field | Value |
|---|---|
| `Method` | `POST` |
| `URL` | `https://<nyxid-host>/api/v1/proxy/s/api-telegram-bot/sendMessage` |
| `Authentication` | `Generic Credential Type` |
| `Generic Auth Type` | `Header Auth` |
| `Credential for Header Auth` | `NyxID API Key` |
| `Send Body` | toggled on |
| `Body Content Type` | `JSON` |
| `Specify Body` | `Using JSON` |
| `JSON` | (see below) |

Body:

```json
{
  "chat_id": "<YOUR_CHAT_ID>",
  "text": "📰 {{ $('RSS Read').item.json.title }}\n\n{{ $json.candidates[0].content.parts[0].text }}\n\n🔗 {{ $('RSS Read').item.json.link }}",
  "parse_mode": "Markdown"
}
```

Replace `<YOUR_CHAT_ID>` with the chat ID from Prerequisites. `{{ $json.candidates[0].content.parts[0].text }}` is Gemini's summary; `{{ $('RSS Read').item.json.title }}` and `link` reference the original RSS item by node name.

### 4. Verify

Click `Execute Workflow` in n8n's top toolbar. The RSS node fetches the feed, the Gemini node summarizes each item, and the Telegram node sends one message per item to your chat.

<!-- TODO: screenshot of a Telegram message that arrived from the workflow -->

Open the Telegram chat: you should see up to 3 short summaries, each with the article title, a 2-sentence Gemini summary, and the article link.

If nothing arrives, see [Troubleshooting](#troubleshooting). Once it works, click the `Inactive` toggle in n8n's top-right to switch the workflow to `Active` so the Schedule Trigger fires daily.

## Common mistakes

- **Predefined instead of Generic.** If the n8n HTTP Request node uses `Predefined Credential Type` → `Telegram` (or `Google`), it calls the upstream API directly with n8n's built-in credentials, bypassing NyxID entirely. Pick `Generic Credential Type` → `Header Auth`.
- **Adding `x-goog-api-key` or the Telegram token in the n8n node.** NyxID injects these. Setting them in n8n's `Headers` or URL path duplicates the credential and puts your upstream secret in n8n's database.
- **Forgetting to activate the workflow.** The `Active` toggle (top-right of the workflow editor) must be on for the Schedule Trigger to fire. Until then it only runs when you click `Execute Workflow`.
- **Wrong chat ID format.** Telegram chat IDs are integers (positive for users, negative for groups). Don't quote it as a string in the JSON body if your IDE strips quotes — but n8n's JSON body field expects valid JSON, so keep the quotes around the value if it contains a leading minus sign or you're storing it as a string.
- **Using a stale Gemini model name.** Google rotates model names. If `gemini-2.5-flash` returns a 404, check [aistudio.google.com](https://aistudio.google.com) for the current name (e.g. `gemini-2.5-pro`, `gemini-2.0-flash-exp`).

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `401` from NyxID, body `Missing API key` / `Invalid API key` | The `X-API-Key` header in n8n is empty or wrong | Re-paste the Agent Key into the `Header Auth` credential |
| `403` from NyxID | Agent Key is missing the `proxy` scope, or `Service Scope` excludes the service | Add `proxy` to the key's scopes, or open the key's `Service Scope` card and enable the service |
| `404` from NyxID | The service slug in the URL is wrong | Confirm the slug on the service detail page; the URL pattern is `/api/v1/proxy/s/<slug>/<path>` |
| `401` from the upstream API | The upstream credential stored in NyxID is wrong or revoked | `nyxid service rotate-credential <ID>` (or rotate via the web console) and re-paste the upstream credential |
| n8n request works with `curl` but not in the workflow | The HTTP Request node is using `Predefined Credential Type` instead of `Generic Credential Type` → `Header Auth` | Switch the node's `Authentication` and pick `NyxID API Key` |
| Telegram returns `Bad Request: chat not found` | Bot can't post to that chat (you haven't messaged the bot from that chat, or the chat ID is wrong) | Send `/start` to your bot from the target chat, then re-fetch the chat ID via `/getUpdates` |
| Gemini returns truncated JSON | `thinkingConfig` is consuming `maxOutputTokens` | Add `"thinkingConfig": { "thinkingBudget": 0 }` to the body's `generationConfig` |

## Add more services or OAuth-protected APIs

This guide stuck to two API-key services. The same pattern extends:

- **More API-key catalog services** (OpenAI, Anthropic, GitHub, etc.) — register each via `AI Services` → `Add Service` → search the catalog → paste the upstream token. Each new service gets its own slug; reuse the same `NyxID API Key` credential in any new HTTP Request node and only change the URL.
- **Custom services not in the catalog** — in the `Add Service` dialog, click `Add custom service` instead of picking a catalog entry. Set `Endpoint URL`, `Auth method` (e.g. `header`), and `Auth key name` (e.g. `x-api-key`).
- **OAuth-protected services** (Google Sheets, GitHub OAuth, etc.) — search the catalog, click `Connect`, paste OAuth client ID/secret, then click `Connect with [provider]` to run the consent flow. NyxID stores the refresh token and refreshes the access token on every request. The full reference walkthrough is in [docs/connecting-services/web-ui.md](../connecting-services/web-ui.md).

For per-workflow blast-radius isolation (one Agent Key per n8n workflow, scoped to only the services that workflow needs), open `AI Services` → `Agent Keys` → `[your key]`, locate the `Service Scope` card, uncheck `Allow all services`, and pick the services this workflow uses.

## Reference

- **Connecting AI Services hub** (Web UI / CLI / AI-driven / Direct API reference): [docs/connecting-services/](../connecting-services/)
- **Per-agent isolation** (one scoped Agent Key per agent): [Claude Code & Codex per-agent quickstart](claude-code.md)
- **Reach localhost APIs from a cloud-hosted n8n**: [Node Proxy quickstart](node-proxy.md)
- **Wrap any REST API as MCP tools**: [MCP wrapping quickstart](mcp-wrapping.md)
- **NyxID architecture**: [docs/ARCHITECTURE.md](../ARCHITECTURE.md)
