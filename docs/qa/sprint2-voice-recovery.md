# Sprint 2 mobile voice recovery — #2411 / #2412

Initial baseline: `4372b638898a272ac8942d90ce099d2fd0ce3dec` (`upstream/main`).
Reconciled with team main `17432a2ff5b7c736f9843a75f4e244a80c0548b1` in merge `bec0c422614e8dceca027d85aeec8dba3a5131a7`.
Branch: `codex/sprint2-voice-recovery`.

## Scope and behavior

- #2411: the shared voice hook retains denied microphone permission as structured state. The shared iPhone/iPad/Android composer offers **Open Settings**, hides a meaningless retry after `canAskAgain=false`, and checks permission after returning to the foreground. Enabling permission clears the stale error; **Try again** starts a new call only after an explicit user action. A failed Settings launch gives an OS-specific manual path. New runtime copy is registered in both Chinese catalogs.
- #2412: an already connected iOS native WebRTC call keeps its client, session, transcript, and original connected timestamp across inactive/background/active transitions. The committed native plist and Expo config both declare `audio` background mode. Backgrounding releases a pressed PTT gesture. Android/WebView media and pending starts retain the existing bounded stop/server cleanup, with an explicit explanation of why the call stopped. Closing calls retain their existing recovery state.
- UI layout, iPad compact controls, backend contracts, and media engines are unchanged. This does not add unsupported Android foreground-service or iOS WebView background capture.

## Local focused verification

From `mobile/`, Node 22:

```sh
/Users/abigaildeng/.npm/_npx/52027bd8fc0022aa/node_modules/node/bin/node --import tsx --test \
  src/features/assistant/voice/useNativeVoice.test.ts \
  src/features/assistant/voice/useNativeVoice.readiness.test.ts \
  src/features/assistant/voice/AssistantVoiceConversation.test.ts \
  src/features/assistant/voice/iosVoiceAudio.test.ts \
  src/features/assistant/voice/AssistantVoiceClient.test.ts \
  src/features/assistant/voice/VoiceMediaBridge.test.ts \
  src/features/assistant/voice/NativeOpenAIMedia.test.ts \
  src/features/assistant/AssistantScreen.voice.test.ts \
  src/i18n/mobileRuntimeMessages.test.ts \
  src/lib/loadEnv.test.ts
```

After the main merge: **430 passed, 0 failed, 0 skipped**. These are focused tests with native/network boundaries replaced; they are not device E2E.

Changed files pass explicit-file Prettier checks, TS/TSX transpile syntax checks, XML plist parsing, and `git diff --check`. No repository-native affected TypeScript target exists; full typecheck, frontend suite, lint, and production build belong to GitHub CI.

## Device E2E matrix

Native validation uses the existing logged-in iPad Pro 13-inch (M5), iOS 26.3 simulator, bundle `dev.nyxid.ipadqa`, and the branch's Metro bundle on port 8095. The signed Debug app includes background audio. The account/container was preserved. These observations do not certify physical iPhone audio routing or OS suspension.

| Path                                      | Required observations                                                                        | Result                                                                                                                                                                                                                                                                                                       |
| ----------------------------------------- | -------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| First microphone denial                   | Denied message plus Open Settings; no unusable retry                                         | Passed after resetting only this app's microphone permission: dismissed the real system prompt immediately; Unable to start, permission explanation, Open Settings, no Try again. The earlier delayed-prompt timeout was a separate run                                                                      |
| Reopen with permanently denied permission | Open Settings remains available; no second system prompt                                     | Passed again on the merged-main branch: Return to chat → Open voice call restored the denied state without another prompt                                                                                                                                                                                    |
| Settings and return                       | Settings opens; permission enabled; stale error clears; explicit retry starts voice          | System Settings launched. On first launch after reinstall it showed Apps; choosing NyxID Mobile exposed its microphone switch. Enabling it restarted the app, preserving login without an automatic call. Same-page refresh was not observable. Explicit call then hit the CoreAudio failure described below |
| Active hands-free background cycle        | Real input/reply, Home/background, return; same timer/transcript; input and reply still work | Passed after cold boot: fixed host speech was captured by the real microphone/provider and produced Hello!; Home for 5 seconds → return kept the first transcript and running timer (00:22 before, 00:46 after); second fixed speech produced Welcome back. without a new call or receipt                    |
| End and immediate next call               | Explicit end settles; next call starts without stale Disconnecting                           | Passed: End → ending/receipt → immediately reopen as soon as enabled → Listening with a new timer. Another spoken request was transcribed and the provider counted to ten; native UI reported Assistant is speaking during playback. Both test calls were explicitly ended                                   |
| Unsupported background media              | Specific reason; local microphone closes; server settles before another call                 | Focused tests only                                                                                                                                                                                                                                                                                           |
| Cannot open Settings                      | Correct manual path; native details not leaked                                               | Focused tests only; real OS failure cannot be forced safely                                                                                                                                                                                                                                                  |

### Native startup failure under investigation

On 2026-11-16 at 21:44 local time, a real call attempt terminated with `SIGABRT`: `CoreAudio SetProperty: RPC timeout. Apparently deadlocked. Aborting now.` The stack enters `AUVoiceIO` / `AURemoteIO::Initialize` from WebRTC's `VoiceProcessingAudioUnit::Initialize` and `InitPlayout`. An independent read-only audit found the same crash chain on this simulator on November 12 with the same WebRTC framework binary, before these fixes. The app's native WebRTC audio configuration matches team main; this does not establish a new regression or prove an environment-only cause.

A shutdown/boot preserving all app data restored real audio. Two consecutive calls, microphone transcription, provider replies, native speaking state, background/foreground continuity, and immediate next-call startup then passed, as recorded above. Test speech came from the host speaker into the configured real microphone; no media/provider boundary was mocked.

After ending both calls, the normal permission-denial/Settings-grant path was repeated. iOS restarted the app after the microphone switch was enabled, and the next explicit call reproduced the same native RPC crash at 22:10. Thus cold-start speech/background passed, but permission recovery into a real call still has a reproducible simulator blocker. A main-branch comparison is pending. No voice-processing bypass, dependency replacement, or simulator-only fallback was added. PR #2537 remains draft until this boundary is resolved or attributed with a controlled baseline comparison.

## Native build preparation

A simulator **Debug** build (not a production/frontend build) completed successfully:

```sh
APP_ENV=dev \
DEV_API_BASE_URL=https://nyx-api.chrono-ai.fun/api/v1 \
DEV_NEW_API_BASE_URL=https://api.nyxid.ai/api/v1 \
DEV_FRONTEND_URL=https://nyx.chrono-ai.fun \
DEV_NEW_FRONTEND_URL=https://nyxid.ai \
DEV_IOS_BUNDLE_ID=dev.nyxid.ipadqa \
DEV_ANDROID_PACKAGE=dev.nyxid.ipadqa \
DEV_IOS_BUILD_NUMBER=45 APP_SCHEME=nyxid \
NODE_BINARY=/Users/abigaildeng/.npm/_npx/52027bd8fc0022aa/node_modules/node/bin/node \
PATH=/Users/abigaildeng/.npm/_npx/52027bd8fc0022aa/node_modules/node/bin:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin \
xcodebuild -workspace ios/NyxIDMobile.xcworkspace -scheme NyxIDMobile \
  -configuration Debug -sdk iphonesimulator \
  -destination id=BBBA8E20-A536-4B64-8882-09BF53584CED \
  -derivedDataPath build/sprint2-voice-derivedData -jobs 4 \
  PRODUCT_BUNDLE_IDENTIFIER=dev.nyxid.ipadqa DEVELOPMENT_TEAM=56DKXVFJS2 \
  'CODE_SIGN_IDENTITY=-' build -quiet
```

The merged-main rebuild initially failed because the restored tracked Podfile.lock did not match the local CocoaPods manifest's worktree paths. Running `pod install` in `mobile/ios` reconciled the local build environment; the same Debug command then passed. Generated CocoaPods path/checksum/project differences were restored and are excluded from the source change.

The rebuilt app is `1.0.4 (45)`, contains `UIBackgroundModes=[audio, fetch, remote-notification]` and main's `NSPhotoLibraryAddUsageDescription`, supports iPhone and iPad, and passes `codesign --verify --deep --strict`. Its simulator ad-hoc signature matches the already logged-in QA app's signing style. No simulator was uninstalled or erased.

Independent review: root agent inspected the complete hook/UI diff, scope and ticket fences, and native-only ready-call background guard; no confirmed blocking findings. The permission-to-call simulator failure remains documented above; passing focused tests do not override it.
