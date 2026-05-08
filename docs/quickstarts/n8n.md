# n8n: One credential for 4 APIs (OAuth + headers + path-auth, all transparent)

**TL;DR** — Four APIs (Gemini, TwitterAPI.io, Google Sheets, Telegram Bot), four different auth styles, **one Header Auth credential in n8n**. NyxID sits in front of the four APIs and injects the right auth per call. The setup took ~15 min because I had Claude Code run the commands — I just approved.

```
n8n  ─┐
Claude─┼──(one key: X-API-Key: nyx_xxx)──►  NyxID Proxy  ──►  Gemini / Twitter / Sheets / Telegram
curl  ─┘                                                    (injects correct auth per service)
```

Bonus: the same NyxID key works unchanged in Claude Code, Cursor, or a curl script — configure auth once, reuse everywhere.

> This walkthrough shows the workflow exactly as I built it. The commands are real `nyxid` CLI calls; you can run them yourself in any terminal — Claude Code is optional, just convenient.

---

## Step 0 — Get NyxID running and create an Agent Key

Skip this if you already have a NyxID account and an Agent Key (`nyx_...`).

**Hosted (recommended):** Sign up at [nyx.chrono-ai.fun/register](https://nyx.chrono-ai.fun/register) with the invite code from the [README Setup](../../README.md#setup). After you're in, open **AI Services → Agent Keys → Create API Key**, give it the `proxy` scope, click **Create**, and copy the `nyx_...` key (shown only once).

**Self-host:** Follow [docs/SETUP.md](../SETUP.md), then create the Agent Key the same way after registering at `http://localhost:3000`.

The rest of this guide assumes the `nyx_...` key is in `~/.nyx_key` (`chmod 600`). Step 4 below pastes it into n8n's Header Auth credential.

---

## The problem

My workflow pulls AI news from a dozen RSS feeds, fetches tweets from ~15 Chinese AI influencers, scores/translates/classifies everything with Gemini, writes the result to a Google Sheet, and sends daily digests to a Telegram group. Four APIs, four auth patterns:

- **Gemini** — header `x-goog-api-key: <key>`
- **TwitterAPI.io** — header `x-api-key: <key>`
- **Google Sheets** — OAuth2 bearer token (refresh_token dance)
- **Telegram Bot** — path-based auth (token goes *into the URL*: `/bot<token>/sendMessage`)

In n8n that's four separate credentials, four rotation paths. Sheets is worst — every teammate who touches the workflow has to re-do OAuth into *their own* n8n credential. Telegram sits in a different node class with its own credential type. And if I also want to call the same APIs from Claude Code or curl, I have to set up auth all over again in each place.

---

## What NyxID is

[NyxID](https://github.com/ChronoAIProject/NyxID) is open-source, self-hosted, Docker run + 3 env vars (or use the hosted instance). Two properties matter for this setup: it **injects per-service auth at proxy time** (so n8n only holds one key), and **its CLI is built for AI agents to drive**, so Claude Code wired the whole thing up end-to-end without me copy-pasting tokens into chat.

> **Token handling:** I kept each downstream token in a local file (`chmod 600`) and had Claude Code read them via `--credential-env VAR_NAME` — raw values never hit the chat transcript, shell history, or command line.

---

## Setup (what I actually did)

### 1. Register the 3 API-key services (Gemini, TwitterAPI.io, Telegram)

I put each token in a local file (`~/.gemini_key`, `~/.twitterapi_key`, `~/.tg_token`, `chmod 600`) and ran `nyxid service add` for each. Catalog services (Gemini, Telegram) just need a slug; non-catalog services (TwitterAPI.io is not in the catalog) use `--custom` plus auth details:

```bash
# Gemini — catalog slug llm-google-ai
GEMINI_KEY="$(cat ~/.gemini_key)" \
  nyxid service add llm-google-ai --credential-env GEMINI_KEY --label "Gemini AI"

# TwitterAPI.io — not in catalog, register as custom
TWITTER_KEY="$(cat ~/.twitterapi_key)" \
  nyxid service add --custom \
    --slug twitterapi-io \
    --endpoint-url "https://api.twitterapi.io" \
    --auth-method header --auth-key-name x-api-key \
    --credential-env TWITTER_KEY \
    --label "TwitterAPI.io"

# Telegram Bot — catalog slug api-telegram-bot (path-auth handled internally)
TG_TOKEN="$(cat ~/.tg_token)" \
  nyxid service add api-telegram-bot --credential-env TG_TOKEN --label "Telegram"
```

Each `service add` prints the user-side slug it landed on (e.g. `gemini-ai-6yd7`, `twitterapi-io`, `api-telegram-bot`). On a fresh account the slug is the catalog/custom name; if you already have a service with that slug NyxID appends `-2`, `-3`, or a random suffix to keep them unique. Claude Code then `sed`-replaced the printed slugs into my workflow JSON placeholders automatically.

### 2. Register Google Sheets (OAuth2 — the hard one)

OAuth needed a Google Cloud OAuth client. The Cloud Console steps Claude Code couldn't do for me:

1. Create an OAuth 2.0 Client ID (Web application)
2. Add `https://<nyxid-host>/api/v1/providers/callback` to Authorized redirect URIs
3. Enable Google Sheets API + Google Drive API
4. On OAuth consent screen → **Data access → ADD OR REMOVE SCOPES**, add `https://www.googleapis.com/auth/spreadsheets`. **Do not skip this.** If you pass scope only on the CLI, Google silently drops it and your token comes back without Sheets permission — you get `ACCESS_TOKEN_SCOPE_INSUFFICIENT` at runtime and spend an hour wondering why.
5. Add your gmail to Test Users

Put `client_id` / `client_secret` in `~/.gc_id` / `~/.gc_secret`. Then:

```bash
GC_ID="$(cat ~/.gc_id)" GC_SECRET="$(cat ~/.gc_secret)" \
  nyxid service credentials api-google --client-id-env GC_ID --client-secret-env GC_SECRET

nyxid service add api-google --oauth \
  --scope "https://www.googleapis.com/auth/spreadsheets" \
  --label "Google Sheets"
# CLI prints a URL; open it, log in, click Allow
```

Catalog default for Google is `www.googleapis.com`, but Sheets lives on `sheets.googleapis.com`. Claude Code caught the 404 on the first test call and ran:

```bash
nyxid service update <id> --endpoint-url "https://sheets.googleapis.com"
```

Then verified an end-to-end write through the proxy **before touching n8n** — row appeared in the sheet.

### 3. Create the n8n credential (a NyxID Agent Key)

Create an Agent Key with proxy scope:

```bash
nyxid api-key create --name "n8n-ai-digest" --scopes "proxy" \
  --output json | jq -r '.full_key' > ~/.nyx_key
chmod 600 ~/.nyx_key
```

> **The JSON field for the key value is `full_key`, not `key`.** I tried `key`, `api_key`, `value`, `token` first — none work.

Read the key into clipboard from your own terminal (don't paste it into chat):

```bash
cat ~/.nyx_key | pbcopy   # macOS; on Linux use xclip -selection clipboard
```

Then n8n → **Credentials → New → Header Auth** (Name: `NyxID API Key`, Header Name: `X-API-Key`), paste, save, then `shred -u ~/.nyx_key` (or `rm -P` on macOS).

**For per-workflow scoping** (so a leak can only hit these 4 APIs), open the key in the web console — **AI Services → Agent Keys → \[your key\] → Service Scope** — uncheck **Allow all services**, pick the 4 services from the list, and save. The CLI takes service UUIDs in `--allowed-services` while the web console resolves slugs for you, so the web console is faster here.

### 4. Import the patched workflow JSON

Because every URL in the workflow already had the right NyxID host, slugs, and Sheet ID baked in (Claude Code `sed`-replaced them), inside n8n I only clicked one HTTP node's credential dropdown and picked **NyxID API Key** — every other HTTP node auto-matched by credential name.

**Total hands-on time: ~15 min**, mostly waiting for OAuth browser redirects.

---

## How the URLs look in n8n HTTP nodes

Proxy path template:

```
https://{nyxid-host}/api/v1/proxy/s/{service-slug}/{downstream-api-path}
```

Same `X-API-Key: nyx_xxx` header on every call. Different slugs route to different downstream APIs, NyxID injects the right auth for each:

```
# Gemini — no x-goog-api-key needed; NyxID injects it
POST /api/v1/proxy/s/gemini-ai-6yd7/models/gemini-2.5-flash:generateContent

# TwitterAPI.io — no x-api-key needed; NyxID injects it
GET  /api/v1/proxy/s/twitterapi-io-jb13/twitter/user/last_tweets?userName=someone

# Google Sheets — no Authorization header; NyxID refreshes + injects OAuth token
POST /api/v1/proxy/s/api-google-3/v4/spreadsheets/<sheet-id>/values/ai_briefing!A:H:append

# Telegram — note: no bot<token>/ in the path; NyxID prepends it before forwarding.
# Actual URL Telegram sees: https://api.telegram.org/bot<token>/sendMessage
POST /api/v1/proxy/s/api-telegram-bot/sendMessage
```

Four URLs, one n8n credential. Four auth patterns handled transparently.

---

## Before vs After

|                              | Before                                                                | After                                                               |
| ---------------------------- | --------------------------------------------------------------------- | ------------------------------------------------------------------- |
| Credentials in n8n           | 4 (Sheets via OAuth2, Telegram via native node)                       | **1** (NyxID Header Auth)                                           |
| Google Sheets authorization  | Each teammate re-does OAuth in their own n8n                          | **NyxID authorizes once**, every workflow reuses the token          |
| Telegram Bot token           | Lives in n8n's Telegram credential                                    | Lives in NyxID; n8n never sees it                                   |
| Key rotation                 | Update each credential manually                                       | `nyxid service rotate-credential <id>` — all workflows keep working |
| Auth complexity              | 4 different patterns (2 headers + OAuth bearer + URL-path token)      | One `X-API-Key`, NyxID handles the rest                             |
| Reuse outside n8n            | No (locked in n8n's DB)                                               | **Same NyxID key works in Claude Code, Cursor, curl**               |
| Credential leak blast radius | Full API account                                                      | Per-workflow scoped — only the 4 APIs this workflow uses            |

---

## What the workflow actually does

Daily at 08:00:

1. Pull 13 RSS feeds (The Verge, TechCrunch, OpenAI Blog, DeepMind, WIRED, 404 Media, MIT Tech Review, etc.) via n8n's RSS node
2. Pull last 24h of tweets from ~15 Chinese AI influencers via TwitterAPI.io (through NyxID)
3. Gemini translates/summarizes (if English), classifies into `Product Launch` / `Research & Blog` / `Other`, extracts a Chinese title (through NyxID)
4. Global dedup by content signature
5. Pick top-10 most valuable tweets with another Gemini call
6. Append every processed row to a Google Sheet (through NyxID, scoped to `ai_briefing!A:H`)
7. Send three formatted digests (general news / deep-dives / Top-10 tweets) to a Telegram group (through NyxID, each split into ≤3800-char chunks to stay under Telegram's 4096 limit)

71 nodes, 4 NyxID services, 1 credential. First run wrote ~150 rows and pushed three Telegram messages.

---

## Gotchas from building this

**Google Sheets quota is 60 writes/min/user.** n8n's default "1 req every 500ms" blows past that around row 60 and returns 429. Set the HTTP node's **Batch Interval to ≥1200ms** and turn on **Retry On Fail** (max 3, wait 30s — Google's quota is a rolling 1-minute window, so 30s usually clears it).

**Catalog default endpoint may not match the actual API subdomain.** Google's default is `www.googleapis.com` but Sheets lives on `sheets.googleapis.com`. If proxy calls return 404 "was not found on this server", run `nyxid service update <id> --endpoint-url <correct-subdomain>`.

**OAuth consent screen must declare every scope you'll request.** Adding `--scope spreadsheets` on the CLI does nothing if the consent screen doesn't list that scope — Google silently drops it. Add it on the consent screen *before* running the OAuth flow.

**Gemini 2.5 Flash's `thinkingConfig` eats `maxOutputTokens`.** I set 4096 and got truncated JSON every time — the model was burning 3000+ tokens on "thinking." Set `maxOutputTokens: 65536` and `thinkingConfig: { thinkingBudget: 1024 }` so there's room for both.

**The JSON field for the key value is `full_key`, not `key`.** I tried `key`, `api_key`, `value`, `token` first — none work. (Repeating from Step 3 because this one bit me twice.)

---

## Next

- **Same flow with the `nyxid` CLI directly:** [docs/connecting-services/cli.md](../connecting-services/cli.md)
- **Per-agent isolation** (one scoped key per agent): [Claude Code quickstart](claude-code.md)
- **Reach localhost APIs from a cloud-hosted n8n:** [Node Proxy quickstart](node-proxy.md)
- **Wrap any REST API as MCP tools:** [MCP wrapping quickstart](mcp-wrapping.md)

Questions / critiques welcome — open an issue on [github.com/ChronoAIProject/NyxID](https://github.com/ChronoAIProject/NyxID/issues).
