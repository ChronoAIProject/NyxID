# iPad adaptation validation report

**Latest voice status - 2026-11-06:** iOS OpenAI voice now uses native
WebRTC. Two real acoustic Chinese test prompts were recognized in the complete
iPad Simulator App and received corresponding assistant replies. Native output
levels and emitted playback samples advanced. The private WebKit experiment has
been removed. UI mute, reconnect, end and the PTT input gate were exercised.
Continuous PTT speech and physical human listening remain unverified; a later
PTT attempt hit temporary server authorization unavailability. See the final
follow-up. Earlier failed WebKit observations below are historical.

Date: 2026-11-05. Related issue: #2226. Pull request: #2233.

This report records the implemented iPad behavior and the evidence available for this revision. The full App was exercised on an iPad Pro13-inch (M5) Simulator running iOS26.3 using a signed universal Debug build and an isolated QA account/backend. The App loaded its complete `mobile/index.ts` entry point. Earlier component fixtures are recorded separately and do not replace full-App validation.

## Implemented behavior

- Universal iPhone/iPad support, all four iPad orientations, and iPad multitasking enabled. Existing iPhone portrait initialization is retained.
- Window widths of at least900pt show a300pt persistent sidebar. Narrow windows use the compact push drawer, capped at360pt. The content navigator remains mounted when layout changes.
- Workspace sheets use centered cards in wide windows, capped at720pt by860pt, and adapt to their measured keyboard-adjusted viewport. Narrow windows retain the compact presentation.
- Settings, account identity rows, standalone device/key login forms and legal documents use readable content widths. Settings rows can stack when their own available width is small. Root legal Back aligns with the document column and has an accessible label.
- Current-device settings use device-neutral wording. iPad desktop presentation does not force an orientation lock.
- Machine deep links retain usable detail/Back history and consume their request once, with live-state guards against obsolete effects.

## Evidence status

“Verified” below applies to the stated action boundary only. A readable error, an empty list, metadata loading, a seeded sample and a successful external execution are different results.

| Area                    | Observed full-App result                                                                                                                                                                                                                                                                   | Remaining boundary                                                                                                                                                                                                                                                 |
| ----------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Startup/authentication  | Real QA sign-in succeeded. A later cold launch restored the authenticated Home. Deliberate UI Sign Out returned to Auth.                                                                                                                                                                   | Session-expiry, account switching and server-side revocation for the final Sign Out were not independently checked.                                                                                                                                                |
| Pro layout and rotation | Auth and wide workspace rendered in landscape; a fixed chat draft survived portrait rotation, opening Settings, and closing it. Settings cards were centered in the observed wide portrait/landscape states.                                                                               | Narrow Split View/Stage Manager and all transition/input variants remain unverified.                                                                                                                                                                               |
| Root forms/legal        | Latest Root Terms rendered in a bounded column in portrait and landscape; Back returned to Auth. Latest Root Privacy was bounded in landscape. Root Device and Agent Key forms were bounded in landscape and Back worked. Signed-in OS links opened their workspace-sheet forms/documents. | Only initial legal viewports were confirmed. Full document scrolling, external links, valid device/key requests, scan/approval/delivery and all keyboard variants remain open.                                                                                     |
| Local preferences       | Chat column/composer controls persisted after leaving/reopening, then were restored. Dictation language persisted after leaving/reopening, then was restored. Automatic appearance restoration persisted through cold launch. Device-neutral wording appeared in the fresh bundle.         | This does not prove microphone, dictation recognition or every locale.                                                                                                                                                                                             |
| Saved changes           | Profile display name, chat title, agent display name and crew name were saved through the actual UI and independently confirmed by API reads. Invalid avatar URL/agent handle validation and draft-discard guards were exercised.                                                          | Agent/crew creation was not submitted. Memory and other offscreen fields were not edited.                                                                                                                                                                          |
| Approvals/security      | UI approve/reject decisions, a grant revocation and a secondary-session revocation were independently confirmed by API reads.                                                                                                                                                              | QA requests had no executor; no external action completion is claimed. The grant's final stable empty UI frame was not captured.                                                                                                                                   |
| Machine navigation      | Fresh-bundle evidence123 reached MachineDetail with Back and no new SET_PARAMS warning. Evidence124 returned to Machines without automatic re-entry. Saved-login and agent-machine metadata screens were reached separately.                                                               | Inventory/loaded detail remained blocked by the isolated backend's unsupported inventory route. Personal saved-login owner labels fall back inaccurately when inventory is missing. No real machine execution, policy/activity flow or desktop input was verified. |
| Plugins/services        | Catalog and existing connection metadata, advanced/history/detail views loaded. An empty provider credential submission produced visible required validation.                                                                                                                              | No real credential save, OAuth connection, provider request or multi-connection grouping fixture.                                                                                                                                                                  |
| Skills/automations      | Skill metadata/history and automation metadata/detail/run records rendered. A run's linked thread opened.                                                                                                                                                                                  | Skill catalog/package/version reads were blocked by Ornn403. Automation records were seeded metadata, not proof of executor success.                                                                                                                               |
| Chat                    | Actual Send created a conversation and stored the QA user message; the assistant failed at `stage: connect`, and the error details expanded.                                                                                                                                               | No successful model/tool response, Retry/Stop or recovery was verified. A separate code/table sample was seeded reading content, not a model execution.                                                                                                            |
| Attachments             | The real native Files picker opened; cancelling returned to Home, cleared busy state and enabled controls.                                                                                                                                                                                 | No file was selected/uploaded. Camera, Photos and connection attachment paths were not exercised.                                                                                                                                                                  |
| Software keyboard       | A real software keyboard was visible above the composer in landscape.                                                                                                                                                                                                                      | The keyboard was closed in the later portrait draft frame. This is not proof of keyboard-with-draft rotation, portrait, floating or hardware keyboard coverage.                                                                                                    |

Evidence numbers refer to the private QA execution ledger. That ledger contains screenshots, accessibility captures, action boundaries and sanitized readback results; private QA artifacts are not committed with this report.

## Route coverage

The App has63 registration positions and59 unique route names. At the time of this report,56/63 positions (52/59 unique names) have evidence of arrival or partial actions. This includes empty/error states and is not a count of fully accepted pages. The latest legal/auth entry variants do not increase the route count.

| Registration still without arrival evidence | Why it remains open                                                                                                                 |
| ------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------- |
| PluginConnections                           | No real catalog connection with multiple entries was available; separate custom NoAuth entries do not exercise this grouping route. |
| MachinePolicy                               | Real inventory/detail data unavailable in the QA backend.                                                                           |
| MachineActivity                             | Real inventory/detail data unavailable.                                                                                             |
| MachineActivityFilter                       | No reachable machine activity list.                                                                                                 |
| MachineActivityDetail                       | No real machine activity event row.                                                                                                 |
| AuthoredSkillReview                         | No genuine pending acknowledgement fixture.                                                                                         |
| MachineDesktop                              | No online machine/display/context available.                                                                                        |

## Local focused verification

The following commands ran from `mobile/`. Existing passing commands were not repeatedly rerun. Their suites overlap, so the counts must not be added into one unique-test total.

```sh
pnpm exec tsx --test src/lib/nativeDeviceSupport.test.ts src/lib/desktopOrientation.test.ts src/features/machines/desktop/MachineDesktopScreen.test.ts src/app/AppDrawerNavigation.test.ts src/app/WorkspaceHeader.test.ts src/app/WorkspacePageSurface.test.ts src/app/navigationHierarchy.test.ts src/app/workspaceChatNavigation.test.ts src/components/workspace/WorkspacePushShell.test.ts src/components/workspace/workspacePushState.test.ts src/components/workspace/WorkspaceSheetSurface.test.ts src/components/settings/SettingsList.test.ts src/features/account/AccountSettingsScreen.test.ts src/features/account/NotificationSettingsScreen.test.ts src/features/display/ChatDisplaySettings.test.ts src/features/auth/AuthHomeScreen.test.ts src/features/auth/DeviceLoginScreen.test.ts src/lib/accessibility/workspaceFocusState.test.ts
# 202/202 passed

pnpm exec tsx --test src/features/auth/rootEntryLayout.test.ts src/features/legal/LegalDocumentScreen.test.ts src/features/auth/DeviceLoginScreen.test.ts src/features/auth/agentKeySelectionFields.test.ts src/features/legal/legalMarkdown.test.ts
# 21/21 passed

pnpm exec tsx --test src/features/account/DisplayPrivacySettingsScreen.test.ts src/features/account/voiceSettingsModel.test.ts src/features/assistant/voice/dictationLocale.test.ts
# 10/10 passed

pnpm exec tsx --test src/features/machines/MachinesScreen.test.ts src/features/machines/MachineSettingsPages.test.ts
# 10/10 passed

pnpm exec tsx --test src/components/ApprovalCards.test.ts src/features/activity/ActivityDetailScreen.test.ts
# 14/14 passed after reproducing two failing dynamic-domain link assertions

pnpm check:native-sync
node --check scripts/check-native-sync.js
node --check scripts/lib/native-device-support.js
plutil -lint ios/NyxIDMobile/Info.plist ios/NyxIDMobile.xcodeproj/project.pbxproj
# All passed
```

The native-sync check covered31 direct native dependencies,2 companion pods,3 declared permissions, and universal-device/orientation/multitasking consistency. Changed-file formatting and diff checks were also performed. AuthHomeScreen and the three copy-only source files have confirmed inherited formatting debt; unrelated full-file formatting was not included. The new root/legal and machine files passed their focused formatting checks.

The Machines regression tests first failed against the prior source with the real StackRouter's unhandled SET_PARAMS action, then passed with the repaired source. They verify list/Back history, request consumption, replay and stale/removed-route behavior. The root/layout tests exercise the actual source's rendered tree, styles, state and callbacks. These harness tests do not prove native Yoga dimensions, hit-testing, scrolling, VoiceOver or OS keyboard behavior.

No reliable repository-native affected mobile typecheck is available. Full frontend suite/typecheck/production build is delegated to GitHub CI under the personal local-validation policy. The simulator's native Debug build is separate from a local frontend production build.

## Changed-file static checks

From `mobile/`, the following root/legal and machine checks passed:

```sh
../frontend/node_modules/.bin/prettier --check src/components/BlurBackButton.tsx src/features/auth/AgentKeyLoginScreen.tsx src/features/auth/DeviceLoginScreen.tsx src/features/auth/deviceLoginStyles.ts src/features/auth/rootEntryLayout.test.ts src/features/legal/LegalDocumentScreen.tsx src/features/legal/LegalDocumentScreen.test.ts
../frontend/node_modules/.bin/prettier --check src/features/machines/MachinesScreen.tsx src/features/machines/MachinesScreen.test.ts
```

The three copy-only files were explicitly checked with Prettier and compared against HEAD using the same formatter. All three have matching pre-existing formatting debt; the final patch preserves only five text/comment substitutions. The earlier CI-accessor patch also has verified pre-existing formatting debt in three of its four files. `git diff --check` passed at the repository root before each commit.

The explicitly requested local CI reproduction ran the failing typecheck before and after the accessor repair:

```sh
CODEX_ALLOW_FULL_FRONTEND_VALIDATION=1 pnpm typecheck
# Before: two TS2305 errors. After: passed.
```

The 1,702-test full-suite result belongs to GitHub CI, not a local full-suite run. The full package typecheck was not repeated for the later focused fixes. A signed native Debug simulator build was performed for the complete App; no local frontend production build was performed.

## GitHub CI revision boundary

At committed head `107f5d1d7`, Mobile passed1,702 tests, typecheck and native-sync. CI Pipeline, Release manifest and CodeQL also completed successfully; other jobs were skipped or neutral. That result predates the 12-file root-layout/copy/machine follow-up patch committed and pushed as `7315fb6dc`. The new revision requires its own CI result; the earlier result is not full CI verification of this follow-up patch.

## Refresh against current team main

During delivery, team main advanced to `10444489b`, including the same two production URL-accessor fixes. This temporarily made the PR conflict and prevented new pull-request Checks from starting. The merge retains the stronger dynamic-domain test mocks and assertions from this PR. All mobile source/test bytes are identical before and after this merge; only upstream non-mobile changes were newly integrated. The 14 approval-card/detail tests were rerun and passed after resolving the two test-only conflicts. The earlier focused iPad results remain associated with unchanged mobile source.

## Follow-up CI test typing repair

At merge head `baea79688`, GitHub Mobile CI passed all **1,709 tests** and native-sync, then reported type errors in the three new regression test files. The Node helper `fileURLToPath` was receiving the ambient React Native `URL` type; these tests now explicitly import Node's `URL`. The machine router fixture now supplies route keys and normalizes router results through `getRehydratedState`, preserving complete navigation-state types without casts or disabled assertions. These changes affect tests only and do not alter the App source used for the Pro observations. The final commit needs a fresh CI typecheck result; the 1,709-test result alone is not a complete Mobile CI pass.

## Remaining acceptance work

- Long-page scrolling and bottom reachability are unproven for legal documents, automation run lists/pagination, the lower key-creation form and memory fields. The final wide-table column is only partly visible; successful horizontal reachability is unproven. CUA scroll attempts did not establish viewport movement, so they are not counted as passes or attributed to a product defect without further evidence.
- Narrow Split View/Stage Manager, live threshold changes in the complete native App, all keyboard/rotation combinations, VoiceOver, maximum Dynamic Type, Reduce Motion, physical iPad touch and external keyboards remain incomplete. Harness geometry/transition coverage does not replace them.
- Billing loaded data, real notifications/Telegram delivery, speech/microphone, camera/Photos permission branches, valid device/key authorization, provider/OAuth connection, model/tool execution and remote desktop input require usable backend/provider/device fixtures.
- Full-App iPad mini: **NOT RUN**. The complete App is installed, but the current Simulator menu/screenshot control session is blocked. Earlier mini component fixtures are not accepted as full-App evidence. This section must be updated only after a real complete-App mini run.

The current evidence supports the implemented source changes and the specific observed Pro action boundaries. Complete iPad acceptance remains open for the items above.

## Production voice startup follow-up — 2026-11-06

The user's manual production test revealed OpenAI `provider_create:403:forbidden`
(NyxID code12501) with the saved voice `aube`. Read-only metadata from the
running App showed a single available platform OpenAI / `gpt-live-1` option.
Using the actual iPad Settings and call controls, a same-source, same-model
comparison connected successfully with `marin` and visibly reached `Listening`;
the ended test receipt recorded0:48. Selecting Aube again reproduced the same
403 at provider creation. This establishes the failure for Aube on the current
platform source, without attributing the generic upstream error to a particular
account policy or claiming it affects every OpenAI account.

The current official [Live create reference](https://developers.openai.com/api/reference/resources/live/methods/create)
lists 31 built-in voices, omits `aube` and spells `shida` rather than the historical
seed's `shitan`. The subsequent compatibility review requires preserving all 32
historically seeded IDs, labels and order, with `shida` appended as choice 33.
Startup backfill only appends new choices to an exact historical voice array on
a non-admin-modified OpenAI catalog row; it never removes a saved voice. The
atomic predicate and voices-only update preserve admin edits, custom arrays,
models, delegation, pricing and other metadata. Keeping legacy IDs selectable
does not establish provider support or fix the observed upstream Aube refusal;
the person can explicitly choose Marin or another available voice. This code
requires backend deployment; the production catalog has not been edited here.

Native calls and Voice Settings now retain the exact saved service, connection,
key source and model. They wait for both options and settings before enabling
startup or saving. If a saved voice disappears from the live catalog, the App
shows the old voice and asks for an explicit replacement in Settings; it cannot
start or overwrite other preferences until a valid voice is chosen. The voice
picker remains usable even when only one replacement voice exists.

The account is left on Marin for continued manual testing. The source-selection
bug was discovered during investigation but did not cause this account's
original failure because it had only one available source. Session creation,
connection readiness and End are distinct from acoustic verification: spoken
input/output, voice task delegation, interruption and all other voices have not
been accepted by this comparison. Earlier incomplete iPad acceptance items
remain open.

After cold-loading the final mobile source in the full native App, the original
production chat displayed `Marin voice`, started successfully, and visibly
reached `Listening 0:09`. End returned to the normal composer and produced a
`0:25` receipt. The App remains signed in with Marin selected, with no active
call. This second successful startup used the final source; it still does not
establish spoken input/output or interruption behavior.

Focused voice validation from `mobile/` passed **51/51**, with no failures or
skips:

```sh
pnpm exec tsx --test src/features/account/VoiceSettingsScreen.test.ts src/features/account/voiceSettingsModel.test.ts src/features/assistant/voice/AssistantVoiceConversation.test.ts src/features/assistant/voice/voiceSelection.test.ts
```

Changed-file checks passed:

```sh
# From mobile/: seven files passed.
../frontend/node_modules/.bin/prettier --check src/features/account/VoiceSettingsScreen.tsx src/features/account/VoiceSettingsScreen.test.ts src/features/account/voiceSettingsModel.ts src/features/account/voiceSettingsModel.test.ts src/features/assistant/voice/AssistantVoiceConversation.test.ts src/features/assistant/voice/voiceSelection.ts src/features/assistant/voice/voiceSelection.test.ts

# From repository root: passed.
rustfmt --edition 2024 --check backend/src/services/inference_service.rs backend/src/services/inference_voice.rs backend/src/services/inference_voice_tests.rs
git diff --check
```

`AssistantVoiceConversation.tsx` has reproduced pre-existing Prettier debt:
both its HEAD baseline and current content fail the same formatter. Its small
functional diff preserves the existing layout instead of reformatting the
whole component. No reliable affected mobile typecheck exists; full frontend
suite/typecheck/build is delegated to GitHub CI. The earlier green CI result at
`99d023c6c` predates this voice follow-up and does not verify these changes.

## Reopened voice input investigation — 2026-11-06

The user's next attempt still produced no spoken conversation. Reproduction
used the complete signed native App, production authentication, the same Marin
source and hands-free mode. Safe diagnostics showed one enabled/live/unmuted
input track, connected ICE/peer, an open provider data channel, a running audio
context and accepted playback. Outbound RTP grew from37,480 to167,422 bytes
between samples9 and26. Inbound packets also advanced, but their audio energy
remained near zero and no user caption was persisted. These counters establish
transport activity only, not intelligible microphone input.

The media-source audio level repeatedly equalled0.1000091555528428. Official
WebKit source provides a concrete explanation to test:

- [Mock capture defaults on for iOS Simulator](https://github.com/WebKit/WebKit/blob/febeda30215e0030c6279cc8a6fdbddd81ec8d31/Source/WTF/Scripts/Preferences/UnifiedWebPreferences.yaml#L4344-L4357).
- [The four built-in test microphone names](https://github.com/WebKit/WebKit/blob/febeda30215e0030c6279cc8a6fdbddd81ec8d31/Source/WebCore/platform/mock/MockRealtimeMediaSourceCenter.cpp#L64-L70)
  are `Mock audio device 1` through `Mock audio device 4`.
- [The Cocoa test capture implementation](https://github.com/WebKit/WebKit/blob/febeda30215e0030c6279cc8a6fdbddd81ec8d31/Source/WebCore/platform/mediastream/cocoa/MockAudioCaptureUnit.mm#L66-L82)
  generates a0.1-amplitude150Hz hum and periodic tones rather than the person's
  microphone input.

At the end of the reproduction, the provider generated its Chinese idle
warning, the App persisted one assistant caption, output level reached0.86
and decoded inbound energy increased to0.253989. The call quiet-ended at3:02.
This proves that assistant speech media and text can return; physical speaker
audibility and a response to an actual user utterance were not established.

The follow-up changes:

- Match only those four exact mock microphone names after capture, emit a
  bounded `mock_capture` boolean, stop all acquired tracks and refuse the
  offer/session. No raw device label or identifier enters diagnostics. The
  original message asked for a physical device; the follow-up below corrects
  that unsupported generalization. A non-match does not certify real input.
- Keep the App connecting until provider-ready, authoritative active controls
  and the open control socket all agree. Mute/PTT state does not prevent the
  connection from becoming ready.
- Report playback rejection as a recoverable Resume-audio prompt. A retry can
  restore playback without terminating the call or misreporting microphone
  permission. An ended microphone track terminates and cleans up media.
- Add bounded track/state/RTP diagnostics with generation fencing and a
  non-overlapping two-second sampler. Missing statistics remain unknown;
  cumulative packet counters do not by themselves prove speech.

No physical iPad or iPhone was connected in the device inventory. This change
does not replace a real microphone with a fabricated utterance or claim
real-device voice acceptance. Full iPad acceptance gaps recorded above remain
open. The transient authenticated503/code12900 observed earlier recovered
before the RTP call and is tracked separately from this capture limitation.

After cold-loading the final source, Start in the original production chat
returned `input_source: ready, mock_capture: 1`. The full App visibly displayed
`Unable to start` and the then-current Simulator/physical-device message
(at revision `b98144603`). A temporary
development observer counted **zero calls to the voice-session creation API**;
the client closed locally. This confirms the known mock source in the current
runtime and the guard's actual UI/creation boundary. No new call receipt or
provider session was created by this attempt.

Focused checks from `mobile/`:

```sh
pnpm exec tsx --test src/features/assistant/voice/AssistantVoiceClient.test.ts src/features/assistant/voice/useNativeVoice.test.ts
# 115 passed, 0 failed
pnpm exec tsx --test src/features/assistant/voice/voiceMediaDocument.test.ts
# 35 passed, 0 failed
../frontend/node_modules/.bin/prettier --check src/features/assistant/voice/AssistantVoiceClient.ts src/features/assistant/voice/AssistantVoiceClient.test.ts src/features/assistant/voice/useNativeVoice.ts src/features/assistant/voice/useNativeVoice.test.ts src/features/assistant/voice/VoiceMediaBridge.tsx src/features/assistant/voice/voiceMediaDocument.ts src/features/assistant/voice/voiceMediaDocument.test.ts
```

The150 focused tests cover readiness ordering, late/obsolete callbacks,
playback failure and recovery, input-track termination, known mock capture,
no provider creation on local failure, bounded diagnostics and stale RTP
sampling cleanup. No reliable affected typecheck is available. Full frontend
suite/typecheck/build is delegated to GitHub CI; no result for this new
revision is claimed.

The backend's targeted test command was attempted from the repository root:

```sh
CARGO_BUILD_JOBS=2 CARGO_TARGET_DIR=/Users/abigaildeng/Documents/work/NyxID/target NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:33018/nyxid_test_probe?directConnection=true' cargo test -p nyxid --bin nyxid-server services::inference_voice_tests -- --nocapture
```

Compilation rebuilt changed dependencies and reached the current backend, but
the external disk guard stopped the process group with exit 75 after free disk
fell below 2 GiB. **No backend tests began executing.** The new migration tests
cover ten MongoDB fixtures, raw document preservation and idempotence, but are
not claimed as passed; backend CI must verify them. No old binary was used to
represent validation of the new backend, and no shared caches were deleted.


## Native iOS voice replacement - 2026-11-06

Apple supports selecting the Mac microphone in the Simulator's
[Sound Input settings](https://developer.apple.com/documentation/xcode/configuring-the-environment-of-a-simulated-device#Set-the-audio-input-and-output).
The earlier physical-device requirement was incorrect. Rebinding this Simulator
to built-in Mac input/output let independent native dictation stay active for
over28 seconds, though dictation recognition was not established. WebKit's last
real-capture attempt acquired a non-mock track but immediately ended it with
`AUIOClient_StartIO failed (-66637)` and captureFailed before offer creation.
Previous sandbox-extension and RemoteIO failures do not imply Simulator speech
is unsupported.

All prior AppDelegate private WebKit SPI and the opt-in environment variable
are removed. iOS OpenAI calls use `react-native-webrtc`155.0.1 through the existing
media facade. The module loads only after the iOS guard. Android and Grok keep
their previous WebView paths; those were not newly accepted by this run.
Native media has bounded startup/ICE/answer timeouts, abort/generation fences,
single microphone ownership, input disabled until readiness, track/channel/peer
failure cleanup and explicit native stream release. Diagnostics contain bounded
states and aggregate counters, never audio, transcripts, SDP, candidates,
device labels/IDs or credentials. Native WebRTC plays remote audio, speaker mute
disables it and PTT attenuates its volume.

Actual complete App on iPad Pro13 M5 / iOS26.3, Marin:

- Signed native Debug build succeeded; installation retained account and chat.
  Normal startup used current complete Metro source with no private SPI or flag.
  Native media, provider-ready and connected were confirmed.
- A known Chinese TTS prompt played through the built-in Mac speaker into its
  microphone was recognized; the assistant replied with the requested
  test-success phrase. A second prompt requesting a sentence about today's
  test passing was recognized and received its matching assistant reply.
- These were acoustic synthetic-speech tests, not injected input, canned captions
  or human-user speech. The safe observer did not intercept provider creation or
  change media behavior. Native input peak0.45048/output0.31483, RTP and emitted
  playback samples corroborate the path. They do not independently prove
  physical speaker audibility to a human.
- UI microphone mute disabled the local track; speaker mute disabled the remote
  track and cleared playback_audible. A subsequent native call reconnected.
  A real PTT drag enabled capture144ms and disabled it upon release. This verifies
  gating, not continuous PTT recognition.
- End left client closed/input disabled/no active native adapter. The first two
  calls also reached server quiet-end after inactivity during UI checks. A fresh
  PTT call subsequently failed with connection loss/network502; one deliberate
  UI retry returned temporary OAuth-authority unavailability. Continuous PTT
  speech, interruption, human listening, physical devices, iPhone, Android and
  Grok remain unaccepted by this run.

Focused commands from `mobile/`:

```sh
pnpm exec tsx --test src/features/assistant/voice/NativeOpenAIMedia.test.ts
# 17 passed, 0 failed
pnpm exec tsx --test src/features/assistant/voice/voiceMediaDocument.test.ts
# 44 passed, 0 failed
pnpm exec tsx --test src/features/assistant/voice/AssistantVoiceClient.test.ts src/features/assistant/voice/useNativeVoice.test.ts src/features/assistant/voice/AssistantVoiceConversation.test.ts
# 147 passed, 0 failed
../frontend/node_modules/.bin/prettier --check package.json src/features/assistant/voice/NativeOpenAIMedia.ts src/features/assistant/voice/NativeOpenAIMedia.test.ts src/features/assistant/voice/createNativeOpenAIMedia.ts src/features/assistant/voice/VoiceMediaBridge.tsx src/features/assistant/voice/voiceMediaDocument.test.ts
# passed
```

The pnpm-lock formatting warning reproduced on unchanged HEAD with the same
Prettier. pnpm's generated format is retained; the patch adds34 dependency lines.
From repository root, `node mobile/scripts/check-native-sync.js` passed32 direct
native dependencies,2 companion pods,3 permissions and universal iPad checks;
`git diff --check` passed. CocoaPods `pod install --no-repo-update` succeeded.
The authorized signed Simulator Debug build succeeded with SKIP_BUNDLING=1,
arm64, current iPad destination, Debug scheme NyxIDMobile, dev.nyxid.ipadqa bundle
and ad-hoc signing. It installs the complete App, not a component fixture.

Independent review checked native155.0.1 API compatibility, cancellation,
readiness, teardown, dependency/Pod/Xcode embedding and private-SPI removal.
No blocking issue was found. No reliable affected typecheck exists; full
frontend suite/typecheck/build belongs to GitHub CI. No current-head CI result
is claimed. Existing iPad route/window/business gaps remain open.

The duplicate-transcript screenshot was audited without redesigning the UI:
main messages and the voice caption ScrollView both render the same speech.
Backend stable segment IDs update one stored message. Recommended design:
one main-chat transcript and compact bottom call controls.

## Chat details preserves transcript position — 2026-11-06

Opening the top-right Chat options button could animate the retained iPad chat
list to its oldest messages. Back then left the reader at offset 0. The actual
long-chat reproduction measured content 6332pt, viewport 811pt and offset 5521pt
before navigation, followed by native offsets 2622, 102 and 0 while details opened.
The conversation identity did not change. No JavaScript scroll command was
observed during that animation; its exact UIKit origin is not established.

The transcript hook now saves the last focused position and follow intent on
blur, ignores hidden native offset/gesture events, and restores the same chat
once on return. History readers keep their saved offset, clamped to current
content; latest followers use the current end. Current hidden focus-cycle
positive dimensions remain available for restoration after keyboard dismissal.
Zero-size hidden layouts, old focus/chat callbacks and unmounted callbacks are
rejected. Chat changes, another blur and an explicit user drag cancel queued
restoration. A screen first mounted while hidden has no reading snapshot to
restore and follows its normal first-visible initialization.

Final-source complete-App Simulator verification used iPad Pro 13 M5 / iOS 26.3,
bundle `dev.nyxid.ipadqa`, retaining the existing signed-in account. Metro was
restarted with its cache cleared after source edits, then the App was reloaded;
CI-mode Metro reload alone had reused an earlier bundle.

- Normal details/Back: before 5521, native offscreen 0, saved visible 5521, returned
  native 5521 / hook 5521 / distance-to-end 0 / following=true.
- With software keyboard: before 5995 at viewport 337, details hid the keyboard
  and measured viewport 811 while preserving the visible 5995 snapshot. Back
  restored native 5521 / hook 5521 / viewport 811 / distance-to-end 0. The keyboard
  stayed dismissed, and the same chat remained selected.
- Earlier iterations also passed repeated normal details/Back cycles. Final
  source passed the normal and keyboard cycles above. No messages were sent,
  no voice call was started and no account/chat data was cleared for this fix.
- Historical offset retention, hidden content changes, stale callbacks, cold
  hidden initialization and gesture cancellation are covered by the real hook
  regression tests. Native accessibility scrolling moved the list, but CUA drag
  attempts did not produce a native reader gesture; manual middle-of-history
  drag is therefore not claimed as passed in this Simulator run. No ambient
  voice transcripts or account identifiers are included in this evidence.

Focused verification from `mobile/`:

```sh
pnpm exec tsx --test src/features/assistant/AssistantScreen.voice.test.ts src/features/assistant/useTranscriptScrollFollow.test.ts src/features/assistant/assistantTranscriptScroll.test.ts
# 74 passed, 0 failed, 0 skipped
```

Changed-file static checks from repository root:

```sh
/Users/abigaildeng/Documents/work/NyxID/frontend/node_modules/.bin/prettier --check mobile/src/features/assistant/useTranscriptScrollFollow.ts
# passed

git diff --check
# passed
```

New regression-test ranges were formatted. Whole-file Prettier warnings for
AssistantScreen.tsx, AssistantScreen.voice.test.ts and
useTranscriptScrollFollow.test.ts also reproduce on their unchanged HEAD
versions; unrelated formatting debt is preserved. Independent review verified
that Assistant, AssistantContext and AssistantDetails share the ContentStack,
that useIsFocused follows the correct navigation boundary and that the direct
screen test has its required navigation mock. No blocking issue was found.
There is no reliable affected typecheck; the full frontend suite/typecheck/build
is delegated to GitHub CI. This focused fix does not complete the earlier
full-iPad acceptance matrix or the paused voice recovery work.


## 2026-11-09 — mobile-only compact voice and main-chat transcription

The mobile voice entry now connects once after options and saved settings load.
The normal bottom bar contains actual capture/playback status, elapsed time,
microphone control and End; saved push-to-talk uses Hold to talk. Voice preferences
and pricing are reached from the chat header. The internal welcome/Start panel
and second transcript were removed. Partial speech updates the main chat by the
server message ID, and sealed history takes over without a duplicate. Existing
messages, attachments, tasks, approvals and complete call receipts are preserved.

This increment changes only mobile files. The preexisting dirty backend and
`docs/chat/11-voice.md` changes were preserved and excluded from its commit.
Account-active discovery is optional: 404/405 falls back to existing conversation
APIs; an incomplete bounded scan retains an unknown state. End stops local capture
immediately and closes the surface only after confirmed terminal/no-active state.
Late starts and cleanup retain their verified session/chat ownership across
unmount; account and operation generations reject obsolete results.

Focused verification, from `mobile/`:

```sh
pnpm exec tsx --test src/features/assistant/AssistantScreen.voice.test.ts src/features/assistant/voice/AssistantVoiceConversation.test.ts src/features/assistant/voice/AssistantVoiceClient.test.ts src/features/assistant/voice/useNativeVoice.test.ts src/features/assistant/voice/voiceActiveCall.test.ts src/features/assistant/voice/voiceStartError.test.ts src/features/assistant/voice/voiceTranscript.test.ts src/lib/api/assistantVoiceTransport.test.ts src/features/assistant/voice/voiceSelection.test.ts src/features/assistant/useTranscriptScrollFollow.test.ts src/features/assistant/assistantTranscriptScroll.test.ts
# 309 passed, 0 failed, 0 skipped
```

After preserving unrelated whitespace in the Screen files, their affected
Screen/helper/scroll command was repeated:

```sh
pnpm exec tsx --test src/features/assistant/AssistantScreen.voice.test.ts src/features/assistant/voice/voiceTranscript.test.ts src/features/assistant/useTranscriptScrollFollow.test.ts src/features/assistant/assistantTranscriptScroll.test.ts
# 87 passed, 0 failed, 0 skipped (overlaps the 309 above)
```

From repository root:

```sh
frontend/node_modules/.bin/prettier --check mobile/src/features/assistant/assistantTypes.ts mobile/src/features/assistant/voice/AssistantVoiceClient.ts mobile/src/features/assistant/voice/AssistantVoiceClient.test.ts mobile/src/features/assistant/voice/AssistantVoiceConversation.tsx mobile/src/features/assistant/voice/AssistantVoiceConversation.test.ts mobile/src/features/assistant/voice/useNativeVoice.ts mobile/src/features/assistant/voice/useNativeVoice.test.ts mobile/src/features/assistant/voice/voiceActiveCall.ts mobile/src/features/assistant/voice/voiceActiveCall.test.ts mobile/src/features/assistant/voice/voiceContracts.ts mobile/src/features/assistant/voice/voiceStartError.ts mobile/src/features/assistant/voice/voiceStartError.test.ts mobile/src/features/assistant/voice/voiceTranscript.ts mobile/src/features/assistant/voice/voiceTranscript.test.ts mobile/src/lib/api/assistantVoiceTransport.ts mobile/src/lib/api/assistantVoiceTransport.test.ts
# passed for all 16 files

git diff --check -- mobile
# passed
```

Changed Screen/test ranges were formatted. The whole-file formatter also checked
those two files and warned; both warnings reproduce on unchanged HEAD. Unrelated
formatting is preserved. No reliable repository-native affected typecheck exists;
the full frontend suite/typecheck/build is delegated to GitHub CI.

Independent transcript/scroll and lifecycle reviews found and resolved the real
capture-gate integration omission, paused-history anchoring during voice, and a
retained partial caption continuing to say Transcribing after close. Regression
coverage also includes same-chat close/reopen callbacks, delayed native menu
choices, exact history takeover, bounded socket-caption rollover, saved provider
and PTT preservation, uncertain POST outcomes, pending End and account fencing.

Actual iPad run this increment: final source was served to the complete signed
Debug App (`dev.nyxid.ipadqa`) on iPad Pro 13-inch M5 / iOS 26.3 after clearing
Metro. An IPv6-only localhost binding was corrected using
`NODE_OPTIONS=--dns-result-order=ipv4first`; IPv4 Metro status and actual bundle
delivery were verified.

The first restart accidentally reused old local QA URLs from the ignored
`mobile/.env.local` rather than the real service URLs used by the user's prior
manual session. That incorrect target returned backend HTTP 500 `database_error`
(1007) during session restoration and failed to load sign-in provider options.
The QA backend on 3401 could not reach MongoDB on 33018; startup and repair of a
separate data copy failed. Original database files were not repaired or replaced.
This historical QA failure is not a prerequisite for the intended manual session.

Corrected on 2026-11-09: restored the ignored local runtime configuration to
`https://nyx-api.chrono-ai.fun/api/v1`, with frontend/legal URLs
`https://nyx.chrono-ai.fun`, and restarted Metro 8096 with its cache cleared.
The real service's public config returned all three providers: GitHub, Google
and Apple. After reloading the complete App, the actual Simulator accessibility
tree and screenshot both showed **Continue with Google**, **Continue with GitHub**
and **Continue with Apple**, with the sign-in-options error absent. Auth screen
source was unchanged. At that checkpoint the App was left at the restored sign-in screen. The user
subsequently authenticated; the live verification below supersedes that pending
login status.

**At the login-restoration checkpoint, authenticated voice was not yet
accepted.** The subsequent live run below records the completed paths. The
original acceptance list was: existing/new chat one-tap entry, automatic speech
and same-bubble partial updates, microphone mute/unmute, header preferences,
PTT press/release, End confirmation, reopening and details/Back scroll retention.
No synthetic captions were inserted into the running app to claim acceptance.
The acoustic checks recorded above are historical and do not verify this head.
Physical-device, human-listening, Android, Grok and broader iPad route/layout
coverage gaps recorded earlier also remain open.


## 2026-11-09 — authenticated voice user paths and recovery fixes

The user authenticated against the restored real service. Tests used the complete
signed Debug App on iPad Pro 13-inch M5 / iOS 26.3 in landscape, with native
OpenAI WebRTC and Marin. Source edits were delivered by restarting Metro 8096
with a cleared cache and explicitly reloading the App. The runtime was not
replaced by a mocked screen, fake media or injected captions.

The actual run found and repaired mobile defects that the previous unit coverage
did not catch:

- Normal server idle close could leave a misleading microphone-permission error,
  stale idle warning and incorrect Unable to start state. A confirmed idle terminal
  state now clears the transient media error and hides the old warning.
- End can legitimately return closing before the final receipt. Local teardown
  removes the socket, so recovery now queries the exact known conversation and
  session, checks identity and generation/revision, and publishes its verified
  terminal state while preserving captions/tasks. Repeated failures rearm up to
  six checks without repeating End or creating another call.
- Non-JSON/empty HTTP errors previously discarded statusCode, preventing the
  optional account-active route from falling back on 404/405. They now retain the
  status with a fixed sanitized message; 401/403/503 do not trigger that fallback.
- The preferences viewport contained all 34 voices before the mode, speaker and
  pricing controls. The existing controls were reordered so those common actions
  appear first; speaker/resume controls share a row.

| Actual user path | Result and evidence |
| --- | --- |
| Home/new chat → one voice tap | Created one conversation and connected to Listening without a second Start or second transcript. |
| Existing chat → voice | Connected with the compact bar and retained prior history. |
| Automatic speech → main-chat transcription → reply | A distinct synthetic Chinese phrase traveled through the Mac speaker/microphone into the native call. One main user bubble updated during transcription and sealed; matching assistant reply and Assistant is speaking were observed. Minor acoustic recognition substitutions occurred. |
| Microphone mute → speech → unmute | Off/Unmute states were shown; the distinct muted fixture did not enter chat. After unmute the recovery fixture was transcribed and answered. |
| Header menu → preferences → dismiss | Opened the bounded preference tray while the call stayed connected. The menu did not jump chat to the top in this run. |
| Speaker mute/enable, Resume audio | Speaker button changed Mute → Enable → Mute. Resume audio returned without a call error. Human listening quality was not measured. |
| Voice pricing | Expanded the actual tariff text in the running App. |
| Save PTT → next new chat call | The existing automatic call retained its mode. The next call opened with Hold to talk, without capture until activated. |
| PTT capture/release through accessibility | The exposed native activate action toggled capture on/off. The known PTT phrase was recognized and answered; a distinct post-release phrase did not enter chat. This validates the accessibility path and capture gate, not a sustained finger gesture. |
| Manual End → composer → reopen | After the recovery fix, a real 0:56 receipt appeared and the composer returned automatically, without manual Check. The same chat reconnected afterward. |
| Muted idle timeout → Close | A real 4:46 receipt said The quiet call ended. The bar showed Call ended / Close, with no stale warning or permission error. Close restored the composer. |
| Active PTT call → Details → Back | Navigation stopped the call; Back returned to the same chat with its real 4:03 receipt and composer, without auto-restarting voice. This run was following the latest messages; older paused-reading anchoring retains its earlier evidence, not a new acceptance here. |

Ambient audio/noise was sometimes transcribed while capture was intentionally
open. It is not evidence of duplicate test phrases. Unrelated captured speech is
not reproduced in this report or the PR. No synthetic transcript was inserted.

The foreground/background recovery path and final handoff are recorded in the
follow-up below. Physical iPad/iPhone, sustained touch PTT, human speech/listening,
Bluetooth/interruption, Android, Grok, task execution/approval effects, and the
last rows of the long voice picker are not accepted by these checks. The earlier
broader iPad coverage gaps remain open.


### Final foreground recovery and handoff

The first background run exposed a further recovery gap: a completed cleanup
could clear retained conflict metadata before a final snapshot reached the hook,
and foreground did not rearm a check whose timer fired in the background.
Recovery now retains the owned snapshot's exact identity, resumes only the
remaining six-check budget on foreground, and conveys verified terminal/empty
results to the UI without fabricating a terminal session. A terminal result for
another conversation clears recovery without publishing that conversation's
snapshot into the current chat. Confirmed recovery closes the controls once and
leaves retained unfinished captions marked as partial by the existing chat path.

An intervening attempt still encountered an older call conflict; that attempt
was not counted as a successful background test. A later fresh PTT call connected
and was actually sent to the system Home screen and reopened. A temporary local
DEV diagnostic observed only sanitized fields: state=closed, end_requested=true,
identity/revision validation=true, same-chat=true. The App returned automatically
to its composer and displayed the real 0:39 receipt, without manual Check, End
again or an automatic replacement call. Diagnostic statements were then removed,
Metro was restarted/cleared, and the same system Home → reopen path was repeated
on the exact final source: real 1:33 receipt, composer restored, no lingering
Confirming controls. Marin and Hands-free were selected again in preferences. The next same-chat call
connected to Listening, confirming that the original mode was restored. A final
fresh Chinese acoustic fixture entered the main chat once and received an
assistant clarification reply (a recognition substitution changed the instruction,
so a literal expected reply is not asserted). Final End produced a real 1:45
receipt and automatically restored the composer. The logged-in App was left
there with local capture/playback stopped and no Confirming controls.

Final dependency-related verification, from `mobile/`:

```sh
pnpm exec tsx --test src/features/assistant/voice/AssistantVoiceClient.test.ts src/features/assistant/voice/useNativeVoice.test.ts src/features/assistant/voice/AssistantVoiceConversation.test.ts src/lib/api/http.session.test.ts src/lib/api/assistantVoiceTransport.test.ts src/lib/api/guardedTransport.test.ts src/lib/api/authenticatedTransport.test.ts
# 256 passed, 0 failed, 0 skipped
```

From repository root:

```sh
frontend/node_modules/.bin/prettier --check mobile/src/features/assistant/voice/AssistantVoiceClient.ts mobile/src/features/assistant/voice/AssistantVoiceClient.test.ts mobile/src/features/assistant/voice/AssistantVoiceConversation.tsx mobile/src/features/assistant/voice/AssistantVoiceConversation.test.ts mobile/src/features/assistant/voice/useNativeVoice.ts mobile/src/features/assistant/voice/useNativeVoice.test.ts mobile/src/lib/api/http.session.test.ts
# all 7 files passed

git diff --check -- mobile
# passed
```

The whole-file Prettier warning for `mobile/src/lib/api/http.ts` also reproduces
on unmodified HEAD; only the new block was formatted. No repository-native
reliable affected typecheck exists. Full frontend suite/typecheck/build remains
delegated to GitHub CI. The 256-test run overlaps earlier runs and is not added
to the historical 309 count. Independent read-only lifecycle, transcript and
recovery reviews found no further actionable mobile blocker in this increment.
The six preexisting backend/chat-document diffs retained the same binary SHA256
and were excluded from delivery.


## 2026-11-09 — compact voice selection

The voice preference tray no longer lays out every provider voice as a chip.
It shows one Voice row with the saved next-call label. The row opens the existing
native searchable single-selection sheet, with Cancel/Apply and genuine server
labels. Search and draft selection do not persist anything. Confirmed selection
uses the existing preference API; failure retains the previous value and shows
an actionable error. A changed next-call voice does not alter the live call,
and the current voice is shown separately only while those choices differ.

The picker is scoped to account epoch, chat and exact voice source. Dismissed,
unfocused and obsolete picker callbacks cannot save. Opening it releases a held
accessibility PTT capture without ending the call. No backend/web code or shared
picker implementation was changed.

Focused dependency-related verification, from `mobile/`:

```sh
pnpm exec tsx --test src/features/assistant/voice/AssistantVoiceConversation.test.ts src/components/workspace/WorkspaceSelectionField.test.ts src/components/settings/SettingsList.test.ts
# 61 passed, 0 failed, 0 skipped
```

Changed-file static checks, from repository root:

```sh
frontend/node_modules/.bin/prettier --check mobile/src/features/assistant/voice/AssistantVoiceConversation.tsx mobile/src/features/assistant/voice/AssistantVoiceConversation.test.ts
git diff --check -- mobile
# passed
```

No reliable affected typecheck exists; full frontend suite/typecheck/build is
delegated to GitHub CI. Independent read-only review found no blocking issue.
The six preexisting backend/chat-document diffs remain unchanged and excluded.

Actual signed-App iPad landscape checks after cleared Metro reload:

- The default tray shows Voice / Marin, not the 34 voice chips.
- Opened the separate Choose voice pageSheet; searched Cedar, selected it, then
  cancelled. The tray still showed Marin. That first muted call subsequently
  reached its normal three-minute idle timeout; it was not counted as continuous
  call evidence after timeout.
- On the final source, searched Cedar and applied it during a connected muted
  call. The tray showed Voice / Cedar plus Current call: Marin at elapsed 2:19,
  with End and Unmute still present. The sheet did not navigate or end the call.
- Reopened the picker and applied Marin again. The row returned to Marin; the
  next call still used Hands-free. Save failures and held-PTT release are covered
  by focused regression tests, not a live fault-injection/touch claim.

- A subsequent same-chat call reopened, opened/cancelled the picker, and closed
  preferences. Unmute followed by a fresh acoustic Chinese fixture produced its
  recognized text in the main chat (minor recognition substitution). This proves
  capture still worked after the picker flow; no literal assistant reply is
  asserted because an idle-warning message overlapped this window. The prior
  slower check had reached idle timeout and is not counted as speech evidence.


## 2026-11-09 — voice task activity and failed outcomes

Voice task snapshots already include an additive `outcome` that distinguishes
failed execution from an explicit Stop. Mobile now retains known outcomes,
accepts old servers with no outcome, and ignores future unknown values without
breaking the live snapshot or implying success. Failed work renders Failed,
explicit Stop renders Stopped, and a legacy unknown terminal result renders
Ended. The task chip counts only queued/claimed work as in progress; a failed
terminal task shows a failure summary instead of an ambiguous task count.

Expanded activity shows numbered states and the existing Stop/approval-review
controls. Generated task request text and server progress explanations are
behind an explicit per-task Details action, labelled Request from voice
assistant. Details remain keyed by task ID through snapshot updates. The
possible-earlier-effects warning is preserved there. Only pending work shows
continuation guidance; an approval-only list asks for review rather than
suggesting an unavailable Stop control. Opening activity/details releases held
PTT without ending the media session. This increment changes mobile files only.

Focused checks from `mobile/`:

```sh
pnpm exec tsx --test src/features/assistant/voice/AssistantVoiceConversation.test.ts src/features/assistant/voice/voiceContracts.test.ts src/features/assistant/voice/AssistantVoiceClient.test.ts src/lib/api/assistantVoiceTransport.test.ts
# 165 passed, 0 failed, 0 skipped
```

Changed-file checks from repository root:

```sh
frontend/node_modules/.bin/prettier --check mobile/src/features/assistant/voice/AssistantVoiceConversation.tsx mobile/src/features/assistant/voice/AssistantVoiceConversation.test.ts mobile/src/features/assistant/voice/voiceContracts.ts mobile/src/features/assistant/voice/voiceContracts.test.ts
git diff --check -- mobile
# passed
```

Actual authenticated signed-App iPad landscape check after a cleared Metro
restart and explicit App reload used a real acoustic request to delegate harmless
arithmetic, expressly without external tools or modifications. The request was
transcribed, accepted and settled as a real failed task while the muted voice
call remained open. Expanded activity showed Task 1 · Failed and Details;
only Details revealed the labelled request and original failure warning. Hiding
it removed that text. The failed-only list had no pending-work footer. Native
media, captions or task results were not mocked or injected.

**Runtime failures at this checkpoint (End is superseded by the next section).** The real arithmetic task and the
user-provided error both report `nyxid / model_proxy / approval_unsupported /
HTTP 403 / NyxID 12502`. Current upstream main deliberately refuses a new
approval request from voice execution. That refused operation was not executed
and created no approval card; the error alone does not identify the exact
service/rule or prove all earlier task actions had no effects. No approval or
backend policy was changed by this mobile increment. Successful delegated or
external task execution is not accepted by this test.

The same actual run also exposed an unresolved End confirmation: End was
requested at 2:50, native WebRTC closed and a 2:55 call receipt appeared, but
the UI remained Confirming call ended even after Check call status and another
explicit End request. This End-to-composer path is **not passed**. A standard
development reload returned to the logged-in App; it is not counted as a fix
or successful automatic settlement. Exact server status/root cause has not been
established; a receipt alone must not authorize a new call. This later observation
qualifies the earlier passing End/recovery runs above.

Independent review found no blocking issue in the task presentation diff.
The six existing backend/chat-document changes are preserved byte-for-byte and
excluded. Full frontend suite/typecheck/build remains delegated to GitHub CI;
no reliable affected typecheck exists, and CI is not monitored.


## 2026-11-09 — bounded End, status recovery and return to text

This mobile-only increment addresses the later End/conflict/loading failures
recorded above. A receipt is not terminal proof: the backend can retain a closing
session while final usage settles. Temporary sanitized response metadata from
one actual run confirmed a schema-valid closing End response followed by a
closed, final-usage-confirmed status. The temporary diagnostics were removed
before final validation; no credentials, SDP or transcript was logged by them.

The native End sequence now immediately disables capture, PTT and playback,
while retaining quiet native WebRTC and the NyxID control socket until a verified
terminal response/snapshot or a seven-second absolute deadline. End uses one
independent abortable request and a race, so a non-cooperative transport cannot
strand cleanup. Generation/revision/resource checks reject stale terminal data.
The actual Bridge unmount also respects client ownership of this quiet drain;
ordinary unmounts and WebView media still close immediately. Drain gates prevent
any input/playback re-enable, and final close resets them for the next generation.

After bounded cleanup, the voice surface returns to the text composer even when
remote settlement is not confirmed. Exact recovery identity remains retained.
Closing/unknown/checking surfaces always offer Back to chat; this only exits the
surface and cannot end another chat's call. End again has been removed. Verified
closing discovery replaces the stale Another voice call diagnostic. New voice
creation stays blocked until the previous call is verified terminal or account
discovery is verified empty. A receipt or leaving the surface does not release it.

Exact status, account discovery, explicit conflict End and its fallback discovery
share an eight-second total request budget. Initial 409/lost-start discovery has
the same bound. Timeout stops loading even if the adapter ignores abort; late
results cannot overwrite recovery. Automatic checks remain limited to six. A new
explicit Start waiting on old cleanup rechecks scope and its original ticket;
a later End/background event cancels that intent. Offline/blur recovery does not
implicitly End another chat's active call.

Focused final verification from mobile/:

```sh
pnpm exec tsx --test src/features/assistant/voice/AssistantVoiceClient.test.ts src/features/assistant/voice/AssistantVoiceConversation.test.ts src/features/assistant/voice/useNativeVoice.test.ts src/features/assistant/voice/VoiceMediaBridge.test.ts src/features/assistant/voice/voiceRequestDeadline.test.ts src/features/assistant/voice/NativeOpenAIMedia.test.ts src/features/assistant/AssistantScreen.voice.test.ts src/lib/api/assistantVoiceTransport.test.ts
# 346 passed, 0 failed, 0 skipped
```

Changed-file checks from repository root:

```sh
frontend/node_modules/.bin/prettier --check mobile/src/features/assistant/voice/AssistantVoiceClient.ts mobile/src/features/assistant/voice/AssistantVoiceClient.test.ts mobile/src/features/assistant/voice/AssistantVoiceConversation.tsx mobile/src/features/assistant/voice/AssistantVoiceConversation.test.ts mobile/src/features/assistant/voice/useNativeVoice.ts mobile/src/features/assistant/voice/useNativeVoice.test.ts mobile/src/features/assistant/voice/VoiceMediaBridge.tsx mobile/src/features/assistant/voice/VoiceMediaBridge.test.ts mobile/src/features/assistant/voice/voiceRequestDeadline.ts mobile/src/features/assistant/voice/voiceRequestDeadline.test.ts mobile/src/lib/api/assistantVoiceTransport.test.ts
git diff --check -- mobile
# passed; 11 explicitly selected source/test files
```

Regression coverage includes the real Client + NativeVoiceMedia + mounted Bridge
cleanup for terminal/deadline, hung requests and late outcomes, duplicate End,
wrong-resource/old-version snapshots, same-chat and account fences, cancellation
of waiting Start, and returning to text without ending unrelated calls. An
independent final read-only review confirmed its three lifecycle findings were
fixed. No reliable affected typecheck exists; full suite/typecheck/build remains
owned by GitHub CI. CI was not monitored.

Final actual signed-App checks used iPad Pro 13 M5 / iOS 26.3 landscape,
real service and native OpenAI WebRTC, Marin/Hands-free. After source freeze,
Metro was restarted with cache clearing and the complete App reloaded once;
there was no reload between the following calls:

| User path | Actual observation |
| --- | --- |
| Home → voice → acoustic arithmetic phrase | Entered Listening. The fixture was transcribed in main chat, with a minor recognition substitution; the assistant replied that two plus two equals four. |
| First call → End | Temporary Ending state, then automatic text composer and a 0:52 receipt. No manual Check, repeated End or reload was needed. |
| Immediately reopen in the same chat | Connected to Listening. No retained-call conflict blocked this new call. |
| Second call → harmless background arithmetic task | A real task was accepted then failed. Task activity showed Task 1 · Failed. The acoustic channel picked up unrelated room speech, so word-perfect recognition is not accepted for this second fixture. |
| Failed-task call → mute → End | Returned automatically to the text composer with a 2:49 receipt, including the accurate failed task result. The previous indefinite End confirmation was not reproduced. |
| Reopen after failed-task call | A third call entered Listening; this establishes that backend admission accepted another call after the failure/End path. |
| Final End | Automatic composer with a 0:37 receipt. Final screenshot shows text entry and normal voice entry; the native peer is closed and no local capture is left running. |

An earlier pre-freeze development hot refresh interrupted a second call. That
run is excluded from final acceptance. The final tests above did not mock media,
transcripts or task outcomes. Network hangs and component unmount timing were
verified by regression tests, not by a live production fault injection. A final
attempt to exercise text typing did not visibly enter text, so only composer
restoration/visibility, not an additional text-send round trip, is accepted here.

The separate prior approval_unsupported / HTTP 403 / NyxID12502 refusal is not
fixed by this mobile increment. The final background task also failed; its exact
provider error was not re-opened in Details in that final run. Successful
background/external task execution is therefore still unaccepted. Physical
human listening, sustained touch, Bluetooth interruptions, other providers and
the broader route/layout gaps recorded above remain outside these checks.

All source changes in this increment are mobile-only. The six preexisting
backend/chat-document diffs remain unchanged and are excluded from the commit.

## 2026-11-10 — additive catalog and stable voice picker/reopen state

The backend keeps the original 32 OpenAI voice IDs, labels and order and appends
`shida`. The exact-legacy startup update uses an append, preserving admin/custom
choices and sibling metadata. Saved voice IDs continue to pass NyxID selection;
provider acceptance of a historical voice is a separate question. No NyxAgent
model, execution policy, provider adapter, pricing or server call-slot rule changes.

The chat voice picker uses an opaque `overFullScreen` modal and keeps its opening
choices and selected voice across same-source query refreshes. A transient
refetch error, offline state or pending save disables Apply while preserving
the modal, draft and controls. Account, chat and source changes still invalidate
callbacks, and Apply rechecks the current available choices. Other selection
sheets retain their existing presentation and disabled-dismiss behavior.

Backgrounding still silences and ends local voice but does not manufacture an
error notification. On re-entry, active or unknown retained call metadata now
receives the existing bounded foreground status checks, including exact terminal
readback. The app clears blockers only on verified terminal/empty state; checking
does not create, reconnect or end a provider call. A real active call in another
chat remains protected by the existing one-live-call-per-person server fence.
Automatic status checks preserve genuine provider and billing failures, while
refreshing status diagnostics and stale 409 hints. An actionable billing failure
is not auto-dismissed just because the old call was confirmed finished.

Focused regression checks reproduced the pre-fix failures for changing picker
content/presentation, retained active/unknown state not being refreshed, transient
disable/refetch dismissals, hidden provider diagnostics and overwritten/dismissed
billing recovery. Automated verification is recorded in the implementation
report: the final mobile suite passed 2,193 tests, with no failures, skips or
cancellations; package typecheck, formatting of the eight changed TypeScript
files, Rust formatting and diff whitespace checks passed. The independent
Opus 5.5 rereview passed with 322 focused tests. The Rust compatibility tests
have been updated but were not compiled or run after these edits; fresh CI is
still required.

Voice re-entry now waits for the same chat's acknowledged closing call and
shows “Disconnecting previous call…” then “Starting new call…” for exactly one
new attempt after terminal/empty-slot verification. A separate explicit “End
previous call and start here” action may replace a confirmed active call.
The queued intent binds the old session and the current account/chat/source,
voice and mode, expires after 30 seconds, and owns read-only checks every two
seconds within that deadline. Scope/focus/network loss, backgrounding, another
discovered call and real errors cancel it. A terminal snapshot with a reason
other than `user_ended` leaves an explanatory notice and requires an explicit
new start. A start refused by the hook's final guards also leaves a retry
notice. Merely foregrounding the app never starts the microphone.

Actual simulator checks on the frozen source used the full Debug App with its
existing native WebRTC framework, loaded from this worktree's Metro server, on
iPad Pro 13-inch M5 / iOS 26.2 in portrait. A real Marin/Hands-free call reached
Listening. The opaque picker stayed full-screen while searching Cedar and
changing the draft; its title and Cancel coordinates stayed identical. Cancel
kept the saved Marin preference, and the call stayed live underneath. Home and
relaunch returned to text with a real ended-call receipt and no manufactured
inactive-app error. An explicit new call then reached Listening with a reset
clock, and explicit End returned to the text composer. All test calls were
ended. Native screenshots and a recording are retained locally, outside git.

The provider closed quickly in this run, so a prolonged closing state and the
delayed restart transition were not reproduced natively; regression tests
cover those states, deadlines and cancellation paths. Search used the
simulator's hardware keyboard; software keyboard geometry, landscape, physical
devices, Android, acoustic input/output quality and cross-device replacement
remain unverified for this increment. No production catalog or provider
configuration was edited. The simulator run did create ordinary voice calls
and their receipts through the existing API.
