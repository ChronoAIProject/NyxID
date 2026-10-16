# Composer voice and dictation personal review

## Overlay resize and dictation mouse-out — October 16

The primary assistant implemented and reviewed both refinements directly.
The complete composer surface now stretches to 90% of the visible viewport,
including toolbar, attachments and padding, bounded below the chat header.
Its measured band stops at half the visible viewport or 480px. Additional
height grows upward over the transcript; its opaque background and fade move
with it. Below that threshold normal chat spacing resumes. The existing
height preference, one-line collapse, keyboard resize/reset and internal draft
scrolling remain. Layout observation includes changing surrounding controls and
the chat container, so software-keyboard and viewport changes recalculate the
limit without putting the grip behind the header.

Dictation settings close 200ms after leaving the combined trigger/popup area.
Entering either cancels the timer, allowing the pointer to cross the portal gap.
Closing releases the microphone preview. Navigation, eligibility loss,
unmounting and starting dictation cancel pending dismissal. Keyboard opening,
switch operation and Escape remain usable; existing touch activation is retained.

Personal review includes desktop/mobile screenshots and the media lifecycle.
All 26 browser cases in composer-settings and composer-voice pass (46.1s).
The new cases use a fixture transcript and verify 90vh, unchanged message
position/padding above the overlay threshold, shrinking, Home/End/reset,
short viewports, simulated software-keyboard changes, retained drafts and
internal scrolling. Hover checks cover popup entry/exit, microphone cleanup
and subsequent keyboard use. All 111 component tests in chat-composer,
composer-dictation and upload-composer pass. TypeScript, scoped ESLint with
zero warnings, and diff checks pass.

Receipts: `target/composer-overlay-browser-final.log`,
`target/composer-overlay-unit-final.log`, `target/composer-overlay-types.log`,
`target/composer-overlay-lint.log`. Final source hashes and reviewed screenshots
are under `target/composer-voice-live/overlay-*`.

The production-connected frontend at http://127.0.0.1:4615/assistant returns
HTTP 200 and serves this worktree. Browser cases use explicit fixtures and do
not establish live provider availability. No CI updates, commits, pushes,
PR edits, deployments or production configuration changes were made.

## Shared action slot and collapsed chevron — October 16

The latest user correction supersedes the reserved-chevron and adjacent-voice
behavior below. Hidden chevrons now occupy zero width. Revealing the chevron
expands the mic control; settings open on actual pointer movement onto the
chevron, not an enter event caused by layout shifting beneath a stationary
pointer. Keyboard activation and touch controls remain available.

Voice and Send now share a single action slot. Text or any attachment replaces
Voice with Send, including a disabled Send for pending uploads. Clearing the
draft restores Voice when eligible. Active assistant Stop and in-composer call
controls retain their existing behavior.

Personally reviewed the final screenshots and diff. All 24 affected Chromium
cases pass, including 320/390/1280px hidden-width, replacement and hover checks,
permission/credit recovery, dictation and resize. All 109 component tests pass;
TypeScript, scoped lint and diff checks pass. Receipts:
`target/composer-shared-action-{browser,unit,types,lint}.log`.
The local production-connected preview remains on port 4615, with no CI, Git
publication, PR or deployment changes.

## Composer polish follow-up — October 16

The primary assistant directly implemented and personally reviewed the user's
four UI corrections. The live local frontend remains at
http://127.0.0.1:4615/assistant against production. No agents, CI updates,
commits, pushes, PR edits, backend runtime changes or deployments were used.

- Send is absent for an empty or whitespace-only draft independently of voice
  availability. Typed text and attachments reveal it; pending uploads retain a
  disabled Send button. Stop remains available for active work.
- The microphone and waveform icons both render at 20px. Settings open from
  the compact right-hand chevron only, including keyboard/touch activation;
  mic hover/focus does not open the popup. The popup aligns to the chevron's
  right edge, with viewport collision handling on narrow screens.
- Personal browser review reproduced a subtle hover regression with a
  collapsing chevron: the mic moved from x=1012 to x=984, moving the chevron
  under a stationary pointer and opening settings. A fixed 20px chevron width
  eliminates that movement. Browser coverage now checks horizontal stability
  as well as menu visibility. On the user's subsequent refinement, the chevron
  fades in only on hover/keyboard focus, while preserving that reserved width.
  It stays visible for open settings and touch devices. The main and preview
  mic now use a slimmer rounded capsule outline without a flat stand base.
- The aura breathes smoothly over 3.2 seconds while active, retaining measured
  audio strength/size. Entry uses a 450ms opacity transition; stopping pauses
  the cycle and fades it out over 800ms. Reduced-motion mode uses a steady
  aura. Real browser samples show the breathing multiplier rising above 0.95
  and falling below 0.05; stop reaches zero opacity. This visual verification
  uses explicit test audio, not a production provider session.

Latest icon/hover refinement verification: 11 dictation component tests and
four Chromium settings/responsive cases pass, including hidden-at-rest,
visible-on-hover, hidden-on-leave and stable horizontal control bounds at
320/390/1280px. TypeScript, scoped lint (zero warnings) and diff checks pass.
Receipts: `target/composer-mic-refinement-{unit,browser,types,lint}.log`.
The earlier broader verification below remains unchanged.

Earlier polish verification:

- 143 component tests passed across six files: 126 in
  `target/composer-polish-unit-final.log` and 17 in
  `target/composer-polish-voice-unit.log`.
- All 29 affected Chromium cases are verified across the final suite and
  focused recheck: the suite passed 27 cases, with two layout assertions
  sampling entrance/collapse animations too early. Their corrected assertions
  wait for popup alignment and check horizontal hover stability independently
  of an earlier newline's vertical collapse. All four responsive/settings
  cases then passed. Logs: `target/composer-polish-browser-verified.log` and
  `target/composer-polish-layout-verified.log`. The suite also covers native
  permission recovery, credit retry, attachments, rich drafts and multiline
  drag resize.
- TypeScript, scoped ESLint and `git diff --check` pass. TypeScript found five
  unsupported `exact` options in the prior retry test; removing them retains
  Testing Library's default exact string matching, and all voice-panel tests
  pass. Logs: `target/composer-polish-typecheck-final.log`,
  `target/composer-polish-lint-final.log` and
  `target/composer-polish-layout-lint.log`.
- Light/dark and 320/390/1280px screenshots were inspected. The visual script
  also verifies keyboard opening, the fade cycle, stop and reduced motion,
  with zero page errors: `target/composer-voice-live/polish-visual-results.json`.
  Dark review images are `polish-dark-empty.png`, `polish-dark-settings.png`
  and `polish-dark-glow.png` in the same directory.

## Current local UI revision (October 16)

This revision starts from published commit `405eca72`. The user requested a
production-connected local frontend and no CI updates pending UI review. All
changes in this revision remain local: no commit, push, PR update, CI trigger,
merge or deployment. The implementation plan and per-step acceptance criteria
are in [COMPOSER_VOICE_DICTATION_PLAN.md](COMPOSER_VOICE_DICTATION_PLAN.md).

The current review URL is **http://127.0.0.1:4615/assistant**. Its loopback-only
Vite proxy forwards HTTP and voice WebSocket traffic to the production backend
using normal authentication. It contains no sample API responses, fake media,
feature overrides or embedded credentials. Configuration and launch instructions
are in ignored `target/composer-voice-live/`. The former sample preview on
4612/4613 has been retired.

Production probes returned 200 for health/runtime configuration, 401 for
unauthenticated voice options and WebSocket upgrade, and 404 for dictation
options. The production dictation route is not yet deployed. Browser login
redirects normally; no authenticated paid-provider or physical microphone
success is inferred from the local checks.

Personal review and local verification are complete. Current-revision execution receipts are recorded
at the end of this document. All older full-suite, backend, build and wizard
receipts below describe `405eca72` or its predecessors, not this UI revision.

## Permission and credit recovery follow-up — October 16

The parent's personal review confirms the permission call already originates
in the main dictation/voice gesture, before draft preparation, settings writes,
provider creation or billing reservation. Hover only reads devices/options.
The follow-up adds an explicit compact **Retry voice** action and recovery
guidance for HTTP 402 `insufficient_credits`. It keeps the selected paid source,
requests fresh capture, and creates a fresh request identity only on the next
gesture. The action is unavailable during active/preparing/conflicting calls or
when source/preferences cannot be used. Error classification no longer labels
an unrelated HTTP 404 as insufficient credits merely because it shares code
11300. The stable error symbol also handles the screenshot's code 1300.

Further personal review found that a direct retry without prepared capture
would use the default microphone, unlike the initial waveform's saved device.
The hook now reads the current actor's selected device for each attempt and
passes it into fresh client capture. Already-prepared capture remains unchanged
and is never requested twice. Hook/client and integrated retry regressions cover
the selected device as well as the selected paid voice source.

Independent browser cases cover both first activations with pending permission,
zero metered connections before grant, late grant followed by dictation 404,
capture release, unchanged draft, voice credit refusal and successful explicit
retry. Native Chromium denial → grant cases exercise the actual Permissions and
`getUserMedia` APIs with virtual audio hardware. A standalone native probe on
the actual local preview also confirmed capture support. These are browser
permission checks, not physical microphone or paid-provider validation.

The default Playwright `chrome-headless-shell` returned `NotSupportedError`
for both denied and granted capture; full `channel: chromium` returned the
expected `NotAllowedError` for denial and captured with a grant. The browser
tests therefore use the full Chromium binary. No production workaround was
added for the test driver's missing capture support. The dictation alert can
move between its main surface and hover popup; the native-permission assertion
checks the visible matching alert through that transition.

Backend review found no cache of grant balances or a frontend credit gate.
Every 30-second reservation reads current applicable allowances/grants and
available wallet funds. Platform voice selects the acting person's payer.
Grant eligibility requires active status, a matching recipient and service,
unexpired available funds and `issued_ledgered_at`. A newly created grant still
awaiting its journal cannot yet fund a call. The checked voice reservation,
grant eligibility and payer code matches production commit `897a89c2b092`.
The 0.00084 credits/second screenshot implies a 0.0252-credit duration hold,
plus any separately configured start fee. A reservation is not consumption.

Two new local backend regressions exercise immediate retry after real grant
issuance, provider invocation ordering, and release of duration/request-fee
holds with zero confirmed usage. Personal review corrected a vacuous ledger
assertion: v2 entries use `event_type: accounting_v2` with a `movement` field,
so the test must check `movement: usage_settled` / `grant_consumed`.
No backend runtime, live account balance or production configuration changed.

Live account diagnosis remains bounded: the parent's browser has no signed-in
production account. The screenshot proves reservation refusal, but does not
identify whether that person's funds are insufficient, scoped elsewhere,
already reserved, or awaiting grant journaling. Production dictation still
requires the route deployment; this local frontend cannot make a missing
server endpoint transcribe. No paid provider traffic was used in this review.

Fresh frontend receipts:

- **137 tests passed across 9 files**, including capture, dictation, voice,
  settings, page ownership and error recovery:
  `target/composer-permission-credit-unit-final-device.log`.
- **29 browser tests passed** in a clean final run, including native permission
  denial/grant, first activation, credit retry, rich drafts, responsive controls
  and multiline drag resize:
  `target/composer-permission-credit-e2e-verified.log`.
- After the selected-microphone repair, the expanded integrated retry case
  passed again (`target/composer-retry-device-browser.log`). It selects the
  microphone through the dynamic menu, observes it on preview/initial start/
  retry, verifies cleanup and a fresh request identity, then ends the call.
- `npx tsc -b`, focused ESLint and `git diff --check` passed:
  `target/composer-permission-credit-typecheck.log` and
  `target/composer-permission-credit-lint.log`.
- The initial concurrent unit run timed out one existing page-lifecycle test
  while Rust and browser work ran. The final run used two Vitest workers and
  passed with the same assertions and timeouts. The initial new browser probes
  also exposed driver and alert-location assumptions described above; the
  final full browser run passes with zero retries.

Backend funding verification completed against the explicit local replica-set
URI, with no skipped cases:

- **2 new regressions passed**, 3.16 seconds, from the rebuilt binary:
  `target/voice-funding-new-regressions.log`. A real empty-wallet refusal invokes
  no provider; an actual personal grant then funds the next call immediately
  using the same billing service. Zero observed usage releases both grant and
  wallet holds, including the authored request fee, and repeated finalization
  creates no debit or consumption posting.
- Existing rollout/ACL/tariff/wallet gates passed, and the actual batch issuance
  journal-recovery case passed:
  `target/voice-funding-existing-gates.log` and
  `target/voice-funding-journal-gate.log`.
- The new tests use MongoDB 8.0.16, FakeLago and a loopback provider. Production
  Rust is unchanged. The initial private-method compiler error in a test was
  corrected to exercise public HTTP response mapping before the successful
  rebuild and test execution.

The local implementation and personal review are complete. CI, draft PR #1907,
commits and deployment remain untouched; account-specific production funding
and paid-provider verification are not claimed.

## Previous implementation review

Personal review of the composer expansion of PR #1907. The primary assistant
reviews the implementation directly; the existing Heca `gpt-6.1-sol` implementer
uses xhigh reasoning and full access. No additional agents or delegated reviewers.
Merge prerequisites #1904 and #1901 were merged upstream on 2026-10-15 and
integrated here at `629138a0` (main `dca7abb7`). This document records verification evidence,
not a claim that production audio or provider billing has been exercised.

## Findings resolved during implementation and personal review

| Finding | Required resolution and evidence | Status |
| --- | --- | --- |
| Stale relay finalization can settle from an outdated duration before checking its lease. | Fence finalization before settlement, use the latest durable confirmed total, prevent checkpoints after finalization, and test recovery racing a stale worker. | Resolved; regression coverage below |
| Automatic time-limit and remote-end paths reuse the startup drain timestamp. | Begin a bounded drain deadline on the first stop transition; test final completion on both paths. | Resolved; regression coverage below |
| Provider readiness accepts an arbitrary session update. | Verify transcription-only session type, expected model, audio format and manual commit before forwarding audio; reject unexpected configuration. | Resolved; regression coverage below |
| Browser disconnect drops the output sink while forwarded audio may still need finalization. | Stop new capture while draining already-forwarded audio independently of UI delivery; test pending and buffered audio on disconnect. | Resolved; regression coverage below |
| Encoder cleanup can close an AudioContext repeatedly, and delayed initialization can lose the browser activation gesture. | Idempotent cleanup and activation-safe context creation/resumption; cover stop, cancellation and late initialization. | Resolved; regression coverage below |
| Transcript ordering depends on first delta arrival. | Preserve canonical item order when a later item's delta arrives before an earlier item's first completion; include a regression. | Resolved; regression coverage below |
| Adding microphone controls can leave no usable editor width at 320px. | Responsive layout preserves a usable draft area, visible controls and no horizontal overflow; inspect real browser screenshots. | Resolved; idle and active browser bounds/visual checks passed |
| Hold release listeners disappear after the recording state rerenders, and the initial popover button unmounts during a hold. | Track the complete pointer/keyboard gesture on stable targets, including pending permission and initial setup. Release, cancellation, lost capture and blur must stop capture. | Resolved; regression coverage below |
| Aurora references nonexistent CSS variables. | Use the actual semantic theme tokens and verify computed gradient plus measured amplitude in the browser. | Resolved; regression coverage below |
| Dictation scope cleanup retains state belonging to the old account or draft. | Isolate/reset choices, thread, text, approval and active state immediately; ignore late callbacks and prove a subsequent start uses the new scope. | Resolved; regression coverage below |
| Hold/click choice is not restored between visits. | Safely persist and restore the non-sensitive control preference with regression coverage. | Resolved; regression coverage below |
| Memoized message rows do not update when playback crosses a caption boundary. | Pass a bounded delivery revision through the actual chat list and test persisted-caption progression. | Resolved; regression coverage below |
| Closing or navigating away from voice can leave stale active state. | Fence panel identity, actor/thread and visibility; preserve final captions while clearing media state. | Resolved; regression coverage below |
| Active voice uses the former large setup panel. | Compact primary controls, expandable pricing/task details, preserved errors and Grok controls. | Resolved; regression coverage below |
| Caption fragments can grow from empty/tiny provider events. | Bound ingest and wire projection; use dedicated response structs and flood regressions. | Resolved; regression coverage below |
| Dictation confirm/cancel removes focused controls. | Restore editor focus/selection after unlock; preserve one-step undo and rich draft content. | Resolved; regression coverage below |
| Provider-first biased selection can starve lifecycle checks. | Prioritize lifecycle/cancellation under event floods; prove bounded closure. | Resolved; regression coverage below |
| Present malformed billing checkpoint fields silently default to zero. | Default absent migration fields only, reject corrupt present fields. | Resolved; regression coverage below |
| Dictation advertises conversational models despite a fixed transcription adapter. | Truthful fixed-model disclosure and coherent source selection. | Resolved; regression coverage below |
| Initial draft compares two absent thread IDs then dereferences a null VoiceView. | Explicit non-null guard and browser initial-render regression. | Resolved; regression coverage below |
| Dictation authorization also requires the unused conversational POST operation. | Authorize exactly GET /realtime for dictation; preserve normal voice authorization and cover restrictive operation scopes. | Resolved; regression coverage below |

The existing delegated-read deny list already covers the entire `/assistant`
namespace. The new execution-shaped dictation route has an explicit regression case in
the route-denial table. Normal conversational voice endpoints must not reattach,
delegate work or claim a transcription session as a conversational call.

## Evidence collected

- Existing PR #1907 backend and security CI passed before this expansion.
- Baseline Chromium run: `npx playwright test e2e/composer-pills.spec.ts
  e2e/composer-settings.spec.ts --reporter=line`: **16 passed**, 18.6 seconds.
  Log: `target/composer-voice-baseline-e2e.log`. Before screenshots are in
  `target/composer-voice-baseline/`.
- The baseline covers attachment pills, selection, undo, paste, draft isolation,
  IME handling, model selection, voice entry, composer resizing and 320px layout.
- Browser driver: Chromium 151.0.7922.34, Playwright revision 1234. Local fixture
  app runs on `http://localhost:4611`, without production credentials or data.
- The task-owned MongoDB 8.0.16 replica set listens on loopback port 27019 for
  transaction-backed tests. Provider fixtures must use loopback transports and
  normal billing services with FakeLago; no paid calls are implied by test passes.
- First revised-UI Chromium inspection: the draft field is 232px wide at a 320px
  viewport, but Send spans x=305.53..337.53 and is clipped. This is a failed UI
  gate, despite `document.scrollWidth === 320`. Evidence:
  `target/composer-voice-review/initial-320.png`. Final tests must inspect actual
  control bounds, not only page overflow.

## Official API contract evidence

Retrieved 2026-10-15:

- [Realtime transcription](https://developers.openai.com/api/docs/guides/realtime-transcription):
  transcription-only sessions, PCM16 at 24 kHz, manual commit, live deltas and
  item-ID completion. Completion order between turns is not guaranteed.
- [GPT-Live-Transcribe](https://developers.openai.com/api/docs/models/gpt-live-transcribe):
  duration-based input audio billing. Provider prices are not hardcoded into NyxID.
- [Realtime server events](https://developers.openai.com/api/reference/resources/realtime/server-events#conversation.item.input_audio_transcription.completed):
  completion usage has a duration or tokens variant. This fixed duration adapter
  must refuse missing or incompatible usage and never fabricate user charges.
- [Live sideband events](https://developers.openai.com/api/reference/resources/live/sideband-websocket#session.output_transcript.delta):
  output fragments have session-timeline start/end timestamps. They do not supply
  individual word timestamps or define complete turns.

## Expanded review evidence

- Earlier review used the draft PR while #1904 and #1901 were open. Both are now
  merged upstream and included in the final integration check below.
- Initial browser smoke probe after voice-view integration could not render the composer.
  The null dereference in the initial-draft `currentVoice` expression was sent back
  to the implementer; browser checks resume after that repair.
- Revised six-file frontend run passed **74 tests** (`target/composer-voice-frontend.log`).
  It includes shared microphone ownership/revocation and caption delivery through the
  memoized `ChatMessageList`. Dictation hook regression run separately passed **7 tests**
  (`target/composer-dictation-hook-tests.log`). These were intermediate runs; the frozen integrated publication results appear below.
- Idle browser rerun passed at **320, 390 and 1280px**, with no page errors. At 320px,
  composer bounds are x=12..308; editor x=25..295 (270px wide); Send x=263..295.
  All attachment, model, dictation, voice and Send controls fit. Screenshots:
  `target/composer-voice-review/idle-{320,390,1280}.png`. Mobile and desktop light
  theme screenshots were personally inspected.
- Further review caught the active voice icon generating a new panel token and ending
  the old session. Preserve a matching actor/thread binding and prove repeated open
  does not end/create a call. Pricing disclosure must also include any configured
  per-start request component. Both are resolved: repeat-open retains the session token; pricing shows the authored request component. Browser and component regressions pass.

## Verification progress

- Revised browser gate: **22 passed in 22.3s** across `composer-voice.spec.ts`,
  `composer-pills.spec.ts` and `composer-settings.spec.ts`; log
  `target/composer-voice-parent-e2e-recheck.log`. The first run was 17 passed / 5
  failed, retained in `target/composer-voice-parent-e2e.log`. The repair fixed the
  real hover/trigger close race, updated two old voice-with-text expectations,
  and awaited the actual asynchronous End request.
- Independent browser probes: **11 passed** covering dictation at 320/1280 in both
  themes, denied permission (zero relay/provider starts), keyboard/mouse/CDP-touch
  hold-release, voice in both themes, and reduced motion. Real analyser samples
  from synthetic oscillator media measured input amplitude around 0.227 and
  produced PCM frames. Evidence: `target/composer-voice-review/probe-results.json`
  plus `probe-recheck-results.json`. The recheck fixed only probe assumptions:
  scoped the denied alert to the review instead of a transient closing popover,
  and accepted the existing global 0.00001s reduced-motion duration; transform is none.
- Final dictation MongoDB/loopback run: **12 passed, 0 failed, 0 ignored in 38.30s**;
  `target/composer-dictation-parent-tests.log`. The earlier 7-pass/5-fail run exposed
  strict BSON i64 reads rejecting valid i32 window values. Integer handling now
  accepts both valid representations while rejecting malformed present fields.
- Existing voice service/credential regression: **54 passed, 0 failed, 0 ignored in
  49.70s**, excluding the 12 dictation tests above; `target/composer-voice-parent-backend.log`.
- A selector `services::assistant_voice::` matched zero tests and is not counted as
  evidence. Remaining voice boundary tests use the actual discovered names.
- Caption colour follows the actual playback timeline, including muted playback,
  rather than proving audible delivery. Output aurora is zero when muted/paused.
  Terminal playback progress is retained; dedicated client/hook/memoized-row lifecycle tests cover it.

## Additional resolved findings

- Hovering the microphone then clicking Configure closed the already-open popover.
  Hover now belongs to the microphone and Configure explicitly opens it; pointer,
  keyboard and touch access are covered.
- Automatic voice termination reset caption progress to zero. The actual media
  timeline is retained on pause/end and continues through speaker mute; output
  activity remains zero when muted. The regression first reproduced the failure,
  then passed through the real client/hook and memoized persisted chat row.
- Full Rust lint found a now-unused authorization wrapper and the eight-argument
  relay boundary. The wrapper was removed, with its binding regression directly
  exercising `inference_authority`. The relay boundary has a narrow, documented
  `too_many_arguments` allowance consistent with its injected-provider helper.
- Reviewed failed provider setup against `billing::meter::settle_voice_window`:
  the request component charges only when the initial window has positive
  provider-confirmed whole seconds. Merely marking a reservation forwarded does
  not charge a per-start request after a failed setup.

## Regression-to-finding evidence

- Backend `dictation_checkpoint_carry_replay_and_stale_owner_cannot_seal_current_usage`
  covers fractional carry, item replay, malformed durable usage, stale leases and
  current-row finalization. The readiness, fresh-drain, disconnect, invalid-usage,
  event-flood, restrictive-operation and transcript-fragment tests cover the
  remaining server findings.
- `microphone-capture.test.ts`, `dictation-pcm.test.ts` and
  `assistant-dictation-client.test.ts` cover permission ownership, late grants,
  revocation, activation/cleanup, ordering and completion replacement.
- `composer-dictation.test.tsx` covers actor/thread rebinding, stale callbacks,
  storage failure, hold/toggle, hover/Configure and both tariff components.
- `voice-captions.test.tsx` and `voice-delivery-lifecycle.test.tsx` exercise fragment
  boundaries in memoized persisted rows, mute/unmute, pause and automatic close.
- `composer-voice.spec.ts` exercises the actual rich editor, mentions, attachments,
  selected-text replacement, focus restoration and one-step undo, plus permission
  cancellation, repeat voice opening and composer bounds.
- Independent real-Chromium probes add light/dark active views, real synthetic
  audio analysis, keyboard/pointer/CDP-touch hold and reduced-motion checks.

## Final verification

The integrated frontend suite, production build, strict Rust lint and source
review pass. Final executable regression and generated-file receipts are recorded
below; no paid provider or physical device claim is inferred from fixture results.


## Upstream integration and interactive preview

Both required PRs merged while final verification was running: #1901 at
`4f402a4f` and #1904 at `dca7abb7`. Merged main into this branch at `629138a0`.
The chat-page conflict preserves both the new voice state/captions and upstream's
platform-thread header label. Generated wizard assets/hash were rebuilt from the
combined tree. Shared message initialization additions merged cleanly. Public
credential purpose remains rejected through the existing `api_key_auth_user` and
`ensure_llm_proxy_access` checks; no public credential exception was introduced.

The final all-targets lint also found the test provider enum's large socket
variant. The implementer boxed that test-only value and updated its construction;
personal review verified the change. No suppression was added.

Integrated frontend checks: **544 files / 5,679 tests passed**; lint exited zero
with 34 existing warnings and none on changed paths; production build passed
TypeScript, Vite, legal prerender, credential-accept and mock-footprint checks.
Integrated Chromium suite: **22 passed**. The initial attempt ran zero tests
because upstream now requires its own Vite server; after moving the user preview
to a separate port, the unchanged command passed. No product/test gate changed.
Logs: `target/composer-voice-integrated-{test,lint,build}.log` and
`target/composer-voice-integrated-e2e-recheck.log`.

The previous interactive sample preview ran on loopback port 4612, backed by a
separate Vite server on 4613 and explicit test fixtures. Its private
adapter blocks backend API forwarding, supplies sample transcripts/synthetic
audio and labels the page as a silent UI preview. An independent browser smoke
verified dictation start/stop/review/cancel and voice/pending captions with no page
errors. No production source or package change is needed for this preview.
These sample-preview processes were stopped when the user requested a local
frontend connected to production. The active review URL is recorded above.

Final integrated Rust compilation and strict lint passed. Executable regression
and generated-file receipts follow below.


### Two-minute ingress budget repair

Final review compared the handler's message cap to the actual encoder: the
worklet emits 480 samples at 24 kHz every 20 ms, so a 4,096-message cap would end
a normal recording at about 81.9 seconds. The handler now derives a finite
6,032-message budget from the 120-second duration plus 32 control messages.
The same dispatch helper used by the handler is exercised with all 6,000 normal
960-byte PCM frames, their exact forwarded bytes, and terminal Stop. A second
regression checks control-only budget exhaustion and unchanged eight-frame
channel backpressure. Frame-size, byte-rate, paid-window and duration bounds
remain enforced independently. This was personally reviewed before rebuilding.

The first integrated Rust build was intentionally interrupted to incorporate
this repair; no passing build or test count is attributed to that interrupted
command. Final input hashes are retained separately in
`target/composer-voice-publication-inputs.json`.


## Publication verification receipt

- `cargo test -p nyxid --no-run`: passed in 10m37s on the integrated source
  including the full-duration ingress repair. Binary:
  `target/debug/deps/nyxid_server-4364b07c7739ce97`.
- `cargo clippy --workspace --all-targets -- -D warnings`: passed in 6m08s.
- Log for both declared commands and the wizard test:
  `target/composer-voice-final-rust.log`. The macOS linker reports its existing
  large unwind-section advisory; Cargo also reports the existing
  `proc-macro-error2` future-compatibility advisory. Neither is a source lint
  finding introduced by this change.
- Integrated `voice --test-threads=2`: **90 passed, 0 failed, 0 ignored**, 59.14s.
- Integrated `operation_scope --test-threads=2`: **26 passed, 0 failed, 0 ignored**,
  248.73s. Log: `target/composer-voice-final-backend-tests.log`.
- All database lanes use the explicit isolated MongoDB replica set; the rebuilt
  executable was identified from the successful Cargo build before execution.
- Final source/build inputs are bound by SHA-256 in
  `target/composer-voice-publication-inputs.json`. The frontend files have not
  changed since the 5,679-test integrated run, production build and 22-test
  browser run. The late repair touches only the backend ingress and its tests.

- Integrated `org_agent --test-threads=2`: **29 passed**, zero failed/ignored,
  102.83s. Delegated account-read filter: **7 passed**, zero failed/ignored.
- Integrated billing-route smoke: **1 passed**, zero failed/ignored, 7.60s.
- `cargo test -p nyxid-cli --test wizard_bundle_freshness`: **1 passed**, zero
  failed/ignored; integrated wizard source hash prefix `922e5fdcda8e`.
- These filters overlap; their counts are not summed as a unique-test total.
- Final formatting and whitespace checks pass. The snapshot comparison confirms
  unchanged consumed source/build inputs. Only the reviewed documentation receipt
  is updated after execution.

**Personal review verdict:** all identified implementation findings are resolved
and covered by the recorded checks. No additional reviewer agents were used.
The final source includes both required upstream PRs. The explicit local-fixture
versus paid-provider/physical-device evidence boundary remains unchanged.


## Current UI revision: final personal review and local verification

The Heca implementer repaired the voice/dictation findings and is idle with
source frozen. The primary assistant personally reviewed those changes and
implemented/reviewed the additional multiline drag-resize repair. No additional
agent or delegated reviewer was created. HEAD remains `405eca72`; this revision
is uncommitted and unpublished as requested.

| Finding | Resolution and acceptance evidence |
| --- | --- |
| Dictation settings were static and indirect | Dynamic split microphone/chevron, live device list, exact device selection, explicit input test, immediate persisted Hold switch. Hover requests no permission. Desktop/mobile light/dark probes pass. |
| Failed no-text dictation left an empty review and listening aura | Return to editable idle with accessible server-unavailable/error message and working configuration; retain real partial text for review without a listening aura. Unit/browser checks pass. |
| Unplugged selected device could close its replacement test through an old callback | Validate the current selected device and reject superseded enumeration results. Red component/browser reproduction becomes green; replacement preview continues measuring. |
| Stop during PCM startup dropped the last transcript | Ignore late encoder setup failure after intentional Stop while keeping bounded transcript drain. Deterministic regression and rich-editor browser insertion/undo pass with the full `first second world` result. |
| Voice permission/default/layout did not follow the requested interaction | Acquire permission/AudioContext from waveform activation before asynchronous work; default OpenAI to Automatic; integrated composer controls and primary icon; measured input/output aura and provider fragment captions. Input forwarding still requires provider readiness and server authority. |
| Closing/reopening voice lost chosen source/mode | Typed confirmed settings update the original actor’s cache; late account changes cannot update either actor incorrectly. Browser End → Close → reopen preserves Hold; source/cache regressions pass. |
| Loading failure had no usable retry | Explicit read-only retry and hydration edit guard. Failed source/preferences loads recover without starting media or selecting another paid source. |
| Short multiline drag had no visible effect | Previously a three-line draft stayed 71px tall after an upward 160px drag because only its maximum changed. Manual sizing now controls visible multiline height; keyboard, reset, compact single-line mode, viewport bounds and scrolling pass. |
| Type errors in new terminal-caption and Grok preparation paths | Explicit null guard and erasable field declaration/assignment. Final TypeScript build check exits zero. |

Final local checks (all on the current UI source):

| Check | Result | Receipt |
| --- | --- | --- |
| Voice/dictation/page/media tests, 8 files | 122 passed | `target/composer-ui-targeted-repaired-final.log` |
| Composer/editor/settings tests, 2 files | 47 passed | `target/composer-resize-unit.log` |
| Chromium composer voice/pills/settings | 24 passed | `target/composer-ui-parent-e2e-verified.log` |
| Independent desktop/mobile light/dark probes | 10 passed, no page errors | `target/composer-voice-live/ui-results.json` |
| TypeScript `npx tsc -b` | Passed | `target/composer-ui-parent-typecheck-final.log` |
| Changed-path ESLint and whitespace | Passed | `target/composer-ui-repaired-lint.log`, `target/composer-resize-lint.log` |

The first parent browser run (22/23) exposed the real PCM Stop race. Its trace is
retained as `target/composer-voice-live/dictation-stop-race.zip`. A later run
(23/24) clicked the resize handle’s stale coordinates during its auto-growth
transition. The diagnostic probe forced four animation frames between measuring
and clicking, recording pointerdown on a paragraph; the corrected action waits
for the locator to stabilize and records pointerdown on `Resize composer`.
No assertion, timeout or retry gate was weakened. This also preserves the new
short-multiline regression, which failed deterministically before the product fix.

Visual inspection covers the current device popover in both themes and mobile
voice controls inside the composer. Read-only production connectivity is proven
through the local proxy; actual paid-provider and physical acoustic success are
not claimed. The dictation production route remains 404 until backend deployment.
The live port 4615 preview is left running. The previous sample preview and the
parent’s isolated test-only server are stopped.

## Landing review after the final interaction revisions

The earlier publication hold and preview status above are historical. The owner
has now authorized updating from main, publishing, monitoring CI, squash merging,
branch deletion and local teardown. Final source was personally reviewed after
merging main `5aed82c5` (including #1901, #1904 and #1915); no reviewer agents were
used. No unintended breaking changes were identified outside the voice/dictation
and shared composer scope.

- The empty composer shows Voice; a text or attachment draft replaces it with
  Send. Pending uploads cannot be sent, and active-turn Stop remains reachable.
- The dictation chevron occupies no desktop space until hover/focus. Only moving
  onto the chevron opens settings; leaving the trigger/popup closes settings and
  microphone preview after the short gap-crossing allowance.
- Multiline drag resizing controls the visible editor height up to 90% of the
  visible viewport, including toolbar/padding. Growth beyond the reserved band
  overlays chat. Mobile keyboard changes, short viewports, scrolling, keyboard
  resizing and reset are covered by browser assertions.
- Permission acquisition precedes paid startup. Late permission responses,
  cancellation, account/thread changes and microphone replacement remain fenced.
  Fresh credit grants are read at admission rather than requiring a UI reload.
- Private platform inference preserves live owner/key/agent and operation
  restrictions. BYOK keeps exact connection grants. Additive database fields
  default for older rows, and dictation recovery settles durable confirmed usage
  without replaying audio or creating a conversational receipt.
- The embedded wizard was regenerated for shared CSS. API, architecture and
  README coverage now link to the canonical transport/accounting contract.

Fresh integrated verification: 6,072 frontend tests in 569 files; frontend lint,
TypeScript and production build; 52 selected Chromium cases; 92 backend voice
tests, 26 operation-scope tests, 29 organization-agent tests, nine delegated-account-read tests, the
billing-route smoke test and the formerly failing X delivery test. These backend
filters overlap and are not a unique-test sum. Strict workspace Clippy and Rust
format and the regenerated wizard freshness test also pass. Database tests used the explicit isolated MongoDB 8 replica
set with transaction support. Full PR CI is tracked on the final published head.

The expanded browser run found two outdated assertions, now corrected and
passing: an empty composer expected Send, and the NyxAgent rename test expected a
dialog instead of main's inline form. No test was disabled, no timeout/coverage
threshold lowered, and no workflow, dependency or release configuration changed.
Credential scanning found only the known test token, source expressions,
documented localhost preview URLs and generated numeric patterns already present
in main's wizard bundle.

An independent upstream publication problem was reported immediately: main's
[Publish Images run 38884258840](https://github.com/ChronoAIProject/NyxID/actions/runs/38884258840)
failed in the Linux AMD64 backend container build with BuildKit
`ResourceExhausted: cannot allocate memory` and a SIGKILL of `rustc`. That run
predates the updated PR and is not a voice/composer regression. The build runner
needs sufficient memory/swap; PR CI success alone does not certify image
publication or a production rollout. Paid-provider acoustic verification is not
inferred from browser fixtures.
