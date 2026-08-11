# Direct Chrono-LLM Chat Mode — Spec (Draft v3.1)

Status: implementation-ready (adversarially reviewed).
Branch: `chat-chronollm-direct`.
v1 2026-08-11 (design from code reading); v2 same day (live prod
verification, recommended FE-only); **v3 same day — owner directive**:
Calvin chose the new-endpoints design over v2's FE-only recommendation.
**v3.1 same day — reconciled against Sol's adversarial review**
(`direct-chronollm-plan.review.md`: 3 BLOCKER / 7 MAJOR / 2 MINOR, all
accepted or dispositioned; changed sections marked `[v3.1]`).

## 0a. Owner directive (v3, verbatim intent)

> "I want a set of new endpoints to be able to call chat. This should be
> admin feature gated where I can toggle between existing aevatar chat
> endpoint and a direct endpoint. We need to be able to directly chat with
> chrono llm, and provide necessary skills and system prompt. It CAN be
> hardcoded for now."

Consequences:

- **Design A is chosen**: new server endpoints on the assistant mount, not
  the v2 FE-only proxy call. v2's Design B analysis stays in §0b as
  evidence, not as the plan.
- **The admin feature flag selects the engine**: flag ON for a user → the
  assistant page chats through the direct endpoints; OFF → the existing
  Aevatar route. Admin flips it in Admin → Feature Flags (global / org /
  user targets — machinery from PR #1197). It is an engine switch, not a
  user-facing toggle.
- **Skills and the system prompt are hardcoded** server-side for now
  (compile-time constants / `include_str!`), explicitly sanctioned.

## 0b. Live verification results (prod, 2026-08-11, Calvin's session)

**The service row** (`GET /api/v1/services`, admin):

```text
id:                        6b2c5a49-d3fd-4582-b971-bd2c05bec34b
slug:                      chrono-llm-public
base_url:                  https://llm.aelf.dev/v1      <- includes /v1
visibility:                public
service_category:          internal
auth_method:               bearer  (master credential on the row)
requires_user_credential:  false
identity_propagation_mode: none
forward_access_token:      false
inject_delegation_token:   false
billing:                   platform_billable: true, platform_metric: "tokens"
```

- **Every prod user can call it** — the row passes
  `proxy_service.rs:is_public_internal_master_credential_service`, so
  slug-path resolution auto-provisions users with no personal row and
  injects the master credential. No connect step exists or is needed.
- **Path join:** `base_url` already ends in `/v1` → upstream path is
  `chat/completions` / `models` (a `v1/...` path 404s downstream).
- **Models (live):** gpt-5.5, gpt-5.4, gpt-5.4-mini, gpt-5.4-2026-03-05,
  gpt-5.3-codex(+spark), gpt-5.2 family, codex-auto-review,
  gpt-4o-audio/realtime-preview, gpt-image-1/1.5/2. gpt-5.5 answered a
  one-liner in 2.8 s non-streaming.
- **Streaming:** textbook OpenAI SSE — `chat.completion.chunk` deltas →
  `finish_reason:"stop"` → usage frame → `data: [DONE]`.
  `stream_options.include_usage` honored (usage incl. `reasoning_tokens`).
  `system` role honored (skill-injection precondition verified).
- **Token billing already works on this path** via the row's
  `platform_metric: "tokens"`; but the proxy force-injects
  `include_usage` only for `llm-openai`/`llm-deepseek`
  (`handlers/proxy.rs:service_supports_stream_options_include_usage`), so
  the caller must set it or streamed turns bill on byte estimates. The v3
  server rebuild (§3.2) always sets it — solved structurally.
- Verbatim stream capture saved as
  `frontend/src/lib/assistant/__fixtures__/chrono-llm-direct-stream.sse`.

## 1. Goal

A second, flag-selected chat engine for the NyxID assistant page that calls
Chrono LLM directly through new NyxID endpoints — bypassing Aevatar.

- **Stateless.** No server-side conversations or history. Conversations
  live in browser memory and reset on reload. Every turn resends the full
  local transcript.
- **Basic calls only.** Text in, streamed text out. No tool execution, no
  action/connect cards, no approvals, no workflow engine.
- **Skill + system prompt server-owned.** The server prepends a hardcoded
  base system prompt and, when the client selects one, a hardcoded skill
  body. The client cannot supply `system` content.

### Non-goals

- Feature parity with the Aevatar chat (that is the Direct Chrono Harness
  spec, Draft v1.3, Desktop docs; this is its "Phase 0 in product form").
- Tool/agent loop, MCP, `tooling_required` sentinel.
- Server-side persistence of any turn content.
- Dynamic skill registry (Ornn fetch) — hardcoded now; §8 lists the seam.
- Agent-key access (human-only surface, like the rest of the mount).

## 2. Architecture

```text
browser (cookie session or bearer)
  -> NyxID POST /api/v1/assistant/direct/completions   [flag-gated]
       typed body -> server rebuild (system prompt + skill + include_usage)
     -> execute_admin_proxy(chrono-llm-public row id, "chat/completions")
        -> https://llm.aelf.dev/v1/chat/completions    (SSE passthrough)
```

Reused invariants (same as the Aevatar mount, docs/chat/01-architecture.md):

- Routes nest under `/api/v1/assistant` in the **human-only** router
  (API-key / service-account / delegated / relay tokens rejected).
- **Admin service resolution**: the caller never chooses the upstream.
  Resolution generalizes `assistant_service::resolve_admin_service` to a
  by-slug variant with the same guards (active,
  `!requires_user_credential`), targeting the hardcoded const
  `DIRECT_LLM_SLUG = "chrono-llm-public"`.
- `execute_admin_proxy` (not `execute_proxy`): platform-selected target;
  caller-owned routing state (personal rows, node pins, connection state)
  must not decide whether the surface works.
- **Server-rebuilt body** from a strict typed request — no passthrough of
  client OpenAI fields; `system` is server-owned; `stream` and
  `stream_options.include_usage` are forced.
- No identity/delegation tokens minted on this path — the row needs none
  (`identity_propagation_mode: none`; the master bearer is the credential).

## 3. Backend

### 3.1 Endpoints (all flag-gated, §4)

```
POST /api/v1/assistant/direct/completions   -> SSE (OpenAI chunk passthrough)
GET  /api/v1/assistant/direct/skills        -> [{ "slug", "label" }]
GET  /api/v1/assistant/direct/models        -> [{ "id", "label", "default" }]
```

Mounted inside `assistant_proxy_routes` (`backend/src/routes.rs`).

**[v3.1] Deletion DEFERRED (Sol MAJOR 6):** the old
`POST /api/v1/assistant/completions` route is a documented retained surface
(`docs/chat/02-wire-contract.md`) — an internal grep cannot prove external
deadness of a published HTTP route. Removal is out of scope for this PR;
filed as a follow-up requiring a deprecation decision. Do not touch it.

### 3.2 Typed request and server rebuild

```jsonc
// POST /assistant/direct/completions
{
  "messages": [                        // required, 1..=64
    { "role": "user" | "assistant", "content": "..." }
  ],
  "model": "gpt-5.5",                 // optional; must be in MODELS
  "skill_slug": "nyxid"               // optional; must be in SKILLS
}
```

Validation (in `assistant_service.rs`, unit-testable pure functions).
**[v3.1] Limit units made explicit (Sol MINOR 1)** — three separate limits:

- raw request body ≤ 256 KiB, read via bounded `to_bytes` (same pattern and
  `BadRequest` overflow mapping as the existing assistant raw-body
  handlers);
- per-message `content` ≤ 32,768 Unicode scalar values;
- aggregate decoded content ≤ 256 KiB UTF-8 bytes; 1..=64 messages;
- roles other than `user`/`assistant` → 400 (`system` is server-owned);
- unknown `model` / `skill_slug` → 400 with the allowed values named;
- rebuilt upstream body is exactly:

```jsonc
{
  "model": "<validated or default>",
  "stream": true,
  "stream_options": { "include_usage": true },
  "messages": [
    { "role": "system", "content": "<BASE_SYSTEM_PROMPT [+ \n\n + skill body]>" },
    ...validated user/assistant messages
  ]
}
```

One system message total. **[v3.1] (Sol MINOR 2)** composition is
`BASE_SYSTEM_PROMPT + "\n\n" + skill body + "\n\n" + DIRECT_MODE_OVERRIDE`,
where `DIRECT_MODE_OVERRIDE` is a server-owned suffix that overrides any
action/tool/CLI-execution instructions inside skill bodies (the embedded
skills were written for tool-bearing agents): the model must describe
user-executable steps and never claim to execute. A prompt-shape test pins
this suffix as last for every skill. Upstream path: `chat/completions`
(§0b path join).

### 3.3 Hardcoded content (owner-sanctioned)

**[v3.2] Prompt content is authored here, derived from the Aevatar support
contract (gist b4dd5182 / nyx-chat-aevatar-support-spec.md — owner
directive 2026-08-11: "what we need the model to do is as mentioned in the
spec").** Direct mode has no tools, so the support spec's five-class
discipline collapses to: Class L treatment for every actionable intent
(exact copyable `nyxid` command, never claim execution), the §2.4/§3
honesty rule ("cannot check" is never "not connected"), the §4 no-drip-feed
rule, and the detection-fallback rule (never invent verbs/URLs). Embed
these strings VERBATIM:

```rust
// backend/src/services/assistant_direct.rs (new)
const DIRECT_LLM_SLUG: &str = "chrono-llm-public";

const BASE_SYSTEM_PROMPT: &str = "\
You are Nyx, the NyxID assistant, running in direct model chat: a \
text-only mode with no tool execution. NyxID brokers credentials for \
external services (LLM APIs, GitHub, Lark, SSH, MCP) so users and their \
agents never handle raw keys; users manage services, keys, nodes, \
approvals, and organizations through the NyxID dashboard and the `nyxid` \
CLI.\n\
\n\
Operating rules (binding):\n\
1. You cannot execute anything: no reads of live account state, no API \
calls, no actions. Never claim to have run, checked, created, or changed \
anything.\n\
2. When the user asks for something actionable, respond with the exact \
copyable `nyxid` CLI command (or the dashboard path) that accomplishes \
it, plus a one-line note that chat cannot run it. Prefer commands over \
prose walkthroughs.\n\
3. When an answer depends on live account state you cannot see, say you \
cannot check from here and give the exact command that shows it. Never \
present a guess as current state; 'cannot check' is never 'not \
connected'.\n\
4. If a request has prerequisites (a service connection, an approval, a \
registered node), name ALL of them up front in one reply - no \
drip-feed.\n\
5. Never invent commands, flags, URLs, service slugs, or API endpoints. \
If you are not sure the exact command exists, say so and point to `nyxid \
--help` or the dashboard.\n\
6. For platform-admin, billing, pre-authentication, or otherwise excluded \
surfaces, decline briefly and point to the nearest supported \
alternative.\n\
7. Answer in the user's language. Be concise; lead with the answer.";

const DIRECT_MODE_OVERRIDE: &str = "\
OVERRIDE - the reference material above may instruct the use of CLI \
execution, agent tools, MCP calls, or API requests. Those instructions do \
not apply to you in this chat: you cannot execute anything. Treat that \
material strictly as knowledge for describing steps the USER can run \
themselves; present commands as copyable suggestions and never state or \
imply that you ran them.";

pub struct DirectSkill { pub slug: &'static str, pub label: &'static str,
                         pub body: &'static str }
pub const DIRECT_SKILLS: &[DirectSkill] = &[
    // include_str! from the repo's skills/ tree, SKILL.md only (never
    // references/ payloads). Implementer verifies each ≤ 64 KiB (const
    // assert) and trims if over.
    // proposed: "nyxid", "github-via-nyxid", "firecrawl-via-nyxid"
];

pub struct DirectModel { pub id: &'static str, pub label: &'static str,
                         pub default: bool }
pub const DIRECT_MODELS: &[DirectModel] = &[
    // gpt-5.5 (default), gpt-5.4, gpt-5.4-mini, gpt-5.2
    // curated: no codex/image/audio/realtime ids
];
```

`GET .../skills` and `GET .../models` serve these tables verbatim (no
upstream call — deterministic, and the live `/models` list contains ids we
deliberately hide).

### 3.4 Flag gating (server-side)

Every direct handler resolves the caller's effective flags via
`feature_flag_service::resolve_personal_features` (grant-union incl. org
grants — the `/users/me` path) and returns `AppError::NotFound` when
`experimental:direct-chat-engine` is off. The flag also drives FE engine
selection (§5); the server check is the authority.

**[v3.1] Cost decision (Sol MAJOR 3), accepted explicitly:** this
resolution performs ~2 uncached Mongo reads per call. On a human-paced
chat turn that already spends seconds in the LLM, this is accepted for v1
— no cache is built. Rationale + revisit trigger (if the route ever serves
agent traffic) documented here so the acceptance is loud, not silent.

## 4. Feature flag

New registry entry `experimental:direct-chat-engine`
(`backend/src/services/feature_flag_service.rs::FEATURE_FLAGS` +
`frontend/src/lib/feature-flags.ts::FEATURE_FLAG.DIRECT_CHAT_ENGINE`,
default off). Admin toggles per global/org/user via the existing Feature
Flags admin page — this is Calvin's aevatar↔direct switch. The key is also
named by the harness spec §13 (engine-stamped conversations); this spec
claims it first with engine-select semantics — reconcile if the harness
ships.

## 5. Frontend

### 5.1 Engine selection — engine-router transport [v3.1, replaces the
install() design per Sol BLOCKER 2 + MAJOR 1]

`DelegatingAssistantTransport.install()` is one-shot and its slot is owned
by the dev scenario interceptor; the singleton is exported as the narrow
`AssistantTransport` interface. The flag-driven switch is therefore a new
**engine-router transport** that permanently owns BOTH delegates:

- Router state: `selectedEngine: "aevatar" | "direct"`, set from the
  feature flag (auth-store capability via `useFeature`; the 60 s
  `/users/me` observer propagates admin flips) through an explicit setter
  the page/hook layer calls — the router itself stays framework-free.
- Routing rules: engine-scoped calls (list, create) go to the selected
  engine; conversation-scoped calls route by **validated id prefix**
  (`direct-` → direct delegate; `nyxid-chat-`/`chatc-`/pending prefixes →
  aevatar), regardless of the selected engine — unknown prefixes fail
  closed.
- Running turns: the router keeps a per-turn delegate registry keyed by
  turn/conversation id; cancel, stream-start-timeout abort, and Stop
  always resolve the ORIGINATING delegate from the registry, never the
  currently-selected one.
- Layering: scenario interceptor (dev) wraps OUTSIDE the router — it keeps
  its `install()` slot; mock/test mode still returns the mock transport
  directly (`selectAssistantTransportKind` untouched). Initialization
  behavior specified for all four contexts: production, dev, dev+`?mock`,
  vitest.
- Query/cache separation (Sol MAJOR 1): assistant query keys gain an
  engine segment; a flip invalidates the other engine's list/history
  queries and clears draft/URL conversation state pointing at the other
  engine; `pendingCreate` is engine-stamped. Flag ON → the page neither
  fetches nor renders Aevatar history; flag OFF → byte-identical to
  today's behavior.

### 5.1b Identity-transition reset [v3.1, Sol BLOCKER 3]

The direct delegate's in-memory store is scoped to the authenticated user:
the store records the owning `user_id`, and every identity transition —
logout, session-invalidating 401, `setUser(null)`, or user A → user B
without reload — aborts in-flight direct turns, wipes conversations,
picker/draft state, and invalidates direct queries. Wired at the same
boundary that clears the other auth-scoped stores (`auth-store` cleanup +
`use-auth` logout). A module-lifetime test proves
`null → A → transcript → logout → B` leaks nothing.

### 5.2 Direct transport

Resurrect the deleted `completions-transport.ts` (439 lines at
`e3c2f36e^:frontend/src/lib/assistant/completions-transport.ts`) as
`direct-transport.ts`:

- POST the §3.2 typed body to `/api/v1/assistant/direct/completions`; parse
  the OpenAI SSE with the existing `sse.ts` `drainSseBuffer` + resurrected
  chunk handling.
- **[v3.1] Drain-through semantics (Sol MAJOR 2):** UI terminal state and
  stream-drain state are separate. On `finish_reason`, close the visible
  message but KEEP READING through the usage frame and `[DONE]`/EOF; only
  then release the reader (dropping the response cancels the upstream
  stream server-side and can lose the usage frame that billing accuracy
  depends on). EOF succeeds only if a terminal marker (`finish_reason` or
  `[DONE]`) was observed; bare EOF → failed turn. Abort settles exactly
  once. The delegate implements the CURRENT expanded `AssistantTransport`
  contract explicitly (projection/delete/cancel/approval/action methods) —
  the historical class predates it.
- Stateless: in-memory conversation store scoped per §5.1b; list/history/
  delete local; reload wipes. First send auto-creates locally.
- Skill picker options from `GET .../skills`; model picker from
  `GET .../models`; per-conversation `skill_slug` (+ model) stored locally
  and sent each turn.
- **[v3.1] 401 handling corrected (Sol MAJOR 7):** skills/models are
  first-party NyxID routes — use normal `api.get` (a 401 there means the
  NyxID session is dead and SHOULD clear auth; `preserveSessionOn401` is
  for downstream calls only). For the raw completions POST, parse the
  pre-SSE JSON envelope and distinguish: NyxID auth error
  (`{error, error_code, message}` shape, e.g. token expiry) → normal auth
  clearing; upstream/downstream 401/403 or flag-off 404 → keep the session,
  show reconnect/unavailable copy. Distinct tests for each.

### 5.3 UX copy

Persistent dismissible banner in direct mode: "Direct model chat — no
tools, no approvals, and conversations are not saved." Same in the empty
state. Skill picker copy: a skill teaches the model about NyxID; it cannot
take actions here.

## 6. Streaming contract & failure behavior

- Upstream shape per §0b. EOF without `finish_reason`/`[DONE]` → **failed
  turn, not success**.
- First-byte deadline 30 s, idle timeout 120 s (client AbortController; the
  server side is a thin passthrough, no watchdog in v1).
- Cancel: client aborts; no upstream cancel exists — accepted.

## 7. Security, billing, audit

- No new secrets; master credential injected by `execute_admin_proxy`
  server-side. No identity/delegation tokens on this path.
- `system` content is server-owned; client `system` role rejected.
- Flag check server-side on every direct route (404 when off).
- **[v3.1] Rate limiting (Sol MAJOR 4):** the global middleware is
  per-IP, and the per-agent limiter only fires for API-key callers — this
  human-only route would otherwise have NO per-user control on a
  platform-credential cost-bearing endpoint. Add a dedicated
  authenticated-user limiter for `POST .../completions` (user-id-keyed
  bucket + small per-user in-flight stream cap, e.g. 2), with defined 429
  behavior and isolation tests. Skills/models stay on the cheap default
  policy.
- **[v3.1] Billing (Sol BLOCKER 1 — spec §7 v3 claim was WRONG):** SSE
  usage observation is currently enabled only for `llm-`-prefixed slugs
  (`handlers/proxy.rs`), so `include_usage` alone changes nothing — the
  frame would be parsed by nobody and turns would bill byte-estimates.
  REQUIRED backend fix: gate usage capture on the effective token billing
  metric (row `platform_metric: tokens`) + OpenAI-SSE response shape (or
  explicitly allowlist `chrono-llm-public` as an SSE usage source), with a
  route-level test through the real assistant POST boundary proving the
  settled billing quantity equals the fixture's reported total (provenance
  = reported, not estimated).
- **[v3.1] Billing inventory (Sol MAJOR 5):** the direct POST must be
  registered in the billing route inventory
  (`services/billing/route_inventory.rs`) and exercised by the billing
  smoke suite at its real authenticated boundary; the metered
  `route_layer` applies only after all forwarding routes are registered.
  Skills/models are inventoried as control-plane exemptions or mounted
  outside the forwarding layer.
- Tracing/logs: metadata only (model, skill_slug, message count, sizes,
  outcome) — never message content (Oracle/WS-frame discipline).

## 8. Later seams (documented, not built)

- Skill registry → Ornn fetch or admin-editable rows; system prompt → admin
  setting. The `DIRECT_SKILLS`/`BASE_SYSTEM_PROMPT` consts are the single
  place to swap.
- Model list → live upstream `/models` with a curation filter.
- Env override for `DIRECT_LLM_SLUG` (e.g. dedicated `chrono-llm-assistant`
  row for separate billing attribution).
- Harness-spec convergence: if engine-stamped conversations ship, this
  flag's semantics widen (§4).

## 9. Testing

Backend:
- Validation matrix (roles, caps, unknown model/skill, empty messages).
- Rebuild shape (single system message, ordering, forced stream +
  include_usage, field stripping).
- Flag-off → NotFound on all three routes; flag-on passes.
- Service-row guard failures (inactive row, requires_user_credential).
- Skills/models tables: const-assert sizes; endpoints serve tables.

Frontend (vitest):
- Resurrected transport suite against the saved fixture
  (`chrono-llm-direct-stream.sse`): delta accumulation, finish, usage
  frame, `[DONE]`, error envelope, EOF-without-finish → failed, timeouts,
  caps.
- Flag-driven engine selection: ON/OFF transport choice, mid-stream flip
  keeps the turn's transport, prefix fail-closed, flag-off = aevatar
  untouched.
- Skill/model pickers: options from routes, per-conversation persistence,
  sent on turn.

Gates (all must pass locally — CI runs on main-target PRs):
- `cargo fmt` clean, `cargo clippy -D warnings`, `cargo test`.
- FE: `npm run build` (tsc -b — the real CI gate; `tsc --noEmit` is not
  sufficient), `npm run test`, `npm run lint`.
- Wizard bundle freshness: if FE source changes trip the source-index,
  rebuild `npm --prefix frontend run build:wizard` AFTER the final rebase
  and commit `cli/src/wizard/` (rebasing after the rebuild stales the
  index hash).

Prod smoke post-deploy: real cookie session, flag flipped for Calvin only
(user-target), one turn with and without a skill, billing row shows
reported tokens; flip off, Aevatar chat unaffected.

## 10. Open questions

1. Initial skill set — proposal `nyxid`, `github-via-nyxid`,
   `firecrawl-via-nyxid` (Calvin to confirm; sizes checked at build).
2. Exposed models — proposal gpt-5.5 default + gpt-5.4 / gpt-5.4-mini /
   gpt-5.2.
3. Flag-key collision with harness §13 — accept (documented) or rename.
