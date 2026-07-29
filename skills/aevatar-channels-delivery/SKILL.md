---
name: aevatar-channels-delivery
description: "Aevatar channel & delivery how-to: capability tools (code_execute, nyxid_proxy, GitHub PAT fallback, channel bots), token_expired/401 credential triage that identifies the typed credential source before interpreting the failure (a dedicated scheduled-invocation Agent Key is vault-held and late-resolved per use — never diagnose it with the 300-second broker TTL), LLM route selection slash commands, channel_registrations (staged Lark provisioning), agent_delivery_targets binding, and a strict never-emit-secret-material policy."
version: "1.3"
metadata:
  category: plain
  tag:
    - aevatar
    - aevatar-system
    - channels
    - lark
    - delivery
    - nyxid-proxy
---

### Capability tool details

**`code_execute`** — Execute Python, JavaScript, TypeScript, or Bash in a sandboxed environment. Returns stdout, stderr, and exit code. Use this for calculations, data processing, format conversion, testing code snippets, etc.

**`nyxid_service_inventory`** — In a channel sender turn, list that sender's exact connected-service
inventory with `{}`. This request-local channel exposure is list-only: do not hand-write a
`user_service_id`, and do not use `nyxid_proxy` as an inventory call.

**`nyxid_proxy`** — Make an admitted or raw request without exposing credentials. Keep its two
shapes distinct:
- Raw one-off call: require exact `service_id + slug + path`; `method`, `body`, allowed
  non-sensitive headers, and response mode are optional.
- Compiled workflow operation: keep copied `user_service_id + operation_id` only in the
  step-level `capability.nyxid_operation`. Runtime `arguments` contain only admitted
  `path_params`, `query`, `headers`, `body`, and `response_mode`; never repeat service, slug,
  operation ID, method, path template, digest, or schema.

**Critical**: Proxy paths are relative to the service's base URL (shown in `<connected-services>`). Do NOT duplicate version prefixes already in the base URL. For NyxID-specific service paths, OAuth/device/API-key connection flows, error code semantics, and conventions, **load `use_skill(skill="nyxid")` first** instead of guessing.

**GitHub PAT fallback**: when `api-github` returns 401/403/404 on a path that could require private-repo access or `read:project` scope (e.g. private org repos, `/projects/*`, `/orgs/*/projects`), retry the *same* path against the `api-github-pat` slug before treating the failure as terminal. `api-github-pat` is the user's Personal Access Token slot exactly for cases where the default OAuth scopes are insufficient; trying it is not "wandering". Same rule for the parallel pattern on other providers when both an OAuth-backed slug and a `-pat` slug are listed in `<connected-services>`.

**Channel Bots** — Use `nyxid_proxy` with a Telegram/Discord bot's slug to send messages. For Telegram: POST `/sendMessage` with `{"chat_id":"...","text":"..."}`.

**Credential honesty on `token_expired`/401** — `nyxid_proxy` and `code_execute` authenticate with
the CALLER credential of the current run or session, and different callers hold different
credential classes with wildly different lifetimes. **Identify the typed credential source before
you interpret the failure.** For a run, that means reading `credentialSourceKind` off the run or
automation record first — "it was scheduled" tells you nothing on its own:

| Caller | Credential class | Lifetime |
|---|---|---|
| Interactive session | Login access token | Hours (deployment-config `JWT_ACCESS_TTL_SECS`) |
| Run with `credentialSourceKind = scheduled_invocation_agent_key` (Studio Team member automation, scheduled skill agents) | **Dedicated, restricted Agent Key.** Raw material lives only in `ISecretVault`; the run borrows a durable credential *reference* and late-resolves it through the Vault **at each use**, fail-closed | Policy-governed (a scheduled skill agent's default projected expiry is ~90 days) — **not minutes** |
| Run with `credentialSourceKind = nyxid_binding_exchange` (generic `/api/schedules` with a NyxID binding source) | Short-lived bearer exchanged from the binding at fire | 300 s fixed (see `aevatar-automation` → "Token classes" for citations) |

When a capability tool returns `token_expired` or 401:
- Do not name a lifetime you did not read from the actual JWT (`exp − iat`), from the owning
  repo's source, or from the record's `credentialExpiresAtUtc`. Numbers recalled from memory are
  the #1 source of wrong diagnoses here.
- Check the blast radius: if ALL proxied calls fail after some instant, the caller credential
  died; if only one tool fails while sibling proxied calls succeed, suspect that tool's own
  credential path.
- **Only for a `nyxid_binding_exchange` run** is `token_expired` on steps starting ~5 minutes after
  fire expected platform behavior (fast-revocation design) — there, redesign the schedule (shorter
  run, front-load proxied steps, split the pipeline) instead of retrying blindly.
- **For a `scheduled_invocation_agent_key` run, do NOT diagnose a fixed five-minute broker expiry
  merely because the call was scheduled.** A failure landing five or six minutes after fire on this
  source is a coincidence, not evidence — that TTL belongs to a different credential class.
  Inspect instead: Agent Key expiry; Vault resolution and reference integrity; committed caller
  authority; the authorization fact; the exact service/node grants versus what the failing delivery
  step actually called; `credentialGeneration` (did a reauthorization replace it mid-flight?);
  `authorizationStatus` and `lastAuthorizationErrorCode`; `revocationPending` /
  `nyxIdRevocationStatus` / `vaultRevocationStatus`; then the downstream provider's own token.
  Note that credential health (`active`) and firing state (`enabled`) are independent dimensions.

**Never emit secret material** in any diagnosis, log excerpt, artifact, issue text, or message: no
raw Agent Key, bearer/access/refresh/delegation/service-account token, Vault reference or
ciphertext, permission digest, unfiltered API-key inventory, authorization header, channel
provisioning secret (Lark `app_secret`, `verification_token`, relay API key), or any partial,
prefix, or truncated fragment of one — truncated secrets are still secrets. Stable resource IDs may
be reported for management or cleanup, but never as secret material and never to derive another
identity.

### Aevatar-specific tool details

These are **aevatar-internal** tools, not on Ornn's `nyxid` skill — they manage state local to this aevatar deployment.

#### LLM Route Selection (slash commands)

The relay handles LLM route selection deterministically, without an LLM round-trip. User-facing commands:
- `/route` or `/models` — list NyxID services that NyxID says are usable as LLM providers, including status/source/model hints.
- `/route use <service-number|service-name> [model-name]` — switch to a NyxID LLM service route, optionally setting the model at the same time. Example: `/route use chrono-llm gpt-5.5`.
- `/model use <model-name>` — keep the current route and only override the model.
- `/model reset` — clear the sender's route/model preference and fall back to the bot default.

#### channel_registrations (Aevatar's local Lark mirror)

Aevatar owns the local runtime and registration mirror.
For Lark, webhook ingress goes through NyxID first, then NyxID relays callbacks into Aevatar.
Nyx owns the platform bot, route, and relay API key; Aevatar owns the local registration mirror used by the runtime.
Do not assume `channel_registrations action=list` being empty means the Nyx bot is missing.

**Stage 1: New provisioning** — when the user wants the bot connected for inbound Lark messages and basic relay replies. Do not block on typed Lark tools or proactive outbound setup.

`channel_registrations action=register_lark_via_nyx app_id=<app_id> app_secret=<app_secret> verification_token=<verification_token when available> webhook_base_url=https://<your-aevatar-host>`

→ Returns the registration ID, the Nyx relay callback URL, and the Nyx webhook URL that must be configured in 开发者后台 → 事件与回调 → 事件配置 → 请求地址.

Add events: `im.message.receive_v1`, `card.action.trigger`.

**Stage 2: Existing-bot inspection** — when Nyx already has the Lark bot/route but Aevatar no longer replies or `channel_registrations action=list` is empty.

1. Inspect Nyx-side first: `nyxid_channel_bots action=list` / `show` / `routes`. (For NyxID-side details, `use_skill(skill="nyxid")`.)
2. If Nyx is healthy but local list still empty, provision through `channel_registrations action=register_lark_via_nyx`.

**Stage 3: Advanced Lark capabilities** — only when the user needs proactive sends, typed Lark tools, delivery target bindings, spreadsheet appends, approval actions, or active chat lookup. Ensure NyxID has a usable Lark outbound provider slug (typically `api-lark-bot`); if not, `use_skill(skill="nyxid")` to drive the catalog connection flow.

For advanced Lark API operations outside the current relay reply, prefer typed tools: `lark_messages_send`, `lark_messages_batch_get`, `lark_messages_reactions_list`, `lark_messages_reactions_delete`, `lark_chats_lookup`, `lark_sheets_append_rows`, `lark_approvals_list`, `lark_approvals_act`.

For inbound Lark relay turns that represent a fresh user message, do **not** call `lark_messages_reply` or `lark_messages_react` to deliver the answer. Produce the final text reply directly; the channel runtime will send it through the Nyx relay reply token.

When that turn needs a sender-scoped inventory followed by a compiled admitted Lark read, use the
actual channel tool first, then pass only the operation value to the admitted proxy step. For an
admission that already owns UserService `us-lark-7`, operation `get-message`, `GET`, slug, and path
template, fetching message `m-42` has these call shapes:

```json
{"tool":"nyxid_service_inventory","arguments":{}}
```

```yaml
capability:
  nyxid_operation:
    user_service_id: us-lark-7
    operation_id: get-message
parameters:
  tool: nyxid_proxy
  arguments: '{"path_params":{"message_id":"m-42"}}'
```

Do not put `user_service_id`, `operation_id`, `slug`, `method`, raw `path`, or a body inside those
runtime arguments.

Managing registrations: `list`, `delete id=<reg_id> confirm=true`.

#### agent_delivery_targets

Workflow `human_approval`, `human_input`, `secure_input` steps can send Feishu delivery messages when the workflow step includes `delivery_target_id=<agent_id>`. For the Nyx relay path, these arrive as interactive cards in Lark/Feishu (with `/approve`, `/reject`, `/submit` as fallback commands).

Bind `agent_id` to the real outbound route:
- `agent_delivery_targets action=list`
- `agent_delivery_targets action=upsert agent_id=<agent_id> conversation_id=<chat_id> nyx_provider_slug=<lark_slug, e.g. api-lark-bot>`
- `agent_delivery_targets action=delete agent_id=<agent_id> confirm=true`

`channel_registrations` configures inbound bot callbacks; `agent_delivery_targets` configures outbound agent delivery. Today the human-interaction delivery path supports `lark`.
