# Agent activity states on avatars (chat streaming view and sidebar)

The renderer decision in this historical plan is superseded by
[Avatar motion direction](2026-11-26-avatar-motion-direction.md).
The current runtime contract is in [Agent avatars](../chat/13-agent-avatars.md).

Status: implemented and verified; all review findings R1-1 to R6-6 closed.
PM/reviewer: Claude Opus 5.5 (owns this file and the review ledger below). Implementer: one Heca teammate, gpt-6.2-sol xhigh
(owns product code, tests and product docs). No pushes, PRs, merges,
deployments or publication.

Baseline: the checkout already contains the completed but uncommitted face-kit
work (`docs/plans/2026-11-25-nyxbot-face-kit{,-evidence}.md`). It is
snapshotted, without touching the working tree or index, as the local ref
`refs/avatar-states/baseline` (commit `4b917c3e65c0`, tree `bab9ea323e21`).
Every change in this stream is measured against that ref.

## Request and current decision (Rive reconciliation)

User, 2026-11-25: when specialists are thinking or working, show the states on
the avatar itself, in the main chat streaming view and in the sidebar, "as part
of the Rive AI piece".

The current, owner-approved decision is recorded in
`docs/chat/13-agent-avatars.md`, `DESIGN.md` (Agent avatars) and the face-kit
plan: the owner-supplied face-kit reference replaced the orb/Rive design with a
shared portable composition drawn by web SVG/Canvas and native
`react-native-svg`. The Rive runtime, rig, build hook and CSP allowance were
removed (`frontend/src/lib/assistant/avatar-rive*.ts`, `avatar-rigs.ts` and the
fixture are deleted in the baseline). No `.riv` asset exists in this checkout.

This stream therefore treats "the Rive AI piece" as the avatar initiative and
does not revive Rive art or runtime. No runtime change is needed: every new
treatment below is a static vector mark, a whole-face transform or an opacity
change, all of which the existing SVG/Canvas/RN-SVG renderers draw today. The
state contract is renderer-independent (`AvatarPresentation` below), so a future
Rive adapter could bind `state` to a state-machine input and `badge` to its own
art without changing facts, precedence or aggregation. Nothing here claims a
Rive file or adapter exists.

## Gaps found in the baseline

- **G1 – compact/static states are indistinguishable.** Below 96px every active
  state draws the same accent ring (`draw.ts`, `dot(256,256,238)`); static
  frames are `rest` for thinking/working/drafting/waiting/retry. The sidebar
  (20px), rail (24px) and compact chat header (20px) are static, so all active
  states look identical there, and also under Still/Reduce Motion everywhere.
- **G2 – quiet motion is indistinguishable.** At 24–95px (chat header 56px,
  home 56/72px) the quiet analogues (`thinking-pulse`, `working-accent`,
  `drafting-pulse`, `slow-pulse`) all apply the same scale pulse
  (`sin(t·2.4)`, amplitude 0.045).
- **G3 – the sidebar/rail/home/details have no state.** Sidebar and rail pass no
  mood; home maps `status === "running"` to `thinking`. The roster only carries
  a boolean `running`, so thinking vs working cannot be shown honestly.
- **G4 – no consistency between the open chat and the roster.** The open chat's
  live SSE state is fresher than the roster; the same agent can disagree.

## State contract

States are the existing `ProcessState` names (`shared/agent-avatars/model.ts`);
no state is renamed and nothing is persisted. User-facing names are in
parentheses.

| State | Evidence (only these) | Single-turn precedence | Enter / exit |
| --- | --- | --- | --- |
| `stopping` (Stopping) | `active_turn.stop_requested` on a live turn | 1 | Stop accepted → turn settles. Not proof execution ended. |
| `approval` (Needs you) | Pending owner action: conversation `pending_acknowledgements`, pending chat acknowledgements/approvals; roster `agent.pending_acknowledgements` | 2 (also without a turn) | Card raised → decided/expired. |
| `waiting` (Waiting) | `queued` or `waiting_for_capacity` | 3 | Admission/capacity wait → claimed. |
| `working` (Working) | Any activity with status `running` | 4 | Tool starts → no running tool. |
| `drafting` (Writing) | Reply text non-empty (server `reply.text` or this page's streamed text) | 5 | First reply text → turn settles. |
| `retry` (Retrying) | `continuations > 0` or ≥2 errored activities | 6 | Neutral recovery treatment, never "stressed". |
| `thinking` (Thinking) | Live turn with none of the above; also the honest fallback for "active, detail not available" | 7 | Turn admitted → a more specific fact. |
| `success` | Open chat only: the watched turn settled to a completed reply (existing `settleAvatarMood`) | settled | One-shot ≤1.6s, never on history load, switch, refocus or reload. |
| `failure` | Open chat only: latest settled reply is `error` | settled | Until the next turn starts. Cancellation is neutral (idle). |
| `idle` | None of the above | — | — |

Voice (`listening`, `speaking`), transport (`connecting`, `reconnecting`,
`connected`), `created` and `choosing` keep their existing separate scope and
sources (actual capture gate, playback, voice-session transport, create
success, editor). They gain no badge and are not derived from text.

### Cross-conversation aggregation (sidebar, rail, home, details)

An agent's roster state is computed from the viewer's own live threads of that
agent, then ranked:

`approval > working > drafting > retry > thinking > waiting > stopping > idle`

Rationale: an owner action first; then real progress; a stalled or stopping
thread never hides another thread that is genuinely progressing. Roster
surfaces never show `success`/`failure`: they cannot judge a reply the viewer
is not watching (documented visibility limit). `destroyed` agents are idle.

Sources, in order:

1. **Open conversation override.** The chat page publishes its live resolved
   state for exactly `(account, conversation id, agent id)`; it replaces that
   conversation's roster entry. Cleared on navigation, conversation switch,
   unmount, account change and logout.
2. **Roster projection.** Additive `activity` on each agent summary: the
   viewer's live threads as `{conversation_id, state}` (bounded), computed on
   the server from the conversation rows the roster already loads
   (`running_agents`, shared-agent `summaries`). No extra query or polling.
3. **Event-driven activity read.** The live stream already emits an
   identifier-only `conversation` event for every non-heartbeat conversation
   write, including tool-activity and reply-text writes, but the client only
   refreshes the roster when `turn_id` or the message count changes
   (`use-nyxagent-live.ts`), and polling backs off to a 30 s backstop while the
   stream is live. A mid-turn thinking → working → drafting change would
   therefore reach the sidebar up to 30 s late. Refreshing the roster on every
   such event would multiply its existing per-agent reads, so a cheap
   `GET /api/v1/assistant/nyxagent/agents/activity` (same human-only router,
   engine flag and acting person as `/agents`; one indexed read, the same rows
   as `running_agents`) returns only `{agent_id, threads:[{conversation_id,
   state}]}`. It has no `refetchInterval`: it loads once and is invalidated
   only by live `conversation`/`group` events (coalesced). When the stream is
   not live, the roster's existing 3 s running poll carries the same field.
   The client uses whichever snapshot is newer. A 404 (older server) disables
   it.
4. **Fallback.** No `activity` field (older server) but `status === "running"`
   → `thinking` (the active fallback). Otherwise idle.

Visibility limits: only threads whose `user_id` is the acting person are read
(same rows as today's `running` flag), so org and shared agents report only
the viewer's own threads; collaboration threads are excluded as today.
Group crew views keep `thinking`/`approval` for `working_agent_ids` (member
hidden-thread detail is not projected; collaboration groups would leak other
requesters' work). The roster does not include voice or transport state.

## Presentation (`AvatarPresentation`)

A pure shared function returns, for `(state, size, motion preference, reduced)`:
`{ state, badge: { glyph, tone } | null, motion signature, oneShot, staticFrame }`.

- **Badge** (new): a small overhanging corner mark drawn from shared vector
  geometry (24-unit glyphs), rendered by each client as a sibling of the face,
  so the face composition, caches and PNG export are unchanged. Present for
  `thinking` (ellipsis), `working` (tool/bolt), `drafting` (text lines),
  `waiting` (hourglass), `approval` (exclamation), `retry` (circular arrow),
  `stopping` (square), `success` (check), `failure` (neutral cross). Tones use
  existing theme tokens: active → primary, approval → warning, success →
  success, waiting/stopping/retry/failure → neutral; no destructive red.
  Glyphs are pairwise distinct and legible at the 20px sidebar, 24px rail and
  20px compact header; the badge never animates.
- **Static/Still/Reduce Motion**: badge plus the state's static face frame
  (approval attentive eyes, stopping calm, success happy, failure calm grey).
- **Quiet (24–95px, motion allowed)**: each active state gets a distinct motion
  signature (period/amplitude/transform), asserted pairwise distinct in tests.
- **Rich (≥96px)**: unchanged existing reactions.
- With `assistant:avatars` off, nothing changes (legacy mark/initials and
  existing status dot).
- Where a corner status dot already exists (sidebar specialist row, rail
  running dot), a non-idle badge replaces it in that corner while avatars are
  on; idle/destroyed keep the existing dot. Status text, cards, counts and Stop
  remain unchanged and authoritative. The avatar stays decorative; the agent
  row's accessible name carries the translated state phrase instead of the raw
  `running`/`idle` word.

## Surfaces

| Surface | Size | Source | Treatment |
| --- | --- | --- | --- |
| Chat header expanded | 56px animated | page live facts + settlement | quiet motion + badge |
| Chat header compact (sticky while reading the stream) | 20px static | same | static frame + badge |
| Sidebar agent row | 20px static | aggregation | badge replaces dot when active |
| Collapsed rail agent | 24px static | aggregation | badge replaces running dot |
| Rail chats popover header | 20px | aggregation | badge |
| NyxBot home greeting / roster | 72px / 56px animated | aggregation | quiet motion + badge |
| Agent details header | 32px animated | aggregation | quiet motion + badge |
| Group working member | 24px animated | `working_agent_ids` + pending | thinking/approval + badge |
| Voice panel / banners | existing | voice/transport facts | unchanged, no badge |
| Mobile chat identity header | xl animated | existing native mood facts | badge |
| Mobile agents list, NyxBot home | native sizes | aggregation (roster) | badge (+ quiet where focused) |

## Acceptance matrix

| # | Requirement | Verification |
| --- | --- | --- |
| A1 | Single-turn precedence and fallbacks exactly as tabled | shared fixture run by TS (web + mobile) and Rust |
| A2 | Cross-thread rank, override, fallback, destroyed, no roster success/failure | shared aggregation tests |
| A3 | Server projection uses existing rows only; viewer's own threads only; bounded; old clients unaffected | Rust tests (owner, org member, shared recipient, other account, collaboration excluded, bound), query count unchanged |
| A4 | Open-conversation override keyed by account/conversation/agent; cleared on switch, unmount, logout | hook/store tests incl. cross-account |
| A5 | Badges distinct per state; static at all sizes; legible at 20/24px | geometry tests + browser screenshots at real sizes, light/dark |
| A6 | Quiet motion signatures pairwise distinct; Still/Quiet/Reduce Motion/hidden/offscreen respected live | draw/scheduler tests + browser emulateMedia |
| A7 | One-shots: no success before final reply, no replay on load/switch/refocus/reload; cancel neutral; failure honest | lifecycle + chat page tests, browser reload |
| A8 | Transition ordering in a real chat: thinking → working → drafting, retry, stopping, approval, settle | browser e2e with fixture transport, screenshots per state |
| A9 | Sidebar/rail/home/details reflect aggregation incl. multi-thread and stale roster vs live chat | component tests + browser screenshots |
| A10 | Existing status text, pending badges, cards, Stop unchanged; legacy flag-off unchanged | existing tests pass; flag-off snapshot |
| A11 | New copy in en, zh-CN, zh-TW (web) and mobile locales | lint i18n guards |
| A12 | Mobile parity for badge and roster aggregation | mobile vitest/jest + typecheck |
| A13 | Docs: `13-agent-avatars.md`, `DESIGN.md` updated; Rive decision recorded | review |
| A14 | Full frontend lint/test/build, wizard freshness/CLI test if closure touched, backend build/clippy/tests, mobile checks | command log in ledger |

Evidence directory: `/tmp/avatars-review/agent-states/` (mirrored to
`~/avatar-states-run/evidence/` because `/private/tmp` is cleared on reboot).

## Review ledger

| ID | Severity | Finding | Fix | Verification |
| --- | --- | --- | --- | --- |
| R1-1 | Medium | U1 `agentRosterMood`: with no `activity` field (older server) a watched override replaces the coarse `running` signal entirely, so `running` + override `failure`/`idle` yields idle and `running` + `stopping` yields stopping, hiding activity that may belong to another thread. | Fixed: `running` without `activity` adds a `thinking` candidate ranked with the watched override. | Fixture cases (stopping/failure/idle keep Thinking; Writing outranks; failure alone idle); PM reran web/mobile contracts. |
| R1-2 | Low | U1 override carries no agent identity; a caller passing another agent's open conversation would contaminate this agent's rank. | Fixed: `agentId` on input and override; mismatched override ignored. | Fixture collision case; PM reran. |
| R1-3 | Medium | U1 quiet signatures are only numerically distinct. Lifts of 4/6/10 units in 512-unit space are 0.2–1.1 px at 24–56 px (web applies `motion.y` in 512 units), so approval≈drafting (2 s) and waiting≈stopping (6 s) are perceptually identical; the test compares JSON strings. | Fixed: Redesigned: thinking 3 s breathe, working 1.2 s pulse, drafting horizontal sway (new `x` channel in SVG/Canvas), retry tilt, approval lift with steady bright ring, waiting ring-only, stopping static. | Pixel-space pairwise test at 56 px (≥1.5× cadence or ≥1 px channel difference, every channel ≥1 px); PM checked values (sway 1.5 px, lift 1.6 px, tilt ≈2 px, scale 1.1–1.6 px). |
| R1-4 | Low | U1 badge glyphs at 10 px / 1×: thinking dots (r 1.7) faint, hourglass strokes muddy, retry marginal (`~/avatar-states-run/evidence/review/u1-badges-light-dpr1.png`). | Fixed: Larger dots (r 2.3), filled hourglass, heavier retry, larger glyphs; geometry extent test; U3 must render ≥12 px badges at 20/24 px avatars. | PM re-rendered at 8–48 px, 1×/2×, light/dark: `~/avatar-states-run/evidence/review/u1fix/`. |
| R1-5 | Nit | `activity-fixture.json` lacks the `description` its sibling `mood-fixture.json` uses to name its consumers (web, mobile, Rust in U2). | Fixed: Fixture `description` names web, mobile and Rust consumers. | Read. |
| R1-6 | Low | Quiet activity signatures now replace a saved reaction's quiet analogue (e.g. Moving light → Blink) for live states; this contract change is undocumented in code and docs. | Fixed: Comment at the draw.ts override point; contract tests for per-state Still and Moving light (quiet vs rich). | PM reran; docs in U5. |
| R1-7 | Nit | `shared/agent-avatars/draw.ts` was Prettier-clean in the baseline and fails `prettier --check` after U1. | Fixed: draw.ts formatted; only activity lines differ from baseline. | PM `prettier --check shared/agent-avatars/*.ts` clean. |
| R2-1 | Medium | U2 shared agents: roster `activity` comes from the shared-thread query (excludes group, channel, voice-child and automation threads) while the endpoint uses every live acting-person row. With newer-snapshot selection, a shared agent working only in such a thread alternates between running and idle. | Fixed: One six-field shared-thread predicate (`shared_roster_thread_filter`/`is_shared_roster_thread`) drives the shared roster query and the endpoint. | `assistant_activity_roster_and_snapshot_agree_across_agent_and_thread_types` (Mongo filter = in-memory predicate; roster = endpoint for personal home+crew, org, shared home; shared crew/voice/automation/channel excluded). PM reran: pass. |
| R2-2 | Low | U2 `GET /agents/{id}` changed the existing `threads[].running` for destroyed agents: the live stopping turn kept by destroy now reports `running: false`. | Fixed: `running` = live turn again (incl. destroyed agent's stopping turn); only `activity` is null for destroyed agents. | Details test with a persisted stopping turn after destroy; PM reran: pass. |
| R2-3 | Medium | U2 model-facing contract change: the maintainer branch of `list_subagents` serializes the whole `AgentSummary` (`json!(r)`), so NyxBot tool results now carry `activity` (up to 200 conversation ids, including hidden crew threads). | Fixed: `AgentSummary.activity` is `#[serde(skip)]` (DTO, not a Mongo model); REST `AgentResponse` opts in explicitly. Tool paths audited (list/wait/read/grant/guest grant). | Same test asserts `nyxid__list_subagents` rows (maintainer and use-only) have no `activity`; PM reran `handlers::assistant_team::tests` 55/55, grant 6/6. |
| R2-4 | Nit | `live_agent_activity` calls `shared_agent_service::load(Some(&ids))` even when `ids` is empty; correct only through the loader's early return, and the marker says it is called only for referenced shared specialists. | Fixed: Loader called only for non-empty eligible shared ids. | Code read. |
| R2-5 | Medium | Baseline (not U2) leaves `cargo fmt --all -- --check` red: 6 hunks in `assistant_team.rs:3756`, `assistant_team_tests.rs:5768`, `assistant_notification_service_tests.rs:3941/3961`, `shared_agent_tests.rs:2937/2985`; the HEAD versions of these files are clean. Same class as the clippy tails. | Fixed: rustfmt on exactly the four baseline files; formatting only. | PM `cargo fmt --all -- --check` exit 0; notification 44/44, team 55/55, shared 48/48 pass. |
| R2-6 | Info | Baseline `clippy::needless_update` tails (3) removed under PM authorization; CI clippy invocation exits 0. | Accepted | Worker log `u2-clippy-final.log`; PM re-runs in U5. |
| R3-1 | Medium | U3 labels the approval state "Needs you", but the roster count it reads (`pending_acknowledgements` = acknowledgements with `decider: "orchestrator"`) is requests awaiting NyxBot, and the chat's conversation count includes both deciders. The label claims owner action the data cannot support. | Fixed: Label is the existing neutral "Waiting for approval" everywhere it is shown or announced. | Sidebar test for singular/plural announcement; PM reran 295 web tests. |
| R3-2 | Medium | Background owner cards never reach the sidebar: server per-thread states ignore pending acknowledgements, so a specialist blocked on an owner card shows working/thinking (live turn) or nothing (no live turn). | Fixed: One viewer snapshot (live rows + one bounded pending-acknowledgement aggregation + at most one projected conversation read for non-live card threads) feeds REST roster, shared roster and endpoint; approval-only entries never set `running`; tool paths keep live-only `running_agents`; new index `{user_id,status,expires_at,conversation_id}` on acknowledgements. Client uses `pendingOwnerActions` only without `activity`. | `assistant_activity_pending_cards_are_scoped_and_do_not_make_an_idle_agent_busy` (scoping, expiry, idle status, list tool shape, wait settles, stop > approval); bound test; PM reran activity 20/20, team 55, grant 6, shared 48, group 48, notification 44, platform models 14, 15 wait/subagent tests. Proxy approvals in background threads: open-chat-only visibility limit (docs in U5). Staleness: signalled changes ≤ ~1 s + latency; unsignalled card expiry stays until the next existing refresh (same as the pre-existing pending badge). |
| R3-3 | Low | With avatars on, a resolved idle state can sit next to the legacy green running dot (row) or rail running dot when the roster `status` is stale relative to the open-thread override; avatar, label and dot disagree. | Fixed: Avatars on: idle dot for resolved idle; rail running dot only when avatars are off. | "Uses the resolved idle state for stale running dots"; PM reran. |
| R3-4 | Low | The open chat header is forced to idle for destroyed agents, hiding an honest stopping/failure state in the transcript the viewer is reading. | Fixed: Header shows the honest page mood for destroyed agents; no roster override published. | "Keeps the stopping mood in a destroyed agent's open chat"; PM reran. |
| R3-5 | Low | Agent-row accessible name concatenates hard-coded English (", specialist, ", "pending request(s)") around the newly translated state label, producing mixed-language names in zh-CN/zh-TW. | Fixed: Single translatable template with localized kind/status and existing plural keys in en/zh-CN/zh-TW. | check:i18n 1,350 files pass; PM reran. |
| R3-6 | Low | A single 404 (e.g. an old replica during a rolling deploy) disables the activity read for the rest of the session. | Fixed: ready/resync resets a 404-disabled activity query once. | "Retries an unavailable activity endpoint once on a live resync"; PM reran. |
| R3-7 | Low | Activity invalidation rides the 150 ms transcript flush on every conversation event, including reply-text writes, allowing several activity reads per second per streaming user. | Fixed: Activity refresh on its own 1 s coalesced flush with in-flight deferral; notification events also schedule it. | Fake-timer cadence test; PM reran. |
| R3-8 | Nit | `placeholderData: undefined` in `useAgentActivity` is a no-op. | Fixed: Removed. | Code read. |
| R4-1 | Nit | `use-assistant-nyxagent.ts` (not Prettier-formatted at HEAD; Prettier is not enforced for the frontend) had three untouched `invalidateQueries` lines rewrapped, and the new activity invalidation omits `cancelRefetch: false` used by its siblings. | Fixed: original single-line calls restored; activity invalidation passes `cancelRefetch: false`. | Diff vs HEAD shows only the new lines; PM read. |
| R5-1 | Medium | U4 replaced existing visible mobile status text instead of supplementing it: chat header status ("Running"/"Needs you" → activity label), agent list status line ("● Working"/"● Ready" → activity label), home badges ("Needs you" → "Waiting for approval", flag-dependent translation of Running/Idle) and the crew live-region announcement ("Agents working. …" → "{name}, Thinking", which overclaims from the coarse group fallback). | Fixed: Original visible strings restored (chat header, list status line, home pills/badges, crew live region); only avatar wiring and additive translated state in list/home row accessibility labels remain. | Flag on/off component tests assert original strings; PM: ChatIdentityHeader.tsx and GroupChatScreen.tsx identical to the committed chain; PM reran mobile node 32, Jest 30 (4 suites), tsc. |
| R5-2 | Low | Native ring layer is located as "the last path of the draw list", an implicit coupling to draw.ts painter order. | Fixed: Ring path tagged `role: "activity-ring"` in the shared draw list; native selects by role. | Shared contract (ring exists only with a live signature; absent in reduced/small/rich/idle/terminal/Still/resting); PM reran web avatar suites 85/85, catalogue check. |
| R5-3 | Low | Web/mobile Chinese copy diverges for the same new phrase ("Retrying": web 重试中, mobile 正在重试). | Fixed: Mobile Chinese phrases aligned with the web catalog. | Read both catalogs. |
| R5-4 | Nit | `use-assistant-nyxagent.ts` pending-count invalidation of the activity query omits `cancelRefetch: false` used by every sibling. | Fixed: `cancelRefetch: false` added, single line. | Read. |
| R5-5 | Medium | U4 changed a Reanimated UI-thread worklet path and native SVG overlay; only Jest (JS fallback) and a Babel transform check ran. Native runtime evidence is required. | Fixed: Isolated native review entry (`mobile/review/activity-states-entry.tsx`, type-checked, not imported by the app) ran the production AgentAvatar on a task-local Android API 37 Hermes release build (both themes × automatic/saved Still/Reduce Motion, UI-thread worklet samples, PID-scoped logcat without Reanimated/Worklets errors) and on an iOS 27.2 simulator (light/dark automatic). iOS numeric samples were not captured by the simulator log; the Android run carries them. | `~/avatar-states-run/evidence/native/` (screenshots, samples, logcat, build logs); PM viewed the Android gallery and measured every native badge tone ≥5.88:1. |
| R6-1 | Nit | `mobile/src/features/assistant/NyxBotHome.tsx`: the updates list's `return (` is indented two spaces deeper than its sibling statements after the mood line was added. | Fixed: Indentation fixed. | Read. |
| R6-2 | Nit | `13-agent-avatars.md` roster paragraph ends the precedence list as a sentence fragment ("…in this order. Approval, working, …"). | Fixed: Single sentence. | Read. |
| R6-3 | Low | The DESIGN decision row and the renderer-contract section do not state the reconciliation explicitly: requested under the Rive initiative, no Rive runtime/rig/.riv is shipped, and a Rive adapter would bind `avatarPresentation` (state → state-machine input, badge → its own art). | Fixed: DESIGN decision row and renderer contract state the Rive request, the SVG/Canvas/native SVG renderer, no Rive runtime/rig/.riv, and the `avatarPresentation` adapter boundary. | Read. |
| R6-4 | Medium | Web dark theme: primary-tone glyphs (thinking/working/drafting) render #5A2AF1 on the #0B0B0D badge disc, measured 2.84:1 (below 3:1 non-text contrast); light theme is ≈7:1. Evidence: `~/avatar-states-run/evidence/review/u5/header-dark-sheet.png`. | Fixed: Web tones: `text-primary-text`, `text-warning light:text-amber-800`, `text-success light:text-emerald-700`, `text-foreground` (existing tokens/variants with precedent). Native unchanged. | Web unit test resolves classes to theme tokens (≥4.5:1 both themes); mobile theme test (4 palettes); PM measured raw crops: light success 4.88, light warning 5.01–6.45, dark primary 7.92, dark warning 8.33, neutrals ≥15.58; native ≥5.88. |
| R6-5 | Medium | Browser legibility evidence at the 20 px sidebar, 24 px rail and 20 px compact header covers only working and waiting in light theme; acceptance A5 needs every badge state in both themes (1× and 2× DPR), plus the 32 px details header. | Fixed: 144 clipped crops (9 states × sidebar 20 / rail 24 / compact chat 20 / details 32 × light/dark × DPR 1/2; terminal states in chat, roster idle by design) and four contact sheets from the real fixture. | PM viewed `badge-dark-dpr2-sheet.png` and `badge-light-dpr1-sheet.png`; all states distinguishable at every size. |
| R6-6 | Low | Commit hygiene: the `assistant-chat-page.toolbar.test.tsx` mock fix belongs with the web U3 change that broke it (commit aa53faffe fails that test on its own). PM will fold it into the local chain when recording U5. | Fixed (PM): local chain rebuilt so the mock fix is in the web commit (4141ed314); later commits inherit it. | `git diff --stat b4a0ba281 19a5ca5c1` = only the toolbar test file. |

## PM final verification (tree `b409280b`, 2026-11-26)

Run by the PM on the final tree after every finding was closed; the tree was
unchanged afterwards. Logs: `~/avatar-states-run/pm-final-*.log`.

| Check | Result |
| --- | --- |
| `npm --prefix frontend run lint` | exit 0; 0 errors, 34 warnings (all pre-existing; the one in a touched file predates this work at HEAD) ; typography and i18n (1,350 files, 14,196 calls) pass |
| `cd frontend && ./node_modules/.bin/tsc -b` | exit 0 |
| `cd frontend && npx vitest run --maxWorkers=4` | 8,451 tests / 747 files pass |
| `npm --prefix frontend run build` | exit 0 |
| `AVATAR_ACTIVITY_EVIDENCE=~/avatar-states-run/evidence/pm-final npm --prefix frontend run test:e2e -- e2e/agent-activity-states.spec.ts e2e/face-kit.spec.ts --workers=1` | 10 passed; 178 captures |
| `npm --prefix mobile test` | 2,849 pass |
| `npm --prefix mobile run test:jest -- --runInBand` | 98 tests / 13 suites pass |
| `cd mobile && ./node_modules/.bin/tsc --noEmit -p .` | exit 0 |
| `node shared/agent-avatars/generate-face-kit.mjs --check` | pass |
| `python3 frontend/scripts/test-nginx-policy.py` | 5 pass |
| `git diff --check refs/avatar-states/baseline b409280b` | clean |
| `cargo clippy --workspace --all-targets -- -D warnings` | exit 0, no warnings (dependency future-incompat notice only) |
| `cargo fmt --all -- --check`, `cargo build -p nyxid` | exit 0 |
| `cargo test -p nyxid --bins assistant_activity` (PM disposable Mongo) | 20 pass |
| Same harness: `handlers::assistant_team::tests` / `grant_services_tests` / `services::shared_agent_tests` / `handlers::assistant_group::tests` / `services::assistant_notification_service::tests` / `handlers::assistant_platform_models::tests` / `avatar` | 55 / 6 / 48 / 48 / 44 / 14 / 13 pass |
| `cargo test -p nyxid-cli --test wizard_bundle_freshness` | 1 pass |
