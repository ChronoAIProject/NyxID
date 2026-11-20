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

### #2218 closure basis

This PR closes #2218 because the issue's stated expectation now holds and is
enforced by tests. Every focused control in the Telegram setup dialog stays
fully inside the dialog without scrolling the page, at desktop and phone
sizes, at 1× text and with larger text. It does not claim to have reproduced
the original report or found its root cause:

- The reported default-text clipping (0.944–0.980 at c8c74e0, Chrome 156) does
  not reproduce on main 31f3d8969 or on this branch.
- The same failure mode does reproduce at larger text sizes. Without the fix,
  1440×900 at 2× fails at 0.998 and 390×844 at 1.5× fails at 0.991. Fable
  reproduced both independently: 0.99825 and 0.99119.
- `scroll-py-1` fixes those cases, and the tab-through regression fails when
  the padding is removed.

If the original default-text clipping reappears, #2218 should be reopened with
the new environment details. No production QA run is claimed.

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

## Independent review and remediation (2026-11-13)

Fable 5.1 reviewed head 638ae7fb7
(`~/.heca/plans/wild-beach-rollup/fable-independent-review-2026-11-13.md`). Its
verdict was "valid with bounded gaps", with no correctness, authorization,
data-preservation or compatibility defect. Following the remediation policy,
Opus checked every item against the code before planning.

### Findings and dispositions

- **F1 (low, introduced by the #2050 commit): fix.**
  `e2e/assistant-sidebar-threads.spec.ts:68,75` write screenshots to
  `../.tmp/sol-b/…`, a fixed path outside Playwright's output directory, so
  each run leaves untracked PNGs at the repository root. Confirmed by reading
  the spec.
  `e2e/channel-telegram-new.spec.ts:988–1072` writes to `../.tmp/telegram-ui/`
  in the same way. That predates this PR, but this PR changes the spec and
  runs it for #2218, so it is fixed in the same mechanical way.
- **F2 (low, closure evidence for #2218): wording only, no code.** The basis is
  recorded in "#2218 closure basis" above and in the PR body. No new issue
  comment, no issue state change, no claimed QA run.
- **O5 (low, introduced by the #2314 commit, not pre-existing): fix.** While an
  Undo or Show request is pending, the restore row unmounts and the disabled
  Hide button cannot take focus, so keyboard focus sits on `body` until the
  response arrives.
  - Reproduced with a throwaway unit test, deleted afterwards: after Enter on
    Show suggestions with the request pending, Hide is disabled and
    `document.activeElement === document.body`.
  - The Hide direction has a shorter version of the same gap: the clicked
    button is disabled before the optimistic flip.
  - This contradicts the #2314 acceptance above: focus must not be lost.
- **O6 (low, documentation made stale by #2314): fix.** The
  `suggestions_hidden_at` comment in `backend/src/models/assistant_onboarding.rs`
  still says the mark is cleared only by retaking the intro. The
  `NYXBOT_FIRST_RUN.md` sentence "pending controls are disabled" must also
  match the O5 behaviour.
- **O1–O4: no change.** They predate this PR and none affects the acceptance
  of an issue this PR closes:
  - O1: no app-wide Button focus ring. Fixing it is a design-system change
    that needs design approval.
  - O2: the ambient status line casing. Enabling it adds a new alarm, which is
    a product decision.
  - O3: other fragment-built "N more" strings.
  - O4: untranslated `telegram-prompt-status.tsx` sentences.
  They stay recorded as follow-ups and were not silently added to scope.

### Remediation acceptance

- **F1:** each screenshot path uses `testInfo.outputPath(…)` with the same file
  names. After running the full sidebar matrix (12 cases) and the Telegram
  spec, `git status` shows no `.tmp/`, and the screenshots exist under the run's
  output directory.
- **O5:** Hide, Undo, Show suggestions and Retry stay focusable while a request
  runs. They use `aria-disabled` with visually disabled styling and ignore
  repeat activation, instead of the `disabled` attribute.
  - After the optimistic flip, focus moves straight to the counterpart control
    (Undo or Show suggestions after Hide; Hide after a restore), even while the
    request is pending.
  - Focus is never on `body` during either request.
  - The serial mutation scope, rollback, Retry, announcements and focus ring
    are unchanged.
  - Unit tests cover both pending directions.
  - The browser spec delays each response and asserts the focused element
    during the delay.
  - Root's review checkpoint: `aria-disabled` does not block activation.
    Therefore every handler that can start a visibility mutation (Hide,
    Undo/Show suggestions, and Retry for both failures) enforces the pending
    guard itself. While a request is pending, repeated click, Enter or Space
    must start no extra write and no opposite operation. Tests check, with a
    deferred response, that focus stays on the intended control for the whole
    interval and is valid after both success and failure. In the browser,
    repeated Enter adds exactly one write per intended action, and a delayed
    failure leaves focus valid with a single-write Retry.
- **O6:** the model comment and first-run doc describe Show and the new pending
  behaviour.

Ownership: one Heca `codex` implementer (`gpt-6.1-sol`, full access, xhigh from
`~/.codex/config.toml`). Opus reviews it personally. No other reviewer agents.

### Remediation review log

Sol-R round 1 (F1, O5 with root's guard checkpoint, O6) is approved:
- `hideSuggestions` and `showSuggestions` return immediately while a request is
  pending. Retry reuses those same guarded handlers.
- Hide, Undo/Show suggestions and Retry use `aria-disabled` with a local
  disabled look, and keep their labels and focus ring.
- A layout effect keeps the focus intent through pending and rollback, so
  focus moves straight to the counterpart control.
- Unit tests check that click, Enter and Space are ignored while pending.
  Focus stays on the intended control throughout, after success and after
  rollback.
- The browser spec adds 600 ms response delays. A requestAnimationFrame
  watcher asserts zero frames with focus on `body`, repeated Enter adds exactly
  one write per action, and a delayed Show failure keeps focus valid with a
  single-write Retry (390, dark, en).
- F1 paths use `outputPath` in both specs. A full run of the three specs left
  no `.tmp`.
- The O6 comment and doc are updated.
- Targeted Vitest 50/50, tsc, lint and i18n, and Playwright 50/50 (12 + 13 + 25)
  all pass.

Opus review finding R1, sent back: if the visibility state flips while a
visibility control has focus, the focused control unmounts and focus falls to
`body`. This happens on a refetch after another tab changes the state, or when
a request succeeds on the server but its response fails and the refetch
reverses the rollback. Focus should move to the counterpart control, with no
focus stealing when focus is elsewhere.

Sol-R round 2 (R1) is approved:
- A document `focusin` listener remembers the focused visibility control.
- The layout effect also recovers focus when that control has been removed
  and focus is on `body`.
- Announcements now follow the user's own request. A success announces once,
  a failure clears the outstanding request, and later refetches or background
  flips announce nothing.
- Seven new unit cases; targeted Vitest 29/29, tsc, ESLint and Playwright 13/13
  pass.

Findings sent back:
- After a background flip, the screen-reader status region could still contain
  the previous announcement (for example "Suggestions hidden." while
  suggestions are shown). Clear it without announcing.
- Background focus recovery should use `preventScroll` so it never jumps the
  page.

Sol-R round 3 is approved:
- A stale announcement (one that no longer matches the visible state, with no
  request outstanding) is cleared to an empty status region without
  re-announcing.
- Only the background recovery branch uses `focus({ preventScroll: true })`;
  user-initiated transfers keep normal focus scrolling.
- Unit tests spy on `focus` and check that the region empties after a
  background flip in both directions.
- Targeted Vitest 29/29, tsc, ESLint and Playwright 13/13 pass.

No open review findings remain for this cycle.

Validation:
- the sidebar matrix (12 cases) and the Telegram spec, with no `.tmp` leakage;
- the suggestion-restore matrix (12 cases);
- targeted Vitest for the Home, hook and chat page tests;
- tsc, lint (including the i18n audit) and `cargo fmt --check`;
- full CI on the new head;
- a Fable re-review, requested through root.

## Main integration (2026-11-17)

On user instruction, latest `main` is merged into this branch.
- Incoming main: c48c1de05, 85 commits since the merge base 31f3d8969.
- Branch head: fcb07a14b, the head Fable approved; 11 commits.
- Safety branch: `backup/nyxid-ui-rollup-before-main-20261117` at fcb07a14b.
- Root started `git merge --no-ff --no-commit c48c1de05`. Opus owns the
  resolution, review and final merge commit. No force-push and no history
  rewrite.

### Conflicts and decisions

- **Catalogs** (`en.json`, `zh-CN.json` and `zh-TW.json` modify/delete, plus
  `app/en-overrides.json`). Main #2570 (abd2fecf5) split the catalogs into
  sorted per-area files and deleted the legacy single-file catalogs. Resolution:
  - Ran the repository's `npm run adopt:i18n` with `MERGE_HEAD` set: branch
    HEAD, base 31f3d8969, onto c48c1de05. It reported 12,244 app keys, no
    divergence and no legacy imports to rewrite.
  - `npm ci` was not needed: `package-lock.json` is unchanged between base and
    main, and the 614 installed top-level packages match it exactly.
  - Verified independently against root's baseline
    (`~/.heca/plans/wild-beach-rollup/localization-merge-baseline-2026-11-17.json`)
    and main, using `~/.heca/plans/wild-beach-rollup/verify_adoption.py`. In
    each of en, zh-CN and zh-TW:
    - all 81 branch-added keys are present, with identical values and
      `{{placeholder}}` sets;
    - none of the 15 deleted fragments came back;
    - no main key was lost or changed;
    - the final count is main 12,178 + 81 − 15 = 12,244, with nothing
      unexplained.
  - Owning areas follow the documented rules (literal references; keys used
    from several areas go to `shared`):
    - `assistant`: 36 keys;
    - `assistant-agents`: 28 keys;
    - `assistant-approvals`: 12 keys;
    - `shared`: 3 keys (the bypass reasons);
    - `unattributed`: 2 keys, the plural forms `Show {{count}} more_one/_other`,
      which have no literal reference. Main keeps 10 of its own plural forms
      there too.
  - The barrels eagerly merge every area, so placement does not affect runtime
    availability.
  - None of the 15 deleted fragments appears as a literal anywhere in `src`
    outside the catalogs, in any quoting style. Main has not reintroduced one,
    including through constants used by dynamic calls.
- **`assistant-sidebar.tsx`** (2 hunks). Main #2452 (3b209194d) added
  persistent pins, Rename and parent ordering; main #2364 added unread markers
  on chat rows. Decision:
  - Order an agent's own chats with `pinnedConversationsFirst`, as main does.
  - Apply the #2050 fold to that ordered list.
  - In the collapsed view, show the first five chats plus every pinned chat
    (hiding an explicit pin would defeat it).
  - Always keep the open, running, drafted and unread chats visible. #2050
    requires unread indicators to keep working, and main now marks unread
    rows.
  - Keep the toggle, its accessibility and the `useShallow` draft-ID selector.
  - Keep main's pins, Rename, unread markers and parent ordering everywhere.
- **`assistant-sidebar.test.tsx`** (1 hunk). `renderAgents` takes both sides'
  options (`activeConversationId` and `onRename`). Both sides' tests stay, and
  new tests cover pinned chats beyond the first five and unread chats.
- **`dashboard.tsx` / `dashboard.test.tsx`** (content conflict, add/add).
  Main #2423 (f22e7048e) fixed #2217 independently and closed it. It counts
  `n.is_connected`, the same condition as the Nodes badge. Decision:
  - Keep main's count line.
  - Keep this branch's translated `{{online}}/{{total}} online` label (main
    still renders English there) and the lowercase mock node statuses.
  - Revert the now-redundant `nodeStatusKey` helper (`status-contract.ts`,
    `node-status-badge.tsx`) to main's versions (FI-007). The badge test is
    kept only as a component-level check of its rendered status.
  - The test file keeps main's suite and adds this branch's mixed-status case
    and a zh-CN label assertion.
- **Auto-merged overlaps checked by Opus.** `routes.rs`, `nyxbot-api.ts`,
  `assistant-chat-page.tsx` and `CLAUDE.md` equal main plus exactly this
  branch's original edits.
- **Issue references.** #2217 is closed by main #2423, and the reporter closed
  #2218 on 2026-11-13 without a PR. The PR switches both to `Refs`, keeping
  #2218's large-text hardening and regression coverage. #2314 and #2050 stay
  `Closes`; #2001 stays `Refs`.

### Integration review log

Implementer: Sol-M, Heca `codex` `gpt-6.1-sol`, full access. Its Codex session
log records `reasoning_effort: xhigh`, inherited from `~/.codex/config.toml`.

Sol-M round 1, approved by Opus:
- **Sidebar.** The net diff against main is exactly the #2050 fold on main's
  pinned-first order: the five-chat limit, pinned, open, running, drafted and
  unread chats kept visible, the `useShallow` draft-ID selector, the
  toggle, and the whole-phrase channel label.
- **Sidebar tests.** All of main's tests remain. The only lines that differ
  are:
  - a named `appI18n` import, which keeps the side-effect import;
  - a language reset in `afterEach`;
  - the union of both `renderAgents` option sets;
  - `activeConversationId ?? thread.id`.
  New cases cover pinned chats beyond five, unread chats beyond five and
  pinned order when expanded.
- **Dashboard.** The net diff against main is only the translated label. The
  merged test keeps main's three tests, uses the real `translateUi`, and adds
  the mixed-status `1/4 online` case and a zh-CN `1/4 在线` assertion.
- **Reverted helper.** `status-contract.ts`, its test and
  `node-status-badge.tsx` are byte-identical to main. The badge test checks
  the rendered labels.
- **Sol-M's validation:**
  - tsc passes, with no fixes needed;
  - `sync:i18n` changed nothing on either run (12,244 keys, file hashes
    stable);
  - `check:i18n` passes (1,338 files, 14,221 static calls);
  - lint has 0 errors;
  - targeted Vitest passes 206/206;
  - the full Vitest suite passes 20,511/20,511 at bounded concurrency;
  - the build passes;
  - Playwright passes 103/103: sidebar threads 12, pins 8, rail 1,
    workspace sidebar 40, suggestions 13, multilingual 4, Telegram 25.
- **Full-suite timeouts.** The first default-concurrency full run timed out in
  9 tests of main's new `test/i18n-adopt-integration.test.ts`. That file
  passes 37/37 on its own, and the whole suite passed at lower concurrency
  with the same timeouts. It is load-sensitive, not a merge regression.

Finding sent back: restore main's three-line `threads` formatting in
`AgentRow`, so the net diff has no cosmetic noise.

Sol-M round 2 is approved: the formatting matches main byte for byte, eslint
passes and the 85 sidebar tests pass.

### Integration commits and final validation

- **Merge commit 6189309b4** (parents fcb07a14b, c48c1de05). Opus staged the
  resolved files by explicit path plus the documented `git add -A` scoped to
  `frontend/src/i18n/locales`. Before committing:
  - no paths were unmerged, unstaged or untracked;
  - the staged content had no conflict markers;
  - the legacy single-file catalogs were no longer tracked.
- **Net diff against c48c1de05.** 55 files, all explained:
  - every non-catalog file is in this branch's original change set;
  - the only branch files that dropped out are the deliberately reverted
    `nodeStatusKey` helper and the legacy catalogs;
  - the per-area catalog changes are +247 lines (81 keys × 3 languages, plus
    4 English overrides) and −45 (15 removed keys × 3).
- **Main advanced** to 186e35678 before the push, with #2598 (legal pages and
  catalog keys in `shared`) and #2589 (deploy). Both overlapped this branch
  (`CLAUDE.md` and the `shared` area). They merged without conflicts as merge
  commit 687405234, whose staged set equals main's own 24 changed files.
  `verify_adoption.py` against 186e35678 passes in all three languages
  (12,179 main keys + 81 − 15 = 12,245). The `shared` area differs from main
  only by this branch's three bypass-reason keys.
- **Opus's final local validation** on the tree after both merges:
  - tsc passes;
  - `sync:i18n` is idempotent;
  - lint has 0 errors (34 pre-existing warnings);
  - `check:i18n` passes (1,339 files, 14,222 static calls);
  - the i18n tooling tests pass 12,124/12,124 (`test/`, after the first merge);
  - the full Vitest suite passes 20,511/20,511 (748 files) at default
    concurrency, with no timeouts on this run;
  - `npm run build` and `cargo fmt --check` pass;
  - Playwright passes 103/103 (sidebar threads, pins, rail, workspace sidebar,
    suggestions restore, multilingual, Telegram setup);
  - the run left no untracked files and no `.tmp`.
- **Telegram flake.** The first post-merge Playwright run had one failure, in
  `channel-telegram-new.spec.ts` "Cancel setup unlocks fields and the next
  start uses the edited destination". That test is unchanged by this branch.
  It reads the captured POST list right after a popup event, without polling,
  so it races Playwright's route handler.
  - 15 repetitions failed once both on the merged branch and on a clean main
    worktree at c48c1de05 (14/15 each).
  - It is pre-existing and not in CI, and is recorded here, not changed.

### Pinned main (526de4b4d)

Fable approved the merges up to 7961f0d7d with no findings. Root then pinned
`main` 526de4b4d for the next merge. It adds two commits on 186e35678:
- 4a31e80ca, #2592: staging images published from `dev`; the backend image
  declares a numeric user.
- 526de4b4d, #2517: privacy policy alignment and self-serve account export.

Together they change 82 files (+27,415/−699). The two `main` commits that
followed are merged in the next section.

- **Merge commit 6d71b8a66** (parents 7961f0d7d, 526de4b4d). Opus ran
  `git merge --no-ff` of the pinned SHA. It had no conflicts, and its tree
  b88b7c6da equals `git merge-tree --write-tree` of the two heads. The staged
  file set equals main's own 82 files.
  - The 74 files only main changed are byte-identical to main. These include
    the staging-image workflow, the backend Dockerfile and the CLI wizard
    assets.
  - The 47 files only this branch changed are byte-identical to 7961f0d7d.
  - Both sides changed 8 files: `routes.rs`, `CLAUDE.md`, and the `shared`
    and `unattributed` catalogs in three languages. Each differs from main by
    exactly this branch's own lines: the show route (4 lines), the route doc
    line (2), the three bypass-reason keys and the two `Show {{count}} more`
    plurals. Main's additions are all kept: `POST /users/me/export`, the admin
    orphan-sweep routes, the personal-data registry rule in `CLAUDE.md`, its
    `.` key and its two rate-limit strings.
- **Catalogs.** In en, zh-CN and zh-TW, `verify_adoption.py` (onto
  526de4b4d) and the new placement audit `verify_final_pin.py` both pass,
  checked against root's final-pin baseline:
  - 12,261 keys = main 12,195 + 81 − 15;
  - all 81 additions keep their original values, placeholders and areas;
  - every main key keeps its value and area;
  - the 15 removed fragments stay removed, and none of the 2,204 source and
    e2e files references them;
  - no key sits in two areas;
  - `en-overrides.json` is main's 67 entries plus this branch's 4.
- **Privacy export and erasure.** #2517 registers `assistant_onboarding` in
  `personal_data_registry.rs`: owner `_id`, local-inventory erasure, and the
  `assistant` export section. Erasure deletes the row after its admission
  fence.
  - Show suggestions is a plain update of an existing row, with no upsert. It
    cannot recreate an erased row, which is why it needs no fence, unlike save
    and hide.
  - It writes only `suggestions_hidden_at` and `updated_at`, and never the
    NyxBot memory note.
  - Main's export field list for this collection omits
    `suggestions_hidden_at`, as it did before this merge.
  - This branch's model change is a doc comment, so export round-trips are
    unaffected. The branch adds no collection, so main's registry rule needs
    no new entry.
- **Wizard bundle.** None of the 55 files that differ from main is a wizard
  hash input. The inputs are the 216 files in `index.manifest` plus the five
  additional hash inputs that `cli/tests/wizard_bundle_freshness.rs` lists:
  `frontend/package-lock.json`, `frontend/vite.wizard.config.ts`,
  `frontend/wizard.html`, `frontend/vite-plugins/wizard-manifest.ts` and
  `.node-version`. Main's committed hash and assets therefore stay fresh.
- **Validation on 6d71b8a66.** The log is
  `~/.heca/plans/wild-beach-rollup/validate-final-pin-2026-11-17.log`.
  - tsc passes.
  - `sync:i18n` is idempotent (12,261 keys).
  - `check:i18n` passes (1,340 files, 14,236 static calls).
  - Lint has 0 errors; its 34 warnings are pre-existing.
  - Targeted Vitest passes 588/588 in 26 files. They cover main's privacy
    claims, account export, settings, legal pages, and API client and error
    tests; the i18n loader, language, resources and wizard tests; and this
    branch's sidebar, dashboard, badge, mock data, Home, onboarding hook, chat
    page, approval, receipt, acknowledgement, skill card, label and message
    tests.
  - `npm run build` and `cargo fmt --check` pass.
  - Playwright passes 92/92: sidebar threads 12, pins 8, rail 1, workspace
    sidebar 40, suggestions restore 13, multilingual 4 and assistant settings
    14. Telegram setup was not run, because neither side of this merge
    touches it.
  - The run left no untracked files and no `.tmp`.
  - The full Vitest suite and the backend tests run in CI.
- Fable reviewed `7961f0d7d..14dc85b3a` and approved it with no findings.
  CI, CodeQL and Release passed on 14dc85b3a.

### Current main (66f29a7cd)

`main` then had two more commits, so the branch was behind. Root asked for
them to be merged as well. The tip was 66f29a7cd, and a merge-freeze ruleset
created at 20:45 (+08:00) now blocks further updates to `main`. The two
commits:
- f976099e9, #2609: two startup migrations survive a crash (`db.rs`, new
  startup migration tests, nextest config).
- 66f29a7cd, #2603: a retry cannot slip in while a model approval is pending
  (activity decision binding, permission dispatch proposals, their tests).

They change 7 files, none of which this branch touches.

- **Merge commit e348944ef** (parents 14dc85b3a, 66f29a7cd). Opus ran
  `git merge --no-ff` of the exact SHA. It had no conflicts, and its tree
  7ac74ff6d equals `git merge-tree --write-tree` of the two heads.
  - The 7 incoming files are identical to main.
  - All 55 files this branch changes are identical to 14dc85b3a.
  - In e348944ef the `frontend`, `cli`, `mobile`, `sdk`, `docs`,
    `integrations` and `.github` trees are identical to 14dc85b3a, so the
    earlier frontend validation and both Fable reviews still apply.
- **Interaction.** #2603 binds an approval proposal's activity inside the
  proposal transaction and publishes the request-local flags only after the
  commit. It changes no API shape and no user-visible string, so the
  approval-card and receipt translations are unaffected. #2609 does not touch
  the onboarding collection or the code paths Show suggestions uses.
- **Checks.** `verify_adoption.py` and `verify_final_pin.py` pass against
  66f29a7cd (12,261 keys per language; main still has 12,195 because these
  commits change no catalog). `cargo fmt --check` passes. The full frontend
  suite and browser matrix were not repeated, because no frontend file
  changed; CI runs the backend and frontend suites on the pushed head.
- **Doc correction.** The wizard bundle note above now lists the five
  additional hash inputs; it previously said four.

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
