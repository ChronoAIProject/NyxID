# iPad adaptation validation report

**Latest voice status — 2026-11-06:** Simulator can use the Mac microphone;
the previous physical-device-only conclusion was too broad. The known WebKit
test source remains blocked before provider session creation. A Debug Simulator
real-capture experiment reaches system capture permission but blocks in WebKit
sandbox-extension issuance. Independent native dictation also aborts during
AudioUnit initialization, including after restarting this iPad. Actual input,
recognition and a reply have not passed. The experiment is opt-in; see the final
follow-up for reproduction and the current environment blocker.

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
lists31 built-in voices, omits `aube` and spells `shida` rather than the historical
seed's `shitan`. The corrected default list follows that contract. Startup
backfill repairs only an exact historical voice array on a non-admin-modified
OpenAI catalog row; the atomic predicate and voices-only update preserve admin
edits, custom arrays, models, delegation, pricing and other metadata. This code
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


## Simulator real-capture investigation — 2026-11-06

Apple explicitly supports choosing the Mac microphone in the Simulator's
[Sound Input settings](https://developer.apple.com/documentation/xcode/configuring-the-environment-of-a-simulated-device#Set-the-audio-input-and-output).
A default mock source does not prove that real capture is impossible. The red
message was application-authored, and the blanket instruction to use a physical
device was incorrect. The fixed, sanitized message now reads:

> A test microphone is active. Select a real microphone and try again.

A development-only hook can request real capture before RN WebView creates the
voice WKWebView. It calls the original configuration method, checks the exact
voice HTML marker, and disables WebKit mock capture through a typed Objective-C
IMP. Missing runtime capabilities preserve the original configuration and mock
guard. It does not change OS permissions or disable the sandbox.

This path requires **both a Debug Simulator build and the explicit process
environment `NYXID_SIMULATOR_REAL_MICROPHONE=1`**. Default launches keep the guard
rather than enabling an experiment that currently hangs. All imports, helper,
call sites and private selectors are excluded at compile time from physical
device and Release configurations. Although only voice configuration is changed,
WebKit's capture factory has process-wide state; this is a diagnostic option,
not a claim of capture isolation or production voice acceptance.

Actual full-App evidence on iPad Pro13 M5 / iOS26.3:

- Signed native Debug builds succeeded with `SKIP_BUNDLING=1`; the same app
  bundle and login were retained. Metro loaded the current complete app.
- With real capture requested, Start reached real system capture validation but
  the app main thread blocked in `GPUProcessProxy::updateCaptureAccess` and
  `_sandbox_extension_issue`. No acquired real track or provider POST was
  established.
- A separate experiment disabled both GPU capture feature flags. The stack
  moved to a different grant-request path but still blocked while issuing a
  generic sandbox extension. Those ineffective routing changes were removed.
- The existing native Composer dictation, which uses AVAudioEngine rather than
  WKWebView capture, independently aborted with `AURemoteIO::Initialize` /
  `AudioUnitInitialize` RPC timeout while constructing its input node. The
  speech-recognition permission was accepted through the normal system prompt;
  the app's microphone permission was already allowed.
- Restarting only this iPad Simulator, preserving data, and rebinding input and
  output to the Mac's built-in devices did not establish a working audio input.
  A second native dictation attempt repeated the failure. These are separate
  observed failures; a shared underlying OS cause has not been proven.
- No speech recognition, actual dialogue, interruption, PTT or audible reply is
  accepted from these attempts. There is no active provider call. Do not hide
  the guard, send WebKit test tones or treat packet growth as spoken success.

To repeat after the Simulator's real audio capture works, install the current
Debug app, select the Mac microphone/output through the Simulator menu and use
an explicit opt-in launch (substitute the target simulator and Debug bundle):

```sh
SIMCTL_CHILD_NYXID_SIMULATOR_REAL_MICROPHONE=1 xcrun simctl launch --terminate-running-process <simulator-udid> <debug-bundle-id>
xcrun simctl openurl <simulator-udid> 'nyxid://expo-development-client/?url=http%3A%2F%2F127.0.0.1%3A8096'
```

First prove local capture, then a short real call with a recognized utterance
and a corresponding reply. The normal launch omits the opt-in variable. Host
system audio services and other simulators were not restarted or reconfigured.

Latest focused verification, from `mobile/`:

```sh
pnpm exec tsx --test src/features/assistant/voice/AssistantVoiceClient.test.ts src/features/assistant/voice/voiceMediaDocument.test.ts
# 99 passed, 0 failed
../frontend/node_modules/.bin/prettier --check src/features/assistant/voice/AssistantVoiceClient.ts src/features/assistant/voice/AssistantVoiceClient.test.ts src/features/assistant/voice/VoiceMediaBridge.tsx src/features/assistant/voice/voiceMediaDocument.ts src/features/assistant/voice/voiceMediaDocument.test.ts
# All matched files use Prettier code style
```

Changed native source is checked through the signed Simulator Debug build and
focused Swift helper compilation in Release Simulator / Debug device / Debug
Simulator configurations, with strings and symbols checked against a positive
Debug Simulator control. These are targeted exclusion checks, not a full
Release App build. Full frontend suite/typecheck/build remains with GitHub CI;
no current-head CI result is claimed. Overall iPad coverage gaps above remain.

Final normal-launch check on the signed default Debug app showed the corrected
message in the actual voice panel. Safe observer output confirmed
`mock_capture=1`, `provider_start_calls=0`, and `client_closed=true`; the app
remained responsive. This check observed the normal API creation boundary.
Real-capture experiments above instead used a temporary local-only preflight
wrapper that intercepted provider creation; they cannot establish a successful
provider session or normal end-to-end call. Diagnostic observers were removed
by a cold restart after recording the final default-path evidence.
