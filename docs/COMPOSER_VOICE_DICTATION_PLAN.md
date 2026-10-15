# Composer voice and dictation implementation plan and receipt

Branch `fix-specialist-bot-voice`, draft PR **#1907**. Preserve original specialist
platform-voice fix **`cc42ae85`**. **Merge only AFTER #1904 and #1901**; neither
prerequisite was merged by this work. Both merged upstream during verification
and are included through main `dca7abb7` at merge commit `629138a0`. The requested Heca implementer is the sole implementer
(gpt-6.1-sol, x_high, full-access); the prohibition applies to spawning any
additional agents/reviewers. Parent personally reviews and publishes. The implementer made no Git publication or live changes. The primary assistant
commits the reviewed work and updates the existing PR. No version source is added (parent classifier: `NO_VERSION`).

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

The user-requested preview runs at `http://localhost:4612/assistant?mock=1`,
using a separate Vite on 4613. It exercises the actual frontend with labelled
sample transcript/synthetic media fixtures, silent playback and no backend API
forwarding. Parent verified dictation review/cancel and voice/narration there
with no page errors, and leaves the preview running for user review. The preview
adapter is an ignored local artifact, not shipped product code.

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


## Final publication acceptance record

This record supersedes intermediate handoff/in-progress entries above. Every
implementation step and identified review finding is complete. Dependencies
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
