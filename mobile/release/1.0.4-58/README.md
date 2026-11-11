# Submit NyxID 1.0.4, build 58

This pack contains replacement English App Store copy, release notes, an engineering changelog, and App Review instructions for the existing NyxID listing. The Desktop copy also contains actual simulator screenshots and binary-verification records.

## Select the existing app and build

| Field | Value |
| --- | --- |
| App | NyxID |
| App Store ID | `6759768423` |
| Bundle ID | `fun.chrono-ai.nyxid` |
| Version | `1.0.4` |
| Build | `58` |
| Signed IPA | `~/Desktop/NyxID-1.0.4-58-ios-rollup.ipa` |
| SHA-256 | `aa4f107b19e21b6d187b15d19b65b13e2fffff7a2491dcdcd96d5523c824ba80` |
| Public listing | <https://apps.apple.com/us/app/nyxid/id6759768423> |

Build 58 has been exported and its signature verified. Upload and processing status have not been verified in App Store Connect. Select the processed build 58 for version 1.0.4; do not replace or reuse an already uploaded build number.

The IPA contains the accepted voice and language fixes. The PR also includes a later merge from `main`, so its final commit is newer than this IPA. The Desktop `verification/source-manifest.json` records the binary's exact source inputs. The subsequent account-deletion test-loader change affects test infrastructure only.

## Paste the English fields

Open the **English (U.S.)** localization. Each text file contains the field value without Markdown.

| App Store Connect field | File | Characters | Limit |
| --- | --- | ---: | ---: |
| Name | [name.txt](en-US/name.txt) | 5 | 30 |
| Subtitle | [subtitle.txt](en-US/subtitle.txt) | 28 | 30 |
| Promotional text | [promotional-text.txt](en-US/promotional-text.txt) | 146 | 170 |
| Description | [description.txt](en-US/description.txt) | 1,758 | 4,000 |
| Keywords | [keywords.txt](en-US/keywords.txt) | 94 | 100 |
| What's New in This Version | [whats-new.txt](en-US/whats-new.txt) | 638 | 4,000 |
| App Review notes | [app-review-notes.txt](app-review-notes.txt) | See file | 4,000 |

Run `python3 mobile/release/1.0.4-58/validate_metadata.py` from the repository root to verify the field lengths after edits. The trailing file newline is excluded from the pasted field value.

The copy describes the current AI workspace. Language support is limited to supported native settings and selected controls; it does not claim a fully translated app. Playable voice samples are not advertised. The release does not add notification settings work.

## Upload dark artwork and screenshots

The current dark-only Desktop package is `~/Desktop/NyxID-58-Submission-DARK`. It contains the submission text, review notes, changelog, two primary landscape assets, and dark iPhone and iPad screenshots.

The **Product Page Information** section has two artwork tabs. Both primary files are **5244 × 2950**, a size explicitly accepted by both uploaders in the supplied App Store Connect screenshots.

| Tab | File in the dark package | Supported sizes shown in App Store Connect |
| --- | --- | --- |
| Header | `01-HEADER-DARK-5244x2950.png` | 5244 × 2950, 3840 × 2560, or 1920 × 1280 |
| Search Results | `02-SEARCH-RESULTS-DARK-5244x2950.png` | 5244 × 2950, 3840 × 2560, or 1920 × 1280 |

Upload one image in each tab. The two files use the same dark artwork with different names to identify the destination. Their dimensions and absence of transparency are verified locally. Server upload acceptance has not been exercised from this session.

If App Store Connect also requests device screenshots, use `app-screenshots/iphone/` for the three 1206 × 2622 iPhone images and `app-screenshots/ipad/` for the genuine 2064 × 2752 iPad capture. The portrait files belong in the corresponding device screenshot slots.

The dark images use actual app captures. The chat contains a real drafting response, and the captured voice call reached Listening and was ended afterward. `source-captures/` preserves the originals. The simulator uses the build 58 Debug native app with Metro from the PR branch, including its final main merge; the IPA's source manifest remains authoritative for the release binary.

Read `READ-ME-FIRST.txt` in the dark package for the field mapping. `verification/image-manifest.json` records the upload image dimensions, sizes and hashes. Apple's device screenshot dimensions are listed in [Screenshot specifications](https://developer.apple.com/help/app-store-connect/reference/screenshot-specifications).

## Complete private review access

Use [review-account-setup.md](review-account-setup.md) to prepare the primary demo account and a disposable deletion-test account. Enter credentials in App Store Connect's private **Sign-in Information** fields. The pack does not contain credentials, and account provisioning or successful reviewer sign-in has not been verified.

Paste [app-review-notes.txt](app-review-notes.txt) into **Notes** after the accounts and feature access are ready. Put the App Review contact person's current name, phone, and email into the corresponding private fields; those details are not inferred from the public support email.

Use [resubmission-message.txt](resubmission-message.txt) as a short covering message if replying to App Review. It describes this release without assuming a rejection reason. Add a response to any specific reviewer question before sending it.

## Finish the remaining App Store fields

- Keep the app's existing category and rights settings unless the owner changes them.
- Confirm the **Support URL** and **Privacy Policy URL** open in an unauthenticated browser. The app has Terms and Privacy screens and publishes `contact@chrono-ai.fun`, but command-line requests to the candidate public domains returned HTTP 403 during preparation. Do not treat those URLs as verified.
- Check App Privacy against account data, chat content, audio, attachments, security records, connected providers, and optional analytics. The current privacy manifest alone does not answer the server-side data questions.
- Complete Apple's current age-rating questionnaire from the actual app. NyxID's terms require users to be at least 18; that is separate from Apple's assigned rating.
- Confirm the processed build's encryption declaration. The native configuration currently sets `ITSAppUsesNonExemptEncryption=false`.
- Choose **Manually release this version** if the owner wants control after approval.
- Complete the [App Store review checklist](../../docs/APP_STORE_REVIEW_CHECKLIST.md), including the final TestFlight check on a physical iPhone.

No metadata upload, App Review submission, release action, or PR merge is performed by preparing this pack.

The detailed engineering changes are in [CHANGELOG.md](CHANGELOG.md). The public release notes are in [whats-new.txt](en-US/whats-new.txt).
