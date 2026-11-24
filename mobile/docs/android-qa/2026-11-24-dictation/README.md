# Android dictation verification — 2026-11-24

Refs #2760 and #2761. Quality gate: **PASS WITH NOTES**. The changed production hook and language picker passed focused regression tests and an Android native integration fixture. Xiaomi's real speech provider and the normal release app still need device retesting.

## Behavior verified

- Language discovery queries the same default Android recognition provider used for recording. The picker displays provider-reported locales; it does not manufacture a supported-language list.
- Empty, rejected or nonresponding discovery has a bounded 2.5-second result. Android exposes retry, speech settings and a validated manual language code with a provider-support caveat.
- Composer dictation recovers from a provider's early no-speech termination only before speech/transcript begins, after native `end`, within a 15-second initial grace period, with at most three restarts and starts spaced at least five seconds apart. A late queued retry cannot reopen capture after the deadline.
- Stop, discard, backgrounding, chat/account changes and sheet dismissal cancel pending recovery and release audio ownership. Crew utterances retain their original silence behavior.

## Native integration evidence

Device: `emulator-5572`, Android 14/API 34, 1080×2400, 420 dpi. No recognition service was installed initially (`voice_recognition_service=null`, recognition-service query returned no services).

A temporary SDK-only `RecognitionService` (`dev.nyxid.voicetest/.Provider`) exercised Android SpeechRecognizer and the real Expo native bridge. It reported three locales, an empty list or no callback, and generated a no-speech error after 4.3 seconds. Its result scenario returned a synthetic phrase after one no-speech/restart cycle. It performed no network requests and did not read or retain microphone audio.

The isolated app `dev.nyxid.voicechecks` loaded the production `useComposerDictation`, `recognitionLocales`, `useDictationLanguage` and `DictationLanguagePicker` unchanged in a minimal navigation/theme/i18n harness. Its native shell came from the existing QA 1.0.4 build 8 APK, with a separate manifest identity and an embedded development fixture bundle. This checks native event delivery and the production JS integration; it is not a full signed release build or a logged-in app end-to-end test. Synthetic results do not prove acoustic recognition or Xiaomi/OEM compatibility.

| Scenario | Observed result |
| --- | --- |
| Provider returns `en-US`, `zh-CN`, `ja-JP` | Actual picker showed all three; searching `zh-CN` found Chinese |
| Provider returns an empty list | Recovery explanation, retry, speech settings and manual-code action appeared |
| Provider never responds to discovery | Loading ended after the bounded timeout; recovery remained usable |
| Choose manual `ja-JP`, then cold-restart app | Selection persisted and subsequent native starts used `ja-JP` |
| Provider reports no-speech after ~4.3 seconds | At six seconds, UI remained listening without an error; native starts occurred around 0, 5 and 10 seconds |
| Continue without speech | Terminal no-speech appeared 15,043 ms after the first listening event |
| Synthetic result after the first automatic restart | `QA recognized after pause` inserted exactly once |
| Stop during recovery session | No later native start; existing draft preserved |

[results.json](results.json) contains assertions, hook events and SHA-256 hashes of the four production source files embedded in the tested bundle. Those hashes matched the final source before committing. [native-events.log](native-events.log) records native/provider and hook events. Screenshots show [supported languages](supported-languages.png), [unavailable-list recovery](unavailable-recovery.png), [listening during grace](grace-listening.png) and [one result after recovery](result-after-pause.png).

Temporary apps were uninstalled after testing. The original recognition setting, display size/density, font scale and accessibility setting were restored and compared. Only the temporary port-8311 reverse was removed; the pre-existing port-8299 reverse and existing app packages were retained. See [cleanup.json](cleanup.json). No other emulator was used.

The local fixture sources, bundle-generation/repack scripts, verification scripts and original raw outputs are retained at `/tmp/nyxid-issues-1124/voice-native` in this task's environment. APKs and generated JavaScript bundles are not committed.

## Focused local verification

Run from `mobile/`:

```sh
pnpm exec tsx --test \
  src/features/assistant/useComposerDictation.test.ts \
  src/features/assistant/voice/useDictationLanguage.test.ts \
  src/features/assistant/voice/recognitionLocales.test.ts \
  src/features/assistant/voice/resolveDictationLanguage.test.ts \
  src/features/assistant/voice/dictationLocale.test.ts \
  src/features/assistant/voice/dictationLanguageLabels.test.ts \
  src/features/assistant/voice/DictationLanguagePicker.test.ts \
  src/features/assistant/useAssistant.dictation.test.ts \
  src/features/assistant/ComposerDictation.test.ts \
  src/i18n/i18nCoverage.test.ts

pnpm exec jest --findRelatedTests \
  src/features/assistant/useComposerDictation.ts \
  src/features/assistant/voice/DictationLanguagePicker.tsx \
  src/features/assistant/voice/recognitionLocales.ts \
  src/features/assistant/voice/useDictationLanguage.ts \
  --runInBand

pnpm dlx prettier@3.6.2 --check \
  src/features/assistant/useComposerDictation.ts \
  src/features/assistant/useComposerDictation.test.ts \
  src/features/assistant/voice/recognitionLocales.ts \
  src/features/assistant/voice/recognitionLocales.test.ts \
  src/features/assistant/voice/DictationLanguagePicker.tsx \
  src/features/assistant/voice/DictationLanguagePicker.jest.test.tsx \
  src/features/assistant/voice/useDictationLanguage.ts \
  src/features/assistant/voice/useDictationLanguage.test.ts
```

Results: **193/193 node tests**, **5/5 related Jest tests**, changed-file formatting passed. `pnpm sync:i18n` completed. `git diff --check` passed from the repository root. No mobile lint command or reliable affected typecheck target exists; local typecheck was skipped. Full frontend suite, typecheck and production build are delegated to GitHub CI under the personal focused-validation policy.

The parent agent reviewed the implementation independently, including native-end ordering, final-error precedence, deadline checks and explicit-provider/offline behavior. The resulting fixes passed the focused tests and the final native run.

## Xiaomi / real-provider retest

1. Install a build containing this change on the reported Xiaomi Android 13/API 33 device. Record its selected speech-recognition service and build version.
2. Open Settings → Voice → Dictation language. Confirm provider languages are listed and a selected Chinese/Japanese/English choice is retained. If discovery is unavailable, retry the list, inspect Speech recognition settings, then try a language code supported by that provider. Confirm the manual choice survives restarting the app.
3. Start composer dictation and stay silent past 4.3 seconds. Confirm the UI remains active during initial grace. Speak around 6–10 seconds and verify the text is inserted once into the existing draft.
4. Start again without speaking. Verify eventual no-speech feedback exposes Retry/Close and Retry starts a new attempt.
5. Repeat with Stop, dismissal/navigation and backgrounding during silence recovery. Verify there is no automatic reopening of the microphone and that the draft is preserved.
6. Verify crew voice utterances still finish with their existing silence behavior. Check network/provider errors are shown directly and do not become automatic no-speech retries.

Keep #2760/#2761 linked for this OEM retest rather than claiming the original device reports are fully verified by the synthetic-provider fixture.
