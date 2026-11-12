# Build 58 testing notes and review accounts

Paste [testflight-testing-notes.txt](testflight-testing-notes.txt) into TestFlight's **What to Test** field. Paste [app-review-notes.txt](app-review-notes.txt) into App Store Connect's **App Review Notes** field. These are separate fields with separate audiences.

The fresh testing notes cover physical iPhone speaker output, headphones, microphone and speaker controls, thin equalizer bars, voice-panel layout, ending and restarting calls, backgrounding, dictation, supported language settings, ordinary chat, and account deletion.

## Account verification

The private Desktop package records the identified App Review account and its live account-status check. No passwords are included in the repository or the submission ZIP. Enter the existing password in App Store Connect's private sign-in fields.

An active, verified account record does not establish that its password still works or that it has usable model, voice, or billing access. Complete a fresh sign-in and feature check on build 58 before submission. Keep a separate disposable account available for deletion testing; do not delete the primary review account.

Follow [review-account-setup.md](review-account-setup.md) for the remaining account checks. The testing instructions describe expected behavior; they are not a claim that every manual scenario has passed.
