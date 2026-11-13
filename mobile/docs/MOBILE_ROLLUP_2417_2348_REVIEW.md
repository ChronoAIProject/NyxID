# Mobile rollup review — PRs #2417 and #2348

Reviewed on 2026-11-13. Branch: `rollup/mobile-2417-2348`.

The rollup combines both PRs on main at
`537fa4bf5c518ac0c9a0878e29a2d370093aea15`. It includes #2417 at
`84cc6e3c7a0f197ca376aa4696f151438412c590` and the updated #2348 at
`0a4fdaed89966fb07729c56800f3d8559765e590`. Both source heads are ancestors
of the rollup. All net changes relative to that main snapshot are under
`mobile/`; backend, web frontend, CLI, SDK, shared code and CI definitions have
no net changes. No dependency versions or lockfiles change relative to main.

The review found three Firebase compatibility issues and corrected them in
`b51178755`. No confirmed blockers remain within the reviewed and tested scope.
Codex performed integration and validation, and Claude Opus 5.5 performed an
independent review of the combined diff and a follow-up review of the fixes.

## Compatibility fixes

| Finding                                              | Trigger and prior result                                                                                                                                                                    | Rollup behavior                                                                                                                                                                                                                          |
| ---------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Android validation blocked iOS config loading        | #2348 unconditionally validated the Firebase file in `app.config.ts`. An iOS build with a missing Android JSON or a custom Android package failed while main could resolve the same config. | Validation runs in an Android manifest mod. iOS config loading and iOS mods do not require the Android file. The Android release script also validates before prebuild; Android generation rejects missing or mismatched explicit files. |
| Existing development FCM was disabled                | An existing dev profile omitted the newly introduced Firebase variable while its package matched the checked-in `google-services.json`. Main supported FCM; #2348 omitted it.               | Omitted dev paths reuse the existing JSON only after exact package validation. The probe never throws for a missing, malformed or mismatched default. An explicit empty dev path still disables FCM.                                     |
| Empty production Firebase paths removed push support | An empty or whitespace `PROD_GOOGLE_SERVICES_FILE` omitted Firebase from production Android builds.                                                                                         | Empty or omitted production paths retain main's `./google-services.json` default. Android release builds validate and print the resolved file before generating native code.                                                             |

Seven new tests exercise the actual Expo mod chain for both platforms and both
profiles: iOS tolerates missing or mismatched Android files, Android rejects
them, matching clients pass, and explicit development FCM opt-out remains
available. Three additional profile regressions cover the legacy matching dev
default, explicit dev opt-out and empty production default.

## Integration and behavior review

The original #2348 conflict resolution preserves both main's `test:jest` command
and the Android build-script tests. The settings harness preserves main's
NyxBot notification hooks and the PR's language support. The author's later
#2348 merge of the same main snapshot was also included; its fixture ordering
was retained. Both complete Node and Jest suites cover the combined branch.

The review covered history deadlines, stale response rejection, pagination,
authoritative refreshes, account and navigation cancellation, shared voice
startup, late owned-session cleanup, microphone permissions, input release,
keyboard handoff, legal navigation, offline plugin behavior and interactions
with main's notification features.

The source PRs intentionally change mobile presentation and developer workflows:

- Android, iPhone and iPad share the compact voice bar, main-chat transcripts
  and voice preferences. Saved preferences are preserved; xAI stays on push to
  talk. Account settings now use the automatic default that calls already used
  on main.
- History reads and shared token refreshes receive a 10-second deadline,
  including response-body reads. Current history failures expose recovery;
  cancelled or superseded requests cannot update the selected chat.
- `pnpm android` cleanly regenerates the ignored Android native directory before
  installing the dev profile. A distinct dev package needs its matching Firebase
  JSON to test FCM; a mismatched production JSON is never selected for it.
- Android declares `MODIFY_AUDIO_SETTINGS` for WebView microphone support.
  Signing passwords reach Gradle through its child environment.

No server endpoint, request or response schema, database migration, credential
format or shared package contract changes in the rollup.

## Validation

Run from `mobile/` with Node 22.21.1 and pnpm 10.17.1, matching CI's Node major
and pnpm version:

| Check                                                                   | Result                                       |
| ----------------------------------------------------------------------- | -------------------------------------------- |
| `pnpm install --frozen-lockfile`                                        | Passed; lockfile unchanged                   |
| `pnpm test`                                                             | 2,496 passed; 0 failed, cancelled or skipped |
| `pnpm test:jest --runInBand`                                            | 49 passed across 5 suites                    |
| `pnpm typecheck`                                                        | Passed                                       |
| `pnpm check:native-sync`                                                | Passed                                       |
| Prettier 3.8.1 on the compatibility-fix JavaScript and TypeScript files | Passed                                       |
| `git diff --check origin/main`                                          | Passed                                       |
| Diff scope and source-head ancestry                                     | Mobile-only; both PR heads included          |

Opus independently ran the profile, Firebase mod-chain and signing-command
tests: 35 passed. These overlap the full suite and are not additional tests.
The final verification also retains main's five Jest suites for notifications.

## Remaining validation limits

The 10-second refresh deadline can discard a response after the server has
rotated the token. If the next refresh occurs after the backend's 120-second
retry grace, reauthentication can be required. Transport failures already have
this failure mode; the client deadline adds another way to encounter it. This
is a documented residual risk, rather than a new backend contract or an
unbounded retry. No backend changes were made to address it.

This session did not run Expo prebuild against a real native project, Gradle or
Xcode release builds, live provider calls, or device/simulator acceptance. The
source PRs contain historical native evidence and incomplete QA matrices; those
records retain their original boundaries. Automated checks and code review do
not establish complete native or release acceptance.
