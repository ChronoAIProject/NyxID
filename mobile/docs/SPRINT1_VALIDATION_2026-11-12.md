# Sprint 1 mobile verification — 2026-11-12

## Scope and source revisions

This record covers the four open Sprint 1 issues assigned to `AbigailDeng`:
[#2326](https://github.com/ChronoAIProject/NyxID/issues/2326),
[#2266](https://github.com/ChronoAIProject/NyxID/issues/2266),
[#2255](https://github.com/ChronoAIProject/NyxID/issues/2255), and
[#2254](https://github.com/ChronoAIProject/NyxID/issues/2254).

- Audited existing PR: [#2348](https://github.com/ChronoAIProject/NyxID/pull/2348), head `2cf63b6fc`.
- Separate implementation branch: `codex/mobile-sprint1-fixes`.
- Implementation base: team repository `upstream/main`, commit `31f3d8969e051e1fe6ed2f42a5594ded90606fe5`.
- The new branch starts from team `main`; it does not stack additional Sprint 1 changes on the voice PR branch.
- Changes are confined to mobile code, mobile regression tests, mobile localization, and this record.

## Audit of PR #2348

| Issue                                                             | Finding at the audited PR revision                                                                                                                                                                                                                                                      | Work in the separate branch                                                                                                                                                                                                                                                                      |
| ----------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| #2326 — saved NyxBot conversation remains in its loading skeleton | Incomplete. History reads could wait indefinitely; the existing loading/recovery changes did not bound an unresponsive history request.                                                                                                                                                 | Add a 10-second history-read deadline, settle loading and recovery state on timeout, preserve the correct active request when callers coalesce, and expose errors from a live replacement read. Add initial-load, retry, pagination, cancellation, and attachment regression coverage.           |
| #2266 — sidebar becomes unresponsive after a fresh login          | Existing code already separates drawer navigation from pending notification activation. This production protection came from merged [PR #2322](https://github.com/ChronoAIProject/NyxID/pull/2322), and is present in both team `main` and PR #2348; it was not introduced by PR #2348. | Add tests that open, close, and select drawer destinations while activation remains pending or later rejects, without another production drawer change. The original fresh-account native reproduction is still outstanding.                                                                     |
| #2255 — software keyboard covers email/password fields            | Unfixed. `AuthHomeScreen` had a normal `ScrollView` with no automatic keyboard inset or focus-reveal handling. Its surrounding `ScreenContainer` supplied safe areas only.                                                                                                              | Enable iOS native keyboard insets and interactive dismissal. Reveal focused email, password, and MFA fields using actual window geometry. Ignore measurements invalidated by focus, keyboard, unmount, or scroll changes. Keep fields, submit actions, and error content in the scrollable form. |
| #2254 — Privacy/Terms body links do nothing                       | Unfixed. Canonical Markdown uses `/terms` and `/privacy`; the Markdown component's default handler forwarded these relative paths directly to native `Linking`.                                                                                                                         | Resolve canonical links to native stack pushes, preserve the source page for Back, and handle HTTPS/mailto opens with localized failure and retry feedback.                                                                                                                                      |

The audit is a combination of source inspection and focused regression tests. An issue remaining open on GitHub does not itself prove that its failure is still reproducible.

## Automated verification

The root agent ran the following explicit selection from `mobile/` with Node `22.23.4`. Result: **177 passed, 0 failed, 0 skipped**, across 15 named test files. The output was saved locally at `/tmp/nyxid-sprint1-focused.log`.

```bash
/Users/abigaildeng/.npm/_npx/52027bd8fc0022aa/node_modules/node/bin/node \
  --import tsx --test \
  src/app/AppDrawerNavigation.test.ts \
  src/components/workspace/WorkspacePushShell.test.ts \
  src/features/assistant/useAssistant.history.test.ts \
  src/features/assistant/useAssistant.test.ts \
  src/features/assistant/useAssistant.dictation.test.ts \
  src/features/assistant/useAssistant.interruption.test.ts \
  src/features/assistant/assistantHistoryTarget.test.ts \
  src/features/assistant/assistantState.test.ts \
  src/lib/api/assistantApi.history.test.ts \
  src/features/auth/AuthHomeScreen.test.ts \
  src/features/auth/AuthTextInput.test.ts \
  src/features/legal/LegalDocumentScreen.test.ts \
  src/features/legal/legalLink.test.ts \
  src/features/legal/legalMarkdown.test.ts \
  src/i18n/language.test.ts
```

The same focused coverage was also exercised in three implementation/review groups: auth/legal **66/66**, history and related state/API tests **63/63**, and drawer/shell **48/48**. These groups comprise the 177-test selection above, not three additional full-suite runs.

The history timeout/retry scenario that restores **10 messages and 2 attachments** uses the actual hook with controlled transport, timer, and response fixtures. It proves the hook behavior under the modeled request failure; it is not an observation of the original affected server conversation on a device.

Changed-file static checks run from the repository root:

```bash
git diff --check

node /Users/abigaildeng/.npm/_npx/75399663b1768a01/node_modules/prettier/bin/prettier.cjs \
  --check \
  mobile/src/app/AppDrawerNavigation.test.ts \
  mobile/src/features/assistant/assistantHistoryRequest.ts \
  mobile/src/features/assistant/useAssistant.history.test.ts \
  mobile/src/features/assistant/useAssistant.ts \
  mobile/src/features/legal/LegalDocumentScreen.tsx \
  mobile/src/features/legal/LegalDocumentScreen.test.ts \
  mobile/src/features/legal/PrivacyPolicyScreen.tsx \
  mobile/src/features/legal/TermsOfServiceScreen.tsx \
  mobile/src/features/legal/legalLink.ts \
  mobile/src/features/legal/legalLink.test.ts \
  mobile/src/i18n/locales/zh-CN.json \
  mobile/src/i18n/locales/zh-TW.json
```

Both commands passed, including all 12 explicitly listed source/test/localization files. The added verification document also passed its own explicit format check:

```bash
node /Users/abigaildeng/.npm/_npx/75399663b1768a01/node_modules/prettier/bin/prettier.cjs \
  --check mobile/docs/SPRINT1_VALIDATION_2026-11-12.md
```

`AuthHomeScreen.tsx` and its existing test file already failed whole-file Prettier checking in unchanged base snapshots. The implementation preserves their surrounding formatting; the added test block was formatted by range. A blanket formatting rewrite was deliberately avoided. This is a disclosed pre-existing formatting limitation, not a claim that a whole-file check passed for those two files.

No local full mobile test suite, full typecheck, or production build was run. The repository has no native affected-project typecheck target for these changes. Full project verification remains assigned to GitHub CI.

## Independent review

Independent review was completed. Confirmed findings were fixed and the affected focused tests rerun:

1. Coalesced history callers must preserve the lifecycle and cancellation ownership of the active request.
2. A live replacement of initial history must expose its own timeout/error and recovery action.
3. An input measurement cannot be combined with a newer scroll offset. The keyboard helper now captures one offset and abandons the measurement if scrolling changes before it completes. The regression case covers a native scroll of 600 points between the two measurement callbacks.

## Native iPhone checks actually performed

The root agent loaded this branch's source through its separate Metro server at `http://127.0.0.1:8083/` into the existing development native app `dev.nyxid.ipadqa`. A separate, unsigned-in **iPhone 17 Pro / iOS 26.3 simulator** was used for the login and public legal-document checks below. Existing authenticated devices and the old Metro on port 8081 were preserved.

| Path                                                       | Observed result                                                                                                             |
| ---------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------- |
| Login with the software keyboard: Email → Password → Email | The focused fields remained visible. At the initial Email focus, Email was visible and switching to Password was available. |
| Switch to Password with the software keyboard open         | Email, Password, and Sign In were all above the keyboard.                                                                   |
| Show password control                                      | Visibility toggled.                                                                                                         |
| Tap a blank area of the login page                         | The software keyboard dismissed.                                                                                            |
| Privacy document → body Terms link                         | Navigated to Terms inside the app.                                                                                          |
| Terms document → scroll to section 1.5 → body Privacy link | Navigated to Privacy inside the app.                                                                                        |
| Back from that Privacy page                                | Returned to the previous Terms document at the section 1.5 scroll position.                                                 |

The legal paths used public canonical documents and the newly served branch source. These observations verify those specific paths; they do not establish that every app path or every original issue environment passed native testing.

The root agent also loaded the new source on an already authenticated iPhone, preserving the existing account. The development menu explicitly confirmed **Connected: 8083** before these checks:

| Path                                                                   | Observed result                                              |
| ---------------------------------------------------------------------- | ------------------------------------------------------------ |
| Home → Open navigation → Search → recent existing chat                 | `Loading messages` was replaced by real historical messages. |
| OS Home to background the app → NyxID icon to return to the foreground | The same conversation history remained readable.             |

This was an existing saved conversation, not the exact conversation identified in #2326. No message-count or attachment-restoration assertion was made for this native run. The native observations support normal history loading and foreground return; the modeled timeout/retry behavior remains a separate automated-test result.

## Remaining verification boundaries

On the same authenticated iPhone running the new source, the root agent also
reloaded the home page, opened the drawer, selected **Machines**, reopened the
drawer, and used the exposed-page **Close navigation** control. The Machines
page returned with its normal navigation control available. This verifies an
established-account path; it does not reproduce the fresh-account native
permission/modal condition described in #2266.

- Native MFA keyboard interaction has not been exercised. MFA focus-reveal behavior is covered by focused component tests.
- The original #2266 first-login/new-account device scenario, including its exact original touch/modal state, has not been reproduced on the new build.
- The exact originally affected #2326 saved conversation has not been rerun on a native device. Timeout, Retry, pagination, and attachment recovery currently have hook/fixture evidence.
- Android and iPad native behavior for these Sprint 1 changes has not been claimed as tested in this record. The iPad-approved voice implementation is outside this change scope.
- The first PR CI result and its follow-up corrections are recorded below. Full verification runs on GitHub CI.

Quality-gate result: **PASS WITH NOTES**. Independent review found no remaining
confirmed blocking defect in the final diff, and the 177 focused tests passed.
The native and CI verification boundaries above remain explicit.

## PR #2417 CI follow-up

The first [Mobile CI job](https://github.com/ChronoAIProject/NyxID/actions/runs/43223968497/job/131202512631)
on `d829be9edd4b9e03277f74f3e124c881c2e0492b` passed installation, native-sync,
and the full unit-test step, then **failed TypeScript checking**. The original
177 focused passing tests did not establish a passing CI result.

Two corrections address all six reported TypeScript diagnostics:

- The history test's `active_turn` fixture now includes the required `activities`
  and `attachments` arrays.
- The keyboard helper now obtains the native viewport through the public
  `ScrollView.getNativeScrollRef()` API before calling `measureInWindow()`.
  This preserves the viewport geometry and existing stale-measurement guards,
  provides the public host type for callback inference, and safely skips an
  unmounted host. The scroll-view mocks follow that public API. A regression
  verifies no measurement or scroll before mounting and successful reveal on
  the next focus after mounting.

Follow-up focused command, from `mobile/`:

```bash
/Users/abigaildeng/.npm/_npx/52027bd8fc0022aa/node_modules/node/bin/node \
  --import tsx --test \
  src/features/auth/AuthHomeScreen.test.ts \
  src/features/auth/AuthTextInput.test.ts \
  src/features/assistant/useAssistant.history.test.ts
```

Result: **57 passed, 0 failed, 0 skipped**. Independent final review found no
blocking issues and separately passed the two directly changed test files
(55 tests). This is a focused follow-up run, not a rerun of the earlier
177-test selection.

Static checks for this follow-up, from the repository root:

```bash
git diff --check
node /Users/abigaildeng/.npm/_npx/75399663b1768a01/node_modules/prettier/bin/prettier.cjs \
  --check mobile/src/features/assistant/useAssistant.history.test.ts
node /Users/abigaildeng/.npm/_npx/75399663b1768a01/node_modules/prettier/bin/prettier.cjs \
  --check --range-start 9181 --range-end 10573 \
  mobile/src/features/auth/AuthHomeScreen.tsx
node /Users/abigaildeng/.npm/_npx/75399663b1768a01/node_modules/prettier/bin/prettier.cjs \
  --check --range-start 8788 --range-end 17013 \
  mobile/src/features/auth/AuthHomeScreen.test.ts
```

The whole-file Auth formatting limitation described above remains. Only the
changed callback and test region were formatted. Full typechecking remains
delegated to GitHub CI because no native affected target exists.

After the public-API correction, the root agent reloaded the separate unsigned-in
QA iPhone, confirmed its development menu was connected to Metro **8083**, and
repeated **Email → Password → Email** with the software keyboard open. Both
fields remained visible; after switching to Password, Email, Password, and
Sign In were all above the keyboard. This smoke check did not submit credentials
or change the existing signed-in devices.

The latest full Mobile CI result is tracked on
[PR #2417](https://github.com/ChronoAIProject/NyxID/pull/2417); the failed first
run above must not be mistaken for a passing result.
