# Composer voice and dictation personal review

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

An interactive user preview is available on loopback port 4612, backed by a
separate Vite server on 4613 and the same explicit test fixtures. Its private
adapter blocks backend API forwarding, supplies sample transcripts/synthetic
audio and labels the page as a silent UI preview. An independent browser smoke
verified dictation start/stop/review/cancel and voice/pending captions with no page
errors. No production source or package change is needed for this preview.
Parent deliberately leaves these two processes running for the user's review.

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
