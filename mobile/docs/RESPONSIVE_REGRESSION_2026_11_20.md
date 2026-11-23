# Mobile responsive-layout regression checks — 2026-11-20

This category starts at `upstream/main` commit `b6d194f9927315b8170223ed4d02b2c5da428b3a` and covers #2701, #2698, #2697 and #2711. The checks below execute current production components in isolated native fixtures with synthetic chat, approval and upload data. They do not log into a production account or perform destructive actions. These are source-level and simulator regression checks, not certification of the original Xiaomi/MIUI or physical iPhone builds.

## Results

| Issue | Repair or current-main finding | Verification |
| --- | --- | --- |
| [#2701](https://github.com/ChronoAIProject/NyxID/issues/2701) | Use safe-area-context for ordinary Android ScreenContainer pages; the legacy React Native SafeAreaView does not supply Android system insets. Preserve the existing iOS and Duo inset rules. | At 1080 × 1920, 560 dpi and three-button navigation, the full Delete Chat target moved from y=1654–1836, overlapping the navigation strip, to y=1486–1668 above it. The native fixture uses the production SettingsPage/SettingsRow. Delete was not invoked. Focused tests cover shared settings, chat details and Account Settings containers. |
| [#2698](https://github.com/ChronoAIProject/NyxID/issues/2698) | At narrow effective widths, keep header actions in their first row and give the chat identity a second full-width row. Stack the home card's badge and metadata, and allow its name and summary to wrap. | At 1440 × 3200, 840 dpi and font scale 1.0, the baseline hides almost all of NyxBot and truncates the away-card title and summary. The fixed native fixture shows the full identity, “Research and release assistant”, its complete summary, status and time. Component tests retain identity presses and ordinary-width layouts. |
| [#2697](https://github.com/ChronoAIProject/NyxID/issues/2697), A | Replace Android Alert button rows with a bounded, scrollable list of Camera, Photos, Files and any connection action. Dismiss the native modal before opening the chosen picker. | Native font scales 1.3 and 2.0 show full-width vertical rows. At 2.0, Files opens the system document picker after dismissal; Cancel returns to the page. Hook tests cover all native picker routes, captured stale callbacks, context changes and connection actions. iOS retains its native action sheet. |
| #2697, B | In compact layouts, wrap the filename and place the file action below the metadata; bound the attachment width and avoid vertical flex growth in the stacked metadata. | At native font scale 2.0, the production ChatMessageTurn and AssistantAttachmentView show the whole filename and a visible Save image target within the chat bubble. Tapping Save image opens Android's system destination picker. Authenticated-download, retention, permission and cache behavior remains covered by the existing executable tests. |
| #2697, C | Current main already measures segment widths and scrolls when labels cannot fit. No additional SegmentControl change is required. | With production fonts loaded at native font scale 2.0, Connectors, Skills, Machines and Saved logins remain on one line. Tapping Saved logins changes the selected index. Existing focused tests also cover overflow, selection and font changes. |
| #2697, D | At narrow effective widths, move the outcome into the history-card body below its metadata, preserving width for the request. | At font scale 2.0, the body grows from 655 px to 992 px in the same fixture. The whole summary, requester, time and Expired outcome remain readable; tapping the card increments the synthetic detail-open counter. |
| [#2711](https://github.com/ChronoAIProject/NyxID/issues/2711) | Wrap pending attachment cards within a bounded vertical scroller; narrow screens use full-width rows and wider screens can fit multiple cards per row. | On Android API 34 and iPhone 14 Pro / iOS 26.3, a native vertical gesture reaches four.txt and its remove button. Removing it leaves one.txt, two.txt and three.txt. The fixture mounts production ChatComposerFrame and AssistantPendingUploads and supplies synthetic selected-file entries. It does not perform selection or upload. |

The original #2711 failure to swipe horizontally on physical devices did not reproduce on these simulators: the baseline showed the same initial clipping, but a horizontal swipe did work. The fix makes all cards accessible through wrapping and vertical scrolling and avoids relying on that horizontal gesture. For #2698, native proof covers layout; an attempted all-controls interaction sweep was blocked by the Expo development tools overlay, so it is not counted as complete native navigation verification.

## Native evidence

The Android checks ran on an isolated API 34 AOSP emulator. The small-window and high-density checks intentionally override display size/density. #2697 uses 1440 × 3200, 560 dpi and font scales 1.3/2.0; its final 2.0 checks load the app's Manrope, Space Grotesk and JetBrains Mono fonts. The attachment download fixture returns a generated PNG, and storage/auth/network adapters are synthetic. Native rendering and picker launches use the platform components. #2711 was checked independently on a second Android emulator and an iPhone simulator. The reusable iOS QA shell uses the supported non-glass fallback; it does not certify the production glass surface.

| Evidence | Before | After |
| --- | --- | --- |
| Android bottom safe area | [Baseline](responsive-regression-20261120/2701-before.png) | [Fixed](responsive-regression-20261120/2701-after.png) |
| High-density header and home row | [Baseline](responsive-regression-20261120/2698-before.png) | [Fixed](responsive-regression-20261120/2698-after.png) |
| Large-text attachment sources | [Baseline](responsive-regression-20261120/2697A-before.png) | [Fixed](responsive-regression-20261120/2697A-after.png) |
| Large-text sent attachment in chat bubble | [Baseline](responsive-regression-20261120/2697B-before.png) | [Fixed](responsive-regression-20261120/2697B-after.png) |
| Current-main large-text segments | [Verified at 200%](responsive-regression-20261120/2697C-main.png) | No code change required |
| Large-text history | [Baseline](responsive-regression-20261120/2697D-before.png) | [Fixed](responsive-regression-20261120/2697D-after.png) |
| Android multiple attachments | [Baseline](responsive-regression-20261120/2711-android-before.png) | [Fourth card](responsive-regression-20261120/2711-android-four.png) / [Removed](responsive-regression-20261120/2711-android-removed.png) |
| iOS multiple attachments | [Baseline](responsive-regression-20261120/2711-ios-before.png) | [Fourth card](responsive-regression-20261120/2711-ios-four.png) / [Removed](responsive-regression-20261120/2711-ios-removed.png) |

## Focused local checks

Run from the repository root:

```sh
mobile/node_modules/.bin/tsx --test --test-concurrency=1 \
  mobile/src/components/ScreenContainer.test.ts \
  mobile/src/components/settings/SettingsList.test.ts \
  mobile/src/features/assistant/AssistantDetailsScreen.test.ts \
  mobile/src/features/account/AccountSettingsScreen.test.ts \
  mobile/src/app/workspaceScreenContainers.test.ts \
  mobile/src/features/assistant/ChatIdentityHeader.test.ts \
  mobile/src/features/assistant/NyxBotHome.test.ts \
  mobile/src/features/assistant/AttachmentSourcePicker.test.ts \
  mobile/src/features/assistant/useAssistantUploads.test.ts \
  mobile/src/features/assistant/attachmentImageSave.test.ts \
  mobile/src/components/ApprovalCards.test.ts \
  mobile/src/components/SegmentControl.test.ts \
  mobile/src/components/workspace/useNativeSheetDismiss.test.ts \
  mobile/src/features/assistant/AssistantPendingUploads.test.ts \
  mobile/src/features/assistant/AssistantUploadSession.test.ts \
  mobile/src/features/assistant/assistantUploadStatus.test.ts \
  mobile/src/features/assistant/AssistantScreen.voice.test.ts \
  mobile/src/features/groups/GroupChatHeader.test.ts
```

262 focused tests pass. New regressions for Android insets, narrow headers/home rows, the Android source chooser, sent-attachment stacking and history outcomes were exercised against the baseline before their respective repair. The source-chooser integration checks additionally verify that both private chat and crew mount the sheet returned by their own upload session. #2711's independent focused run passed 149 tests; its `jest --findRelatedTests src/features/assistant/AssistantPendingUploads.tsx --passWithNoTests --runInBand` found no additional tests.

```sh
python3 ~/.codex/skills/frontend-incremental-pr/scripts/frontend_change_scope.py --repo . --base upstream/main
git diff upstream/main --check
```

All 20 changed TS/TSX files parse without TypeScript syntax diagnostics using `createSourceFile` over the analyzer's `staticCheckFiles`. No repository-native affected mobile typecheck or lint target is available. Full frontend tests, semantic typecheck and production builds are delegated to GitHub CI under the user's local-validation policy.
