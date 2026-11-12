# NyxID iOS 1.0.4 (58)

This release includes the mobile work in PR #2322, which incorporates #2233 and #2321, plus the reviewed follow-up changes. It uses the existing `fun.chrono-ai.nyxid` App Store listing. Build 57 was already uploaded, so this export uses 58.

## Voice and dictation

- Use native WebRTC for GPT Live audio on iOS. A native rebuild is required.
- Add `defaultToSpeaker` to WebRTC's startup configuration while retaining its existing voice-processing mode and Bluetooth support. Playback gain remains 1.0 unless explicitly muted.
- Reduce the expanded waveform from 6pt bars to 2pt bars. Compact voice and dictation also use 2pt bars, with rounded ends and slimmer resting states.
- Keep the normal voice panel compact. Grow it for additional content, and enable scrolling only when available space constrains the content at approximately 60% of the viewport.
- Keep dictation in the message composer. Put its waveform between Discard and Finish, allow text to grow like ordinary typing, and show recoverable error information outside the composer.
- Keep voice selection stable while the picker is open. Search server-provided voices and save a selection for the next call.
- Remove playable voice samples.
- Show the previous call's disconnect/restart state and poll exact-session closure every 500ms during recovery. Retain ownership, cancellation, and late-result fences.
- Include sealed voice captions in the conversation transcript. Input and playback state determine Listening and Speaking.

## Workspace and account settings

- Add persisted English, Simplified Chinese, and Traditional Chinese choices under Settings → Language. Localization covers supported native settings and selected workspace controls; it is not a claim of complete app translation.
- Retain separate dictation-language settings and the original language of messages.
- Improve chat loading, scroll positioning, composer behavior, crew composer parity, and attachment progress presentation.
- Refine light-mode sign-in and keep a white label on the primary Sign In button.
- Check embedded API hosts before mounting the app and use the working host for that launch.
- Retain profile, security, active-session, privacy, sign-out, and account-deletion controls. This release does not replace the existing account-erasure contract.
- Add adaptive iPad workspace layout, rotation, and multitasking support.

## Backend compatibility

- Preserve all 32 previously seeded OpenAI voice identifiers, labels, and ordering. Append `shida` without replacing `shitan`.
- Only append to an exact, unmodified historical seed. Preserve administrator edits, explicit clears, and sibling inference metadata.
- Add the first-party-human-only, read-only `GET /api/v1/assistant/nyxagent/voice/active` endpoint to find the caller's active call across conversations. Keep existing routes and response shapes.
- Keep persisted NyxAgent model selection and its validation unchanged by these voice additions.

## Build and evidence

Build 58's signed IPA and the matching iPhone simulator binary were verified before the final main-branch merge. The final PR also includes newer changes from main, so its commit is newer than that IPA; the IPA's source manifest remains the record of its exact inputs.

The iPhone simulator confirmed `playAndRecord`, `voiceChat`, `allowBluetooth`, and `defaultToSpeaker` on two successive real GPT Live calls without a runtime route override. Physical-device audible playback remains a separate QA check.

See the Desktop submission package for screenshot sources, image dimensions, release verification, and submission-field files. App Store metadata and this changelog contain no review-account credentials.
