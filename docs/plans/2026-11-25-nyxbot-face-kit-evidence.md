# Face-kit implementation evidence and direct review

Implementation and scoped checks are complete. Final native measurements and
review artifacts are recorded below. No commits, pushes, merges,
deployments, releases, publications, or protection changes were performed.
The initial user-owned `CLAUDE.md` prohibition remains intact.

## Current product defaults and scope

NyxBot starts with **Round / Tall / Core Nyx purple / Gradient face /
Treatment background / Tile**. The purple pair is `#b48cff → #5e00f5`.
Every supported appearance axis remains editable. Randomise includes all
permitted NyxBot heads. The optional original logo remains NyxBot-only.
Specialists keep the original six tint roles and pinned 6/4/6 identity split.
All 35 pairs, 11 treatments, nine dither, nine character styles and twelve
independent voice colourways are implemented. Native general rich animation
uses the accepted static textures and quiet analogues; the expanded native
voice field animates shared precomputed frames on the UI thread.

The original fixture remains byte-exact: 56,264 bytes, SHA256
`9d91953d4e8a97a190ae75ce32f3b9acec1cb94e4024914b8d618a95ef30ea2e`.
The generator preserves original path strings, uses Node-20-compatible JS,
and pins reference extraction; frontend/mobile CI filters include the fixture.

## Direct review ledger

| Review | Resolution and executable evidence |
| --- | --- |
| R1 | Independent choices survive treatment changes. `chooseAvatar` only changes its requested field; `background:treatment` explicitly follows the selected treatment. Shared contracts retain explicit backgrounds and reactions. |
| R2 | Rust `Avatar` models only template/choices; fixtures use face-v1. Stored future maps project defensively, with no retained orb renderer/schema. Backend avatar suite passes 13 tests. |
| R3 | Live lifecycle consumers use the honest resolver: stopping, approval without a turn, waiting, work, drafting, thinking, neutral retry. Shared lifecycle fixtures and web/mobile tests pass. |
| R4 | Reactions resolve character/dither/vector families, per-state stills, quiet analogues and finite one-shots. Shared contracts cover resolution. |
| R5 | Web voice consumes connection states and observes reconnecting→ready within the actual session; an independent accent survives simultaneous listening. Native uses only connection/input/output facts its hook exposes. Hook and voice-panel integration tests pass. |
| R6 | All 11 treatments are distinct from the actual current default through `chooseAvatar`; treatment-derived backgrounds preserve their source appearance. Explicit backgrounds remain stored. Shared contracts and browser/native galleries cover output. |
| R7 | Inversion clips against the full even-odd face complement, including apertures. Portable, browser and native output checks include inverted style. |
| R8 | Rich sampling deliberately keeps eye geometry static; quiet vector frames use complete posed even-odd masks. Native pre-mounts open/closed/left/right frames and switches discretely. |
| R9 | Phosphor has glow/vignette/scanlines, neon has layered glow, halftone has its source background field. All 19 surfaces have distinct native/browser composition hashes. |
| R10 | Randomise samples only valid eligible choices, including constrained outline/texture combinations and constant RNG edge cases. Shared validation contracts pass. |
| R11 | Progress identity includes state and event. One-shots unregister, retain paused progress, do not replay after completion/refocus, and explicit preview/randomise gets a fresh nonce. Scan has its own full-reveal duration. Renderer tests cover completion, pause, fresh events, state changes and creation pause/supersession. |
| R12 | Levels modulate current speaking motion through refs/shared values without restarting its clock. Web test verifies changed output plus unchanged scheduler registration; native quiet/voice tests verify level gates. |
| R13 | Shuffle changes preview faces; reconnect offsets channels briefly; listening draws the halftone/ripple field. Compact/native quiet analogues remain explicit. Shared effect contracts and actual voice banners exercise the paths. |
| R14 | Raw-envelope equality compares maps semantically, preserving unknown keys. A save-twice test exercises sorted server maps. |
| R15 | Both editors scope drafts/baselines by agent and actor, with mounted/auth fences. Web tests cover switched identity and preflight account change; native guard and editor tests cover stale completions. |
| R16 | Compact preview wrappers and renderer roots have explicit 16/24/44 bounds. Actual native/browser galleries exercise those sizes. |
| R17 | Catalogue labels and human analogue explanations replace raw reaction ids. New phrases are present in both Chinese catalogs; typography/i18n guards run in lint. |
| R18 | Latest Round/Tall/Purple default replaces the earlier Ember/tall steering. Six specialist theme tints remain exact. Shared contracts pin defaults and source tint roles. |
| R19 | PNG signature, dimensions, CRC, zlib stream, allowed chunks, alpha, determinism and sync/async equality are tested. Actual browser Canvas comparisons cover all 19 surfaces. Parent independently checked 42 PNG cases and browser/native parity. |
| R20 | Voice end/start/generation/thread/actor/closed boundaries reset capture. Listening requires an actual active connection plus capture gate. The end regression remains intact and passes. |
| R21 | Optional-logo nondefault eye footprints are bounded to keep separate apertures; original round circles remain exact. All offered eye/head topology cases pass. |
| R22 | Web save lock covers preflight and mutation, blocks double save and late edits, and unlocks on errors. Tests exercise pending GET, edit attempts and failure recovery. |
| R23 | Rust and TS use catalogue entry kinds, central bounds and compatibility with shared fixtures; backend authority and source parity checks pass. |
| R24 | Successful create calls record bounded local events; one eligible avatar claims only the matching event while idle. Real activity permanently supersedes decoration even while hidden. Tests cover actual create success, failure, reused A→B with another claimant, hide/show retention and hidden work supersession. |
| R25 | Native quiet rendering pre-mounts complete open/closed/glance masks, switches discretely and uses state-specific accents and actual output levels. Native component tests and the Hermes fixture exercise the path. |
| R26 | Every head/eye/reference grid is checked. Unsafe texture grids use a declared crisp analogue, retain saved settings and show an editor explanation. No topology assertion was weakened. |
| R27 | Shared sampled background and foreground planes eliminate adjacent-cell seams. SVG/native/Canvas/PNG clip the same plane through the exact vector face; parent native and browser screenshots confirm clean gradients. |
| R28 | Confirmed saved appearance drives export/discard/review when parent props lag. Account/mount fences prevent old preflight writes. Web tests cover saved export/discard and acting-account changes. |
| R29 | Explicit adversarial mixes choose contrasting existing palette inks at draw time; saved axes remain unchanged. Transparent/ambient themes and all twelve voice rings are covered. Actual RGB hue/chroma checks reject destructive-red endpoints independently of names. |
| R30 | Native amplitude reads levels only while eligible and leased; reduced/offscreen/unfocused fields remain still. Web/native banner names come from the actual agent. Native gate tests and browser/editor integration pass. |
| R31 | Scheduler clamps negative RAF elapsed time, defensively bounds frame indices and isolates/removes failing renderers. Regression tests plus real browser cycling of all twelve palettes pass. |
| R32 | Native editor uses scroll/layout visibility gates with boolean transitions; mounted previews retain event identity. The actual editor test scrolls both previews off/back. Screen/AppState and live reduced-motion tests cover the other gates. |
| R33 | Exact original fixture path is in frontend/mobile CI path filters as well as relevant Rust filters; generator check pins its bytes. Check/job names and protections are unchanged. |
| R34 | Foreground gradients use a seam-free sampled plane through exact native vector masks. Native frame preparation yields outside React render, with a complete poster and stale-palette cancellation. Actual native screenshots and preparation tests pass. |
| R35 | Explicit painter order is backdrop plane/paths → masked foreground plane → face details/accents. Actual PNG pixel tests across eight backgrounds prevent the blank-face regression. |
| R36 | Native export yields between bounded raster rows/path batches and PNG copy/CRC/Adler blocks. It shows preparation, locks repeated taps, permits navigation and fences file/share against stale actor/mount. Async bytes match sync, independent CRC/zlib checks pass, and stale-share tests pass. Isolated Hermes timings and byte parity are recorded below. |

All numbered review findings R1–R36 are implemented and covered by the
checks described here. Additional scope includes full mix-and-match defaults,
visible voice swatches, Rive/CSP removal, wizard artifacts and source formatting.

## Commands and results

Logs and native screenshots are local review artifacts under
`mobile/review/evidence/` (ignored). The review entry is retained at
`mobile/review/face-kit-entry.tsx`; normal application entries do not import it.

| Command / check | Result |
| --- | --- |
| `node shared/agent-avatars/generate-face-kit.mjs --check` | PASS: 35 pairs, 12 ripple palettes, 18 textured surfaces, 144 specialist defaults. Parent independently compared 74 reference values. |
| `npm --prefix frontend test -- --maxWorkers=4` | PASS: **8,411 tests / 745 files**, 217.20s. Earlier failures were fixed; this is the final full run. Happy DOM emits expected fixture-network/script diagnostics. |
| Focused web renderer/editor/voice/typography run | PASS: **190 tests / 9 files**. Latest targeted run after PNG changes: **28 tests / 5 files**. |
| `npm --prefix frontend run test:e2e -- e2e/face-kit.spec.ts --workers=1` | PASS: **2 Chromium tests**, all twelve palette switches plus actual Canvas/PNG comparisons for all nineteen surfaces. |
| Browser pixel tolerance | Mean RGBA error **0.091–4.033 / 255** per channel; at most one interior channel sample differs by >8/255 outside a two-pixel edge band measured on both rasters. Dense glyph edges account for the largest means. |
| `npm --prefix mobile test` | PASS: **2,842 tests**, 31.77s. Includes shared contracts, honest voice lifecycle, deterministic export, cancellation and preparation. |
| `npm --prefix mobile run test:jest -- --runInBand` | PASS: **80 tests / 12 suites**, including editor scroll gates, native voice reduced/offscreen gates, creation identity, export lock and stale share. |
| `npm --prefix mobile run typecheck` / frontend `tsc -b` | PASS. Rechecked after final encoder changes. |
| `python3 frontend/scripts/test-nginx-policy.py` | PASS: **5 tests**, Rive-only WebAssembly CSP allowance removed. |
| `npm --prefix frontend run lint` | PASS, exit 0: typography and i18n guards pass; 34 pre-existing warnings outside the changed avatar code. No new warnings remain. |
| `npm --prefix frontend run build` | PASS: app, prerendered legal pages, credential-accept and production fixture-footprint checks. Build reports existing large-chunk warnings. |
| `npm --prefix frontend run build:wizard` | PASS: refreshed embedded CLI HTML/manifest/hash after frontend lock/locale changes. |
| `cargo test -p nyxid-cli --test wizard_bundle_freshness` with CI stack | PASS: **1 freshness/source-closure test**; no publish action. |
| `npm run check:native-sync` with temporary npm-exec pnpm | PASS: **33 direct native dependencies, 2 companion pods, 4 permissions**, device/orientation policy. Direct invocation initially failed with `spawnSync pnpm ENOENT`; npm-exec supplied the tool without global configuration changes. |
| Parent `RUST_MIN_STACK=8388608 cargo test -p nyxid avatar` and focused authority projections | PASS: **13 avatar tests**, real MongoDB set/clear/flag and shared owner/admin/member/manager authority, audit privacy, group/notification/machine/future-map projections. No Rust logic changed after that pass; service/model formatting only. |
| `git diff --check` | PASS. |

All commands above exited successfully. Earlier failing intermediate logs
were superseded by the final runs recorded in those files.

## Native execution and limits

Both actual iOS and Android Hermes entries pass **1,765 assertions**. Parent
verified V8/iOS scene hashes, PNG bytes and 144 ripple frame hashes. The parent independently verified **19 scene hashes, 3 PNG byte arrays and
all 144 ripple frame hashes** between Android/iOS, and the independent V8
comparison also passed. Its result is saved locally in
`mobile/review/evidence/parent-cross-platform-parity.json`. Caches remain bounded: masks 48 entries /
6 MiB; stills 16 entries / 8 MiB; ripple one palette / at most 7 MiB of frame URI
storage, plus platform-decoded images. Native voice uses 12 frames at about
7.6 changes/sec, with one focused motion lease.

Android uses an isolated headless API-37 arm64 AVD at `emulator-5580`, no user
AVD snapshots or data changed. `expo prebuild --platform android --no-install`
created an ignored verification project. `:app:assembleRelease` succeeded;
only local test ids/configuration/debug access and review entry selection were
used. The review package contacts no backend. Emulator startup/stressed gallery
numbers are not treated as phone performance measurements.

iOS is parent-owned verification. Xcode 27 required a temporary isolated scene
host because the baseline Expo55 AppDelegate lacks that SDK's required scene
lifecycle. Parent also installed pinned Pods using local release archives and
used deployment-target/arm64 verification overrides. Parent restored the AppDelegate and Podfile.lock byte-exactly after the last
build; `git diff --name-only -- mobile/ios` is empty. Those host changes are
not part of the face-kit implementation.
General native rich character/rain motion remains the explicitly optional
spike; static texture and quiet-motion behavior are implemented.


## Final isolated export measurements

Sync and async outputs are byte-identical for all three cases on each platform.
The fixture displays a lightweight running screen during measurement and only
mounts the gallery/voice frame bank afterward. Heartbeat maximum includes the
last interval before completion, including the PNG encoding stage.

| Platform / style | Async elapsed | Heartbeats / yields | Maximum heartbeat gap |
| --- | --- | --- | --- |
| iOS / flat | 406 ms | 22 / 21 | 34.0 ms |
| iOS / inverted | 869 ms | 50 / 49 | 34.4 ms |
| iOS / halftone | 1,461 ms | 85 / 84 | 29.0 ms |
| Android emulator / flat | 726 ms | 25 / 29 | 78.5 ms |
| Android emulator / inverted | 1,890 ms | 62 / 66 | 138.0 ms |
| Android emulator / halftone | 2,908 ms | 110 / 101 | 143.7 ms |

The Android figures come from the headless API-37 arm64 emulator using software
rendering; they are not physical Android-device latency claims. The final
synchronous 200 ms iOS encoder stall was removed by yielded checksum/copy work.
The native editor stays interactive between batches and cancels before file or
share effects if account/mount ownership changes.

Android Gradle initially reused a bundle older than changed shared files.
Verification explicitly reran `:app:createBundleReleaseJsAndAssets --rerun-tasks`
then assembled/installed the APK; the final bundle timestamp follows `png.ts`.
The final APK build passed. Retained review artifacts are
`android-native-evidence.json`, `android-native-voice.json`,
`android-face-kit-{flat,ascii-inverted,dither-halftone}.png`, and
`android-final.png` in the ignored evidence directory. After the parent’s artifact review, the isolated Android emulator and its
task-local AVD were stopped/deleted. User AVDs are untouched. The parent’s
iOS review simulator was deleted separately. The labelled local component
preview remains running at port 8876.

## Web editor simplification follow-up

The actual `/assistant` agent-details editor now uses Face → Style → Motion
steps with Back/Next and direct step navigation. Palette families, texture
families/variants, collapsed finish/frame/reactions/export controls and compact
previews replace the expanded catalogue. All twelve voice colourways remain
available with a voice ripple preview. Existing save/authority/conflict/export
logic is retained.

Validation after this refinement:

- Editor and agent-details suites: **77 tests / 2 files passed**. Regressions
  cover navigation without dirtying, transient preview state, choices retained
  across steps, collapsed preferences in the save, incompatibility guidance,
  keyboard pickers, reviewed saves and the existing authority/conflict guards.
- Frontend TypeScript and production build passed. ESLint on the changed
  editor/picker/tests passed; typography and i18n guards passed. English,
  Simplified Chinese and Traditional Chinese cover the new copy.
- Browser check used the actual local `/assistant?mock=1&nyxbot=1&avatars=1`
  route and its real details editor with built-in sample data. Changed head,
  eyes, palette, texture and voice colour, saved through Review changes,
  reloaded and checked persistence, then saved Reset to default. Review
  included only persisted preferences. No browser page errors occurred.
- All three steps fit at 320px and 390px viewport widths with no horizontal
  overflow. Screenshots are in `.context/avatar-stepper-*.png`.
- Rebuilt the embedded CLI wizard after the locale updates and verified its
  216-file source closure using the exact SHA-256 algorithm in
  `cli/tests/wizard_bundle_freshness.rs`. Cargo was unavailable in this shell
  and its usual `~/.cargo/bin` location, so the Rust test was not rerun.

Local preview remains running on 127.0.0.1:8876. This is a web editor refinement;
native renderers and the native editor were not changed by this follow-up.

## Save, propagation and step clarity follow-up

Addressed the reported difficulty saving and inconsistent application to the
page. Every web editor step now contains a Save avatar button with local
review, progress/error/saved status and validation feedback. This review
writes only appearance and leaves unrelated profile drafts untouched; the
existing drawer-wide save registration remains available. Removed the web
Download avatar section and its export code.

Confirmed writes now patch existing roster/detail and crew/member/message
caches, bound to the acting account. Older pending reads are cancelled before
they can restore an old appearance. The already-open chat header and voice
panel take the roster appearance ahead of the conversation snapshot, with
null respected as an explicit reset. The previous nullish fallback could
reintroduce a conversation's older custom avatar after resetting it.

The stepper is now connected numbered nodes with a purple active step, a
Step N of 3 count, a task title, short instruction and named Next action.
Full detail and In the app distinguish preview scales; textured choices
explain the simplified small-icon treatment. Full detail remains 96px.

Validation:

- **101 tests / 5 files passed**: editor, real details/review, page integration,
  agent mutation/cache and owner-fencing suites. New regressions cover write
  failures/retry, local review preserving a profile draft, old pending reads,
  owner switching, immediate cache publication and null-reset propagation.
- **Actual application Playwright regression passed** in
  `frontend/e2e/agent-avatar-editor.spec.ts`: saves Wide/Wink/Mint/Dither,
  compares painted SVG geometry/colours on the sidebar, existing chat and
  details header to the in-app previews before reloading, verifies persistence,
  then verifies Reset restores all three appearances without a reload.
  It also checks the 96px preview, absent downloads and all three steps at
  320px and 390px. No page errors occurred.
- Frontend TypeScript and production build passed. The embedded wizard is
  rebuilt for the shared hook and locale source changes.
- Final full frontend lint passed with zero errors and the same 34 unrelated
  existing warnings; typography and i18n passed. Wizard build and independent
  216-file source-closure SHA-256 freshness check passed.
- The user's local Canvas app remains on the actual `/assistant` details
  editor, with Round/Tall/Purple reset saved, Avatar saved status and no
  Download controls.

## Separate avatar page inside the drawer

Overview now shows only the saved appearance and an Edit avatar disclosure.
The existing GroupedDetail navigation opens the editor inside the same drawer.
The controls mount on opening and unmount after a guarded return to Overview;
profile drafts remain mounted. A confirmed save updates the compact preview
even while parent props lag, and a later server appearance supersedes that
local confirmation. Leaving with unsaved changes offers Keep editing / Discard.
After a discard confirmation closes, keyboard focus returns to Edit avatar.

Removed the In the app sample group and redundant Full detail caption. There
is one 96px preview in the editor. Renamed the logo head label to Nyx in the
shared catalogue/generator, preserving the stored logo id and geometry. Chinese
labels on web/mobile also preserve the Nyx name.

Validation so far:

- **115 tests / 5 files passed**: avatar editor, real details drawer, shared
  drawer page navigation, avatar model and full face-kit contracts.
- **Actual application Playwright regression passed**: compact overview with
  no inline form; Edit avatar navigation; saved tile unchanged by drafts and
  updated after save; existing sidebar/chat/details propagation; reopening and
  reload persistence; reset; 320px/390px layout; no obsolete preview/download
  controls. No page errors.
- New integration tests cover keeping/discarding a draft on return, retaining
  an unrelated profile draft, focus restoration, and confirmed saves before
  parent refresh.
- Generated catalogue freshness and the full frontend lint/i18n/typography
  checks passed, with the same 34 pre-existing lint warnings.
- Final production and embedded wizard builds passed, including TypeScript;
  independent wizard source-closure freshness passed. Targeted lint after the
  focus-return change, i18n and typography passed.
- The actual browser regression also passed Keep editing / Discard with normal
  motion enabled and restored focus after the discard dialog fully left.
- Local Canvas remains on the actual Overview tile with the purple default
  saved, the editor closed, and the removed sample group absent.

## Save placement and effects on real chat avatars

Moved Save avatar and save status beside the preview at the top of every
editor step. Request errors and validation feedback also sit there. Removed
the previous bottom action row and obsolete small-icon simplification notice.

The selected dither/character surface previously became Flat below 64px, and
character styles also became Flat below 96px. Removed those display-size
cutoffs from the shared drawing code. The same authored texture, framing,
palette, background and exact eye clip now reach the small avatar surfaces.
The existing grid/eye topology fallback remains specific to unsafe grids.
Reaction selection still uses the actual display size, so chat keeps quiet
motion. Quiet textured reactions reuse cached still paths and pixel planes,
changing only whole-face transforms and process accents; no per-frame texture
reconstruction is introduced. Rich motion stays confined to large surfaces.

Validation:

- **106 web tests / 6 files passed** across the editor, real details drawer,
  avatar renderer, avatar component, composition and complete shared contracts.
- Follow-up renderer/contract run: **9 tests / 2 files passed**, including quiet
  textures at 44px and 112px, unchanged path/plane identities and no canvas for
  quiet rendering.
- **2,842 mobile portable tests passed; 80 native Jest tests / 12 suites passed.**
  Corrected the locale test to require the proper name Nyx verbatim while
  retaining translated-label checks for every other catalogue label.
- **Actual-app Playwright regression passed**: Save is above the face controls;
  saves Wide/Wink/Mint/Dither and Nyx/Dots/Graphite/Braille; matches the painted
  geometry/colours of the sidebar, open chat header and details header to the
  saved preview; checks those effects on the actual home card; verifies
  persistence/reset, drawer navigation and phone widths. No page errors.
- Actual-app screenshots are `.context/avatar-effects-home-card.png` and
  `.context/avatar-effects-save-at-top.png`.
- Full frontend lint, typography, i18n and embedded wizard build passed. The
  default Homebrew Node executable failed to load libsimdjson.36.dylib during
  this work; validation used the installed Node 22.23.5 runtime instead.
- Final production build (including TypeScript), embedded wizard build and
  independent source freshness check passed on Node 22. Final changed-file
  frontend lint passed; the shared TypeScript sources are outside the frontend
  ESLint base path and are checked through TypeScript and shared contracts.
- The local Canvas editor is open with Save avatar above the pickers, next to
  the preview. Existing saved preferences are retained; no data migration is
  needed for the rendering correction.

## One avatar action row, larger app avatars, and visible quiet motion

Replaced the overlapping inline and drawer-wide save controls with one sticky
avatar action row. Save avatar opens an appearance-only review, including on
native form submission. Discard appears for a draft and restores the last
confirmed saved appearance. Saving, completion and errors stay beside these
actions. The avatar registers with the dismissal guard independently of the
drawer-wide save scope. The wider bar is suspended while this page is active,
then restored with unrelated profile drafts when returning to Overview.

The actual home greeting uses a 72px avatar; roster cards and the expanded chat
header use 56px. Increased card padding, row padding and gaps around identities.
Compact scrolled headers remain in the static tier. Home avatars now opt into
the existing visibility-aware scheduler, deriving active moods only from an
observed running status. Textured idle faces now breathe through a cached
whole-face transform, and process pulses have enough amplitude to read at app
size. Static texture paths, pixel planes and eye masks remain shared. Native
quiet motion uses corresponding amplitudes. Still and reduced-motion settings
continue to stop animation.

Validation completed:

- **122 web tests / 9 files passed**, covering editor, drawer, home, headers,
  avatar components, scheduler, composition, shared contracts and page navigation.
- **83 editor/drawer tests / 2 files passed** after the final error placement
  change. Added checks for exactly one avatar Save/Discard pair, hidden general
  save controls during editing, retained profile drafts, scoped discard, and
  reviewed native form submission.
- **Both actual-app Playwright regressions passed.** Saved Dither and Braille
  propagate to the sidebar, chat, details and home; persistence/reset and
  navigation pass. New coverage measures 72px/56px sizes, observes textured
  idle transforms on home and chat, follows a live reply through drafting and
  settlement, verifies reduced motion and checks 390px/320px layout. No page
  errors. Editor Save remains in the viewport on every phone step.
- **7 portable native tests and 12 native Jest tests passed**, including the
  complete shared face contract, voice ripple and animation visibility.
- Production and embedded wizard builds passed, including TypeScript. Full
  frontend lint, typography and i18n passed (34 existing lint warnings, no
  errors). Whitespace checks passed. Validation used installed Node 22.23.5.
- Actual-app screenshots: `.context/avatar-spacing-home.png` and
  `.context/avatar-spacing-chat.png`. The local application continues at
  `http://127.0.0.1:8876/assistant?mock=1&nyxbot=1&avatars=1` with sample data.


## Natural Rive emotes and pinned details preview (2026-11-26)

This revision supersedes the preceding web breathing/pulse direction. Web Home
cards now remain still with factual badges. The greeting and focused chat use
calm eye poses. The details drawer keeps its 112px avatar above scrolling
content, and that avatar previews the isolated editor draft. Saved appearances
continue to propagate through the existing cache/authority path.

The local script-free Rive source/compiled asset live in
`shared/agent-avatars/rive/`. Twelve timelines drive shared true eye apertures,
with fixed head and material. Default reactions are labelled Natural in the
web editor; explicit expressive alternatives remain available. Outline and
inverted ASCII keep still eyes with a visible explanation. The one lazy worker
is capped at 24 scenes, retains paused progress, stops settled work, handles
pending-state/cancellation races and restores complete vector artwork on
failure. Only that worker has the WASM CSP allowance.

The all-emote page is `http://127.0.0.1:8876/avatar-emotes.html`. It uses the
actual app renderer with randomized avatars, same-avatar comparison,
pause/replay, themes and Flat/Dither/Braille. A single voice preview switches
between Listening/Speaking and all twelve colourways. No microphone or
preference write occurs. Actual voice playback levels use 80ms attack / 240ms
release smoothing. This page is a development entry excluded from the normal
production app build.

Verified locally on Node 22:

- 136 frontend tests / 11 files passed, plus 28 focused editor/renderer tests
  after final review changes. Covers saved/draft isolation, exact rest geometry,
  texture preservation, failure fallback, visibility, creation and state contracts.
- 13 browser tests passed: every real Rive emote, actual app save/reset across
  roster/chat/details/Home, live text states and settlement, motion preferences,
  themes, DPR1/2 badges, all voice colourways and Canvas/PNG raster parity.
- 13 mobile shared/export contracts and 29 mobile Jest tests passed. These are
  regression checks of the existing native renderer, not native Rive proof.
- The built worker passed an enforcing-CSP Chromium probe for pending state
  changes, removed pending scenes, pause/resume and interrupting a pose. The SPA
  policy remained unchanged. Five nginx policy checks passed.
- An independent browser inspected the running 8876 server, including the
  gallery and pinned details at 1440/390/320px. The 112px avatar remains pinned,
  Save stays reachable, and no horizontal overflow or page errors occurred.
  Screenshots: `.context/rive-emotes-final-desktop.png` and
  `.context/rive-details-sticky-{1440,390,320}.png`.
- Production build/TypeScript, asset freshness, reference generation and
  whitespace checks passed. Full lint/typography/i18n passed with zero errors
  and 34 existing warnings. The embedded wizard rebuilt successfully; its
  216-file source hash and copied HTML were verified independently. Cargo is
  unavailable here, so no Rust freshness-test execution is claimed.

Native Rive SDK adoption remains a separate iOS/Android build and device
performance proof. No merge, deployment or publication was performed.


## Activity props in Rive (2026-11-26 follow-up)

The user approved extending the restrained eye-only scenes with recognisable
props. Natural web motion now includes a keyboard with coordinated key/tap
bursts, wrench engaging a rotating bolt and then disengaging, a clock, thought
bubble, unchecked approval card, retry arrow, stop tile and warning tile.
Reply completion retracts the keyboard. Voice colourways remain unchanged.

`prop-art.json` supplies one finite vector-art tree for SVG and generated Rive
artwork. The rig authors 21 parent/child transforms across all twelve timelines;
the worker emits changed transforms and the SVG layer applies them directly.
The fixed face, texture and real eye holes remain separate. Full props appear
at 96px+, compact props at 44–95px, and icons omit them. Quiet mode holds props
still; reduced motion and runtime failure retain a representative static scene.
Saved/exported identity never includes a process prop. The local rig is about
44KB and retains its source/asset freshness checks.

The preview adds actual 56px/112px size comparisons beside its larger scene,
randomisation, same-avatar comparison, textures, themes and replay. Local
Chromium inspection verified real changes in key/tap, wrench and clock-hand
transforms with no page errors or phone overflow. Screenshots are
`.context/rive-props-desktop.png`, `.context/rive-props-working.png`,
`.context/rive-props-small.png`, and `.context/rive-props-phone.png`.

Validation: 139 frontend tests across 11 files passed. Eleven browser cases
passed in the full run; two follow-ups passed after scoping the gallery's size
count to its cards (voice has its own prop layer) and waiting for the details
sheet to unmount between repeated badge captures. The combined 13 cases cover
all emotes, actual save/reset propagation, live states, voice colourways,
compact/full sizes, motion preferences, themes/DPR and raster parity.
Production build/TypeScript, isolated worker CSP, five nginx policy checks,
full lint/typography/i18n (zero errors, 34 existing warnings), and whitespace
checks passed. The sticky details UI was also inspected on the running local
preview at desktop/390/320px. The embedded wizard was rebuilt and its hash
verified independently. No native prop-animation parity is claimed.


## Softer motion and matching prop materials (2026-11-26)

The screenshot review requested more natural emotes and a closer match to the
avatar theme. Props now derive their body/highlight/ink material from the
chosen palette; full-size textured props carry a restrained matching grid or
grain. Compact props keep solid fills. Marks retain at least 3:1 contrast
against their material across all 35 palette pairs. The effective rendered
surface drives the prop texture, including a face-grid fallback.

The keyboard has a lower, thinner profile and small rounded tap pads. Clock,
request, retry, stop and warning artwork are 15% smaller with thinner strokes.
Typing uses uneven authored bursts and short key travel; tap pads compress
slightly on contact. Wrench strokes vary in length and pauses while keeping
the bolt coordinated with each turn. The clock ticks and settles, thought dots
use a small swell instead of a five-unit bounce, and gaze shifts look toward
the active prop. The head, eye design and texture remain the saved identity.

Verified: 24 focused rendering/lifecycle/material tests passed, including
readability for every palette and palette/texture propagation. Five browser
scenarios passed for all live Rive emotes, full/compact sizing, saved/reset
appearance across app surfaces, all voice colourways and Canvas/PNG parity.
An independent browser verified the running local preview and pinned details
at desktop, 390px and 320px without page errors or horizontal overflow.
Production build/TypeScript and the enforcing worker-CSP probe passed. Full
lint/typography/i18n and source/asset freshness checks passed. The embedded
wizard's independent source hash remains current; whitespace checks passed.


## One pixel-art presentation for every emote (2026-11-26)

The user rejected the remaining mixture of smooth props and Flat/Dither/ASCII
faces. The preview now uses one Pixel art presentation and one tile frame for
all twelve states, while randomising heads, eyes and palettes. The mixed-style
selector is removed. The actual Natural renderer also uses the shared pixel
compositor, so preview, chat, details and voice characters share this treatment.

Face, true aperture geometry and props are composited together at low resolution,
then reduced to at most eight colours with hard alpha. Nearest-neighbour display
preserves one pixel lattice across the complete scene, including moving props.
The 192/112/56px views use 64/56/56 cells. Saved appearance remains the material
source on actual app surfaces; transient props stay outside stored/exported
identity. A complete SVG remains available if canvas rendering is unavailable,
and the accessible name is exposed on only the visible representation.
Parsed static glyph/texture paths use a WeakMap; per-scene colour caches are
limited to 4096 entries, keeping repeated poses from reparsing large paths.

Validation: 24 focused component/lifecycle/material tests passed. Three browser
scenarios passed in the first run; save/reset and voice tests passed after
preserving the original reduced-motion SVG fallback and scoping the ripple
canvas selector. The combined five scenarios verify all real Rive emotes,
visible canvas changes, at most eight colours, zero partial-alpha pixels,
nearest-neighbour scaling, compact/full sizes, pause/replay, motion preferences,
actual app save/reset, all voice colourways and Canvas/PNG parity. Production
build/TypeScript, lint/typography/i18n and the enforcing worker-CSP probe passed.
Independent local-browser review checks the sticky details at desktop/390/320px;
the embedded wizard source hash remains current. The local comparison remains
`http://127.0.0.1:8876/avatar-emotes.html`.


## Agent details header collapses on scroll (2026-11-26)

The user requested the compact production layout while reading agent details.
The header now registers with the existing CompactTitle chrome/scroll system.
It starts with a 112px avatar, then uses a real 32px avatar next to the name
when the active page scrolls. The status line remains visible and aligns under
the name; the specialist's Created by line hides in the compact view. Returning
to the top restores the expanded identity. The same rule applies to retained
editor/detail pages. The border between header and tabs is removed. Scroll
insets and measured chrome preserve the reader's content position.

Validation: 75 details/compact-title tests passed. Three browser scenarios passed
for actual save/reset, avatar propagation, live chat and the new researcher
header collapse/restore at desktop, 390px and 320px. A separate inspection of
the local 8876 app measured a 32px collapsed avatar, a 78.6px header and a 0px
bottom border. Screenshots: `.context/agent-header-expanded.png`,
`.context/agent-header-compact.png`, `.context/agent-header-compact-phone.png`.
Production build/TypeScript and full lint/typography/i18n passed (zero errors,
34 existing warnings). The embedded wizard was rebuilt for the shared CSS.
