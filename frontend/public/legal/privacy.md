---
title: Privacy Policy
effective_date: 2026-05-12
---

# Privacy Policy

**Effective date: 2026-05-12**

## 1. Introduction

NyxID ("we", "our", "the Service") is an identity and access management platform with a web dashboard and a mobile authenticator app. This Privacy Policy explains what data we collect, how we use it, and how we protect it across both surfaces.

By using NyxID, you agree to the practices described in this policy.

## 2. Information We Collect

We collect the minimum data necessary to provide secure authentication and approval services.

- **Account identity:** email address (or Apple private relay address), display name, and user ID from your chosen sign-in provider (Google, GitHub, Apple, or email + password)
- **Authentication credentials:** password (stored as a salted Argon2id hash, never in plaintext) when using email/password sign-in
- **Authentication tokens:** access tokens, refresh tokens, and MFA secrets — stored encrypted at rest server-side, and in OS-protected secure storage (iOS Keychain via Expo SecureStore, Android EncryptedSharedPreferences) on mobile clients
- **Device information (mobile):** push notification token (FCM on Android, APNs on iOS), device platform, and app identifier — used to deliver approval challenges
- **Usage data:** approval decisions (approve/deny/revoke), timestamps, and idempotency keys for security audit trails
- **Server-side only:** our servers receive IP address and request headers (e.g. user-agent) as part of normal HTTPS requests, used for security and rate limiting

## 3. How We Use Your Information

- Authenticate your identity and maintain your session
- Provide single sign-on (SSO) to connected services
- Deliver push notifications for time-sensitive approval challenges (mobile)
- Process your approval, denial, and revocation decisions
- Register and manage your device for push delivery
- Enforce security policies (rate limiting, anomaly detection)
- Send transactional emails (verification, password reset)
- Maintain security audit logs for compliance and abuse prevention
- Refresh expired sessions automatically to minimize re-authentication

## 4. Sign in with Apple

If you sign in with Apple, we receive a verified identity token and your email address (or Apple's private relay address if you choose "Hide My Email"). We do not receive your Apple ID password or any data beyond what Apple provides through its identity service. You may manage your Sign in with Apple connections in your Apple ID settings.

Apple's terms of service and privacy policy also apply to your use of Sign in with Apple.

## 5. Push Notifications (Mobile)

The mobile app uses the push notification services supported by your device platform (FCM on Android, APNs on iOS) to deliver approval challenges. Your device push token is registered with our server upon login and removed upon sign-out or account deletion.

Push notification payloads contain only minimal identifiers (challenge ID). Sensitive details are fetched separately over an authenticated API connection.

## 6. Data Storage and Security

All data is stored in encrypted MongoDB databases. Sensitive server-side fields (OAuth tokens, MFA secrets, API credentials) are encrypted with AES-256 at the application layer.

All communications between clients and our servers use TLS 1.2+. JWT tokens are signed with RSA-256 keys rotated periodically. Access tokens have scoped expiry. Refresh tokens are rotated and can be revoked at any time.

On mobile, authentication tokens use the OS-provided secure storage (iOS Keychain via Expo SecureStore, Android EncryptedSharedPreferences).

## 7. Data Sharing

We do **not** sell, rent, or trade your personal data. Data may be shared only in the following circumstances:

- **With your consent:** when you authorize a third-party service via OAuth/OIDC, or sign in via a third-party identity provider (Google, GitHub, Apple)
- **Legal obligations:** when required by law, regulation, or valid legal process
- **Security:** to prevent fraud or protect the rights and safety of our users

## 8. Data Retention

Account data is retained while your account is active. When you delete your account (available in Account Settings or the mobile app), all personal data and server-side records are permanently removed within 30 days.

Security audit logs may be retained for up to 90 days for security compliance before automatic purging. Push tokens are removed from our server when you sign out or delete your account.

If you sign in again with the same provider (Apple, Google, GitHub) after deletion, a new account will be created; your previous data will not be restored.

## 9. Your Rights

You have the right to:

- Access and export your personal data
- Correct inaccurate information in your profile
- Delete your account and all server-side data permanently
- Revoke consent for third-party service connections at any time
- Revoke any active approval grants
- Disconnect third-party sign-in providers
- Disable push notifications through your device settings
- Opt out of non-essential communications

These actions are available through the Settings page in your NyxID dashboard or the Account Settings screen in the mobile app, or by contacting us directly.

## 10. Cookies, Local Storage, and Telemetry

**Web:** NyxID uses HTTP-only secure cookies for session management and browser local storage to persist authentication state.

**Mobile:** The app stores authentication tokens and push token references using Expo SecureStore and platform-protected local storage. The app does not use tracking cookies, advertising identifiers, or cross-app tracking.

**Telemetry (opt-in, both surfaces).** When you explicitly allow it via the consent banner on web or the Settings toggle on mobile, NyxID collects anonymous usage events (pageviews, clicks, screen visits, uncaught errors) through a third-party analytics provider (PostHog, US region). No credentials, form content, tokens, or the body of any request you make are ever captured. Sensitive URL segments (reset tokens, OAuth callback codes, approval IDs) are dropped at the egress layer before any event leaves your browser or device.

Events are keyed to your NyxID account UUID after you sign in, allowing us to understand product usage in aggregate without requiring your name or email. Raw events are retained for 90 days; aggregated metrics may be retained longer. If you delete your NyxID account, the backend enqueues a matching delete request to the analytics provider so your event history is removed.

**Per-surface scope.** Your telemetry choice is stored on the surface you set it on and does not sync between web dashboard, mobile app, and CLI. Each surface manages its own telemetry setting. The CLI uses `nyxid telemetry enable|disable` or the `DO_NOT_TRACK=1` environment variable. The mobile app exposes a matching toggle in its Settings screen. The web honors the browser Do-Not-Track signal.

Self-hosters of NyxID can run with analytics disabled by default, or point at their own analytics project.

## 11. Children's Privacy

NyxID is not intended for use by children under 16 (or the applicable minimum age in your jurisdiction). We do not knowingly collect personal information from children. If you believe a child has provided data to us, please contact us for immediate removal.

## 12. Policy Updates

We may update this Privacy Policy from time to time to reflect changes in our practices or legal requirements. Material changes will be indicated by a new effective date at the top of this document. Continued use of the Service after changes constitutes acceptance of the revised policy.

## 13. Contact

For privacy inquiries, contact us at: **privacy@chrono-ai.fun**
