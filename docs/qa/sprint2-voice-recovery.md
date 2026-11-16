# Sprint 2 mobile voice recovery — #2411 / #2412

Baseline: `4372b638898a272ac8942d90ce099d2fd0ce3dec` (`upstream/main`).
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

Result: **429 passed, 0 failed, 0 skipped**. These are focused tests with native/network boundaries replaced; they are not device E2E.

Changed files pass explicit-file Prettier checks, TS/TSX transpile syntax checks, XML plist parsing, and `git diff --check`. No repository-native affected TypeScript target exists; full typecheck, frontend suite, lint, and production build belong to GitHub CI.

## Device E2E matrix

Device validation is pending; no E2E pass is claimed by the unit results above.

| Path | Required observations | Result |
| --- | --- | --- |
| First microphone denial | No provider call; denied message plus Open Settings; no unusable retry | Pending |
| Reopen with permanently denied permission | Open Settings remains available; no second system prompt | Pending |
| Settings and return | NyxID settings opens; permission enabled; stale error clears; explicit retry starts voice | Pending |
| Active hands-free background cycle | Actual audio exchange, Home/background, return; same call/timer/transcript; microphone and reply still work | Pending |
| End and immediate next call | Explicit end fully settles; new call starts without stale Disconnecting state | Pending |
| Unsupported background media | Specific reason; local microphone closes; server settles before another call | Focused tests only |
| Cannot open Settings | Correct manual path; native details not leaked | Focused tests only; real OS failure cannot be forced safely |

Simulator background behavior alone cannot certify physical iPhone audio routing or OS suspension. A target-device run remains separate from simulator validation.

## Native build preparation

A simulator **Debug** build (not a production/frontend build) completed successfully:

```sh
APP_ENV=dev \
DEV_API_BASE_URL=https://nyx-api.chrono-ai.fun/api/v1 \
DEV_FRONTEND_URL=https://nyx.chrono-ai.fun \
DEV_IOS_BUNDLE_ID=dev.nyxid.ipadqa \
DEV_ANDROID_PACKAGE=dev.nyxid.ipadqa \
DEV_IOS_BUILD_NUMBER=1 APP_SCHEME=nyxid \
NODE_BINARY=/Users/abigaildeng/.npm/_npx/52027bd8fc0022aa/node_modules/node/bin/node \
PATH=/Users/abigaildeng/.npm/_npx/52027bd8fc0022aa/node_modules/node/bin:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin \
xcodebuild -workspace ios/NyxIDMobile.xcworkspace -scheme NyxIDMobile \
  -configuration Debug -sdk iphonesimulator \
  -destination id=4DDA5A51-433B-40FF-B186-BBB755DA4D07 \
  -derivedDataPath build/sprint2-voice-derivedData -jobs 4 \
  PRODUCT_BUNDLE_IDENTIFIER=dev.nyxid.ipadqa DEVELOPMENT_TEAM=56DKXVFJS2 \
  'CODE_SIGN_IDENTITY=-' build -quiet
```

The app contains `UIBackgroundModes=[audio, fetch, remote-notification]`, supports iPhone and iPad, and passes `codesign --verify --deep --strict`. Its simulator ad-hoc signature matches the already logged-in QA app's signing style. Generated CocoaPods path/checksum differences are excluded from the source change. No simulator was uninstalled or erased.

Independent review: root agent inspected the complete hook/UI diff, scope and ticket fences, and native-only ready-call background guard; no confirmed blocking findings. Device acceptance remains pending below the automated checks.
