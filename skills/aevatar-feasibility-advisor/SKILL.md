---
name: aevatar-feasibility-advisor
description: Decide — honestly — whether a thing the user wants to build on Aevatar is possible, what its prerequisites are, or why it cannot be done, BEFORE anyone starts building. Use this first whenever a user describes a goal rather than a concrete artifact — "can aevatar do X", "I want a bot that…", "build me something that posts to Twitter / reads my GitHub / replies on Telegram", "is it possible to…", "automate … every day", "let Lark Base trigger a workflow". It teaches the one hard premise (every third-party capability is brokered by NyxID), the two distinct surfaces (outbound connector vs inbound channel), external HTTP trigger options such as Lark Base automation, how to check what is actually connectable, the prerequisite for each capability class, what is host-gated (and so not self-serve), and what is genuinely impossible without new NyxID/Aevatar platform work — so you can negotiate scope and give the user a straight answer plus next steps instead of over-promising. It scopes; it does not build (hand off to workflow-authoring / team-builder / service-publisher / scheduler).
version: "1.1"
metadata:
  category: plain
  tag:
    - aevatar
    - feasibility
    - capability
    - scoping
    - nyxid
    - prerequisites
    - advisor
    - negotiation
---

# Aevatar feasibility advisor

Before you (or the user) commit to building something on Aevatar, answer three questions
**honestly**: *Is it possible? What must be in place first? If not, why — and what's the
alternative?* This skill exists so you negotiate scope up front instead of discovering a
hard blocker halfway through. It only **advises** — once a plan is feasible, hand off to
`aevatar-workflow-authoring` → `aevatar-team-builder` → `aevatar-service-publisher` →
`aevatar-scheduler` (see `aevatar-platform-map`).

## The one premise: NyxID is the universal gateway

Aevatar holds **no third-party credentials and talks to no external service directly.**
Every external capability is brokered by **NyxID**. That single fact drives every
feasibility answer below. It splits into **two surfaces that people constantly conflate** —
get this right first:

| Surface | What it gives you | How it's used | Supported set |
|---|---|---|---|
| **Connector** (outbound) | Your workflow/agent **calls** a third-party API (read data, post, act) | `nyxid_proxy` tool (or a typed connector tool) with the service `slug` | Anything in the NyxID **catalog** (see below) — broad |
| **Channel** (inbound) | A third-party chat platform **delivers user messages to your agent**, which replies **in that platform** | An Aevatar **channel module** + NyxID relay webhook | **Narrow** — only platforms with a built module |

> **The trap:** "I want a Twitter bot." A Twitter *connector* (`api-twitter`) exists, so your
> agent **can post/read tweets** (outbound). But there is **no Twitter inbound channel
> module**, so you **cannot** have an agent that auto-replies to mentions/DMs the way a Lark
> or Telegram bot does. Same word, two very different feasibility answers. Always separate
> "call the API" from "be a bot on that platform."

## Step 0 — Inspect what is actually connectable (don't guess)

You hold a NyxID bearer (`~/.nyxid/access_token`; base in `~/.nyxid/base_url`, e.g.
`https://nyx.chrono-ai.fun`). Two read-only calls tell you the ground truth — make them with
the **`curl` binary** (a WAF may 403 Python HTTP clients):

```bash
NYX=$(tr -d '\n' < ~/.nyxid/base_url); TOK=$(tr -d '\n' < ~/.nyxid/access_token)
# What the user already has wired up (slugs you can nyxid_proxy right now):
curl -s -H "Authorization: Bearer $TOK" "$NYX/api/v1/services"   # -> [{slug,name,...}]
# What CAN be connected, and exactly how (auth model + setup instructions):
curl -s -H "Authorization: Bearer $TOK" "$NYX/api/v1/catalog"    # -> {entries:[{slug,auth_method,credential_mode,requires_credential,api_key_instructions,api_key_url,documentation_url,...}]}
```
(Inside an aevatar session with the nyxid MCP, the equivalent is the `nyxid_services` tool,
`{"action":"list"}`.) **The live catalog is the source of truth — never assert a connector
exists or doesn't without checking it.** The examples below are illustrative, not a fixed list.

### Reading a catalog entry (this is the "what's the prerequisite" answer)
- `requires_credential: true` → the user must connect it before any call works.
- `credential_mode: user` + `auth_method: bearer`/`provider_type: oauth2` → **the end user
  self-connects via an OAuth flow** in the NyxID console (their own account). Low friction.
  *(e.g. `api-twitter`, `api-slack`, `api-github`, `api-google`, `api-lark`, `api-reddit`,
  `api-tiktok`, `api-facebook`, `api-microsoft`.)*
- `credential_mode: admin` (`auth_method: api_key`/`bot_bearer`/`token_exchange`/`path`) → a
  **token/secret must be supplied** (often a bot token or an org/admin key), per the entry's
  `api_key_instructions` + `api_key_url`. *(e.g. `api-telegram-bot` — a @BotFather token;
  `api-discord-bot` — a Bot Token; `api-lark-bot`/`api-feishu-bot` — token_exchange; the
  `llm-*` provider keys.)*
- The entry's `api_key_instructions`, `api_key_url`, and `documentation_url` are exactly what
  you relay to the user as "here's how to connect it."

## The feasibility procedure

1. **Restate the goal as a capability, not a product.** "A bot that tweets a daily summary"
   = *(a)* generate text (pure LLM — always available) + *(b)* **post to X** (outbound
   connector `api-twitter`) + *(c)* run **daily** (schedule). Decompose into capability
   classes before judging.
2. **Classify each piece** against the matrix below and collect its prerequisite.
3. **Find the gating piece** — the answer to the whole request is the *weakest* piece (a
   single host-gated or impossible piece caps the whole thing).
4. **Report honestly** with the template at the end: possible + prereqs, or host-action-needed,
   or not-feasible + alternative.

## Prerequisite matrix (capability class → can we? → prerequisite)

| The user wants… | Possible? | Prerequisite / who must do it |
|---|---|---|
| Pure LLM / text / transform / branching pipeline | ✅ Always | Author a workflow (`aevatar-workflow-authoring`). No external anything. |
| **Call** a third-party API (read/post): GitHub, Slack, Google, X/Twitter, Reddit, a custom HTTP API… | ✅ If the connector is in the catalog | User **connects the `api-*` connector in NyxID** (OAuth for `user` mode, or supplies a token for `admin` mode — per the catalog entry). Then the workflow calls it via `nyxid_proxy`. |
| A connector that is **NOT in the catalog** | ⚠️ Only if it's a plain HTTP API | If it speaks HTTP + a supported `auth_method`, NyxID can add it (platform/admin work — not self-serve). If not HTTP, ❌. |
| **Inbound bot** that replies in-platform: **Lark / Telegram** | ✅ Yes | Connect the bot connector (`api-lark-bot` / `api-telegram-bot`) **and** register the channel (channel-admin / `channel_registrations`); NyxID provisions the webhook to Aevatar's relay. |
| **Inbound bot** on a platform with a connector but **no channel module** (Discord, Slack, X, …) | ❌ Not self-serve | Outbound calls work, but inbound-reply needs a new Aevatar **channel module** + relay wiring = Aevatar platform work. Offer the outbound-only version as the alternative. |
| **Publish** a workflow/team as an **invocable service** in-scope | ✅ Yes | Just bind it (`aevatar-service-publisher`). Usable within the user's scope immediately. |
| An external automation **triggers an existing Aevatar workflow** (e.g. Lark Base row status changed → HTTP request → run member workflow) | ✅ Usually, without service externalExposure | Use the external system's HTTP action to call the NyxID proxy for the existing `aevatar` service with a NyxID API key (`proxy` scope), targeting an explicit `/api/scopes/{scopeId}/members/{memberId}/invoke/...` or `/teams/{teamId}/invoke/...` path. This is an external trigger, not a NyxID connector registration. See `aevatar-service-publisher`. |
| Have that service **registered as a NyxID-brokered connector** (callable by others/externally) | ⚠️ Host-gated | The **host** must enable external exposure (`GAgentService:ExternalExposure: Enabled=true` + `RegisterAllPublishedServices` or an opt-in policy). You **cannot** turn this on as a client — verify `externalExposure` on the service and, if empty, tell the user to ask the host. |
| **Schedule** a recurring run (cron) | ⚠️ Yes, with a binding | The scope owner needs a durable **NyxID broker binding** — i.e. an interactive **console** NyxID login, not just a CLI token. Without it, schedule creation 400s ("Authenticated NyxID owner binding is required"). |
| A service backed by an **arbitrary custom agent / actor type** | ⚠️ Constrained | Member implementations are `workflow`, `script`, or **registered** `gagent` kinds (`GET /api/scopes/gagent-types`). You can't point a service at an arbitrary actor; wrap custom logic in a workflow or script, or use a registered gagent kind. |
| A genuinely **new service *shape*** (e.g. streaming/WebSocket/gRPC endpoint, a runtime kind beyond workflow/script/gagent) | ❌ Not currently | Service endpoints are unary **HTTP** over the fixed implementation kinds. A new shape needs Aevatar platform work. |
| **Exactly-once** external side effects (e.g. "charge exactly once") | ❌ Not guaranteed | The workflow saga is **at-least-once** with idempotency keys. Require an idempotent connector endpoint, or do the exactly-once elsewhere. |

## Hard engine/platform limits (make some asks impossible or need a workaround)

State these plainly when they bite:
- **No clock.** The engine has no time source. "When it's 9am", "every N minutes from inside
  the run", relative dates — must be injected at the input or driven by an external **schedule**
  (`aevatar-scheduler`), never computed inside the workflow.
- **No unbounded background loops / polling / fan-out-forever.** A run is a finite stepped
  pipeline with **one terminal step**; long waits use durable `delay`/`wait_signal` events, not
  busy loops. "Watch a feed continuously and react" → model as a *scheduled* run that polls.
- **Step/tool execution timeouts** — long synchronous external calls fail; design around it
  (chunk, or use `wait_signal`/human-in-the-loop for long external waits).
- **`nyxid_proxy` file artifacts cap at 100 MiB.** Bigger downloads aren't feasible that way.
- **Async settling.** Bindings/deployments/runs are eventually consistent — never promise a
  result from a 2xx alone.

## How to satisfy each prerequisite (what you tell the user to do)

- **Connect a connector** → "In the NyxID console, connect **`<slug>`**. <`credential_mode:user`:
  it's a one-click OAuth to your own account.> <`admin`: you'll paste a token — `<api_key_instructions>`;
  get it at `<api_key_url>`.>" Then confirm with `GET /api/v1/services` that the slug appears.
- **Register an inbound channel** (Lark/Telegram) → connect the bot connector, then register the
  channel via the channel-admin tool so NyxID wires the webhook to Aevatar's relay.
- **External HTTP trigger** (Lark Base / webhook sender / external cron) → do **not** ask for
  Aevatar `externalExposure` first. If the external system can call a public HTTPS URL with
  headers and JSON, call NyxID's proxy route for the already-connected `aevatar` service using
  a NyxID API key with `proxy` scope, and include the real scope/member/team id in the Aevatar
  path. If the sender cannot tolerate SSE or cannot shape the typed payload, add a small
  adapter/custom NyxID service or ask the host to configure Aevatar's webhook ingress.
- **NyxID service registration** → ask the **host** to enable external exposure for the service;
  you can only drive publish + verify the `externalExposure` block.
- **Scheduling** → "Do an interactive NyxID login in the Aevatar console once (establishes the
  scope-owner broker binding); then I can create the cron schedule."
- **Missing connector / new shape / new channel** → this is NyxID/Aevatar **platform work**;
  it's a request to the platform team, not something you or the user can self-serve. Say so and
  offer the closest feasible alternative.

## Negotiation / report template

Give the user a straight answer in this shape — never a vague "maybe":

- ✅ **Yes** — "<goal> is possible. One thing to do first: connect **`api-github`** in NyxID
  (OAuth, your account). After that I can build it as a workflow + schedule it."
- ⚠️ **Yes, but it needs an action you can't self-serve** — "The pipeline is fine, but exposing
  it as a NyxID connector for *others* to call requires the **host** to enable external exposure.
  In your own scope it works today without that."
- ✅ **Yes, no externalExposure needed for this shape** — "Lark Base can trigger your existing
  member workflow by sending an HTTPS request to NyxID's `aevatar` proxy with a NyxID API key,
  targeting `/api/scopes/{scopeId}/members/{memberId}/invoke/chat:stream`. ExternalExposure is
  only needed if you want this workflow registered as a reusable NyxID connector/slug."
- ❌ **Not as described** — "An auto-replying **Twitter bot** isn't possible: there's no inbound
  Twitter channel on Aevatar (only Lark and Telegram). What *is* possible: a workflow that
  **posts** to X on a schedule (via the `api-twitter` connector), or an inbound bot on **Telegram**
  instead. Want either of those?"

Always: name the exact connector/prereq, say who must do it, and offer the nearest feasible
alternative when you say no.

## Honesty rules

- **Check the live catalog/services** before claiming a connector exists or not. Examples in
  this doc are illustrative and can drift.
- **Connector ≠ channel.** Outbound API access never implies an inbound bot.
- **Never promise host-gated outcomes** (NyxID registration, anything needing host config) or
  features that need platform work — surface them as dependencies, not done deals.
- If you genuinely can't determine feasibility from the catalog + this matrix, say what you'd
  need to confirm rather than guessing.
