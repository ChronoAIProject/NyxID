# App Store review checklist

Use this checklist with the metadata and review instructions in [the build 58 submission pack](../release/1.0.4-58/README.md). Source inspection and automated tests do not establish that an App Store Connect field or a production review account is ready.

## Prepare review access

- Supply a dedicated email/password account in App Store Connect's private **Sign-in Information** fields. Verify that it signs in on a fresh installation without a developer-controlled device or one-time code.
- Enable the chat and voice features described in the listing. Verify that the account has working provider access and sufficient usage credit for review.
- Supply a separate disposable account for deletion testing. Keep the primary review account available throughout review.
- Do not commit passwords, tokens, recovery codes, or review-account credentials to the repository or public release notes.
- Test sign-in, chat, voice, account settings, sign-out, and sign-in again on the selected TestFlight build. Apple, Google, and GitHub sign-in depend on their production OAuth configuration.

## Check account deletion

Deletion starts at **Settings → Delete Account**. The confirmation sheet discloses managed Machine removal, paused workspace disks, cleanup timing, and retained records. The request sends the current account-deletion policy header to `DELETE /users/me` only after explicit confirmation.

The app signs out after deletion is accepted. Cleanup and provider erasure may continue afterward. Do not describe queue acceptance as verified immediate erasure, and do not promise that signing in with the same external identity always creates a new account immediately.

The account-deletion tests exercise the actual screen and HTTP helper, including the confirmation disclosures, cancellation, and policy header. A destructive end-to-end check requires the disposable account.

## Match privacy disclosures to the current app

| Area | Current behavior to account for |
| --- | --- |
| Microphone and speech recognition | Live voice and dictation request access when the person uses those features. Audio, transcripts, and voice-session metadata are covered by the privacy policy. |
| Camera and photos | QR scanning, taking chat photos, and selecting attachments use their respective permissions. |
| Account and AI data | Account identifiers, conversations, attachments, approvals, security records, and connected-provider data must be represented accurately in App Store Connect. |
| Optional product analytics | Account permission defaults off. Account and device controls are separate. Analytics does not automatically capture screens, session replay, messages, or form contents. Essential operational records are separate. |
| Privacy manifest | `NSPrivacyTracking=false` and API-reason entries do not by themselves prove that no server-side data is collected. An empty collected-data array is not a substitute for App Store privacy labels. |
| Age | The published terms require users to be at least 18. Complete Apple's current age-rating questionnaire from the actual content and capabilities; do not infer an Apple rating from that terms clause. |

Use [the privacy policy source](../../frontend/public/legal/privacy.md) and [terms source](../../frontend/public/legal/terms.md) when checking declarations. Verify the public Privacy Policy and Support URLs from an unauthenticated browser before submission. Command-line requests to the candidate public domains returned HTTP 403 during preparation; public-browser accessibility remains to be checked.

## Check the selected binary and screenshots

- Select version **1.0.4**, build **58**, bundle **fun.chrono-ai.nyxid** in App Store Connect after processing completes.
- The native deployment target is iOS **15.1**. Confirm the processed binary's supported devices and OS version.
- Verify both embedded API domain sets remain reachable during review. See [domain migration](DOMAIN_MIGRATION.md).
- Verify real-device microphone input, audible speaker output, call end/restart, and permission-denial recovery. Simulator route inspection cannot establish physical-device loudness.
- Use actual app screenshots. Remove personal data, debug menus, and development error overlays. Build 58's source manifest identifies the binary's exact inputs; the PR includes a later merge from main.
- Provide the required iPhone display-size screenshots. This is a universal app, so provide a genuine 13-inch iPad screenshot as well. Do not stretch an iPhone capture to impersonate an iPad.
- Upload opaque PNG or JPEG files at dimensions accepted by [Apple's screenshot specification](https://developer.apple.com/help/app-store-connect/reference/screenshot-specifications).
- Review the existing encryption declaration against the distributed app. The current native configuration declares `ITSAppUsesNonExemptEncryption=false`.

## Check review notes and content controls

Completed assistant replies expose a Report control. Support is available under **Settings → Contact support**, using `contact@chrono-ai.fun`. Describe the actual enabled group or crew access model; do not imply unrestricted public chat.

Use [the build 58 review notes](../release/1.0.4-58/app-review-notes.txt). Keep credentials in App Store Connect's private fields. Complete all account and production-service checks before marking review access ready.

The listing text and local screenshots can be prepared without uploading metadata or submitting the app. Final approval depends on Apple's review.
