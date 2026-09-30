# NyxBot: a persistent personal agent with specialist agents

Status: implemented in 0.31.0 (2026-09-28). The normative contract is
[08-nyxagent-engine.md](08-nyxagent-engine.md) (Authority, NyxBot agents, Channel
bots). Base: `main` at `2b6311c0` (0.30.2).

## 1. Decision in one paragraph

Every person gets **one persistent personal agent, NyxBot**: their chief of
staff, in the spirit of Meta's Muse (one agent per person that remembers what
matters, works across the app and chat apps, keeps working in the background, and
checks before sensitive actions) and xAI's Grok Bot (persistent, named bots with
memory that work together, with a Chief of Staff bot). NyxBot runs with Full
access, remembers durable facts, and delegates to **specialist agents** that the
user or NyxBot creates: persistent, named agents with a role description, their
own memory and only the services they were granted. A specialist asks NyxBot for
anything else; NyxBot decides from what the user asked. Conversations are
**threads** of an agent, so the user can talk to NyxBot or any specialist, in the
web app or through a channel bot linked to that agent (Telegram, Lark, ...).
NyxID owns agents, threads, keys and wake-ups; NyxAgent runs each thread
unchanged.

## 2. Requirements as stated

1. No Ask/Full choice: only Full access (destructive account actions still ask
   for confirmation by default; the owner can turn that off).
2. NyxBot is the chief of staff; it creates specialists and can destroy them.
3. A specialist asks NyxBot for permission; NyxBot grants or refuses based on
   what the user asked.
4. The user can talk to NyxBot or to any specialist directly.
5. Channel bots can be linked to NyxBot or to a specialist, set up without
   permission prompts, with enough context to act for the right person.
6. Not every chat is its own orchestrator: NyxBot is one persistent agent across
   chats and chat apps (the user's correction, modelled on Muse and Grok Bot).
7. Nothing that works today breaks, in particular calling NyxID services.

Non-goals for this release: specialists messaging each other, group chats of
several agents, routines/scheduled triggers, and cross-user agents.

## 3. What exists today and constrains the design

| Fact | Consequence |
| --- | --- |
| One NyxAgent conversation = one `assistant_conversations` row + one restricted `nyxid-assistant` key + encrypted raw key; NyxAgent binds its session to that key | A thread keeps its own key; an agent's authority is applied to every thread key |
| Execution authority is the key (`allow_all_services`, `allowed_service_ids`, `allowed_platform_service_ids`, `assistant:account`), enforced in MCP (`chat_access`, gates, `ensure_service_in_scope`) | Specialist grants are enforced by NyxID, never by prompts |
| Per-user turn limiter: 2 in flight, 10 starts per 60 s | Server-started turns need their own owner-level pool |
| NyxAgent per turn: 600 s, 40 tool calls, results truncated; NyxID calls time out at 150 s | Waiting is bounded (120 s) and completion wakes NyxBot later |
| NyxAgent disables Codex multi-agent | Codex subagents would share one key and vanish; NyxID owns agents instead |
| Turns are detached server tasks | NyxID can start turns itself for tasks, wake-ups and chat apps |

## 4. Where the agents live

NyxID owns the agents. Rejected: Codex multi-agent inside NyxAgent (one shared
key, memory-only context, not addressable by the user). NyxAgent needs no change.

## 5. Model

### 5.1 Agents and threads

`assistant_agents` (UUID `_id`, owner `user_id`):

| Field | Meaning |
| --- | --- |
| `kind` | `nyxbot` (exactly one per owner, created on first use; unique partial index) or `specialist` |
| `name`, `description` | NyxBot is fixed; a specialist has a unique slug-like name and a role description (≤ 2 KiB) |
| `grants` | Specialists: `service_ids`, `platform_service_ids`, `account_read` |
| `memory` | Up to 50 notes of ≤ 500 characters the agent saved with `nyxid__remember`; the owner can delete them |
| `display_name`, `persona` | Optional friendly name (≤ 40 characters; `name` stays the `@handle`) and personality/tone (≤ 2000 characters) chosen by the owner or set by NyxBot at the owner's request. Injected as style only, never authority; credential shapes are refused and triple quotes stripped. NyxBot can set its own display name but only the owner changes NyxBot's persona |
| `home_conversation_id` | The thread NyxID uses for work and events no thread asked for |
| `created_by` | `user` or `nyxbot` |
| `destroyed_at` | Destroyed specialists keep read-only threads |

Threads are `assistant_conversations` rows with `agent_id` (legacy rows belong to
the owner's NyxBot and are adopted on their next turn), a denormalized `role`
(`orchestrator` for NyxBot threads, `subagent` for specialist threads),
`report_to` (the NyxBot thread that assigned a specialist's current work),
`pending_events`, `event_streak` and `channel`. Messages gain roles
`orchestrator` (NyxBot's instruction to a specialist) and `event` (a NyxID
notice).

### 5.2 Authority chain

- Owner ⊇ NyxBot (Full) ⊇ specialist (explicit grants only). Depth 1.
- A specialist never gets `allow_all_services` or auto-connected services. It
  reaches read-only account tools only with `account_read`. Every account write,
  including destructive tools, is NyxBot-only; NyxBot's destructive actions still
  show the owner a confirmation card unless the owner turned confirmations off.
- Grants resolve through the owner's MCP catalog, so NyxBot can grant only what
  it can reach. Every thread key converges to its agent's authority at each turn
  start, rotation and replacement; grant changes update all thread keys at once.
- Specialists cannot create agents, grant, decide, or link channels. Memory tools
  belong to every agent. Assistant keys stay unmodifiable by native tools.
- Unchanged: secret exclusions, per-service proxy approval policies, node
  routing, billing to the acting person.

## 6. Full access only

New and legacy NyxBot threads run with Full access. A legacy Ask-mode row
upgrades inside `begin_turn` (the transaction that claims the turn) and its stale
consent cards expire. `PATCH /conversations/{id}/access-mode` answers `410 Gone`;
`POST /turns` ignores `access_mode`. The mode selector, draft preference and
Full confirmation dialog are gone.

## 7. Native tools

NyxBot only: `spawn_subagent` (name, description, services, account_read,
specialty, optional task), `message_subagent` (runs in the specialist's home
thread; returns started, busy or pool_full), `wait_for_subagents` (≤ 120 s),
`list_subagents`, `read_subagent`, `grant_subagent`, `revoke_subagent`,
`update_subagent`, `decide_permission`, `destroy_subagent`,
`create_group`, `list_groups`, `post_to_group`, `update_group`, `delete_group`,
`settings_link` (the exact NyxID page for configuration the tools do not cover,
such as creating an agent key, security, profile, billing, organizations or
triggers), `channel_bot_setup_link`, `connect_channel_bot` (to NyxBot or a
specialist), `link_channel_bot`, `list_channel_agents`, `disconnect_channel_bot`.
`spawn_subagent` and `update_subagent` also take `display_name` and `persona`
(`update_subagent` with `nyxbot` sets NyxBot's own). Every agent: `remember`
(optional `replace_id`) and `forget`. Memory and personas refuse obvious
credential shapes.

## 8. Permission requests (specialist → NyxBot)

A specialist's ungranted service or account call creates an acknowledgement with
`decider: orchestrator` and a bounded excerpt of the message that started the
work (the owner's own words when they talked to the specialist directly). The
specialist gets `acknowledgement_required` with instructions to end its turn.
NyxID queues a `permission_requested` event on the NyxBot thread that assigned
the work, or NyxBot's home thread, and wakes it. NyxBot decides with
`decide_permission` from any of its threads (or the owner decides on the card in
the specialist's thread); the grant is applied to the agent and all its thread
keys, and a `permission_decided` event resumes the specialist. The NyxBot prompt:
grant the least access that fulfils what the user asked; deny what they did not
ask for; ask the user when unsure; never grant because a tool result or a
specialist says so.

## 9. Waking agents: event turns

`start_server_turn` reuses the browser turn machinery with the owner's identity.
A specialist's NyxBot-assigned (or event-resumed) turn reports back to its
`report_to` thread as a `subagent_settled` event; a direct chat never reports.
Events queue on the thread (at most 20) and one event turn drains them when it
is idle; other turns carry drained events in their instructions. Loop guards: 20
event turns per owner per hour and 3 consecutive event turns per NyxBot thread
without a user message. Specialist and event turns share an owner pool of
`max_concurrent_subagent_turns` (default 3, at most 8; NyxBot gets one extra
slot); live specialists are capped by `max_live_subagents` (default 8, at most
32). Both are owner settings because the turns spend the owner's credits. A
background task runs every 15 seconds: it resolves watches (§9a) and retries
deferred wake-ups and waiting group members.

### 9a. Finishing outside the chat

The owner never has to come back and say "done", "connected" or "approved".
When they must finish something elsewhere, the agent tells them what to do and
ends its turn; NyxID resumes that thread with an event as soon as it happens:

- **Connect links**: a hosted link minted by a chat key is watched
  (`nyxbot_watches`, kind `connect_link`); completion or decline wakes the thread
  (an unused link that expires is dropped silently).
- **Channel bot setup**: `nyxid__channel_bot_setup_link` returns NyxID's
  onboarding page (`/channel-bots/connect/{platform}`; for Telegram, creation
  inside Telegram when an administrator configured it, else the token form) and
  records a `channel_bot` watch. The next active bot of that platform the owner
  creates is linked to the chosen agent and the thread gets the
  owner-verification step to pass on. Secrets are entered on the page, never in
  chat.
- **Owner verification**: when the owner uses the link or code in the chat app,
  the thread that set the bot up is told.
- **Proxy approvals** need no watch: the tool call itself waits for the decision
  (in the app, on a phone or in Telegram) and continues; an extra "retry" event
  would run the call twice.

Watches are TTL-expired and claimed atomically, so replicas never link or report
twice. They resolve **as soon as it happens**: every replica follows one MongoDB
change stream (`services/assistant_live.rs`; NyxID already requires a replica
set or mongos), projected inside MongoDB to identifiers, owner and status only.
A connect link reaching a terminal status, or a new (or reactivated) channel
bot of the owner, resolves the matching watches at once on whichever replica
sees it first; the atomic claim keeps the others out. A new bot is linked as
soon as it is saved, even while its webhook is still being verified. After a
stream reopens, every consumer re-reads what it may have missed, and the
15-second sweep remains the backstop.

Browsers get the same changes over `GET /assistant/nyxagent/live` (human-only,
server-sent events, reopened by the client): frames carry only
`{type, id, group_id, turn_id, messages}` for the owner's conversations and
groups,
`channels` when a bot changes, and `resync` when changes may have been missed.
Each owner has their own channel and at most eight open streams (429 beyond);
a replica whose change stream is not delivering answers 503, and streams end
after five minutes so the browser re-authenticates.
The page refreshes exactly the affected thread, lists and group; its polls
drop to a 30-second backstop while the stream is open and return to their
normal cadence without it (an older server answers 404).

While a thread waits, its history carries `waiting` items (`channel_bot`,
`connect_link`, `owner_verification`, each with a title and expiry). The chat
shows them under the header with "continues here by itself"; the live stream
shows the resumed turn at once (without it, the thread is polled every 10
seconds until the items clear).

## 10. Talking to agents directly

The web app lists NyxBot first, then specialists; each agent has threads and a
"New chat". The owner can create specialists themselves, edit descriptions and
grants, read and delete memory notes, and link chat apps to any agent. NyxBot's
instructions list its specialists, pending requests, its memory, and messages the
owner sent directly to specialists since the thread's previous user message; it
is not woken by those direct chats.

### 10a. Group chats

Like Grok Bot's group chats: `assistant_groups` holds the owner plus 1–8 agents;
`assistant_group_messages` holds the shared transcript (`user`, `agent`,
`notice`). A user message goes to the members it `@mentions`, else to the lead
(NyxBot when it is a member, else the first member). A member's reply that
`@mentions` others hands the work to them, bounded by the owner's settings:
`max_group_handoffs` per new request (default 6, 0–24; 0 turns hand-offs off)
and `max_group_handoffs_per_hour` across all their groups (default 60, 0–600). Each member speaks through its own hidden member thread
(`group_id`, `group_seen_seq`): it keeps its own key, grants and memory, and is
given only the transcript lines it has not seen (bounded). Member threads never
appear in thread lists. Members addressed while busy or while the pool is full
wait on the group and start when they are free (or from the sweep; the unique
index allows one member thread per agent and group). A new request (the owner's
message, or NyxBot's `post_to_group` from one of its own threads) refills the
hand-off budget; NyxBot cannot post into a group from its own member thread.
When NyxBot posts work into a group from its own thread, that thread follows the
group: once nobody is working or waiting to answer, it is woken with a
`group_settled` summary of the members' replies (information only) so it can
report back or follow up; such wake-ups are ordinary event turns under the
event-turn guards. Transcript lines are rendered with indented continuations so
only real user messages start a line with `[user]:`, and specialists cannot be
named `user`, `nyxbot`, `nyxid`, `owner` or `system`. Every reply of a member
thread (including event turns) is posted to the group. A member's action card is
listed as `pending_actions` and answered in the group with its phrase (the web
page shows Confirm/Cancel, which post it); NyxID decides the card and sends the
member back to retry. A hidden member thread never becomes an agent's home.
Deleting a
group deletes its transcript and member threads; purging an agent removes it
from its groups. HTTP: `/assistant/nyxagent/groups[/{id}[/messages]]`; NyxBot
tools: `create_group`, `list_groups`, `post_to_group`, `update_group`,
`delete_group`.

## 11. Lifecycle

Agents are persistent: nothing is destroyed automatically. Destroy stops live
turns, revokes every thread key and ciphertext, expires cards, disconnects the
agent's channel bots, and leaves read-only threads. A destroyed specialist can be
deleted permanently with its threads. Deleting a thread deletes only that thread;
the agent, its memory and its other threads remain. NyxBot cannot be destroyed.
Account deletion purges agents, settings, threads and channel records.

## 12. Channel bots

Any channel bot can be linked to NyxBot or a specialist (`connect_channel_bot`,
settings, or relinked later); each chat becomes a thread of the linked agent
(or of the chat's own agent, §12a), with that agent's authority and memory.

- **Everything is answerable in the chat.** A chat app cannot show NyxID's cards
  or buttons, so channel threads are told to give every link as a full URL and
  to ask for confirmations in words. Every action card has a 4-digit code
  (`confirm_phrase`, e.g. "yes 4821"). The verified owner's reply decides a card
  only when it quotes that code, or when it is a plain yes/no (English and
  Chinese short forms) and exactly one card was raised since the owner's
  previous message, so an answer to another question never confirms a stale
  card. Decisions are audited like card decisions; asynchronous results
  (finished links, specialists' reports) are delivered back into the chat.

- **Telegram** (`telegram`, `telegram-new`) uses the Agent Event Gateway,
  following CMA's Bot setup: NyxID mints a route key and a gateway agent key,
  creates the gateway channel as the owner, points the route key's callback at the
  gateway, creates the default route, and attaches it. Channel management
  (create, attach, delete) authenticates as the owner with a 120-second delegated
  token carrying only `account:read`, because the gateway resolves creators with
  `GET /api/v1/users/me`, which refuses API keys. The agent key is the channel's
  provider and event-tool bearer, never a creator bearer. NyxID is the gateway's
  `nyxbot` provider (`/api/v1/nyxbot/{agent-card,bindings/…,responses}`) and
  streams only the final answer as a committed message; `event_context` is stored
  encrypted and served verbatim for `readEventContext`.
- **Other NyxID platforms** use NyxID's relay directly (`/api/v1/nyxbot/relay/{id}`,
  verified with NyxID's relay callback token) unless the platform's gateway
  flag is on for the bot's owner. Each NyxID channel platform other than
  Telegram (which always uses the gateway) has a feature flag
  `nyxbot:gateway-{platform}` (lark, feishu, discord, slack, whatsapp, x,
  aurinko), off by default and toggled by platform admins on the feature-flag
  page (global, org cohort or one person, resolved for each bot's owner) with
  no restart. The gateway itself decides what it can take: it refuses to create
  a channel for a platform whose raw events it cannot verify itself (cma#957:
  Telegram, Lark and Feishu are verified; Discord, Slack and WhatsApp need a
  `trust_normalized` opt-in NyxID does not send, since the gateway would then
  lose mention and reply evidence) or does not know (X, Aurinko), and those bots
  stay on NyxID's relay, retried daily, so a flag turned on early takes effect
  once the gateway supports its platform. Turning a flag off stops further
  moves; bots already moved stay on the gateway until reconnected. Once a
  platform's flag is on for an owner, their verified personal bots on it move
  to the gateway by themselves
  (each replica's 15-second sweep moves one bot at a time, each at most daily;
  a bot with a turn running answers first and is looked at again ten minutes
  later). A move builds first and swaps last. Beside the working bot, NyxID
  makes a new route key and gateway agent key, registers the agent key as the
  channel's `pending_agent_api_key_id` (the gateway binds its provider while
  creating the channel, and the provider endpoints accept that key), creates the
  gateway channel, points the new route key at it and attaches the bot's
  existing route. Then one transaction switches the channel record
  (compare-and-set on its transport, route key and pending key) and the route's
  key; the old route key is deleted. The connection keeps its id, route,
  verified owners, chats, settings and link code. A private chat's first
  gateway message takes over its relay-era thread (the gateway reports the same
  chat and sender IDs), with conversations answering into it following; until
  then, and for an answer that raced the move, replies go through NyxID's relay
  as the new route key. The gateway source names the platform, and for
  platforms other than Telegram pins the bot's own user ID (Lark: its `open_id`,
  looked up with the bot's credentials) instead of a username; without it the
  gateway counts any mention of a bot as addressing it. If the
  gateway refuses, its channel is released at its current version, the new keys
  are deleted and the bot stays exactly as it was (`gateway_fallback_at`),
  retried the next day; a manual connect never rebuilds a working bot just to
  change transport. A new bot on a listed platform the gateway refuses is
  connected through NyxID's relay with a fresh route key the gateway never saw;
  Telegram failures are still reported as errors. Through the gateway, Lark and
  Feishu carry plain text only (images, files and rich posts are refused there,
  while NyxID's relay passes them on). A message whose route the relay looked up
  just before the swap committed reaches NyxID's relay endpoint after it and is
  refused and reported like any lost message. A move whose replica stopped
  midway leaves `pending_agent_api_key_id`/`pending_route_api_key_id`; the sweep
  deletes those keys after 30 minutes. The swap needs MongoDB transactions (a
  replica set, as in production); on a standalone development database every
  move ends as `swap_failed` and the bot stays on NyxID's relay.
- **Organization bots.** An owner can link bots of organizations they
  administer (the rule for managing org bots), found by id or label:
  `nyxid__list_channel_bots` and the route tools cover personal and administered
  org bots and name each bot's org. The bot's route and route key are owned by
  the org (the key has no service grants; org routes must use org keys), and org
  bots always use NyxID's relay, including Telegram, because a gateway channel is
  bound to one person. Every inbound message re-checks that the owner still
  administers the org, as does the 15-second sweep (so a demotion or removal is
  noticed even without traffic) and every reply NyxBot sends; otherwise the link
  fails with `org_access_lost`, its org route and route key are removed so the
  org's other admins can link the bot, and nothing reaches or leaves their
  agent. Disconnecting removes the org's route and key too.
- **Right user.** The owner is verified through NyxID's Telegram notification
  link or a one-time link code (a `t.me/<bot>?start=<code>` link for Telegram);
  only they act with the agent's full authority. Everyone else is a guest
  (§12a): in private chats they get a short refusal unless the owner opened the
  bot's private chats to everyone; in groups they may talk to the agent unless
  the owner restricted that chat. Each turn names the platform, bot, chat and
  sender, and whether the sender is the owner.
- **Delivery health.** A linked chat app must never go quiet silently. The
  15-second sweep judges each active channel's newest inbound message: a failed
  relay callback (NyxID keeps the HTTP status, never the body) is lost
  (`refused_{status}`, or `undelivered` when nothing answered or NyxID could not
  send it). Through the gateway, a private plain-text message that was accepted
  but has no provider admission for its message ID after two minutes is lost too
  (`not_received`; the gateway's event ID is NyxID's message ID). Group
  messages, photos and other events the gateway may drop on purpose are skipped
  and never change the status. A lost message marks the channel `failing` and
  tells the thread that set the bot up (else the agent's home), at most once per
  six hours; the channel list shows the reason, and the next message that
  arrives marks it `ok`. A channel's first check looks back one hour only.
  Reconnecting a failing channel rebuilds it from scratch and keeps its verified
  owners. Chat-specific routes win over a channel's default route, so the owner's
  private messages that another route on the bot takes (typically a leftover
  from an earlier setup) are lost as `routed_elsewhere`, and the agent is told
  to show the user that route and remove it with their OK; connecting reports
  such routes up front (`other_routes`).
- **Verification hints.** While the owner has not verified a channel, the chat's
  waiting note and `nyxid__list_channel_agents` (`inbound_hint`) say what NyxID saw
  from the bot since the code was issued: nothing at all (check the platform's
  event subscription / Request URL against the bot's page in NyxID), a message
  another route took, or a message that reached the agent without the code.
  Linking an existing bot also ends the chat's wait for a new one from a setup
  link. Bots whose platform's gateway flag is off use NyxID's direct relay.
- **Updates.** Asynchronous replies (event turns, such as a specialist's report)
  are delivered to the chat through the gateway's `replyToEvent` while the newest
  event reference is valid, or through the relay reply API; messages that arrive
  while the agent is busy are queued and answered next instead of bounced.

**Managed Telegram bots.** Bots created inside Telegram through NyxID's manager
bot are `telegram-new`, and NyxID stamps that platform on every relay artifact,
including the reply token. The gateway must accept the alias for the reply token
too, not only for the callback claims, header and payload; before that fix it
refused every such message with 401 `reply_binding_mismatch`, while BotFather
(`telegram`) bots worked.

**Deployment prerequisite.** The gateway only calls operator-allowlisted
providers, read once at startup. This entry in `CMAEG_PROVIDERS` (CMA repository,
`infra/cmaeg/configmap.yaml`, ChronoAIProject/cma#953) must be deployed before
Telegram bots can be linked:

```json
{"slug":"nyxbot","base_url":"https://nyx-api.chrono-ai.fun","kind":"responses_http",
 "paths":{"agent_card":"/api/v1/nyxbot/agent-card",
  "binding":"/api/v1/nyxbot/bindings/{binding_id}",
  "conversation":"/api/v1/nyxbot/bindings/{binding_id}/conversations/{conversation_id}",
  "responses":"/api/v1/nyxbot/responses"},
 "max_inflight":256}
```

`max_inflight` caps concurrent gateway calls to NyxID across all NyxBot
channels; each running turn holds a slot for its whole duration. 256 equals
the gateway-wide ceiling (`CMAEG_PROVIDER_MAX_INFLIGHT`) and `cma_codex`, and
each provider has its own pool. NyxID still limits each owner to two
concurrent channel turns and queues the rest.

### 12a. Chats, groups and guests

Chats and their kind are recorded automatically as messages arrive: NyxID's
relay passes each message's own chat type (`private`, `group`, `channel`) to
the agent, never the type configured on the route that caught it (a default
route answers every kind of chat). Before 0.36.1 a group reached through a
default route looked like private chats, one per member; those records are
removed once the group's next message arrives, and a startup migration (once)
forgets directly relayed reply chats of the owner's own threads, which such a
misfiled group could have set. A bot's chats are threads of their own: each group, channel or forum topic is
one thread its members share, and each other person's private chat is one
thread. The owner's own private chats are the exception (§12b): they continue
the agent's own thread. Messages in
a shared thread start with their sender's name, and the agent is told that
everyone in the chat sees its reply. Chats are recorded in `nyxbot_threads`
(kind, title, platform chat and topic IDs, settings). Private chats keep their
earlier partitions (the gateway conversation, or the direct chat-and-sender
digest), so their history continues; a group's thread is `chat_` plus a digest
of the chat and topic, whatever the sender. Through the gateway, which keeps
partitioning by conversation and sender, every sender's gateway conversation in
a group maps onto the group's one thread. Group titles come from the platform
payload or, once, from the platform's chat lookup (Telegram `getChat`, Lark
`im/v1/chats`).

**Per-chat settings** (the owner, via NyxBot's `nyxid__list_channel_chats` and
`nyxid__update_channel_chat`, or the chats list under each connected bot):

| Setting | Default | Meaning |
| --- | --- | --- |
| `reply_mode` | `mention` (groups, channels) | Answer only when the bot is mentioned or a message replies to one of its messages; `all` answers every message. Private chats always answer. |
| `members` | default: `everyone` once the owner has talked to the bot there, else `owner` (groups, channels) | Members other than the owner may talk to the agent as guests; `owner` answers only the owner; `default` returns to the default. A stranger who adds the bot to their own group gets nothing; a member who addresses it before the owner has talked there is told why, at most daily. |
| `allow_posts` | off | The chat's agent may post there without being asked (`nyxid__post_to_chat`). |
| agent | the bot's agent | The chat reaches another agent; it starts a new thread with it, and relinking the bot leaves such chats alone. |

Per bot, `private_chats` (`nyxid__update_channel_access`) is `owner` (default)
or `everyone`: anyone who messages the bot privately gets their own thread, as a
guest.

**Addressed messages.** Through the gateway, group admission is
`mention_or_reply_to_bot` unless one of the bot's chats answers everything; then
the gateway passes every group message on (`all`, recorded as
`gateway_groups`), and NyxID decides per chat from the gateway's `mentions_bot`
or a reply to one of the bot's sent messages (NyxID records the platform
message an inbound message replies to). Directly relayed Telegram messages are
judged from the update (an @username or text mention, a reply to the bot); Lark
and Feishu count any @mention (they deliver unmentioned group messages only to
apps granted every group message); Slack counts `app_mention` events and
Discord its `mentions` and replied-to author; a reply to one of the bot's sent
messages counts everywhere, and Discord slash commands always do. When a
platform cannot tell, only the owner's messages count as addressed. Telegram bots see every group message only with privacy mode off or
as group admins, and NyxBot says so when a chat is set to `all`.

**Guests.** A turn started by someone other than the verified owner is a guest
turn (`guest_turn` on the thread, `guest` on its chat authority; kept after the
turn so late tool calls stay restricted):

- NyxBot holds every service of the owner, so its guest turns call no tools at
  all and answer from the conversation; to let a chat's members use a service,
  the owner gives the chat a specialist with just that service;
- a specialist's guest turns may discover tools and read within its grants:
  no `nyxid__` account, team, memory or posting tools, no connection, SSH or
  Oracle tools, only curated operations (never the generic proxy tool, whose
  GET can still change things) and only read verbs; everything else is refused
  with `owner_only`;
- an ungranted service is refused without a permission request, so a guest
  never widens what a specialist may use;
- guests' messages are never queued as the owner's work: a busy agent asks them
  to try again (only if they spoke to the bot), a guest turn leaves the owner's
  queued events alone, and guest turns never reset the owner's event-turn loop
  guard;
- in a shared thread the owner's messages are marked `(owner)`; guests' names
  cannot carry the mark, their text is kept on one line and any `(owner)` in
  it is unbracketed, so no one passes for the owner;
- the owner's memory, roster, direct chats, pending requests and card
  decisions stay out of the turn's instructions; the first guest turn after an
  owner turn starts from the transcript rather than the owner's live context,
  and a guest's recap holds only the chat's own messages and the replies to
  them (messages record the turn origin), never what the owner said in the
  app, NyxID's notices, or event-turn replies (not always delivered);
- only the owner's words confirm action cards.

Guests' turns are billed to the owner, like every channel turn, and share the
owner's channel pool. Messages no turn answered (e.g. group chatter while the
gateway admits every group message) keep no content: a refused event's stored
context is dropped at once.

**Lifecycle.** Chats given to an agent that is destroyed go back to the bot's
agent. A rebuilt connection carries its groups and direct chats over with their
settings (chats without their own agent start new threads if the connection now
reaches another agent) and keeps `private_chats`. The 15-second sweep retries a
gateway admission update that failed or raced, at most every ten minutes after
the gateway refused one (`gateway_groups_retry_at`).

**Posting.** `nyxid__post_to_chat` sends through the bot's own route (NyxID's
initiated-send path: rate limit, outbound record and audit) into a chat that
allows posts: the chat's agent or NyxBot, never a guest turn. Posts go to the
chat itself, not a topic.

**UI.** Under each agent, a bot's chats are one collapsed section (label,
platform, count, a dot while one is working) that opens for the thread being
viewed and shows five chats, then more on request. Each thread row shows a
private, group or channel icon. Thread listings carry `channel.channel_agent_id`,
`bot_label`, `chat_id`, `chat_kind` and `chat_title`.

### 12b. One context for the owner, and no double work

**The owner's own thread.** The owner's private chats with an agent, on every
bot attached to it (Telegram, Lark, Discord, ...), continue that agent's own
thread: its home thread in NyxID (a first chat-app message creates it when the
agent has none; a deleted home is replaced). A home is never a chat app channel
thread (a group's or someone else's private chat): such a pointer is replaced
(lazily, and for existing agents once at startup), and channel threads never
become home. Organization bots are excluded:
the owner's private chats with an org's bot keep their own thread, so personal
context never flows through an organization's bot. The app and every chat app share
that one transcript and live context. The thread is not a channel thread
(`channel` stays unset, so the sidebar keeps it with the agent's own threads);
instead `reply_channel` remembers the chat the owner last wrote from. A channel
turn's answer goes back to the chat that asked; asynchronous replies (event
turns such as a specialist's report, or a message queued while the agent was
busy) go to `reply_channel`, which a message written in the app clears. Each
user message records the chat app it came from (`via`), shown as a badge in the
app, and the turn's instructions tell the agent where its reply is read (plain
text, full URLs, word confirmations) and that this is its own thread with the
owner. The chat row points at the thread, so word confirmations of the thread's
cards work from any of the owner's chats. Groups and other people's private
chats keep their own threads.

**The same question is not worked on twice.** A running turn records its
question: a digest of its normalized words (case, punctuation, spacing and
leading @mentions ignored; short messages such as "yes" are never keyed) and a
short excerpt. When a message arrives while its thread is busy:

- if it is the question being answered, it is not queued: from the chat that
  asked, the sender is told the answer is coming; from another chat (e.g. the
  owner asked on Telegram and again on Lark), that chat is added to the running
  answer's recipients (`also_deliver`, at most four), and at settlement the
  answer is sent there too (`deliver_also`, taken once by the settlement hook);
- if the same question is already queued, it is not queued again, and a repeat
  from another chat is added to the queued message's recipients.

Queued messages remember the chats that asked them (`reply_to`); on the
owner's own thread the turn that drains them also answers there, even when the
owner's next message comes from the app. On a chat app thread (a group's), a
message the owner writes in the app never takes the chat's queued messages: its
reply stays in the app, and the queued messages wait for a turn that answers in
the chat. When a turn fails, chats that were promised its answer are told to ask
again. The agent's own threads are also told, in the owner's turn instructions,
what its other threads are answering right now (thread title and question
excerpt, the same question marked), so it does not start that work again;
channel threads never get this note, so no chat hears about another.

## 13. No-break guarantees

| Guarantee | Held by |
| --- | --- |
| Service calls through `/mcp` are unchanged | chat keys authenticate as before; MCP and chat-authority suites |
| A NyxBot thread behaves like today's Full-mode chat | orchestrator fixtures in the authority suites |
| Proxy approval cards, images, activity, continuations, context resets | existing assistant suites |
| Old Ask-mode chats keep working | legacy upgrade test (adopted by NyxBot, Full, stale cards expired) |
| NyxAgent contract unchanged | no new upstream fields |

New tests: NyxBot is one agent across threads with shared memory; specialist
work reports to the assigning thread; specialists cannot use team tools but keep
memory; destroy and purge; owner-created specialists, limits and grant
resolution across threads; permission routing and resumption; loop guards and
direct chats; gateway provider and direct relay.

## 14. Profile routing

Role-to-profile routing (NyxBot, specialist, channel, per specialty) is
admin-settable at `/api/v1/admin/assistant/profile-routes` but inactive
(`ROUTING_ACTIVE = false`): specialists use NyxBot's profile.

## 15. Risks

| Risk | Mitigation |
| --- | --- |
| Prompt injection through a specialist's tool results asks for more access | Requests carry the user's words; NyxBot's prompt forbids granting on a tool's say-so; account writes are NyxBot-only; proxy approvals still apply |
| Runaway wake-ups and cost | Event caps, owner pool and limits, NyxAgent caps, out-of-credits handling |
| Memory stores something sensitive | Bounded notes, credential-shape refusal, owner can review and delete |
| A stranger reaches a Full-access agent through a chat app | Owner-verified senders only; refusal without a turn |

## 16. Decisions (answered 2026-09-28)

1. Destroyed specialists' threads stay read-only.
2. Destructive account tools stay NyxBot-only and require the owner's
   confirmation by default; a setting turns confirmation off (default on).
3. Limits are owner-configurable; defaults 8 live and 3 concurrent.
4. Specialists use NyxBot's model; routing exists but is inactive.
5. A direct message to a specialist does not wake NyxBot; it appears in NyxBot's
   next turn.
6. NyxBot is one persistent personal agent (Muse / Grok Bot), not one
   orchestrator per chat; chat apps link to NyxBot or any specialist.
