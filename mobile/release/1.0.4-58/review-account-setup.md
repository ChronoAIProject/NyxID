# Prepare App Review accounts

A live admin lookup on 12 November 2026 confirmed that the existing primary App Review account is active, email-verified, a regular user, and has MFA disabled. Its identifier and last-login status are recorded in the private Desktop package. Fresh password sign-in and model/voice access remain unverified, and a separate deletion-test account has not been identified. Use the steps below to complete review access.

## Prepare the primary demo account

1. Create or choose a dedicated NyxID email/password account under team control. Use non-personal demo data.
2. Verify the email address. Confirm that a fresh sign-in does not require approval on a developer's device or an unavailable authenticator.
3. Enable the assistant and voice features used in the description. Verify a usable model connection, voice source, and enough credit or provider access to complete review.
4. Add a harmless sample conversation. If demonstrating connected tools or crews, use isolated demo resources and membership that remain available throughout review.
5. Sign in on a fresh installation of the processed build 58. Send a chat message, start and end a voice call, change a voice setting, open Language settings, and check Account & security.
6. Sign out and sign in again using the exact credentials supplied to Apple.
7. Enter the email and password in App Store Connect's private **Sign-in Information** fields. Keep the account active until review finishes.

Do not supply a personal administrator account. Do not bypass account-security controls on other accounts to simplify review.

## Prepare the deletion-test account

1. Create a separate disposable email/password account and verify its email.
2. Verify that it reaches **Settings → Delete Account** on build 58.
3. Test deletion with a separate sacrificial account if the reviewer account must remain available. Confirm acceptance, sign-out, and cleanup status using the existing operational process.
4. Supply the disposable account through App Store Connect's private review information or a private review attachment. Explain which account is safe to delete.

Deleting an account can revoke connected access and remove managed Machine data. Use only isolated test resources. Queue acceptance is not proof that every provider has completed erasure.

## Record readiness privately

Record the two account identifiers, credential location, last successful fresh sign-in, enabled features, available provider access, and the responsible operator in the team's approved private store. Do not put that record into this repository, the screenshot package, a public PR, or chat.

Before submission, replace the pending state in the team's private release record with the verified account results. The public release docs remain free of credentials.
