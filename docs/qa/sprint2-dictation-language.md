# Sprint 2 — Dictation language picker (#2450)

Test date: 2026-11-16 (Asia/Shanghai). Implementation commit: `1ffc58acfe9b622f3cb165006b8cf41051570d99` on `codex/sprint2-dictation-language`; base: `4372b638898a272ac8942d90ce099d2fd0ce3dec` (`upstream/main`). The native E2E below was performed against that implementation. The PR was subsequently synchronized with team `upstream/main` at `0d37c7397`; the merge preserved both sides' locale additions and did not change the dictation picker implementation.

## Scope and minimum fix

The picker used the small `SettingsMenuRow` without search. Its `Intl.DisplayNames` fallback returned raw codes on Hermes, which does not implement that API. The fix reuses the existing `SettingsSelectionRow` and native `WorkspaceSelectionSheet` for search, temporary selection, Cancel and Apply. A small bundled CLDR name table supplies English, simplified/traditional Chinese and native-language aliases without an additional dependency. Codes remain secondary text and search terms. Device language stays first and shows its resolved locale; the explanation distinguishes message-field dictation from live voice. The existing AsyncStorage preference is retained. An unsupported saved value now selects the same automatic fallback already used by the recognizer.

All three native surfaces use these same TypeScript components. No backend, permissions, native modules or native configuration changed.

## Actual native end-to-end verification

Environment:

- iPad Pro 13-inch (M5), iOS 26.3 simulator, UDID `BBBA8E20-A536-4B64-8882-09BF53584CED`.
- Existing installed NyxID development app `dev.nyxid.ipadqa`; its prior authenticated session was reused. Nothing was uninstalled and no credentials were inspected or changed.
- Real application entry point, native views, Hermes, native recognizer locale discovery and AsyncStorage. No fixture or mocked backend was used for this user path.
- Metro served this branch on port 8091. The configured real backend was `https://nyx-api.chrono-ai.fun/api/v1` (production legacy hostname). Voice settings finished loading the live configuration.
- UI actions used the Simulator accessibility tree and actual controls through CUA. Cold start used `xcrun simctl terminate` followed by `xcrun simctl launch`.

| Step | Action and visible result | Result |
| --- | --- | --- |
| 1 | Open Settings → Voice → Dictation → Language. Names such as Arabic (Saudi Arabia), Chinese (China) and native aliases appeared, with locale codes as secondary text. Device language was the first option, checked, and resolved to English (United States) · en-US. Search and the explanation were visible. | PASS |
| 2 | Search `日本語`. Exactly Japanese (Japan), with `日本語 · ja-JP`, remained. Select it, then Cancel. The Voice page still displayed English (United States). | PASS |
| 3 | Reopen and search `ja-JP`. The same Japanese option appeared. Select it and Apply. The Voice page immediately displayed Japanese (Japan). | PASS |
| 4 | Terminate and launch the installed app without uninstalling. Reopen Settings → Voice. The page still displayed Japanese (Japan), confirming native persisted selection across cold start. | PASS |
| 5 | Reopen the picker and search `中文`. Chinese (China), Chinese (Hong Kong SAR China), and Chinese (Taiwan), with their native aliases and distinct codes, appeared. | PASS |
| 6 | Search Device language, select it and Apply. The Voice page returned to English (United States). This restored the user's original automatic preference. | PASS |

Screenshots are retained locally under `.planning/sprint2-dictation-language/e2e/` in this worktree: `01-native-picker.png`, `02-native-search-japanese.png`, `03-native-selected-japanese.png`, `04-native-after-cold-start.png`. They are not committed because the retained signed-in app can show personal workspace names behind the sheet.

An initial Metro localhost failure was an environment issue: Node bound only IPv6 while the development client used IPv4. Restarting Metro with `NODE_OPTIONS=--dns-result-order=ipv4first` resolved it. It did not require an application change.

## Focused automated verification

Run from `mobile/`:

```sh
node --import tsx --test src/components/workspace/WorkspaceSelectionField.test.ts src/features/groups/GroupEditorScreen.test.ts src/features/plugins/PluginSkills.test.ts src/features/agents/AgentsScreen.permissions.test.ts src/features/plugins/PluginConnectionHistory.test.ts src/features/assistant/voice/AssistantVoiceConversation.test.ts src/features/account/VoiceSettingsScreen.test.ts src/features/assistant/ComposerDictation.test.ts src/features/assistant/useComposerDictation.test.ts src/features/groups/CrewVoiceComposer.test.ts src/features/assistant/useAssistant.dictation.test.ts src/features/account/MobileSettingsMessages.test.ts src/features/assistant/voice/dictationLanguage.test.ts src/features/assistant/voice/useDictationLanguage.test.ts src/features/assistant/voice/dictationLocale.test.ts src/i18n/mobileRuntimeMessages.test.ts
```

Result: **257 tests passed**, zero failures/skips. This includes every existing test file consuming the changed selection sheet, plus corresponding settings/dictation/localization regressions. The two new dictation test files were also rerun after the final fallback assertion: **3 tests passed**.

Changed-file formatting check, run from the repository root:

```sh
node /Users/abigaildeng/Documents/work/NyxID/frontend/node_modules/prettier/bin/prettier.cjs --check mobile/src/components/settings/SettingsSelectionRow.tsx mobile/src/components/workspace/WorkspaceSelectionField.tsx mobile/src/features/account/VoiceSettingsScreen.tsx mobile/src/features/account/VoiceSettingsScreen.test.ts mobile/src/features/account/settingsTestHarness.ts mobile/src/features/assistant/voice/dictationLanguage.ts mobile/src/features/assistant/voice/useDictationLanguage.ts mobile/src/features/assistant/voice/dictationLanguage.test.ts mobile/src/features/assistant/voice/useDictationLanguage.test.ts
git diff --check
```

Result: **PASS**. TypeScript `transpileModule(..., reportDiagnostics: true)` additionally checked syntax for those nine changed TS/TSX files; JSON parsing validated the three changed/new dictionaries. This is explicitly not a project typecheck. The mobile package has no native affected typecheck or mobile lint target, so neither was substituted with a full command. Full frontend suite/typecheck/build is delegated to GitHub CI under the user's local validation policy.

## Independent review and limits

The parent agent independently read the relevant implementation diff and reported no confirmed blocking issue. Self-review checked reuse, existing consumer regressions, unchanged recognizer locale values, script/region handling and cancellation/persistence.

**PASS WITH NOTES:** the changed picker completed its actual authenticated native user path on iPad. The iPhone QA simulator loaded the real app and all normal login providers, but had no signed-in session; its picker path was not executed. Android and physical devices were not exercised for this PR. Shared components and Hermes-specific regression tests support consistency but do not replace those device checks. Audio recognition accuracy, live voice and push notifications are outside #2450 and are not claimed as validated by these picker tests.


## Team main synchronization (2026-11-16)

Merged team `upstream/main` commit `0d37c7397` into the existing PR branch. Only the two Chinese catalog tails conflicted; both dictation phrases and the team's organization settings phrases were retained. There were no conflicts or upstream changes in the dictation implementation or shared selection sheet. Independent review of the final diff against the new team baseline found no confirmed blocking issue and no new selection or persistence behavior.

Re-ran the dependency-related tests on the merged tree, including the new main callers of the shared selection sheet and the locale completeness regression:

```sh
cd mobile
node --import tsx --test src/components/workspace/WorkspaceSelectionField.test.ts src/features/groups/GroupEditorScreen.test.ts src/features/plugins/PluginSkills.test.ts src/features/agents/AgentsScreen.permissions.test.ts src/features/plugins/PluginConnectionHistory.test.ts src/features/assistant/voice/AssistantVoiceConversation.test.ts src/features/account/VoiceSettingsScreen.test.ts src/features/assistant/ComposerDictation.test.ts src/features/assistant/useComposerDictation.test.ts src/features/groups/CrewVoiceComposer.test.ts src/features/assistant/useAssistant.dictation.test.ts src/features/account/MobileSettingsMessages.test.ts src/features/assistant/voice/dictationLanguage.test.ts src/features/assistant/voice/useDictationLanguage.test.ts src/features/assistant/voice/dictationLocale.test.ts src/i18n/mobileRuntimeMessages.test.ts src/features/auth/rootEntryLayout.test.ts src/features/auth/loginRequests.test.ts src/features/auth/agentKeySelectionFields.test.ts src/features/agents/AgentCollaborationPanel.test.ts src/features/groups/CollaborationGroupControls.test.ts src/i18n/ws2LocaleCompleteness.test.ts
```

Result: **466 tests passed**, zero failures/skips. The exact nine-file Prettier command above passed again; TypeScript syntax and all three JSON dictionaries also passed, as did `git diff --check`. The local log is `/tmp/nyxid-sprint2-dictation-main-sync-tests.log`.

The actual iPad E2E evidence remains the completed six-step run above; it has not been represented as a second simulator run after synchronization. The parent agent reserved the native device for the other categories' E2E. Full frontend suite/typecheck/build remains delegated to GitHub CI.
