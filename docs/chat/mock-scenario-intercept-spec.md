# Assistant Mock Scenario Interception — Implementation Spec

Status: **IMPLEMENTED (v2)** — code and test commits `4fe63f71..1a7f6f6b`
(2026-08-04).
Author: Claude (Fable). Adversarial review: CodexSol — 14/14 findings accepted
(disposition log §15; full review text in `mock-scenario-intercept-spec.review.md`).

---

## 1. Summary

A runtime-toggleable mock layer for the assistant chat. A header button opens a
popover; switching it on **intercepts** `sendMessage` calls whose text matches a
scenario regex and plays a scripted turn (text, tool steps, action cards,
approval cards) through the exact same `TurnEvent` pipeline the real Aevatar
transport uses. Unmatched messages pass through to the live backend untouched.

Purpose: rehearse and verify real FE flows — "connect to my github" shows the
real connect journey off a scripted action card; "what are my gh issues" on a
cold world runs connect-first-then-search, exercising the block → card →
continue → resume UI path end to end, deterministically, without Aevatar.

### Goals
- G1: Author flows as typed TypeScript config (regex → steps), editable with HMR.
- G2: Runtime toggle in the assistant header (button + popover), no URL param,
  no reload.
- G3: Scripted turns are **protocol-faithful**: card-parked turns end
  `turn.completed status:"blocked"`; resumes are new turns driven by the UI's
  own `continueActions` / `decideApproval` calls.
- G4: Mock chat turns coexist with a **real session**: real conversations, real
  auth, real connect journeys — only the assistant's replies are scripted.
- G5: World state (`connected` services) so `need`-gated flows branch like
  reality and feel alive across repeated asks.
- G6: Zero footprint when off — a session without the
  `experimental:assistant-mock-scenarios` flag fetches no scenario chunk, and
  no scenario code reaches the entry graph or the assistant page chunk.

### Non-goals (v1)
- N1: No changes to the existing full-mock transport (`?mock` /
  `MODE === "test"`). `MockAssistantTransport`, `mock-data.ts`, and
  `createScriptedTurn` are untouched; every existing test passes as-is.
- N2: No server persistence of mock turns. They are a client-side overlay and
  do not survive a hard reload (see §6.3).
- N3: No wire-log capture of mock turns (nothing crosses the network). The
  wire-log panel simply records nothing for them.
- N4: No recorded-SSE replay step (`wire-replay.ts` stays replay-only). Possible
  follow-up.
- N5: No mobile/app surface. Web assistant page only.

---

## 2. Modes after this change

| Mode | Selection | Transport | Scope |
|---|---|---|---|
| Live (default) | — | `AevatarAssistantTransport` | prod + dev |
| Full mock | `MODE === "test"`, or dev + `?mock` | `MockAssistantTransport` | vitest, e2e, demo — **unchanged** |
| **Intercept** (new) | Live mode + popover toggle ON | `ScenarioInterceptTransport` wrapping the Aevatar transport | dev builds (v1) |

`selectAssistantTransportKind` keeps its two-value return. The `"aevatar"`
branch returns a thin delegating shell; in dev builds a boot-time dynamic
import installs the interceptor around the delegate (§8.3). With no mock
state and the toggle off, every method is a direct delegate call, so live
behavior is bit-identical; prod builds contain no interceptor code at all.

---

## 3. Architecture

```
pages/assistant.tsx
  └─ headerActions: <MockScenariosAction/>            (new, beside AssistantWireLogAction)
        └─ reads/writes stores/assistant-mock-scenarios-store.ts   (zustand, persisted)

lib/assistant/transport.ts
  └─ DelegatingAssistantTransport shell (prod: bare delegate; dev: boot-time
     dynamic import installs ScenarioInterceptTransport — new file:
     scenario-intercept-transport.ts)
        ├─ no mock ownership → delegate to AevatarAssistantTransport
        └─ mock-owned ids / matched sends → route per §6; scripted turns from
              lib/assistant/scenario-engine.ts        (new; pure TS, no React)
                └─ scenarios from lib/assistant/scenarios.config.ts (new; authored file)
```

The interceptor, engine, config, and store load via flag-gated dynamic
`import()`s per §8.3 — `transport.ts` keeps zero static mock references, and
the install path reports `loading`/`error` engine states to the store so a
failed install is visible rather than silently passing through (drafted in v1
as a store-triggered load, shipped as a dev-only boot install, and now driven
by the platform feature flag; deviations declared and accepted).
`MockScenariosAction` is the only static importer of the store, and nothing
statically imports either.

---

## 4. Scenario config surface

File: `frontend/src/lib/assistant/scenarios.config.ts`. Typed builder — no
parser, no YAML dependency (a yaml dep would also trip the Wizard Bundle
Freshness CI check for nothing).

```ts
import { flow, scenario } from "@/lib/assistant/scenario-engine";

export const flows = {
  "connect-github": flow((s) => s
    .say("I need access to your **GitHub** account first.")
    .action("service.connect", { service: "api-github", scopes: ["repo", "read:org"] })
    .await({
      declined: (s) => s.say("No problem — I'll skip anything that needs GitHub.").stop(),
      failed:   (s) => s.say("The connection didn't complete. Ask me again when ready.").stop(),
    })                                       // disposition "completed" falls through
    .connect("api-github")                   // world.connected += api-github
    .say("GitHub connected — credential sealed in NyxID, never shared with the model.")),
};

export const scenarios = [
  scenario("connect-github", /connect (to )?(my )?github/i, (s) => s
    .whenConnected("api-github", (s) => s.say("You're already connected to GitHub.").stop())
    .run("connect-github")),

  scenario("github-issues", /(what|show|list).*(gh|github).*issues/i, (s) => s
    .need("api-github", "connect-github")    // not connected → splice the flow in first
    .tool("github.listIssues", "7 open, 2 stale")
    .say("You have **7 open issues** on `acme/web`. Two haven't moved in 30 days.")),

  scenario("github-issues-repo", /issues (?:in|on) (\S+)/i, (s, m) => s
    .need("api-github", "connect-github")
    .tool(`github.listIssues repo=${m[1]}`, "3 open")
    .say(`\`${m[1]}\` has **3 open issues**.`)),
];
```

First match wins, in array order. Per-scenario enable/disable comes from the
store (§7), evaluated at match time. No match → pass through to Aevatar.

### Verb table

| Verb | Emits | Notes |
|---|---|---|
| `.say(md)` | `message.started` → `block.started(text)` → `block.delta`×N → `block.completed` → `message.completed` | chunked ~40 chars at the existing 100 ms cadence |
| `.tool(label, result)` | run block per the `createScriptedTurn` shape: step `active` → `done` with `result` as meta | consecutive `.tool`s share one run block |
| `.toolFail(label, err)` | step settles `failed`; run block state `failed` | |
| `.action(action, {service, scopes} \| {custom})` | `action_card` block, `status:"pending"`, params via `assistantActionRequestSchema.parse` + `resolveAssistantAction` — same as `createScriptedTurn` does today | `action_request_id` = `mockchat-act-<n>` |
| `.approval(slug, body, opts?)` | `approval_card` block, `decision:null`, `expires_at` now+15 min | |
| `.artifact(name, preview, mime?)` | `artifact` block | `download_url` is a `data:` URL |
| `.await(branches)` | **ends the segment**: closes open blocks/message, emits `turn.status waiting` then `turn.completed status:"blocked"` | branches: action cards `completed/declined/failed`; approval cards `approved/denied`; missing branch = fall through |
| `.need(slug, flowName)` | if `world.connected` has slug → no-op; else splice `flows[flowName]`'s segments ahead of the remaining steps | |
| `.run(flowName)` | unconditional splice | flows may not `.run`/`.need` other flows in v1 (no recursion; enforced at config load) |
| `.whenConnected(slug, sub)` / `.whenNotConnected` | conditional sub-script, evaluated at play time | |
| `.connect(slug)` / `.disconnect(slug)` | mutate `world.connected` (store-backed) | |
| `.wait(ms)` | pacing gap | capped at 10 s |
| `.stop()` | end turn `completed` | |
| `.fail(code, message)` | end turn `failed` with error | for testing FE error states |

---

## 5. Engine semantics

File: `frontend/src/lib/assistant/scenario-engine.ts`. Pure TypeScript (builder,
compiler, runtime); React-free; unit-testable without the DOM.

### 5.1 Segmentation & turn lifecycle
A compiled scenario is a list of **segments** split at each `.await()`.

- Segment 1 plays as the send turn: `turn.status running` → content events →
  (if it ends at an `await`) `turn.status waiting` → `turn.completed
  status:"blocked"`. A segment with no await ends `status:"completed"`
  (`"failed"` after `.fail`).
- On emitting an await, the engine registers a **continuation**:
  `key = (conversationId, requestId)` where `requestId` is the
  `action_request_id` or `approval_request_id` of the parked card, holding the
  branch map + remaining segments + captured regex groups + world snapshot ref.
- The resume arrives via the UI's own calls (§6.3). The selected branch plus
  the remaining segments play as a **new turn** with a fresh
  `mockchat-turn-<n>` id — matching the real wire, where blocked turns end and
  resumes are new deliveries.
- **Exactly one resumable card per `.await()` (v1).** The config compiler
  rejects a segment that parks with more than one un-decided action/approval
  card: one card ↔ one continuation ↔ one branch map. Aggregate barriers
  (mixed `completed + declined`, partial reports across cards) are
  deliberately out of scope. (CodexSol F10.)
- Cursors are **conversation-monotonic, not turn-scoped**: `applyTurnEvent`
  drops any event whose cursor is ≤ the reducer's `lastCursor`
  (`stream.ts:94`), and a continuation lands in the same overlay reducer as
  the turn that parked. Every emitted event is therefore stamped
  `overlay.lastCursor + 1` at delivery time — the same re-stamping
  `MockAssistantTransport.startScript` does (`transport.ts:443-449`). A
  continuation restarting at cursor 1 would be silently swallowed by the
  overlay. (CodexSol F1.)
- One active scripted turn per conversation, enforced with the same
  `AssistantTurnActiveError` the mock transport throws.
- Cancel (composer stop button) mirrors `cancelScript`: close open blocks via
  `toTerminalBlock`, `message.completed`, `turn.status cancelled`,
  `turn.completed status:"cancelled"`. Cancelling a parked (blocked) turn
  additionally drops its registered continuation and settles the card terminal.

### 5.2 Event delivery
Events are delivered through `onEvent` with the same 100 ms `setTimeout`
cadence as `MockAssistantTransport.startScript` (`EVENT_CADENCE_MS`), timers
tracked per running script for cancel/reset.

### 5.3 World state
`world = { connected: string[] }` lives **in the zustand store** (§7), not in
module state — single authoritative record (FI-004), reactive popover rendering
for free, persisted with the toggle. The engine reads/writes through the store
API. Global across conversations, mirroring reality. Popover exposes chips with
remove, plus "Reset world". `.connect(slug)` never applies on report receipt
alone — it requires the §6.6 verification to confirm the real journey actually
connected the requested service. (CodexSol F5.)

### 5.4 Card-state guards
The engine replicates the transport-level guard behavior for mock-owned cards:
- `setActionCardInProgress` / `blockActionCard`: same status-gating and
  `block.updated` patches as `MockAssistantTransport` (`transport.ts:164-219`)
  — the UI calls these mid-journey and the card must respond identically.
- `continueActions`: same `refusedByCardState` rules (conflicted; blocked +
  completed-report) including the `composeUnreportedCompletedNote` patch path.
- On resume, cards patch to `completed/declined/failed` with the same outcome
  notes the mock transport uses today, then the continuation turn streams.

### 5.5 Approval expiry

Mock approval cards carry a real `expires_at`, and the approval card's expiry
is display-only — the decision buttons stay clickable after it passes
(`approval-card.tsx:247-265`), so enforcement must live in the transport, as
it does on the real server. The engine enforces lazily at decision time: if
`now > expires_at`, `decideApproval` patches the card to
`decision:"expired"`, drops the registered continuation, and returns `null`.
(CodexSol F11.)

---

## 6. Interceptor transport

File: `frontend/src/lib/assistant/scenario-intercept-transport.ts`, implementing
`AssistantTransport` and holding the real `AevatarAssistantTransport` as its
delegate.

### 6.1 Ownership, not the toggle, decides routing

The load-bearing rule (CodexSol F2): **routing is decided by ownership, and
ownership is independent of the master toggle.** Everything the engine mints
is prefixed `mockchat-` (turns, messages, blocks, action/approval request
ids); nothing real uses this prefix (real ids are server-minted; full-mock
never coexists with the interceptor). Any call referencing a mock-owned id —
decide/continue/wake, card patches, Stop of a running script, delete of a
conversation with mock state — routes to the engine **whether the toggle is
on or off**, until that state is settled. The toggle governs exactly one
thing: whether a new `sendMessage` may match a scenario.

This kills two concrete failures: flipping the toggle off with a parked
GitHub card and then finishing the wizard would otherwise POST `mockchat-`
action reports to the real actor (the real transport validates and forwards
unknown origin ids upstream, `aevatar-transport.ts:1881-1946`); and Stop
during a running script would otherwise reach a delegate that has no such
run, leaving mock timers firing.

### 6.2 Per-conversation ownership state machine

Each conversation is in exactly one state, tracked by the wrapper (CodexSol
F3 — `blocked` is not "active" per `isTurnActive` (`types/assistant.ts:328`),
so neither the hook nor the real transport enforces any of this; the wrapper
is the only place the invariant can live):

| State | Meaning | Leaves via |
|---|---|---|
| `idle` | no mock state, no known delegate turn | send → match: `mock-running`; miss: `delegate-active` |
| `mock-running` | engine script streaming | terminal → `idle`; await → `mock-parked`; cancel → `idle` |
| `mock-parked` | blocked turn with registered continuation | resume → `mock-running`; settle → `idle` |
| `delegate-active` | pass-through turn live (wrapper observes the delegated `onEvent` terminal) | terminal → `idle` |

Rules:
- `sendMessage` in `mock-running` throws `AssistantTurnActiveError` (same
  guard as both existing transports).
- `sendMessage` in `mock-parked` first **settles** the parked state — patches
  the parked card to `conflicted` with note "superseded by a newer message",
  drops the continuation — then proceeds (match → engine, miss → delegate). A
  parked mock card never coexists with a newer live turn.
- Mock resumes (`decideApproval`/`continueActions`/`wakeActions` on mock ids)
  while `delegate-active` throw `AssistantTurnActiveError` — the hook already
  surfaces that error path.

### 6.3 Per-method routing

| Method | Behavior |
|---|---|
| `sendMessage(convId, content, onEvent)` | Per §6.2. Matching requires toggle ON **and** engine ready (§6.7). Miss → delegate, with `onEvent` wrapped so the wrapper observes the delegated terminal for the state machine. Claimed placeholder conversations never delegate (§6.5). |
| `cancelActiveTurn(convId)` | `mock-running` → engine cancel (toggle-independent); else delegate. |
| `decideApproval(convId, blockId, approved)` | `mockchat-` block → engine: expiry check (§5.5), settle card, return continuation `TurnHandle` or `null`. Else delegate. Hook passes the card `block_id` and awaits the handle (`use-assistant.ts:706-727`) — flow-through verified. |
| `continueActions(convId, originTurnId, reports, onEvent)` | `mockchat-` origin turn → engine: `actionReportSchema` validation, §5.4 guards, §6.6 verification, branch by disposition, continuation handle. Else delegate. Hook passes the card's `origin_turn_id` (`use-assistant.ts:797-808`) — flow-through verified. |
| `wakeActions(convId, originTurnId, onEvent)` | Mock-owned → engine (authored `wake` branch, else a one-line resumed turn). Else delegate. |
| `setActionCardInProgress` / `blockActionCard` | By `blockId` prefix (engine per §5.4 / delegate), toggle-independent. |
| `getHistory(convId)` | Per §6.4. |
| `listConversations()` | Delegate, apply metadata overlay (§6.4), re-sort by adjusted `last_message_at`. Claimed conversations are served from the wrapper. |
| `createConversation()` | Delegate (returns the delegate's `workflow-pending-…` placeholder; §6.5). |
| `deleteConversation(convId)` | **Cancel-first, matching the real transport's own order** (`aevatar-transport.ts:1302-1337`): synchronously cancel any engine script, drop overlay + continuations, tombstone the conversation in the wrapper (no further mock activity even if the DELETE fails), then delegate. The real transport clears its deletion reservation in `finally` and removes local state only on success (`aevatar-transport.ts:1358-1383`) — do **not** copy that shape: the wrapper tombstone persists across DELETE failure. (CodexSol F13, P10.) |

### 6.4 History projection — snapshot, anchored merge, metadata

Three review findings, one mechanism (CodexSol F7, F8, F9). Background:
`use-assistant` re-projects on every event and that projection calls
`getHistory`; the delegate serves its local mirror without network only while
*its own* turn is active (`aevatar-transport.ts:1402-1408`), so a naive
wrapper would issue a real transcript GET per mock event — observable
traffic, and it would surface in the wire-log panel despite N3.

- **Base snapshot — history AND list.** The wrapper caches the last
  delegated history per conversation and the last delegated conversation
  list, refreshed whenever the respective call runs in `idle` /
  `delegate-active`. While any conversation is `mock-running` /
  `mock-parked`, both that conversation's `getHistory` and
  `listConversations` serve snapshot + overlay with **zero** delegate calls
  — the delegate refreshes its list on a 5 s TTL and would otherwise issue
  (and wire-log) a real index GET mid-script (CodexSol P7). Bases refresh on
  the next request after the mock terminal.
- **Anchored merge, not tail append.** Each overlay turn records an anchor:
  the id of the last real message in the base when the turn started (`null`
  in a claimed conversation). Projection inserts each overlay group after its
  anchor (anchor gone → after the previous overlay group, else at tail). A
  later pass-through turn therefore renders *after* an earlier mock turn
  instead of leapfrogging it on every refetch.
- **Metadata overlay.** Conversations with overlay content project
  `last_message_at = max(real, newest overlay message)` in both `getHistory`
  and `listConversations`; a claimed conversation also takes its title from
  the first user message (first 40 chars — the store's own convention).
  `message_count`, when the base reports one, projects
  `base + overlay message count`; a claimed conversation reports the overlay
  count; identical values in history and list projections (CodexSol P12).
  Sidebar ordering and thread content stay in agreement.

Session-only semantics, stated in the popover: overlay is in-memory; a hard
reload drops mock turns (and a claimed mock-only conversation entirely).
Toggle OFF stops **new** interception but existing overlay keeps projecting
until reload or delete — mock turns must not vanish from an open thread while
their cards may still be interacted with (that is what "settle", §6.2, is
for).

### 6.5 New chats: claimed placeholder conversations

`createConversation` returns a client-local `workflow-pending-…` id,
materialized and aliased to a server `chatc-…` id only by the first
*delegated* turn (`aevatar-transport.ts:1278-1299`). If a conversation's
first turn is mock and a later unmatched send materializes it, the overlay
and parked continuations would be stranded under the placeholder id (CodexSol
F4). v1 rule: when the **first** turn of a placeholder conversation is
intercepted, the wrapper **claims** it as mock-only — `sendMessage` there
never delegates again (an unmatched send gets a scripted fallback turn: "No
scenario matched — this is a mock-only chat"), history and listing are served
from the wrapper, and delete settles locally before delegating. The
conversation never materializes server-side, so the alias problem cannot
occur. A placeholder whose first turn passed through is an ordinary real
conversation and is never claimed. Side benefit: a claimed chat is a fully
offline demo surface.

### 6.6 Real-journey verification before world mutation

The connect wizard launched from a scripted card is the *real* AddKeyDialog;
it lets the user navigate Back and connect a **different** catalog service
than the card requested, and its completion report carries only a
`userServiceId` (CodexSol F5). A `completed` disposition therefore does not
prove the requested service was connected.

- **Endpoint** (corrected in round 2 — CodexSol P1): there is no
  `GET /user-services/{id}` route; the detail read is `GET /api/v1/keys/{id}`
  (`routes.rs:963-973`), whose `KeyInfo` exposes `catalog_service_slug`
  (`types/keys.ts`). The engine compares that slug to the card's requested
  slug.
- **Synchronous contract** (CodexSol P2): `continueActions` returns
  `TurnHandle | null` synchronously while the lookup is async, and the hook
  disowns its event pump if it receives neither handle nor events. A
  completed report therefore enters a mock-owned **verifying** state
  immediately: the conversation transitions to `mock-running`, a real
  provisional `TurnHandle` is returned, and the card patches to the standard
  "Reported — awaiting assistant verification." note before the GET
  resolves. Cancelling that handle (or deleting the conversation) aborts the
  lookup and suppresses all later delivery.
- **match** → `completed` branch; `.connect(slug)` applies.
- **mismatch** → `failed` branch; card note "a different service was
  connected"; world untouched.
- **lookup fails (404 / network / malformed response)** → `completed`
  branch, world untouched, card note marks the connection unverified.

Locking the dialog to the requested service would be the stronger fix but is
a product change to AddKeyDialog — open question §14.5.

### 6.7 Engine loading

`sendMessage` is synchronous; the engine + config load via dynamic import,
triggered on rehydrate-when-enabled and on toggle-on. Matching cannot happen
without the config module, and silently delegating an intended-mock message
to the live assistant is not acceptable (CodexSol F6: persisted-enabled
reload, user rehearses a destructive flow, their content reaches the real
assistant). While `enabled && engineState !== "ready"`, `sendMessage`
**throws `MockScenariosLoadingError`** — surfaced through the composer's
existing send-error path; the popover shows the loading state.
`engineState === "error"` behaves the same with a distinct message. The
window is one local chunk load, preloaded at store rehydration — normally
gone before a user can type. One residual dev-only race remains: the
interceptor itself installs via the boot-time dynamic import (§8.3); a send
fired before installation completes reaches the bare delegate. Accepted:
install runs at app boot, milliseconds; reaching the composer takes seconds.

---

## 7. Store

File: `frontend/src/stores/assistant-mock-scenarios-store.ts` — zustand +
`persist`, key `nyxid.assistant.mockscenarios.v1`, modeled on
`assistant-wire-log-store.ts`.

```
enabled: boolean                      // master toggle
disabledScenarioIds: string[]         // per-scenario opt-outs (default all enabled)
world: { connected: string[] }        // engine-authoritative world state
engineState: "idle" | "loading" | "ready" | "error"
lastActivity: { scenarioId: string | null, matched: boolean, at: number } | null
                                      // last sendMessage seen while enabled
actions: setEnabled, setScenarioEnabled, connectService, disconnectService,
         resetWorld, noteActivity, reset
```

Persisted subset: `enabled`, `disabledScenarioIds`, `world`, plus the owning
`userId`. Version field for future migration. `reset()` restores defaults
(used by tests and the popover's "Reset world").

Scoping (CodexSol F14): the persisted record carries the authenticated user
id; rehydrating under a different user resets to defaults, so a shared dev
origin never leaks one account's mock world into another. Two tabs share the
key last-write-wins with no cross-tab sync — accepted for a dev tool, noted
in the popover footer.

---

## 8. UI

### 8.1 Header button
`frontend/src/components/assistant/mock-scenarios-action.tsx`, mounted in
`pages/assistant.tsx` beside `<AssistantWireLogAction/>` (both `headerActions`
call sites, lines 487 and 498). Ghost icon button (`FlaskConical` from
lucide, matching header icon sizing), with a small colored dot overlay when
interception is ON — you must always be able to see at a glance that the
assistant is partially scripted. `aria-label="Mock scenarios"`. Rendered only
when the gate (§8.3) passes. Both mount points wrap the lazy action in a
local `<Suspense fallback={null}>` — the assistant page is itself lazy and
the nearest boundary wraps the whole route outlet, so an unguarded nested
lazy chunk would blank the entire workspace while the dev-only control loads
(CodexSol P8).

### 8.2 Popover
Radix `Popover` from `components/ui/popover.tsx` (+ `switch.tsx`), aligned end,
~320 px. Content top-to-bottom (copy per DESIGN.md tone; final visual pass
against DESIGN.md before implementation):

1. **Master row** — "Mock scenarios" + `Switch`. Sub-copy: "Intercepts matching
   chat messages with scripted flows. Session-only; other messages reach the
   assistant normally." Plus a visible warning line: "Action cards open real
   connection journeys and can create real keys on your account." (CodexSol
   P9 — the popover must not read as an offline simulator.) Shows "Loading…"
   while `engineState === "loading"`; error state renders the failure inline.
2. **Scenario list** — one row per config entry: name, regex source (mono,
   truncated with title tooltip), per-scenario `Switch`. A "matched" tick +
   relative time on the row that `lastActivity` points at; a muted "no
   scenario matched" line when the last send passed through.
3. **World** — "Connected (mock)" chips with per-chip remove; "Reset world"
   ghost button. Empty state: "Nothing connected — `need` flows will run their
   connect step."
4. **Footer** — muted: "Edit flows in `src/lib/assistant/scenarios.config.ts`".

No API calls from the popover. All state is the store.

### 8.3 Gating
**Platform feature flag** `experimental:assistant-mock-scenarios`, declared in
`backend/src/services/feature_flag_service.rs::FEATURE_FLAGS` with
`default_enabled: false` and mirrored in `frontend/src/lib/feature-flags.ts`.
Platform admins toggle it globally, per org cohort, or per user from Admin →
Feature Flags; no redeploy, and a deployed preview can be armed for one person.
This supersedes the v1 dev-only (`import.meta.env.DEV`) gate — §14.1 answered.

Enforced at a **module boundary**, not a render branch (CodexSol F12 — a static
`pages/assistant.tsx → action component → store` import chain keeps the
side-effectful `create(persist(...))` store module in the eager graph even when
the render branch folds away; "tree-shaking will get it" is not a plan). Three
boundaries:

- **Transport**: `transport.ts` keeps zero static references to any mock
  module. The `"aevatar"` branch always returns a `DelegatingAssistantTransport`
  shell around the real transport, and installs nothing on its own.
  `applyAssistantScenarioFeature(featureEnabled)` is the sole install path: on
  a flag-off session it returns before loading anything, so the
  interceptor/engine/config/store chunks are never fetched and no localStorage
  is read.
- **UI**: the header action mounts through `MockScenariosGate`, which reads
  `useFeature(FEATURE_FLAG.ASSISTANT_MOCK_SCENARIOS)` and renders `null` —
  never requesting the lazy chunk — while the flag is off. Fail-closed:
  loading, unknown, and an older backend that omits the key all read as off.
- **Runtime**: `ScenarioInterceptTransport.sendMessage` requires
  `featureEnabled && enabled`. `featureEnabled` is the flag mirrored into the
  store by the gate (the same bridge `AssistantWireLogAction` uses) and is
  never persisted, so a flag revoked mid-session disarms an already-installed
  tab and every send falls back to the live delegate. The user's own popover
  toggle is left untouched.

#### Operator runbook — turning it on

Requires the platform-admin role. The flag itself never appears to ordinary
users, and it is a QA tool that **intercepts real sends**, so scope it as
narrowly as the situation allows.

1. Sign in as a platform admin and open **Admin → Feature Flags**
   (`/admin/feature-flags`). `experimental:assistant-mock-scenarios` is listed
   from the backend registry with its code default (off).
2. Add an override at the narrowest scope that works:
   - **User** — one person, on any deployment. The default choice: this is a
     rehearsal tool, not a rollout.
   - **Org** — everyone in one org cohort. Reaches members' personal surfaces
     too (the resolver unions org grants into `/users/me`).
   - **Global** — everyone on the deployment. Preview/staging only; a global
     enable arms the send interceptor for every user who then flips their own
     popover toggle.
3. The target picks it up within about a minute — the flag ships in
   `/users/me` capabilities, which `useUser` refetches on a 60 s interval for
   visible tabs; a reload is immediate. A flask icon appears in the assistant
   header, with the popover's master switch still off: the flag only *offers*
   the tool, it does not start intercepting.
4. To revoke, clear the override. Already-open tabs disarm on that same
   refetch without a reload — the gate clears the store mirror and every send
   goes straight back to the live Aevatar transport, whatever the user's own
   toggle says.

Local development is the same path — there is no longer a dev-only bypass, so
a local backend needs the override too.

Build assertion, run as the last step of `npm run build`
(`frontend/scripts/assert-mock-footprint.mjs`): the mock modules must remain
**dynamic-import only**. It reads vite's `dist/.vite/manifest.json`
(`build.manifest` is enabled for this purpose alone, and the script deletes the
manifest once it passes so the deployed artifact gains no new file) and fails
if any chunk statically reaches an entry point, if the entry graph or the
assistant page chunk carries `mockchat-` / `mockscenarios`, or if the
credential-accept entry does. "Absent from `dist/`" is no longer the invariant
— the code has to ship for an operator to switch it on; "nobody without the
flag downloads it" is.

---

## 9. transport.ts changes (complete list)

1. `createAssistantTransport()`: `"aevatar"` branch returns the
   `DelegatingAssistantTransport` shell and installs nothing.
2. `applyAssistantScenarioFeature(featureEnabled, transport?, loaders?)`: the
   sole install path, called by `MockScenariosGate` with the resolved flag
   (§8.3). Off-and-never-armed returns before loading any chunk; on, it mirrors
   the flag into the store and dynamically imports the interceptor; off after
   arming clears the mirror. Never rejects — a failed chunk surfaces through
   `engineState: "error"`. A full-mock transport is left alone.
3. `installAssistantTransportInterceptor` evicts a rejected installation from
   its `WeakMap` so a transient chunk failure can be retried on the next flag
   resolution rather than poisoning the shell for the tab's lifetime.
4. Nothing else. `selectAssistantTransportKind`, `MockAssistantTransport`, the
   fault hooks, and `resetAssistantTransport` are untouched.

---

## 10. Protocol fidelity rules (checklist for implementation + review)

- [ ] Card-parked turns end `turn.completed status:"blocked"` (`types/assistant.ts:228`; mirrors `RUN_FINISHED status:"blocked"` handling in `wire-replay.ts`).
- [ ] `turn.status waiting` precedes the blocked terminal (as `wire-replay.ts` emits on approval prompts).
- [ ] Resumes are new turns with new turn ids; no event is ever emitted under a completed turn id.
- [ ] Every `block.started` is eventually closed by `block.completed` (including cancel paths via `toTerminalBlock`).
- [ ] `message.completed` closes every `message.started`; terminal ordering matches `cancelScript`'s sequence.
- [ ] Action card params go through `assistantActionRequestSchema.parse` + `resolveAssistantAction` — config typos fail at engine load with a named error surfaced in the popover, not mid-stream.
- [ ] Events delivered strictly in cursor order; cursors are conversation-monotonic (`overlay.lastCursor + 1` at delivery), including across continuations — never restarting at 1.
- [ ] `AssistantTurnActiveError` semantics identical to the mock transport, extended per §6.2 (send while `mock-running`; mock resume while `delegate-active`).
- [ ] Mock-owned ids route to the engine regardless of the master toggle — including Stop, card patches, and resumes after toggle-off.
- [ ] A parked card is settled (`conflicted` + note) before any newer send proceeds in its conversation.
- [ ] Approval decisions after `expires_at` yield `decision:"expired"` and never a continuation.
- [ ] At most one resumable card per `.await()`, enforced at config load.
- [ ] Zero delegated `getHistory` calls while a scripted turn streams; wire-log untouched by mock activity.

---

## 11. Security & safety

- Dev-only surface (§8.3); the prod entry chunk carries no scenario code.
- **Action cards trigger real journeys.** In intercept mode, clicking Connect
  on a scripted card runs the *actual* connect wizard against the *real*
  backend — that is the point (verify the true FE flow), but it means real
  keys/connections can be created while "testing". The popover sub-copy states
  this. World state records the mock's belief, not the backend's.
- No credentials, tokens, or user content leave the browser via this feature;
  scripted content is static config. Scenario config must never embed real
  secrets (review rule).
- No new network surface, no new env vars, no backend changes.

---

## 12. Test plan

Vitest, colocated per repo convention. No new deps.

- `scenario-engine.test.ts` — builder→segment compilation; blocked/completed/
  failed terminals; await branch selection (action dispositions + approval
  approved/denied); `need` splicing cold vs satisfied; `whenConnected`;
  config-load rejections (flow recursion, >1 resumable card per await);
  cancel mid-segment and while parked; **conversation-monotonic cursors
  across send → park → continuation** (first turn's terminal cursor above the
  continuation's would-be restart — the F1 scenario); approval expiry with
  fake timers just before/after `expires_at`, including across toggle-off/on.
- `scenario-intercept-transport.test.ts` — ownership routing per §6.2/§6.3
  against a stub delegate asserting exact delegation counts; pass-through on
  miss; **toggle-off suite: Stop, approve/deny, dialog progress/close,
  completed and failed action reports on parked mock state — zero delegate
  mutations** (F2); interleave orders: send while `mock-parked` settles the
  card then proceeds, mock resume while `delegate-active` throws (F3);
  claimed placeholder lifecycle: mock-first new chat claims, unmatched send
  gets the fallback turn and never delegates, delete, reload semantics (F4);
  anchored merge chronology `mock → live → mock` and repeated alternation
  (F7); snapshot serving: zero delegated `getHistory` during a 20-event
  scripted stream, wire-log store untouched (F8); metadata overlay: sidebar
  title/recency/ordering after a mock-first turn (F9); loading/error states
  throw with zero delegate calls (F6); journey verification: slug mismatch →
  failed branch + world untouched, lookup failure → unverified note (F5) —
  exercised against the real `GET /keys/{id}` path and `KeyInfo` shape incl.
  match/mismatch/404/malformed and cancel/delete during the lookup (P1/P2);
  delete cancel-first with a delayed DELETE while timers advance (F13) **and
  a rejected DELETE**: tombstone survives failure, later send/resume/patch/
  history/list on that conversation stay inert (P10); list snapshot: zero
  delegated list GETs during a >5 s script, exercised through the real
  Aevatar delegate with mocked HTTP and a stale list TTL (P7);
  `message_count` asserted identically in history and list projections
  (P12).
- `assistant-mock-scenarios-store.test.ts` — persistence shape, version,
  world mutations, reset; account-switch rehydration resets a foreign user's
  persisted world (F14).
- `transport-shell.test.ts` — via an injectable installer/loader seam: bare
  delegation before install, in-place interception after, import failure,
  idempotent install, full-mock non-installation (P5); auth lifecycle
  null → user A → logout → user B rescopes world in one module lifetime
  (P6/F14).
- `mock-scenarios-action.test.tsx` — gated rendering, toggle flow incl.
  loading **and error** states, matched/unmatched `lastActivity` rendering,
  per-row switches actually driving `disabledScenarioIds` (a disabled
  scenario stops intercepting via store round-trip), chip removal/reset,
  empty world, the real-journey warning copy, and both mount points
  rendering inside local Suspense without blanking the shell (P8/P9/P11).
- `use-assistant` integration case — scripted send → blocked turn → simulated
  `continueActions(completed)` → continuation renders; asserts the #1304
  preservation path keeps the parked card across a refetch (with overlay).
- `scenarios.config.test.ts` — config loads, names unique, regexes valid, every
  referenced flow exists.
- Build-artifact assertion (§8.3) — prod `dist/` free of mock symbols (F12).

CI note: pure frontend change; runs under the existing `npm run build`
(tsc -b with `noUncheckedIndexedAccess`) + vitest gates. No wizard-bundle
impact (no dep/lockfile change).

---

## 13. File-by-file change list

| File | Kind | Est. size |
|---|---|---|
| `frontend/src/lib/assistant/scenario-engine.ts` | new | ~450 lines |
| `frontend/src/lib/assistant/scenarios.config.ts` | new | ~80 lines |
| `frontend/src/lib/assistant/scenario-intercept-transport.ts` | new | ~350 lines |
| `frontend/src/stores/assistant-mock-scenarios-store.ts` | new | ~140 lines |
| `frontend/src/components/assistant/mock-scenarios-action.tsx` | new | ~180 lines |
| `frontend/src/lib/assistant/transport.ts` | edit | ~20 lines (delegating shell + dev install) |
| `frontend/src/lib/assistant/transport-shell.test.ts` | new | ~150 lines |
| `frontend/src/pages/assistant.tsx` | edit | ~8 lines (dev-gated lazy + local Suspense) |
| `frontend/scripts/assert-mock-footprint.mjs` | new | ~40 lines |
| `frontend/package.json` | edit | scripts-only: append footprint step to the FULL existing build chain (credential-accept stage preserved) |
| `docs/chat/README.md` + spec/plan status headers | edit | index + status flip |
| tests (8 files incl. lifecycle, integration, footprint) | new | ~1,200 lines |

Estimated total: ~2.5 k lines including tests. One PR, no backend involvement.
This table is the **authoritative diff scope** for implementation and the
final gate — WP7/WP8 files included by construction.

---

## 14. Open questions for Calvin

1. ~~**Prod/preview access** — is DEV-only right for v1, or do you want the
   capability-flag gate now so deployed previews can use it?~~ **Answered
   (2026-08-04):** flag gate. Shipped as `experimental:assistant-mock-scenarios`,
   default off, admin-toggled — see §8.3.
2. **Fallback scenario** — in *real* conversations an unmatched message
   passes through to Aevatar (specced). Claimed mock-only chats (§6.5)
   already give you a fully offline surface. Do you additionally want a
   "strict" sub-toggle that blocks pass-through in real conversations too?
3. **World seeding** — should `world.connected` optionally seed from the real
   `/user-services` list on toggle-on, so `need` reflects your actual account?
   (v1: starts empty, mock-owned.)
4. **Scenario set** — beyond connect-github and gh-issues, which flows do you
   want in the initial config? (approval-card flow? failure/`fail` flow for
   error-state QA?)
5. **AddKeyDialog locking** — when launched from an action card, should the
   dialog be locked to the requested catalog service (product change, also
   benefits the real assistant flow), or is the specced post-hoc verification
   (§6.6) enough for v1?

---

## 15. Adversarial review log (CodexSol)

CodexSol (codex / GPT-5.6-Sol) reviewed draft v1 against the ground-truth
sources; full text in `mock-scenario-intercept-spec.review.md`. Verdict on
v1: "implementable in principle, but not safely implementable as written."
All 14 findings accepted; every cited evidence line spot-checked or
consistent with reviewed code. Dispositions:

| # | Sev | Finding (short) | Disposition → spec change |
|---|---|---|---|
| F1 | high | Turn-scoped cursor resets are swallowed by `applyTurnEvent`'s `<= lastCursor` guard | Accepted → conversation-monotonic cursors (§5.1, §10) |
| F2 | high | Toggle-off delegation POSTs mock action reports upstream; Stop misses running mocks | Accepted → ownership routing independent of toggle (§6.1) |
| F3 | high | `blocked` isn't "active", so live and mock turns can interleave | Accepted → ownership state machine; parked cards settle before newer sends (§6.2) |
| F4 | high | Placeholder→canonical aliasing orphans overlay + continuations | Accepted → claimed mock-only placeholder conversations (§6.5) |
| F5 | high | Completed journey may have connected a different service than requested | Accepted → post-completion service verification before world mutation (§6.6); dialog locking raised as §14.5 |
| F6 | med | Engine-not-ready pass-through sends intended-mock content to the live assistant | Accepted → `MockScenariosLoadingError` thrown instead of delegating (§6.7) |
| F7 | med | Tail-append overlay corrupts chronology after later live turns | Accepted → anchored merge (§6.4) |
| F8 | med | Per-event re-projection triggers real transcript GETs + wire-log entries | Accepted → base-snapshot serving during mock activity (§6.4) |
| F9 | med | Title/recency/count metadata stale for mock activity | Accepted → metadata overlay in history + listing (§6.4) |
| F10 | med | Multi-card awaits have no coherent continuation semantics | Accepted → v1 compiler restriction: one resumable card per await (§5.1) |
| F11 | med | Approval expiry is display-only; expired cards could resume scripts | Accepted → lazy decision-time expiry enforcement (§5.5) |
| F12 | med | Tree-shaking claim unsupported by the static import graph | Accepted → delegating shell + dev-only dynamic install + build-artifact assertion (§8.3) |
| F13 | med | Delete ordering lets timers write through an in-flight deletion | Accepted → cancel-first + tombstone before delegating (§6.3) |
| F14 | low | World state not account-scoped; tabs race | Accepted → userId-scoped persistence; tab last-write-wins accepted and documented (§7) |

**Round 2 (plan review).** CodexSol reviewed the implementation plan
(`mock-scenario-intercept-plan.review.md`; findings referenced here as
P1–P11 in file order, P12 = the message-count finding). All accepted. Four
corrected THIS spec: P1 verification endpoint → `GET /keys/{id}` with
`KeyInfo.catalog_service_slug` (§6.6); P2 synchronous verifying-state +
provisional cancellable handle (§6.6); P7 list snapshot joins the history
snapshot (§6.4); P12 `message_count` semantics defined (§6.4). Also folded:
P8 local Suspense (§8.1), P9 real-journey warning copy (§8.2), P10
tombstone-on-failure anti-pattern note (§6.3), P5/P6/P11 new test
obligations (§12), and the §13 scope table expanded to include the WP7/WP8
files the plan's original ground rules contradicted (P3), with the
credential-accept build stage explicitly preserved (P4).
