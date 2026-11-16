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

| Path                                      | Required observations                                                                        | Result                                                                                                                                                                                                                                                                                                                                           |
| ----------------------------------------- | -------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| First microphone denial                   | Denied message plus Open Settings; no unusable retry                                         | Passed after resetting only this app's microphone permission: dismissed the real system prompt immediately; Unable to start, permission explanation, Open Settings, no Try again. The earlier delayed-prompt timeout was a separate run                                                                                                          |
| Reopen with permanently denied permission | Open Settings remains available; no second system prompt                                     | Passed again on the merged-main branch: Return to chat → Open voice call restored the denied state without another prompt                                                                                                                                                                                                                        |
| Settings and return                       | Settings opens; permission enabled; stale error clears; explicit retry starts voice          | Passed in the final retest: Open Settings launched Apps; selected NyxID Mobile and enabled microphone. OS restarted the app with login preserved and no automatic call. Explicit start reached Listening and real speech produced welcome back. Same-page refresh was not observable; earlier intermittent native failures remain recorded below |
| Active hands-free background cycle        | Real input/reply, Home/background, return; same timer/transcript; input and reply still work | Passed after cold boot: fixed host speech was captured by the real microphone/provider and produced Hello!; Home for 5 seconds → return kept the first transcript and running timer (00:22 before, 00:46 after); second fixed speech produced Welcome back. without a new call or receipt                                                        |
| End and immediate next call               | Explicit end settles; next call starts without stale Disconnecting                           | Passed: End → ending/receipt → immediately reopen as soon as enabled → Listening with a new timer. Another spoken request was transcribed and the provider counted to ten; native UI reported Assistant is speaking during playback. Both test calls were explicitly ended                                                                       |
| Unsupported background media              | Specific reason; local microphone closes; server settles before another call                 | Focused tests only                                                                                                                                                                                                                                                                                                                               |
| Cannot open Settings                      | Correct manual path; native details not leaked                                               | Focused tests only; real OS failure cannot be forced safely                                                                                                                                                                                                                                                                                      |

### Native startup history and controlled retest

On 2026-11-16 at 21:44 local time, a real call attempt terminated with `SIGABRT`: `CoreAudio SetProperty: RPC timeout. Apparently deadlocked. Aborting now.` The stack enters `AUVoiceIO` / `AURemoteIO::Initialize` from WebRTC's `VoiceProcessingAudioUnit::Initialize` and `InitPlayout`. An independent read-only audit found the same crash chain on this simulator on November 12 with the same WebRTC framework binary, before these fixes. The app's native WebRTC audio configuration matches team main; this does not establish a new regression or prove an environment-only cause.

A shutdown/boot preserving all app data restored real audio. Two consecutive calls, microphone transcription, provider replies, native speaking state, background/foreground continuity, and immediate next-call startup then passed, as recorded above. Test speech came from the host speaker into the configured real microphone; no media/provider boundary was mocked.

After ending both calls, the normal permission-denial/Settings-grant path was repeated. iOS restarted the app after the microphone switch was enabled, and the next explicit call reproduced the same native RPC crash at 22:10. This was a real failed native run and remains part of the stability evidence.

The final controlled comparison used the same signed binary, owned simulator, backend, default system sounds and host audio settings, serving main `17432a2ff` on Metro8097 and this PR on Metro8095. Each case began after a cold boot without a preceding audio call. No audio route, voice processing, dependency, product timing or system sound setting was changed.

- **Main:** actual first denial at 22:38:14 → manually open system Settings → NyxID Mobile microphone 0→1 → OS restart → explicit call at 22:39:17 → Listening at 22:39:25 → real microphone transcription and Hello! reply. The call was explicitly ended. An earlier automation attempt did not actually dismiss the prompt; that delayed sample was discarded, and only this app's microphone permission was reset before the valid denial.
- **PR:** first tap landed during the permission alert animation and did not dismiss it. The actual denial at 22:42:01 was within the 45-second permission deadline, not immediate. Unable to start then showed Open Settings and no unusable Retry. Open Settings launched Apps; selecting NyxID Mobile exposed microphone 0→1. The OS restarted the app, preserving authentication without an automatic call. Explicit start at 22:43:49 → Listening at 22:43:57 → real microphone transcription and welcome back reply.
- **Recovered PR call:** Home/background for 5 seconds → return preserved transcript and the same advancing timer (01:04→01:11, including automation overhead). Another real speech turn was transcribed and answered hello again.
- **End and next call:** explicit End at 22:45:42 → Return to chat → next explicit start at 22:45:46 → Listening at 22:45:50 with a new timer. Both PR calls were explicitly ended; no new crash report appeared during this retest.

The previously unpassed permission-to-real-call path now has a successful native sample on both main and the PR. This permits the narrow permission/background fix to enter **ready for review**. It does not prove the earlier CoreAudio fatal is fixed, simulator-only, or unrelated to the PR; the intermittent native startup risk remains unassigned. Same-page permission refresh is covered by focused tests because this OS restarts the app on a permission change. System Settings initially opening Apps rather than the exact app page is recorded as observed. Physical-device suspension/audio routing and Android native behavior remain unverified by this iPad run.

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

Independent review: root agent inspected the complete hook/UI diff, scope and ticket fences, and native-only ready-call background guard; no confirmed blocking source findings. Quality gate: **PASS WITH NOTES** for this narrow fix, with the intermittent native startup history and untested device paths documented above. Successful final retests do not establish universal voice stability or merge readiness.
