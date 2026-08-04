---
name: aevatar-feasibility-advisor
description: Use before building when a user asks whether Aevatar can achieve a goal, what prerequisites it has, or why it is unavailable. Triggers include bots and third-party APIs, inbound channels, external HTTP triggers, schedules, service exposure, Agent Profiles and tool ceilings, and bounded managed or private-host codex_exec work. It distinguishes outbound connectors from inbound channels and separates not connected, host-gated, not deployed, and genuinely unsupported outcomes. It chooses managed_sandbox versus private_ssh without promising repository, model, credential, runtime, or deployment capabilities the caller does not control, then routes feasible work to the owning Aevatar skill.
version: "1.6"
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
    - agent-profile
    - codex-exec
---

# Aevatar feasibility advisor

Before you (or the user) commit to building something on Aevatar, answer three questions
**honestly**: *Is it possible? What must be in place first? If not, why — and what's the
alternative?* This skill exists so you negotiate scope up front instead of discovering a
hard blocker halfway through. It only **advises** — once a plan is feasible, hand off to
`aevatar-workflow-authoring` → `aevatar-team-builder` → `aevatar-service-publisher` →
`aevatar-scheduler`, or to `aevatar-agent-profile-management` for the Agent Profile surface
(see `aevatar-platform-map`). For `codex_exec`, choose the target here, then hand setup or
repair to `aevatar-codex-exec-node-setup`, require the public
`aevatar-codex-exec-workflow-sample` proof, and only then author the real workflow.

## Three different reasons a thing can be unavailable — never blur them

They have different owners and different fixes. Collapsing them into "it's host-gated" is the most
common wrong answer here.

| Reason | What it looks like | Who fixes it |
|---|---|---|
| **Not connected** | The connector is in `/api/v1/catalog` but missing from `/api/v1/services` | **The user**, self-serve |
| **Host-gated** | The capability is deployed and works, but a host policy governs one aspect (NyxID external exposure; Agent Profile *rollout admission*) | **The host** — not you, not the user |
| **Not deployed** | The capability's whole API surface is absent from the running build | **The host**, by deploying a build that exposes it |

"Not deployed" is the one people get wrong most: the feature exists in the codebase and in design
docs, so it *sounds* available. **Probe the live surface, never a branch or a memory** —
`GET /api/openapi.json` and check whether the complete route family is advertised.

Agent Profile management is an **owner-managed** capability — an authenticated user manages their
own profile; it is not host-only configuration. It remains deployment-gated, so probe the live
`GET /api/openapi.json` and require the complete `agent-profiles` route family before mutation.
Do not preserve a historical claim that production lacks or exposes it; the live OpenAPI is the
only answer for the current deployment.

## Choose the `codex_exec` target before workflow authoring

`codex_exec` is an in-session workflow capability with two separate infrastructure targets. It is
not an HTTP endpoint or a Studio lifecycle stage. Choose exactly one target from the requirement;
never mix their fields.

| Requirement | Feasibility verdict | Exact boundary and prerequisite |
|---|---|---|
| Bounded one-shot Codex work that can start from a clean empty Git repository | ✅ Use `managed_sandbox` | Requires the exact native NyxID user to be admitted by the internal rollout and to directly own one active usable `chrono-sandbox` UserService plus one usable `chrono-llm-public` route. The request must use `workspace.kind=empty_git`; timeout defaults to and is capped at 180 seconds. |
| Codex work that requires an existing private repository, host files, or host Codex login/configuration | ✅ Use `private_ssh` | Requires a user-owned hardened NyxID SSH service, fixed principal and host workspace, and a working host Codex CLI/login. Send no `workspace`; the host wrapper owns the repository path. Timeout defaults to 30 seconds and is capped at 300 seconds. |
| Persistent managed repository/session, caller-selected repository/path, image, model, provider, credential, shell, or work beyond the bounded synchronous contract | ❌ Not provided by `managed_sandbox` | Narrow or split the work so it can start from `empty_git`, or use a separately authorized private-host/long-running workflow. Do not promise managed repository persistence. |

Both targets accept only a nonblank prompt of at most 6000 UTF-8 bytes plus their typed target
fields and optional timeout. A managed caller cannot select a repository, path, image, model,
provider, credential, Codex profile, command, approval policy, or sandbox flag. A private caller
supplies only `target.private_ssh.service` and `principal`; that service is a UserService slug or
UUID, not a node ID.

Configuration, health, service inventory, a direct chrono call, or a direct SSH command is only a
prerequisite. Readiness requires mounting the public `aevatar-codex-exec-workflow-sample` and
getting exact `CODEX_EXEC_READY` through Aevatar:

- Managed proof: `status=succeeded`, `target=managed_sandbox`, trimmed
  `output=CODEX_EXEC_READY`, `exit_code=0`, and a non-empty sanitized `diagnostic_id`.
- Private proof: the original NyxID SSH result has `exit_code=0`, `timed_out=false`, and trimmed
  stdout exactly `CODEX_EXEC_READY`.

After choosing `managed_sandbox`, explicitly call authenticated `POST /api/managed-codex/credential` to provision or reconcile, then read `GET /api/managed-codex/credential`. Proceed only when `execution_ready=true` and `execution_readiness_reason=ready`; `status=active` alone is insufficient. Normal `codex_exec` is credential-read-only and never provisions, reconciles, rotates, repairs, or retries after a credential failure. The caller never supplies or sees the key. Then use `aevatar-codex-exec-workflow-sample` for the mandatory proof and load `aevatar-workflow-authoring` only after it succeeds. Preserve the deadline chain: chrono execution 180s < Aevatar managed request 300s < NyxID/ingress at least 315s < NyxID client 330s < workflow canary at least 360s. Landlock, Bubblewrap, sandbox-side Credential Vault substitution, and a credential proxy are not repair paths for the managed runtime.

## The one premise: NyxID is the universal gateway

Aevatar holds **no third-party credentials and talks to no external service directly.**
Every external capability is brokered by **NyxID**. That single fact drives every feasibility answer below. Keep inbound channels separate from three outbound invocation modes:

| Surface | What it gives you | How it's used | Supported set |
|---|---|---|---|
| **Connector** (outbound) | Your workflow/agent **calls** a third-party API (read data, post, act) | Raw `nyxid_proxy` requires exact `service_id + slug + path`; current-turn dynamic `nyxid_service_operation__*` tools are retired; a compiled workflow uses exact `capability.nyxid_operation` or typed `capability.nyxid_request` admission | Anything in the live NyxID catalog/inventory — broad |
| **Channel** (inbound) | A third-party chat platform **delivers user messages to your agent**, which replies **in that platform** | An Aevatar **channel module** + NyxID relay webhook | **Narrow** — only platforms with a built module |

> **The trap:** "I want a Twitter bot." A Twitter *connector* (`api-twitter`) exists, so your
> agent **can post/read tweets** (outbound). But there is **no Twitter inbound channel
> module**, so you **cannot** have an agent that auto-replies to mentions/DMs the way a Lark
> or Telegram bot does. Same word, two very different feasibility answers. Always separate
> "call the API" from "be a bot on that platform."

## Step 0 — Inspect what is actually connectable (don't guess)

Use the authenticated NyxID CLI; never read its stored access token or build an Authorization
header. Two read-only CLI calls tell you the ground truth:

```bash
nyxid service list --output json
nyxid catalog list --output json
```
(Inside an Aevatar session, use the typed connected-service tools actually present.) For a request
to connect, add, or authorize a named service, `nyxid_catalog` is discovery only: use it to resolve
the exact catalog slug when necessary, then **always** call `nyxid_require_service`. Finish from
that typed readiness result; it is the authority for a missing-service blocker and interactive
`service.connect` handoff. Never stop at catalog prose.

**The live catalog and authoritative `/api/v1/keys` instance inventory are the source of truth —
never assert a connector exists or does not without checking them.** `/api/v1/keys` is the exact
UserService authority for execution. `/api/v1/user-services` is only a routing projection; it
cannot prove discovery, readiness, or execution authority. The examples below are illustrative,
not a fixed list.

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
   In-session connect/add/authorize requests must resolve the catalog slug, then pass it to
   `nyxid_require_service`; do not substitute prose or `/api/v1/user-services` for typed readiness.
3. **Find the gating piece** — the answer to the whole request is the *weakest* piece (a
   single host-gated or impossible piece caps the whole thing).
4. **Report honestly** with the template at the end: possible + prereqs, or host-action-needed,
   or not-feasible + alternative.

## Prerequisite matrix (capability class → can we? → prerequisite)

| The user wants… | Possible? | Prerequisite / who must do it |
|---|---|---|
| Pure LLM / text / transform / branching pipeline | ✅ Always | Author a workflow (`aevatar-workflow-authoring`). No external anything. |
| Bounded one-shot Codex work that can start from empty Git | ✅ With the managed target | Choose `managed_sandbox`; require internal eligibility, the user's own usable `chrono-sandbox` and `chrono-llm-public` UserServices, `workspace.kind=empty_git`, timeout ≤ 180 seconds, and the public `CODEX_EXEC_READY` proof. |
| Codex work requiring an existing private repository or host Codex configuration | ✅ With the private target | Choose `private_ssh`; require the user's hardened NyxID SSH service, fixed principal/workspace and working host Codex setup, no request `workspace`, timeout ≤ 300 seconds, and the private public-sample proof. |
| **Call** a third-party API (read/post): GitHub, Slack, Google, X/Twitter, Reddit, a custom HTTP API… | ✅ If the connector is in the catalog | User **connects the `api-*` connector in NyxID** (OAuth for `user` mode, or supplies a token for `admin` mode — per the catalog entry). Then use the invocation mode matching the surface; never paste raw route fields into a compiled admitted workflow call. |
| A connector that is **NOT in the catalog** | ⚠️ Only if it's a plain HTTP API | If it speaks HTTP + a supported `auth_method`, NyxID can add it (platform/admin work — not self-serve). If not HTTP, ❌. |
| **Inbound bot** that replies in-platform: **Lark / Telegram** | ✅ Yes | Connect the bot connector (`api-lark-bot` / `api-telegram-bot`) **and** register the channel (channel-admin / `channel_registrations`); NyxID provisions the webhook to Aevatar's relay. |
| **Inbound bot** on a platform with a connector but **no channel module** (Discord, Slack, X, …) | ❌ Not self-serve | Outbound calls work, but inbound-reply needs a new Aevatar **channel module** + relay wiring = Aevatar platform work. Offer the outbound-only version as the alternative. |
| **Publish** a workflow/team as an **invocable service** in-scope | ✅ Yes | Just bind it (`aevatar-service-publisher`). Usable within the user's scope immediately. |
| An external automation **triggers an existing Aevatar workflow** (e.g. Lark Base row status changed → HTTP request → run member workflow) | ✅ Usually, without service externalExposure | Use the external system's HTTP action to call the NyxID proxy for the existing `aevatar` service with a NyxID API key (`proxy` scope), targeting an explicit `/api/scopes/{scopeId}/members/{memberId}/invoke/...` or `/teams/{teamId}/invoke/...` path. This is an external trigger, not a NyxID connector registration. See `aevatar-service-publisher`. |
| Have that service **registered as a NyxID-brokered connector** (callable by others/externally) | ⚠️ Host-gated | The **host** must enable external exposure (`GAgentService:ExternalExposure: Enabled=true` + `RegisterAllPublishedServices` or an opt-in policy). You **cannot** turn this on as a client — verify `externalExposure` on the service and, if empty, tell the user to ask the host. |
| Give an agent a **fixed persona + pinned Ornn skills + an enforced tool ceiling** (an Agent Profile) | ⚠️ Modelled, owner-managed, deployment-dependent | Probe `GET /api/openapi.json` for the complete `agent-profiles` route family. Absent means unavailable in this deployment; present means the owner can manage it through `aevatar-agent-profile-management`. |
| Have a published Profile actually **drive a running agent** | ⚠️ Host-gated, and narrow | Publication is not runtime binding. Only *newly created* NyxID direct conversations admitted by a **host-owned rollout** consume a Profile; existing conversations never hot-upgrade, and workflows/teams/services/schedules/channels/AgentRuns are not consumers at all. |
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
- **Lark Base has two permission layers.** `bitable:app:readonly` is an application API scope;
  it does not grant the Bot access to a specific Base document. Lark error `91403` means the
  Base document ACL denied that application. In the current Base UI, open the Base's `...`/More
  menu, choose **Add Applications**, and add the exact Bot application used by the selected NyxID
  UserService with view access for read-only workflows. Do not ask the user to change API scopes
  when that scope is already enabled, and do not confuse a Base with a spreadsheet file.

## How to satisfy each prerequisite (what you tell the user to do)

- **Connect a connector** → "In the NyxID console, connect **`<slug>`**. <`credential_mode:user`:
  it's a one-click OAuth to your own account.> <`admin`: you'll paste a token — `<api_key_instructions>`;
  get it at `<api_key_url>`.>" In-session, finish through `nyxid_require_service`; for REST/CLI
  verification, confirm the exact UserService and readiness through `/api/v1/keys`, not merely the
  `/api/v1/user-services` projection.
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
- ⚠️ **Yes in the product, but not in this deployment** — "Aevatar models exactly that: an Agent
  Profile with a purpose, instructions, an always-on skill, a billing-triggered skill, and a hard
  tool ceiling — and it would be **yours** to manage, not the host's. But this deployment
  advertises no `agent-profiles` routes, so I can't create one today. What I *can* build now is
  the same behavior expressed in a workflow member with on-demand skill discovery; the pinned-skill
  and enforced-ceiling guarantees need a build that exposes the contract."
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
- **Never promise host-gated outcomes** (NyxID registration, Agent Profile rollout admission,
  anything needing host config) or features that need platform work — surface them as
  dependencies, not done deals.
- **Distinguish not-connected / host-gated / not-deployed.** Each has a different owner and a
  different fix. Probe the live OpenAPI before calling a capability available or unavailable —
  source on a feature branch proves the code exists, never that production exposes it.
- **Don't quietly substitute a different resource.** If the user asked for an Agent Profile and the
  contract isn't deployed, say that and offer the nearest alternative explicitly — never build a
  workflow member and present it as the profile they asked for.
- If you genuinely can't determine feasibility from the catalog + this matrix, say what you'd
  need to confirm rather than guessing.
