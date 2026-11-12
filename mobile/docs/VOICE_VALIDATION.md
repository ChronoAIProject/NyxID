# Mobile voice fixes: scope and verification

This branch extracts mobile voice work from #2233 onto team `main`
(`65c61850b99417a55138699ea51709b213935864`). It includes native iOS OpenAI
WebRTC, saved-source/voice preferences, main-chat transcript integration,
compact controls and voice selection, task outcome presentation, bounded End,
and recovery. The two transcript-scroll files preserve position across voice
updates and Details/Back navigation.

It changes only `mobile/`. Universal iPad device-family/orientation/layout
adaptation remains in #2233. The iOS project changes here add only the WebRTC
framework embed paths. Backend catalog, authorization and approval behavior are
not changed. The original worktree's six uncommitted backend/chat-document
changes are excluded. Current-main system-message conversation identity and
attachment `image_input_reason` parsing are preserved.

## Ending and recovery presentation

End immediately disables capture, PTT and playback. Native OpenAI can retain
quiet media/control transport for a verified terminal acknowledgement, bounded
by seven seconds including the End request. Final cleanup releases local
resources and returns to text even if the remote slot is not confirmed terminal.
A receipt or surface dismissal does not manufacture that confirmation.

During the explicit End request, only compact Ending controls remain; the
recovery explanation and disabled Check call status action are hidden. After
cleanup, the normal text composer returns automatically. If voice is entered
again with a retained closing/unknown/conflicting call, the genuine recovery
controls remain available. Check is read-only and does not end another call.
Its status/discovery request chain has one eight-second budget. Automatic checks
are limited to six, with explicit retry available afterwards.

The optional account-active endpoint falls back on HTTP 404/405 to bounded
metadata discovery of the current chat and at most ten recent other chats,
with two reads in flight. An incomplete scan remains unverified. Exact known
session status can settle End without the optional endpoint; fresh Start still
passes ordinary backend admission. HTTP 401/403/503 never trigger this fallback.

## Focused local verification on the extracted branch

Dependencies were restored using the committed lockfile:

```sh
# mobile/
pnpm install --frozen-lockfile --offline
```

All 18 changed/new test files and the two directly related scroll/dictation tests
were run explicitly:

```sh
# mobile/
pnpm exec tsx --test src/features/account/VoiceSettingsScreen.test.ts src/features/account/voiceSettingsModel.test.ts src/features/assistant/AssistantScreen.voice.test.ts src/features/assistant/useTranscriptScrollFollow.test.ts src/features/assistant/voice/AssistantVoiceClient.test.ts src/features/assistant/voice/AssistantVoiceConversation.test.ts src/features/assistant/voice/NativeOpenAIMedia.test.ts src/features/assistant/voice/VoiceMediaBridge.test.ts src/features/assistant/voice/useNativeVoice.test.ts src/features/assistant/voice/voiceActiveCall.test.ts src/features/assistant/voice/voiceContracts.test.ts src/features/assistant/voice/voiceMediaDocument.test.ts src/features/assistant/voice/voiceRequestDeadline.test.ts src/features/assistant/voice/voiceSelection.test.ts src/features/assistant/voice/voiceStartError.test.ts src/features/assistant/voice/voiceTranscript.test.ts src/lib/api/assistantVoiceTransport.test.ts src/lib/api/http.session.test.ts src/features/assistant/assistantTranscriptScroll.test.ts src/features/assistant/useComposerDictation.test.ts
```

Initial extraction result: **497 passed, 0 failed, 0 cancelled, 0 skipped,
0 todo**. After the compact End presentation change, its affected component
tests were rerun:

```sh
# mobile/
pnpm exec tsx --test src/features/assistant/voice/AssistantVoiceConversation.test.ts
# 61 passed, 0 failed, 0 cancelled, 0 skipped, 0 todo
```

These runs overlap; their counts are not added together.

The scope analyzer identifies only the mobile package. Changed-source/test
Prettier checks cover its exact 38 staticCheckFiles: 34 pass and four have
pre-existing whole-file formatting warnings. Those warnings
in AssistantScreen.tsx, AssistantScreen.voice.test.ts,
useTranscriptScrollFollow.test.ts and http.ts also reproduce on the unchanged
team-main versions; their surrounding formatting is preserved. The two final
component source/test files pass formatting. `git diff --check` and
`plutil -lint mobile/ios/NyxIDMobile.xcodeproj/project.pbxproj` passed.

There is no repository-native affected mobile typecheck or mobile lint target.
Per the personal local-validation policy, full frontend tests, typecheck and
production builds are delegated to GitHub CI. No CI monitoring is included.
Native dependency changes require rebuilding the native App; an OTA JavaScript
update cannot install WebRTC.

## Existing actual runtime evidence

The voice implementation before extraction was exercised on 2026-11-09 using
the complete signed Debug App, iPad Pro 13-inch M5 simulator / iOS 26.3 landscape, real
NyxID service and native OpenAI WebRTC with Marin/Hands-free. Its source was
`codex/ipad-adaptation@f507b95e15edbb86154c25e06e4b3f877ef3ef96`.
There was one cleared Metro reload after source freeze and no reload between
the following final calls:

| User path | Observation |
| --- | --- |
| Enter voice and speak a harmless acoustic arithmetic fixture | Listening, real main-chat transcription and a matching spoken-answer transcript. Minor recognition substitutions occurred. |
| End and immediately reopen | Automatic composer with 0:52 receipt, then Listening in the same chat. |
| Background task fails, then mute and End | The real task was accepted and failed. Task activity showed Failed. End still returned to the composer with a 2:49 receipt. |
| Reopen and final End | Listening, then automatic composer with 0:37 receipt and native peer close. |

Earlier same-source runs also exercised saved voice search/Cancel/Apply,
mute/unmute, accessibility PTT release, Details/Back, background recovery and
idle closure. Runtime transcripts, task outcomes and media were not injected.
The user's subsequent manual iPad feedback on 2026-11-10 indicated the basic
flow was working and identified the redundant recovery display during End.
No private screenshot or business conversation is included in this document.

These are provenance records from the combined iPad branch, not a claim that
the newly extracted branch and its additional presentation condition were
rebuilt and rerun on a device. Regression tests validate the extraction and
presentation condition. Client/NativeVoiceMedia/Bridge lifecycle tests use a
controlled native adapter; they cannot alone prove physical audio behavior.

## Remaining acceptance boundaries

- Background task execution is not accepted as successful. An earlier
  `approval_unsupported` / HTTP 403 / NyxID12502 refusal was observed; the final
  failed task's Details were not reopened, so its exact cause is not inferred.
  Current server policy permits only already granted voice execution authority.
- Composer restoration was observed, but a subsequent text-send round trip was
  not completed in that last recorded run.
- Live network fault injection, physical iPad/iPhone, human listening quality,
  sustained touch PTT, Bluetooth/phone interruptions, alternate providers and
  Android runtime paths remain unverified by these runs.
- This PR does not claim to fix dictation-language issues #2222/#2223 or the
  composer-entry report #2264. It relates to the voice findings encountered
  during iPad acceptance under #2226, without closing the broader iPad issue.
