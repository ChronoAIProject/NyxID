# Web UI rollup: #2217, #2314, #2218, #2050 and the #2001 localization gaps

Branch `nyxid-simple-issue-triage`, base `main` 31f3d8969. One draft PR. No
merge or deploy from this work.

## Dispositions of the original six

| Issue | Disposition |
| --- | --- |
| #2217 Dashboard online-node count | Fix in this rollup (Closes). |
| #2314 Suggested for you: Hide has no restore | Fix in this rollup (Closes). Adds one human-only backend route. |
| #2218 Telegram setup dialog clips focused Cancel setup | Investigate and harden in this rollup. Not reproducible on current main (see below). |
| #2050 Agent conversation list renders every thread | Fix in this rollup (Closes). |
| #2001 Approval/skill-review card localization | Stays OPEN. The app-language MVP from #2333 has real gaps; this rollup fixes them and says `Refs #2001`. Conversation-language following (option B) is not implemented and is not claimed. Audit comment: <https://github.com/ChronoAIProject/NyxID/issues/2001#issuecomment-6820664532>. |
| #2067 First-run wizard | Closed by root as NOT_PLANNED, superseded by #2172, #2350 and #2342, with the design deltas listed: <https://github.com/ChronoAIProject/NyxID/issues/2067#issuecomment-6820629428>. No work here. |

#2265 was closed earlier by the parent (email login is reserved for reviewers
and local testing). Mobile, pinning (#2051), empty states (#2066), the details
rework (#2065), gateway/voice stability and permission changes are out of
scope.

### #2001 audit (current main)

#2333 translated most approval, skill-review and receipt copy. These strings
still render in English with a Chinese UI:

- `nyxagent-acknowledgement-card.tsx`: the service and account card titles are
  template literals (the catalogs already hold `Allow this chat to use
  {{service}}?` and the other titles); the routed-request help text.
- `decision-receipt.tsx`: the receipt states (`Allowed`, `Denied`, `Used`,
  `Expired`, the two Full access labels), the decider name inside `{{name}}
  allowed`, three bypass reasons and the English relative time.
- `blocks/approval-card.tsx`: the decided titles `Approved`, `Request denied`,
  `Request expired`, `Request cancelled` (also on the Approvals page).
- `authored-skill-card.tsx`: interrupted-publication copy, post-action notices
  and receipt summaries.
- `nyxbot-labels.ts` `describeEventNotice`: client-authored notice summaries.

Residual, not in this rollup: the legacy Aevatar approval body fallbacks in
`chat-approval-presentation.ts` are computed when the event is converted, not
at render, so they cannot follow a language change without a different design.
The server-supplied body is the normal path.

### #2218 reproduction

`npx playwright test e2e/channel-telegram-new.spec.ts -g "survives reload and
a closed browser page"` passes at 1440×900 and 390px on main 31f3d8969. A
repeat matrix (`--repeat-each=5`) passed 15/15 at 1440×900 across Chromium
DPR 1, Chromium DPR 2 and Chrome stable. The report was against c8c74e0;
dialog typography and footer rules changed since (#2333, #2174). The
implementer explores other heights, languages and text sizes before deciding
between a root-cause fix and hardening plus a stronger regression check.

## Items, acceptance and ownership

Implementers are Heca `codex` sessions on `gpt-6.1-sol` (xhigh, full access) in
this worktree. Opus (PM) reviews every change personally and commits.

### A. #2217 and #2314 (Sol-A)

#2217: the Dashboard counts a node as online exactly when the Nodes page badge
shows it Online (`is_connected`), through one shared derivation. The `x/y
online` label is a translated whole phrase. Mock node data uses the API's
lowercase status values. The Studio ambient status line has the same casing
defect but turning it on would add a new alarm for every offline node; it is
recorded as a follow-up, not changed.

#2314:

- `POST /api/v1/assistant/nyxagent/onboarding/suggestions/show` (human-only,
  NyxAgent engine flag) clears `suggestions_hidden_at` on the caller's existing
  row and returns the refreshed state. It never upserts, and never changes
  answers, intro version, the NyxBot memory note or audit history. Reusing
  `PUT /onboarding` was rejected: it rewrites the memory note (recreating one
  the owner deleted), bumps the intro version and fails on unknown outcomes.
- Home: after Hide, the section is replaced by a compact restore row with an
  immediate Undo; after a reload the same row offers Show suggestions. Focus
  moves to the restore control, the change is announced politely, and the
  button is disabled while a request runs. Failures roll back to server state
  and show a localized retryable error. No restore row when there are no
  recommendations.
- Hide and Show mutations share one TanStack mutation scope so an Undo cannot
  reach the server before the Hide it undoes.
- Docs: `docs/NYXBOT_FIRST_RUN.md`, the CLAUDE.md route list.

### B. #2050 and #2218 (Sol-B)

#2050: an expanded agent shows its five newest regular conversations, plus the
open conversation, any conversation with a running turn and any conversation
holding a local draft, in list order. A `Show N more` / `Show fewer` toggle
(button, `aria-expanded`, keeps the mobile drawer open) expands the full list.
New chat, the channel-bot groups, folding, the compact rail and per-agent
pending badges are unchanged.

#2218: reproduce first across desktop heights (900, 800, 720), zh-CN, text
sizes and both themes. Fix the root cause if found; otherwise make the setup
dialog keep one scroll owner and add a regression check that tabs through
every focusable control at desktop and phone sizes. Keep clear of PR #2382's
Lark link changes in the same file and avoid broad dialog redesign.

### C. #2001 localization gaps (Sol-C)

Translate every client-authored string listed in the audit at render time,
reusing existing catalog keys and adding whole-phrase keys with placeholders
to `en`, `zh-CN` and `zh-TW`. Server text, service names, slugs and NyxBot stay
verbatim. Notice parsing still matches the English server notices. zh-CN
render tests cover each surface.

## Coordination

- File ownership is disjoint. Shared catalogs (`src/i18n/locales/{en,zh-CN,zh-TW}.json`)
  are edited only by anchored, exact-match insertions next to related keys,
  never at the end of the file and never by whole-file rewrites.
- Heavy commands (tsc, lint, i18n audit/sync, builds, broad vitest,
  Playwright on port 4611, cargo) run through
  `~/.heca/plans/wild-beach-rollup/heavy-lock.sh <owner> <cmd>`.
- Implementers do not commit, push, create PRs or spawn agents.
- Backend: `cargo fmt` locally; CI compiles and runs the backend tests (local
  swap is nearly full).

## Validation

- Frontend: `npx tsc -b --pretty false`, `npm run lint`, `npm run check:i18n`,
  targeted vitest per item, then the full vitest suite once.
- Playwright (mocked APIs only): Telegram setup spec, new Home and sidebar
  checks at 1440 and 390 widths with keyboard navigation, light and dark,
  English and Chinese where copy changed.
- Backend: `cargo fmt --all -- --check`; CI runs clippy and nextest.

## Review log

### Sol-A round 1 (#2314, #2217)

Approved:
- `show_suggestions` is a plain `update_one` on the caller's row, with no
  upsert and no answer, memory or intro-version writes.
- Tests cover preservation, absent rows and owner isolation.
- Hide and Show share a per-user TanStack mutation scope with snapshot
  rollback.
- The Home Undo is disabled while Hide is pending, so the UI cannot race it.
- The Dashboard count uses the badge's derivation, and the docs are honest
  about mobile.

Findings sent back:
1. `nodeStatusKey` was exported from a component file behind an
   `eslint-disable`; move it to `lib/status-contract.ts`.
2. The new focus targets (Undo, Show suggestions, Retry, Hide) have no visible
   keyboard focus because the ghost Button primitive suppresses outlines (seen
   in `hidden-undo-1440-light-en.png`). Add a local indicator following the
   `.agent-header-icon:focus-visible` precedent; leave the shared primitive
   alone.
3. A restore is not announced; add a polite "Suggestions shown." for restores
   in the current session.
4. Two chat-page test files mock the onboarding hooks without
   `useShowSuggestions` (27 failures). Sol-A is authorized to extend those
   mocks.
5. The two Playwright matrix failures coincided with Vite reloads caused by
   concurrent edits. A full rerun is deferred until the worktree is quiet.

### Sol-A round 2

Approved:
- `nodeStatusKey` now lives in `lib/status-contract.ts`, with no lint
  suppression.
- Undo, Show suggestions, Retry and Hide carry a local 2px input-focus outline.
  It is visible in `hidden-undo-390-dark-zh-CN.png`.
- "Suggestions shown." is announced after a session restore. Nothing is
  announced on load, while pending or on failure.
- The chat-page test mocks are extended (28/28 passing).
- Targeted unit tests pass (32/32), and both requested browser cases pass.

The backend commit `ee9f02c25` is pushed in draft PR #2414 for CI.

### Sol-C round 1 (#2001 gaps)

Approved:
- Every audited string now translates at render time:
  - service and account card titles, using the existing `{{service}}` keys;
  - routed help text;
  - receipt states, with whole-phrase "You allowed" / "NyxBot allowed" so the
    Chinese word order is correct;
  - bypass reasons;
  - decided titles;
  - skill-review notices and summaries;
  - every `describeEventNotice` template.
- Stored notices stay source keys until render.
- Fragment-concatenated sentences, which garbled Chinese word order, became
  whole phrases with placeholders: the approval countdown and grant duration,
  `via`, submitted decision, repeat-write explanation, coverage labels and the
  reduced-scope consent (plural).
- The relative time reuses `formatRelativeTime`.
- The English server-notice regexes are unchanged.
- 224 focused tests, tsc, lint, `check:i18n` and one multilingual Playwright
  case pass.

Findings sent back:
1. zh-TW `policy confirmation` used 政策; the catalog uses 原則 for software
   policies (確認原則, 存取原則, 核准原則).
2. Fifteen fragment keys became unreferenced; delete them (FI-007).

### Sol-C round 2

Approved:
- zh-TW `policy confirmation` is now 原則確認.
- Before deleting, Sol-C confirmed by fixed-string search that each of the 15
  fragment keys had no references. They were removed from all three catalogs
  (45 entries), and the English overrides needed no resync.
- `check:i18n`, tsc and the 224 focused tests pass.

### Sol-B round 1 (#2050, #2218)

#2218 evidence:
- The reported default-text clipping at 1440×900 still does not reproduce.
- Sol-B reproduced the same class of focus clipping at larger text sizes:
  - 1440×900 at 2×: Reopen Telegram at 0.998 visibility, its bottom 0.094 CSS
    px past the scrollport; the platform selector at 0.996.
  - 390×844 at 1.5×: Open Telegram at 0.991.
- Native focus scrolling lands on a fractional scrollport edge.
- `scroll-py-1` on the setup dialog's existing scroll owner gives it a 4px
  inset. After the fix, 120/120 matrix cases passed: five viewports × en/zh-CN
  × 1/1.5/2× text × light/dark × reduced motion on/off, with 840 focused
  controls and a minimum ratio of 1.
- The permanent spec now tabs through every control at 1440×900, 1440×720,
  390×844, and 2× text at 1440 and 390, asserting ratio 1 and no background
  scroll.

Approved:
- #2050 visibility: the five newest conversations plus the open, running and
  drafted ones, in list order.
- The toggle: aria-expanded/aria-controls, an inset input-focus ring, and it
  keeps the drawer open.
- ChannelThreadsGroup now uses the whole-phrase key.
- 64 unit tests, 12 sidebar browser cases and 25 Telegram cases pass.

Findings sent back:
1. AgentRow subscribed to the whole drafts map, so every row re-rendered on
   each 300 ms draft save while typing. Select drafted IDs with `useShallow`
   instead.
2. Comment the purpose of `scroll-py-1` so it is not removed as unexplained.

### Sol-B round 2

Approved:
- AgentRow selects drafted conversation IDs through `useShallow`, so a longer
  draft in an already-drafted chat changes nothing (new unit test).
- `scroll-py-1` carries its #2218 rationale as a comment.
- 65 sidebar tests, tsc, eslint and two browser cases pass.

## Final validation (quiet worktree, Opus)

Frontend, run from `frontend/`:
- `npx tsc -b --pretty false`: pass.
- `npm run lint`: 0 errors. The 34 warnings are pre-existing, in files this
  rollup does not touch. The typography guard and i18n audit pass.
- `npm run check:i18n`: 1,322 files, 14,125 static calls, pass.
- Full Vitest: 729 files, 8,306 tests, all pass.
- `npm run build`: pass, and no tracked files changed.

Playwright, all APIs fixtured:
- Changed areas: 53/53 pass, covering `nyxbot-suggestions-restore`,
  `assistant-sidebar-threads`, `channel-telegram-new` and `multilingual`.
- A temporary Dashboard check (1440 and 390, en and zh-CN) showed `2/2 online`
  and `2/2 在线` from mock nodes. The spec was deleted after the run.
- Related existing specs `nyxagent-authority` and `assistant-account-menu`:
  34 failures and 31 passes, with exactly the same failing titles on a clean
  `main` worktree at 31f3d8969. They are pre-existing: for example,
  `/billing/benefits` is not stubbed. Neither spec runs in CI.
  `onboarding-display` and `assistant-mobile-layout` pass.

Backend: `cargo fmt --check` passes locally; compile, clippy and tests run in
the #2414 CI.

## Milestones

- Audit of #2001/#2067 sent to root; #2001 audit comment posted; #2067 closed
  by root.
- Implementers spawned (Heca `codex`, `gpt-6.1-sol`, full access). Heca rejects
  an explicit effort for this model (it advertises no thinking options);
  `~/.codex/config.toml` sets `model_reasoning_effort = "xhigh"`, which applies.
  Sol-A `01a1cf41-e876-7f40-af39-823a54d0f9f3` (#2314, #2217); Sol-B
  `01a1cf42-389f-70e1-b746-975935f05af9` (#2050, #2218); Sol-C
  `01a1cf42-7a63-7160-a2c5-b58956b0c697` (#2001 gaps).
- Draft PR #2414 opened with the backend commit first, so CI compiles the new
  route while the web lanes finish.
- All lanes approved after two review rounds each. Final validation passed on a
  quiet tree, and the work is committed per issue.
