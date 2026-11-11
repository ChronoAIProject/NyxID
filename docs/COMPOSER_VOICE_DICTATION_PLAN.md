# Composer voice and dictation implementation plan and receipt

Branch `fix-specialist-bot-voice`, draft PR **#1907**. Preserve original specialist
platform-voice fix **`cc42ae85`**. **Merge only AFTER #1904 and #1901**; neither
prerequisite was merged by this work. Both merged upstream during verification
and are included through main `dca7abb7` at merge commit `629138a0`. The requested Heca implementer is the sole implementer
(gpt-6.1-sol, x_high, full-access); the prohibition applies to spawning any
additional agents/reviewers. Parent personally reviews and publishes. The implementer made no Git publication or live changes. The primary assistant
commits the reviewed work and updates the existing PR. No version source is added (parent classifier: `NO_VERSION`).

## Current UI revision — October 16

### Overlay resize and mouse-out follow-up

| Step | Acceptance criteria | Verification |
| --- | --- | --- |
| 1. Expanded height | Drag reaches 90% of the visible viewport for the whole composer surface, including controls and padding. The grip stays below the header; text and actions remain usable on narrow/short screens. | Browser checks at 390/1280px, 900/400px height and simulated software-keyboard viewport changes. |
| 2. Chat overlay | The reserved band stops at half the viewport or 480px, whichever is smaller. Further expansion leaves transcript padding and message position unchanged; shrinking below the threshold restores normal measured spacing. | Real fixture transcript, bounding boxes, padding, keyboard arrows/Home/End, reset and internal scrolling. |
| 3. Dictation dismissal | Leaving both trigger and popup closes settings after 200ms and releases the mic preview. Crossing between them cancels dismissal. Pending timers cannot affect a different draft. Keyboard and touch activation stay available. | Component lifecycle checks and browser hover, microphone cleanup and keyboard checks. |
| 4. Personal review | Review source and screenshots; run affected local checks and keep the production-connected preview available. | Receipt in COMPOSER_VOICE_DICTATION_REVIEW.md; no CI or publication changes. |

### Composer polish follow-up

The primary assistant implements and reviews this follow-up directly, with no
additional agents. It supersedes the earlier left-chevron and persistent-aura
design below. The production-connected preview remains on port 4615; no CI,
publication, backend runtime or production configuration changes are included.

| Step | Acceptance criteria | Verification |
| --- | --- | --- |
| 1. Empty composer | Empty and whitespace-only drafts show Voice when eligible. Text or attachments replace Voice with Send in the same slot; the two never appear together. Pending uploads keep Send disabled and an active turn retains Stop. Clearing the draft restores Voice. | Existing composer/upload/steering tests updated; browser checks cover empty, whitespace, typed and cleared drafts. |
| 2. Dictation trigger and layout | Rounded capsule mic without a flat stand base. A compact right-hand chevron fades in on hover/keyboard focus and has zero width when hidden. Touch and open settings retain a visible chevron. Hover/focus on the mic does not open settings; actual pointer movement onto the chevron or its activation opens the right-aligned popup. Leaving both trigger and popup closes settings after a short grace period and releases its microphone preview. Both mic and waveform render at 20px. Hover never requests capture. | Dynamic-settings and responsive browser checks cover zero hidden width, visibility, the trigger boundary, alignment, measured icon sizes and 320/390/1280px controls. |
| 3. Glow | Active aura gently fades in then out every 3.2s, with live amplitude controlling strength/size. Entry and exit are smooth; inactive sessions do not animate. Reduced-motion mode uses a steady aura. | Inspect browser animation values and light/dark screenshots, including stopped and reduced-motion states. |
| 4. Review | Personally review changes, run affected component/browser tests, TypeScript and scoped lint, and retain the local preview for user review. | Final results recorded in COMPOSER_VOICE_DICTATION_REVIEW.md. |

### Earlier UI revision

The user rejected the earlier composer preview. This revision starts from
`405eca72` and changes frontend UI/interaction only. The earlier test receipt
below covers the transport implementation; it does **not** validate this revision.
No backend runtime, production configuration, checked-in Vite/environment configuration, CI,
generated wizard artifacts, commit, push, or publication changes are part of
this phase. Parent owns the real local frontend on port 4615 connected to the
production API, personal review, and any later publication. A sample fixture is
only regression evidence, never the deliverable or proof of provider service.

Both new reference images were inspected, together with the rejected preview.
Confirmed causes were the extra dictation Start form, missing selected-device
capture, the automatic voice default incorrectly set to push-to-talk, fallback
from an unavailable saved paid source, a detached active voice card, and an aura
that disappeared whenever measured amplitude reached zero.

| Step | Change and acceptance criteria | Current evidence |
| --- | --- | --- |
| 1. Dynamic mic settings | Compact mic immediately left of AudioLines; hover/focus reveals a left chevron and rounded settings popup below (collision may flip). Current microphones/labels/check update on `devicechange`; exact selected device reaches capture. Hold switch applies and persists immediately. Hover never asks permission or provisions a draft. An explicit test/enable action owns the real meter and closes on dismissal, blur, unplug or teardown. | Implemented; focused component/device regressions, parent browser captures in `target/composer-voice-live/`. |
| 2. Direct dictation | Main mic click-toggle/hold works from the first activation. Gesture creates capture/encoding context before draft/options waits. Release during permission/preparation cannot open a later metered socket. Source, payer, duration and per-start prices render before provider spend. No-text startup failures return to editable idle with accessible error; real partial text remains reviewable. Review/finalizing do not show a listening aura. Rich selection, mentions, attachments, confirm/cancel/undo remain intact. | Implemented; transport/interaction regressions and updated browser cases. |
| 3. Composer-owned voice | Waveform requests permission and starts the exact available saved/default configuration. OpenAI defaults to Automatic; explicit saved input mode and Grok Hold remain. No silent paid-source fallback. Compact status/mute/speaker/end live inside the composer. Secondary Voice settings is available before start, after End, and from microphone settings. End retains next-call choices; repeated waveform on an active call preserves its binding/token. Confirmed typed settings save updates only the original actor cache, surviving Close/reopen. Initial preference loading disables editing; failed source/settings reads have an explicit retry that does not acquire media/start a provider. | Implemented; focused panel/page/media, remount/cache/account-fence and fail-then-success retry regressions. |
| 4. Media and visual feedback | Active icon stays primary through hover/focus/press. Whole-composer subtle aura remains visible during an open/preparing session and responds only to real input/playback levels. Existing real fragment playback captions remain unchanged. Reduced-motion and 320px layout retain their existing contract. Pending ownership survives only an explicit handoff; navigation/account/flag changes and late grants close media. | Implemented; parent reports integrated mobile controls and active-hover color passing. Final browser review remains parent-owned. |
| 5. Local validation and handoff | Run changed interaction/media/page tests and targeted lint. Keep browser fixtures explicit. Parent reviews real local frontend and runs local type/browser checks. Do not run broad CI/build/wizard lanes for this review phase. | Focused execution results recorded in the current revision receipt below. |
| 6. Multiline drag resize | Drag changes visible height even when a short multiline draft does not overflow. Keyboard resizing changes the same height, double-click restores automatic sizing, single-line drafts stay compact, and viewport bounds/scrolling/content remain intact. | Parent reproduced 71px staying unchanged after a 160px drag, repaired the height calculation, and verified the new regression plus existing resize/overflow cases in the final 24-test browser run. |

Production availability must stay precise: parent read-only probes found voice
options mounted (401 without authentication), while dictation options returned
404 on the connected server. The dynamic device/switch/test UI remains usable
in that condition. Dictation shows an actionable server-unavailable alert; it
produces no fabricated transcript and makes no browser Web Speech fallback.
This frontend revision does not deploy the dictation relay or establish paid
provider availability on production.

### Permission and new-credit follow-up — October 16

The user requested first-click microphone permission for dictation and voice,
and a check that a newly granted permission or new credit grant cannot cause a
misleading credit failure. The supplied screenshots show independent failures:
dictation discovery returns 404, while voice reaches `billing_reservation` and
returns `insufficient_credits` (code 1300 in the screenshot). Current local code
uses 11300, so recovery follows the stable error symbol and HTTP status, while
retaining the actual returned code for diagnosis.

| Step | Acceptance criteria | Verification |
| --- | --- | --- |
| 1. First activation | Main mic click/hold and waveform activation call `getUserMedia` in the user gesture, before async preparation or a metered session. Hover opens settings without capture. Pending/denied permission starts no provider connection. | Browser cases exercise hover → click → pending permission → explicit grant for both controls. Native Chromium cases cover denial followed by a new browser grant; virtual microphone hardware avoids physical recording. |
| 2. Separate service failures | After permission is granted, a missing dictation route reports server unavailability, releases capture, preserves the draft and opens no transcription socket. A credit rejection releases voice capture and active styling. | Browser cases cover both failures, track release and unchanged draft. Permission success alone is never presented as transcription or voice success. |
| 3. Credit recovery | A compact `Retry voice` action appears after startup failure only when the exact selected source and preferences are usable, with no active/conflicting/pending call. Only another user gesture retries; it acquires new capture from the actor's currently selected microphone and sends a fresh request identity. Credit guidance uses the stable error key; unrelated 404s sharing an old numeric code remain unrelated. | Parent wrote failing browser and error-classification regressions before implementation; component tests also cover loading, conflict, active and unavailable-source fences. Further review caught and repaired default-microphone fallback on retry; OpenAI/Grok and integrated device-selection regressions pass. |
| 4. Fresh funding and no-usage settlement | Real local Mongo billing rejects an empty wallet before provider creation. A newly issued, journaled personal grant covering the selected service funds the next attempt without a cache wait. Zero confirmed seconds release duration and start-fee holds without debiting the grant or wallet. | Both new transaction/provider regressions passed against an isolated replica set and loopback provider (2/2, 3.16s). Existing grant-journal and rollout/ACL/tariff/wallet gates also pass. No production balances changed and no paid provider was called. |
| 5. Local handoff | Personally review the final diff, run focused unit/browser/type/lint checks, retain the production-connected preview, and leave CI/PR/production untouched. | Complete: 137 focused tests, 29 browser cases plus the expanded selected-device retry recheck, TypeScript, lint and diff checks pass. Execution results are recorded in the personal review document. Production account eligibility remains a separate live diagnostic unless an authenticated account is available. |

Voice reserves 30 seconds before provider creation. At the screenshot's rate of
0.00084 credits per second, the duration hold is 0.0252 credits; a configured
per-start fee is additional. Funding reads allowances, then applicable active
and journaled grants, then available wallet credits. Platform voice uses the
acting person's funding, even for an organization-owned specialist. Microphone
permission is local and free; accepting it after intentionally starting voice
allows the startup to proceed, and actual connected voice usage is billable.

### Current UI revision receipt

Personal review is complete for this local UI revision; product source is frozen
for the user’s frontend review. No live paid-provider verification is claimed. Parent
owns TypeScript and updated browser execution; neither was run by the implementer
for this revision. Earlier browser/full-build receipts below are historical.

From `frontend/`, the final focused command was:

```sh
npx vitest run \
  src/components/assistant/composer-dictation.test.tsx \
  src/components/assistant/voice-panel.test.tsx \
  src/components/assistant/upload-composer.test.tsx \
  src/components/assistant/assistant-chat-page.toolbar.test.tsx \
  src/lib/microphone-capture.test.ts \
  src/lib/assistant-dictation-client.test.ts \
  src/lib/assistant-voice-client.test.ts \
  src/hooks/use-assistant-voice.test.tsx
```

Result after final review repairs: **8 files passed; 122 tests passed; 0 failed;
4.33s**. Exact output: `target/composer-ui-targeted-repaired-final.log`. The
earlier 116-test receipt is preserved at `target/composer-ui-targeted-final.log`;
it predates the added regressions. Focused changed-path ESLint passed with empty
output in `target/composer-ui-repaired-lint.log`; `git diff --check` passed.
Changed source sections were formatted locally. No broad frontend
suite, build, wizard refresh, CI run or publication was performed for this phase.

The focused lint command (also from `frontend/`) was:

```sh
npx eslint \
  src/components/assistant/{composer-dictation,voice-panel,upload-composer,nyxagent-composer,assistant-chat-page}.tsx \
  src/lib/{microphone-capture,assistant-dictation-client,assistant-voice-client,grok-audio}.ts \
  src/hooks/use-assistant-voice.ts \
  src/components/assistant/{composer-dictation,voice-panel,upload-composer,assistant-chat-page.toolbar}.test.tsx \
  src/lib/{microphone-capture,assistant-dictation-client,assistant-voice-client}.test.ts \
  e2e/{composer-voice.spec,voice-fixture}.ts
```

Coverage includes first click/hold release through permission/preparation;
keyboard/touch-pointer release across rerenders; exact microphone selection,
persistence and hotplug; intentional test ownership/cleanup; 404 idle recovery
versus partial-transcript review; rich draft review mechanics; saved unavailable
voice source → explicit choice → next start on the same panel; automatic default
and retained explicit Hold; source/payer/start-price disclosure; repeated active
open and page navigation/account/flag fences; prepared media handoff and
cancellation without a provider start. The transport regression explicitly
requires both backend snapshot and provider `session.started` before automatic
microphone forwarding; it keeps the production authority gate intact.

Final personal-review repairs have concrete red/green evidence:

| Finding | Regression and retained evidence |
| --- | --- |
| Old selected-device callback closed a replacement preview after USB unplug | `target/composer-ui-device-race-red.log` failed the replacement-track ownership assertion. Current selection ref plus superseded-enumeration fence keeps the built-in preview alive; final focused suite passes. |
| Stop closed the PCM context during asynchronous startup; its late rejection discarded pending transcript finals | `target/composer-ui-pcm-stop-red.log`: one failed/one passed. Deferred startup rejection after Stop now preserves bounded final drain and the second final transcript. Genuine startup failure during capture still fails. |
| Raw settings save left actor query cache stale after Close/reopen | `target/composer-ui-settings-remount-red.log` failed. Typed confirmed response now updates the original actor key. Regression changes both source and mode, verifies no optimistic update, remounts from cache and verifies persistence. Account-change regression rejects the delayed authorization without replacing either cache. |
| Mounted panel retained failed options/settings queries with no recovery | `target/composer-ui-retry-red.log`: two failed. Explicit read retry and preference-loading edit fence now pass fail-then-success tests; retry itself starts neither capture nor a provider and never selects another paid source. |

The three repaired component/client files passed 34 tests in 1.43s in
`target/composer-ui-repairs-final.log`, before the final eight-file run. Parent's
typecheck diagnostics (explicit nullable previous-view guard and erasable Grok
constructor syntax) were also repaired. Parent's previous browser run was
22/23: the rich-draft failure preserved `first second world` as its expectation
and exposed the real PCM Stop race above. That failed run remains evidence,
not passing validation. Parent owns final type/browser reruns after this freeze.
The browser settings fixture now returns a complete typed settings response and
echoes confirmed saves; it does not bypass production parsing. Parent separately
owns the newly requested multiline drag-resize repair in `chat-composer.tsx` and
its tests; this implementer leaves those files untouched during the final repair.
The parent reproduced a 71px three-line draft staying at 71px after an upward
160px drag (`target/composer-resize-red.log`) and repaired explicit visible
multiline height, viewport clamping, reset and real-height ARIA feedback. Parent
owns its three-case browser execution; the changed composer is included in a
separate targeted ESLint check (`target/composer-ui-parent-resize-lint.log`).
That ESLint check passed with zero output. The 122-test voice/dictation receipt
precedes the parent's resize-specific verification and does not replace it.

Updated `e2e/composer-voice.spec.ts` uses direct mic/waveform interactions, checks
review has no listening aura, computed primary icon color on hover/focus, controls
inside the composer, and **input amplitude independently of output**. Its fake
peer emits `session.started` at remote-description readiness; synthetic oscillator
streams still drive actual browser analysers/PCM. This fixture never connects to
a paid provider and is not the user-facing production-connected preview.

Parent independent checks reported reference-shaped device settings, integrated
mobile controls, and primary active-hover behavior passing. Parent retains their
visual evidence in `target/composer-voice-live/` and owns final browser/type review.
Pollable handoff is `/tmp/nyxid-composer-voice-status.md`; source hashes are in
`/tmp/nyxid-composer-ui-source-inputs.json`.

### Final parent validation of the local UI candidate

- TypeScript: `npx tsc -b` passed; `target/composer-ui-parent-typecheck-final.log`.
- Chromium composer voice, pills and settings: **24 passed**;
  `target/composer-ui-parent-e2e-verified.log`.
- Parent browser probes: **10 passed**, including light/dark at 320/1280px,
  unplug/replacement testing, denied permission, unavailable dictation, and
  End → Close → reopen with the saved mode; `target/composer-voice-live/ui-results.json`.
- Additional composer regression files: **47 passed across 2 files**;
  `target/composer-resize-unit.log`. The 122 voice/dictation tests above also pass.
- Changed-path lint and whitespace pass. Resize-specific lint is recorded in
  `target/composer-resize-lint.log`.
- The long-draft browser test initially clicked coordinates captured during
  auto-growth animation and missed the handle. A forced four-frame interleaving
  reproduced a pointerdown on the paragraph; locator stabilization targets the
  separator. Both resize tests now wait for that real actionability condition,
  without changing height assertions, timeouts or retry settings. Evidence:
  `target/composer-voice-live/resize-results.json`.

The live review remains **http://127.0.0.1:4615/assistant**, connected to production.
No commit, push, PR update, CI trigger, backend deployment or wizard regeneration
was performed during this revision. Production dictation remains unavailable
until its backend route is deployed. Full prior release checks below are historical.

## Established evidence and approved decisions

The baseline composer uses Tiptap, atomic mention nodes, attachment handles and
editor-owned selection/history. Existing voice already owns source/billing,
Grok PTT, mute/speaker/resume/end, tasks and conflict/rejoin; these remain available.
All five supplied design screenshots were inspected for permission, aurora,
controls, narration and dictation. The implementation uses NyxID theme tokens and
responsive composer geometry, including a separate mobile text row.

The baseline had no dedicated transcription route. A oneshot `transcrib` match
was a model exclusion, and generic WebSocket response-token metering did not
establish a transcription-duration accounting contract. The recommendation was
written early to `/tmp/nyxid-composer-voice-status.md`: use a dedicated bounded,
server-owned audio-only relay rather than expose a generic provider socket.
Parent approved that transport and reuse of the administrator-authored OpenAI
`voice_seconds` tariff and normal prepaid-window billing primitives. There is no
browser Web Speech fallback or browser/provider credential exposure.

Official documents retrieved by parent directly:

- `/tmp/openai-realtime-transcription.md`: transcription sessions, manual commits,
  PCM16 mono24kHz, `gpt-live-transcribe`, item-ID deltas/completions, no word timings.
- `/tmp/openai-live-transcribe-model.md`: realtime/transcription sessions and
  audio-input-duration billing.
- `/tmp/openai-realtime-server-events.md`: completed transcription usage is
  required; duration variant is `{type:"duration",seconds:number}`.
- `/tmp/openai-live-sideband.md`: Live output fragments expose real provider
  session-timeline `start_ms`/`end_ms`. Baseline assembly retained segment timestamps
  but the socket projection dropped original fragment boundaries.

The approved initial dictation source is platform OpenAI, paid by the acting
person. The fixed provider model is visible in choices and duplicate conversational
catalog model rows do not create duplicate dictation sources. Every actually
charged synced input-duration and per-start request component is disclosed before
capture; rates come from existing authored tariffs, never constants. Dictation
charges confirmed **input audio seconds**, distinct from conversational call time.

## Completed implementation steps and acceptance criteria

1. **Bounded transport and accounting — completed.** Added first-party human,
   same-origin dictation options/WS routes and a fixed server-owned OpenAI
   transcription session. Browser ingress accepts bounded PCM and Stop only.
   Expected provider type/model/audio/null-VAD configuration is validated before
   forwarding. Reserve before spend, renew within120sec cap, bound frames/backlogs
   and periodically commit. Acceptance: no secret or arbitrary provider command
   reaches the browser; no admission bypass; usage comes only from required valid
   provider duration, never elapsed clocks. Real Mongo/loopback regressions pass.

2. **Authority, ordering, settlement and recovery — completed.** Reused exact
   live private-thread/key/agent/org/platform ACL/flags/funding/concurrency gates.
   Dictation selects GET `/realtime`; conversational voice retains POST
   `/live/sessions` plus WS attach. No additional tool grant for a valid bound
   platform specialist; no BYOK widening. Canonical server order survives B delta
   arriving before A completion with no A delta. Completed items replace interim
   text and deduplicate. Exact fractional carry and item IDs checkpoint atomically
   before UI acknowledgement. Fenced closing reads latest durable total before
   sealing money and freezes checkpoints. Stop/disconnect drains forwarded audio
   even without a UI sink; recovery never replays unknown audio. Acceptance:
   stale/revoked/rebound/missing credentials and stale workers cannot gain authority
   or lose/double-charge current usage; malformed present billing fields refuse;
   unrelated provider events cannot starve lifecycle checks. Mongo regressions pass.

3. **Intentional media lifecycle and draft review — completed.** Shared capture
   owner and generation fences cover pending/denied/unavailable/granted permission,
   late grants, cancel, scope/account change, feature loss, revocation and unmount.
   Encoder context starts during user activation; close is idempotent. Separate
   microphone and AudioLines actions, persisted hold/toggle configuration, hover
   and explicit keyboard/touch entry, stable pointer/key release tracking, actual
   waveform, interim grey text and X/Check review are integrated. Acceptance:
   no simultaneous capture and no auto-send; cancellation preserves draft;
   confirmation inserts at saved rich selection in one undo transaction with
   mentions/attachments intact; confirm/cancel/Escape restore editor focus and
   selection. Unit regressions and parent browser checks pass.

4. **Integrated voice controls and live narration — completed.** Compact active
   mute/speaker/end/status controls retain expandable settings/pricing/tasks,
   Grok PTT, resume and conflict/rejoin. Same actor/thread repeat-open preserves
   panel token/session; close/navigation/account/flags fence stale callbacks.
   Active voice remains accessible during typed work/attachments. Real analyser
   levels drive themed aurora, with zero output level on speaker mute and reduced
   spatial motion. Exact provider fragment bounds reach dedicated bounded DTOs;
   persisted memoized rows receive a meaningful fragment-boundary revision and
   segment IDs deduplicate history. Acceptance: no invented word clocks or fake
   amplitudes; captions follow the actual **playback timeline**, including speaker
   mute, not a claim that speech was heard. Pause/close retain the final timeline.
   Unit red/green and parent light/dark/mobile/reduced-motion probes pass.

5. **Verification, personal review and publication — completed.** The primary
   assistant reviewed the final implementation and upstream integration, resolved
   all identified findings with the requested implementer, and verified the frozen
   source. Acceptance: full frontend tests/lint/build, browser flows, database-backed
   voice/authorization/billing regressions, strict Rust lint, formatting and generated
   wizard freshness all pass. Exact final counts are recorded below. The user preview
   remains available; no paid-provider or physical-acoustic verification is claimed.

## Architecture and changed surfaces

- `backend/src/services/voice/dictation.rs` and `dictation_tests.rs`: bounded relay,
  configuration validation, operation-specific authority, provider order/usage,
  lease-fenced carry/dedup and settlement/drain/recovery with private test injection.
  Production has one fixed official destination; test injection cannot configure it.
- `backend/src/handlers/assistant_voice/dictation.rs`, route/auth/session/runtime
  changes: authenticated browser ingress, shared slot/lifecycle, no conversational
  attach/control misuse or dictation chat receipt. Existing delegated-read denial
  covers the whole assistant namespace; new GET route is in its regression table.
- `services/assistant_voice.rs` and `voice/credentials.rs`: request-local authority
  reused across read-only source eligibility; actual-operation selection avoids
  accidentally requiring both voice and dictation permissions. Discovery does not
  reserve funds and cannot promise later capacity, entitlement or unchanged state.
- Transcript model/service/socket projection: bounded exact provider fragments
  (256 per segment,4096 text bytes), no fabricated fragment/word timing.
- `microphone-capture.ts`, `dictation-pcm.ts`, `assistant-dictation-client.ts`:
  intentional ownership, real sample analysis/encoding, bounded transport and
  canonical item rendering; cancellation drains billing without reviving capture.
- `composer-dictation.tsx`, composer/editor/upload and page/panel integration:
  saved control mode, disclosure, draft-safe review, compact controls, responsive
  layout, source/session identity fences and focus/selection restoration.
- `voice-caption-state.ts`, `voice-captions.tsx`, voice client/hook: live caption
  dedup/revision, exact-fragment playback styling and monotonic final timestamp.
  Playback telemetry is never money, execution/confirmation authority or audibility.

## Verification execution log

Counts below are lane-specific and may overlap; do not sum them as unique tests.
No paid live provider requests or deployment evidence are claimed.

| Lane | Actual outcome | Evidence |
| --- | --- | --- |
| Initial Mongo dictation red |7 passed,5 failed,0ignored,34.98s | `target/composer-dictation-tests-red.log` |
| Fixed BSON checkpoint / full dictation |12 passed,0failed,0ignored,38.30s (parent) | `target/composer-dictation-parent-tests.log` |
| Voice compatibility excluding dictation |54 passed,0failed,0ignored,49.70s (parent) | `target/composer-voice-parent-backend.log` |
| Boundary/billing compatibility |22 passed,0failed,0ignored,19.19s (parent) | `target/composer-voice-boundaries-backend.log` |
| Mute timeline red |1 passed,1 failed: timeline100ms at media250ms while muted | `target/composer-voice-playback-red.log` |
| Focused mute/lifecycle + dictation green |2 files,11 passed | `target/composer-voice-playback-green.log` |
| Full declared `npm test` |540 files,5659 passed,242.57s | `target/composer-voice-npm-test.log` |
| Final affected page/composer/dictation/lifecycle |4 files,68 passed,4.24s | `target/composer-voice-frontend-final-focus.log` |
| Declared `npm run lint` |Exit0;0errors,34 existing warnings; none on changed paths | `target/composer-voice-npm-lint.log` |
| Initial `npm run build` |Failed on formatting cleanup introducing JSX duplication; repaired before final build/affected68-test recheck | `target/composer-voice-npm-build.log` |
| Final `npm run lint` |Exit0;0errors,34 existing warnings; none on changed paths | `target/composer-voice-npm-lint-recheck.log` |
| Final `npm run build` |Exit0: tsc, production Vite, legal prerender, credential-accept bundle, mock-footprint assertion | `target/composer-voice-npm-build-recheck.log` |
| `cargo fmt --all -- --check` |Passed (parent) | Parent review receipt |
| Parent browser regression |22 passed,22.3s | `target/composer-voice-parent-e2e-recheck.log` |
| Parent independent browser probes |11/11 green; real oscillator analyser~0.227 and PCM | `target/composer-voice-review/probe*.json/log/png` |
| Rust Clippy |Final integrated `--workspace --all-targets -- -D warnings` passed | `target/composer-voice-final-rust.log` |
| Required generated wizard artifacts |Integrated rebuild passed; declared Rust freshness test 1 passed | `target/composer-voice-wizard-integrated.log`; `target/composer-voice-final-rust.log` |

Mongo red root cause was confirmed: `doc!{30}` stores valid Int32
`reserved_until`, rejected by the initial Int64-only reader. Parser now accepts
Int32/Int64 with bounds while malformed present checkpoint values still fail.
Assertions/timeouts were not weakened. The initial browser17/22 run is preserved
at `target/composer-voice-parent-e2e.log`; hover/config trigger race was fixed,
stale voice-hidden-with-text expectations updated, and async End assertion now
polls for the actual request. Parent reran once to22/22. Independent probe
assertion corrections (exiting-popup alert scoping and near-zero reduced-motion
duration) did not change product code.

Rust lint repairs remove an unused wrapper and document the explicit relay
boundary argument-count allowance. Parent inspected both; production source is
frozen pending their final lint/authorization/browser receipts. The wizard bundle
manifest includes `frontend/src/app.css`, so parent owns its required generated
artifact refresh and freshness verification. No version source is introduced.

## Reproducible commands and fixtures

From repository root, use the built test executable with the explicit isolated
replica URI (parent owns all remaining DB lanes; never run concurrently):

```sh
NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27019/?replicaSet=voice-test'   target/debug/deps/nyxid_server-4364b07c7739ce97 dictation --test-threads=1
NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27019/?replicaSet=voice-test'   target/debug/deps/nyxid_server-4364b07c7739ce97 services::voice:: --skip dictation --test-threads=1
cargo clippy --workspace --all-targets -- -D warnings
```

From `frontend/`:

```sh
npm test
npm run lint
npm run build
NODE_ENV=test npx vitest run src/hooks/voice-delivery-lifecycle.test.tsx
```

Parent-owned browser command (already green; do not duplicate unchanged):

```sh
npx playwright test e2e/composer-voice.spec.ts e2e/composer-pills.spec.ts e2e/composer-settings.spec.ts
```

`frontend/e2e/voice-fixture.ts::installVoiceFixture(page, "granted" | "pending" |
"denied")` supplies synthetic oscillator MediaStreams through real browser
analysers and PCM worklet; peer and NyxID HTTP/WS/provider events are explicit
mocks. `window.__composerVoiceFixture` exposes permission `grant`, `deny`,
`revoked`, request/stop counters; returned counters track starts/ends/dictation,
stop and PCM. It is test-only and excluded from production. The fixture exercises
B-before-A completion ordering and exact narration fragments; its sample prices
are fixture values, not production constants. Parent's private standalone probe
is `target/composer-voice-review/probe.ts`, with original/recheck JSON retained.

## Frozen source receipt, parent gates and limits

Implementer production source is frozen. SHA-256 source inputs are recorded in
`target/composer-voice-final-source-sha256.json` (modified/new backend and frontend
source, original commit and branch). This excludes parent-owned generated wizard
artifacts and review metadata. Parent's final full receipt supersedes the earlier
backend-input snapshot after the two inspected lint repairs.

Parent authorization/Clippy/final browser confirmation, generated wizard freshness
and Mongo cleanup are in progress at handoff; their final outcomes belong in the
parent review receipt. These are not reported as passing prematurely. The personal review is maintained separately by parent in
`docs/COMPOSER_VOICE_DICTATION_REVIEW.md`, never overwritten here.

Real Mongo transactions plus a loopback provider and browser synthetic-stream
fixtures verify local contracts. They do not verify live account entitlement,
paid-provider invoices/retention, physical microphones, iOS hardware autoplay,
Bluetooth or acoustic AEC. Those limits are explicit; no fabricated provider or
word-synchronization success is reported. Existing conversational provider rollout
gates remain in the subsystem documentation. Parent owns Mongo cleanup and
publication; no additional implementer permission or approval step is introduced.


## Integrated branch verification and preview

The required upstream PRs are now merged: #1901 (`4f402a4f`) and #1904
(`dca7abb7`). Both are included in `629138a0`. The primary assistant resolved the
chat-page conflict by retaining the revised voice integration and upstream's
platform-thread header label, and regenerated the embedded wizard rather than
combining generated hashes. No source-level integration decision remained open.

Integrated frontend verification passed **5,679 tests across 544 files**, full
lint (zero errors; 34 existing warnings outside changed paths), the production
build and **22 Chromium composer tests**. The browser run uses the upstream
configuration's own Vite server. The earlier attempt refused an occupied port
and ran zero tests; it is not counted as verification.

The previous sample-only preview used `http://localhost:4612/assistant?mock=1`
with a separate Vite on 4613. The user rejected that preview, and it is superseded
by the current real frontend on port 4615 connected to production. Its labelled
sample transcript/synthetic media and ignored adapter are historical local
fixtures, not a deliverable or evidence of live provider behavior. Existing
processes remain under parent ownership.

Frozen integrated input hashes are in
`target/composer-voice-integrated-inputs.json`; implementation HEAD `629138a0`.
Final Rust and generated-file execution receipts are recorded below.


Final personal review also caught a duration mismatch in the WebSocket ingress
message count: normal 20 ms PCM frames need 6,000 messages for two minutes,
exceeding the former 4,096-message limit. The handler now derives its finite
limit as `MAX_SECONDS * 50 + 32` and retains the existing per-frame, byte-rate,
funding and backpressure limits. Two regressions exercise full-duration PCM/Stop
dispatch, finite control-flood exhaustion and bounded channel refusal. Parent
reviewed this repair and restarted the Rust build for the final source. The
interrupted integrated build is not counted as a pass. Final source hashes are
retained in `target/composer-voice-publication-inputs.json`.


## Previous transport implementation acceptance record (before UI revision)

This historical record applies to the previous transport implementation only;
it does not validate the current UI revision. Its implementation and review
findings were complete before the user requested the UI revision. Dependencies
#1901 and #1904 are included; no merge or deployment of this PR is performed.

| Final integrated check | Result |
| --- | --- |
| Full frontend tests | 5,679 passed across 544 files |
| Frontend lint | Passed; zero errors, 34 existing warnings outside changed paths |
| Production frontend build | Passed; types, Vite, legal prerender, credential-accept, mock footprint |
| Composer Chromium suites | 22 passed in 28.3s |
| Additional interaction/visual probes | 11 passed |
| Backend test compilation | Passed in 10m37s |
| `voice --test-threads=2` | 90 passed, zero failed/ignored, 59.14s |
| `operation_scope --test-threads=2` | 26 passed, zero failed/ignored, 248.73s |
| `org_agent --test-threads=2` | 29 passed, zero failed/ignored, 102.83s |
| `mw::auth::tests::delegated_account_read --test-threads=2` | 7 passed, zero failed/ignored, 0.01s |
| `billing_route_coverage_smoke --test-threads=1` | 1 passed, zero failed/ignored, 7.60s |
| Strict workspace/all-target Rust Clippy | Passed in 6m08s |
| Integrated generated wizard build | Passed; 175-file source manifest, hash prefix `922e5fdcda8e` |
| `cargo test -p nyxid-cli --test wizard_bundle_freshness` | 1 passed, zero failed/ignored |
| Rust formatting and diff whitespace | Passed |

Backend filter counts overlap and are not a unique aggregate. Database tests use
`NYXID_TEST_DATABASE_URL=mongodb://127.0.0.1:27019/?replicaSet=voice-test` and the
freshly built `target/debug/deps/nyxid_server-4364b07c7739ce97`. Logs are
`target/composer-voice-final-rust.log`, `target/composer-voice-final-backend-tests.log`,
`target/composer-voice-integrated-{test,lint,build}.log`, and
`target/composer-voice-integrated-e2e-recheck.log`. Existing macOS linker/dependency
advisories are recorded in the personal review; no new source lint is waived.
The final snapshot check found no source/build-input changes after these checks.
