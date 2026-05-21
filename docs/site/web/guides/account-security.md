---
title: Account & security (MFA, sessions)
description: Enable multi-factor authentication, manage active sessions, change your password, and secure your NyxID account.
---

This page covers everything in **Settings** that affects account security: multi-factor authentication (MFA), session management, and password changes.

## Multi-factor authentication (MFA)

NyxID supports TOTP-based MFA (Time-based One-Time Password) using any authenticator app — Google Authenticator, Authy, 1Password, Bitwarden, etc.

### Enable MFA

1. Go to **Settings → Security**.
2. Click **Set up two-factor authentication**.
3. NyxID generates a QR code and a backup secret.
4. Open your authenticator app, scan the QR code (or enter the secret manually).
5. Enter the 6-digit code from your app to confirm the setup.
6. Save the **backup codes** shown on the confirmation screen. These are single-use emergency codes for when your authenticator is unavailable.

:::warning
Store your backup codes in a secure location (password manager, printed and locked away). If you lose access to your authenticator and have no backup codes, account recovery requires manual admin intervention.
:::

### Sign in with MFA enabled

After entering your email and password on the login page, NyxID prompts for a 6-digit TOTP code. Enter the current code from your authenticator app.

### Disable MFA

1. Go to **Settings → Security**.
2. Click **Disable two-factor authentication**.
3. Confirm with your current TOTP code or a backup code.

:::note
MFA setup is idempotent on the backend — calling setup again while a factor is unverified replaces the pending factor without error. A verified factor must be explicitly disabled before setup can be repeated.
:::

## Password management

### Change your password

1. Go to **Settings → Security**.
2. Click **Change password**.
3. Enter your current password, then your new password twice.
4. Click **Save**.

Password changes do not invalidate existing sessions.

### Reset a forgotten password

On the login page, click **Forgot password**. Enter your email address. NyxID sends a single-use reset link valid for 1 hour. Click the link, set a new password, and sign in.

:::note
If your account uses social sign-in (Google / GitHub / Apple) and you never set a password, the forgot-password flow creates a password credential in addition to the existing social login.
:::

## Sessions

Every sign-in creates a session. Sessions expire after 7 days of inactivity (refresh token TTL). Active sessions stay alive with automatic token refresh on every page load.

### View active sessions

1. Go to **Settings → Sessions**.
2. The list shows all active sessions with their last-active timestamp, approximate location, and device/browser identifier.

### Revoke a session

Click **Revoke** on any session in the list to sign it out immediately. The access token from that session becomes invalid; any tool using it will get `401 Unauthorized` on the next API call.

To sign out everywhere at once, click **Revoke all sessions**.

:::tip
If you suspect unauthorized access, revoke all sessions, change your password, and enable MFA if it is not already on.
:::

## Account email

Your account email is used for:

- Sign-in (email + password path)
- Password reset links
- Approval notifications (if enabled)

To change it, go to **Settings → Profile**, update the email field, and confirm the verification email sent to the new address.

## API keys and Agent Keys

Agent Keys (`nyx_...`) are not session credentials — revoking a session does not revoke Agent Keys. Manage them separately from **AI Services → Agent Keys**.

If you suspect an Agent Key has been compromised:

1. Go to **AI Services → Agent Keys**.
2. Find the key and click **Rotate** to immediately invalidate the old key and generate a new one.
3. Update the key in all tools that use it.

See [Manage keys & credentials](/docs/web/guides/manage-keys) for full key management procedures.

## Encryption

External service credentials stored by NyxID are protected with AES-256 envelope encryption. The encryption key is never logged or returned in API responses. For the technical details, see [Encryption](/docs/shared/concepts/encryption).
