---
title: Privacy Policy
effective_date: 2026-05-11
---

# Privacy Policy

**Effective date: 2026-05-11**

## 1. Introduction

NyxID ("we", "our", "the Service") is an identity and access management platform with a web dashboard and a mobile authenticator app. This Privacy Policy explains how we collect, use, store, and protect personal information across both surfaces.

By using NyxID, you agree to the collection and use of information in accordance with this policy.

## 2. Information We Collect

We collect the minimum data necessary to provide secure authentication and approval services.

**Account information**

- Email address (for registration and communication)
- Display name (optional, user-provided)
- Password (stored as a salted Argon2id hash, never in plaintext) when using email/password sign-in

**Authentication data**

- Session tokens and refresh tokens (encrypted at rest server-side, stored in OS-protected secure storage on mobile clients — iOS Keychain via Expo SecureStore, Android EncryptedSharedPreferences)
- Multi-factor authentication (MFA) secrets (encrypted at rest)
- OAuth provider tokens when you connect third-party accounts (Google, GitHub, Apple)

**Device information (mobile)**

- Push notification token (FCM on Android, APNs on iOS), device platform, and app identifier — used to deliver approval challenges

**Usage data**

- Approval decisions (approve/deny/revoke), timestamps, and idempotency keys for security audit trails

**Technical data**

- IP address and approximate geolocation (for security and audit)
- User-agent string and device type
- Timestamps of login events and API requests

These are received by our servers as part of normal HTTPS requests. The applications do not collect, store, or share this technical metadata beyond what the server needs for security and audit.

## 3. How We Use Your Information

- Authenticate your identity and manage your sessions
- Provide single sign-on (SSO) to connected services
- Deliver push notifications for time-sensitive approval challenges (mobile)
- Process your approval, denial, and revocation decisions
- Register and manage your device for push delivery
- Enforce security policies (rate limiting, anomaly detection)
- Send transactional emails (verification, password reset)
- Maintain security audit logs for compliance and abuse prevention

## 4. Data Storage & Security

All data is stored in encrypted MongoDB databases. Sensitive server-side fields (OAuth tokens, MFA secrets, API credentials) are encrypted with AES-256 at the application layer. Passwords use Argon2id with per-user salts.

All communications between clients and our servers use TLS 1.2+. JWT tokens are signed with RSA-256 keys rotated periodically.

On mobile, authentication tokens use the OS-provided secure storage (iOS Keychain via Expo SecureStore, Android EncryptedSharedPreferences).

## 5. Data Sharing and Sub-processors

We do **not** sell, rent, or trade your personal information. We share data only in the following cases:

- **With your consent:** when you authorize a third-party service via OAuth/OIDC
- **Legal obligations:** when required by law, regulation, or valid legal process
- **Security:** to prevent fraud or protect the rights and safety of our users

**Sub-processors and service providers**

NyxID engages the following sub-processors to deliver parts of the Service. We have data processing agreements in place with each that require them to apply appropriate security measures and to process personal data only on our instructions:

- **PostHog Inc.** (United States) — opt-in product analytics. Data category: anonymous usage events keyed to your NyxID account UUID. Processing region: US.
- **Firebase Cloud Messaging** by Google LLC (United States) — Android push-notification delivery, where push is enabled. Data category: device push token.
- **Apple Push Notification service** by Apple Inc. (United States) — iOS push-notification delivery, where push is enabled. Data category: device push token.

NyxID also engages cloud infrastructure providers (for hosting and database services) and a transactional email service provider (for verification emails, password resets, and security notices). A current copy of our service-provider register and executed data processing agreements is available on request to [contact@chrono-ai.fun](mailto:contact@chrono-ai.fun). This list may be updated as the Service evolves; material changes will be reflected in this Privacy Policy with a revised effective date.

> Note: third-party platforms you connect yourself — including messaging-platform integrations (Telegram, Lark / Feishu, Discord, OpenClaw), Channel Bots you register, OAuth providers you use for social login (Google, GitHub, Apple), and any third-party APIs you call via the Credential Proxy — are *not* sub-processors of NyxID. They are independent services governed by their own terms and privacy policies.

## 6. Data Retention

Account data is retained while your account is active. When you delete your account (available in Account Settings or the mobile app), all personal data and server-side records are permanently removed within 30 days.

Security audit logs may be retained for up to 90 days for security compliance before automatic purging. Push tokens are removed from our server when you sign out or delete your account.

If you sign in again with the same provider (Apple, Google, GitHub) after deletion, a new account will be created; your previous data will not be restored.

## 7. Your Rights

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

## 8. Cookies, Local Storage, and Telemetry

**Web:** NyxID uses HTTP-only secure cookies for session management and browser local storage to persist authentication state.

**Mobile:** The app stores authentication tokens and push token references using Expo SecureStore and platform-protected local storage. The app does not use tracking cookies, advertising identifiers, or cross-app tracking.

**Telemetry (opt-in, both surfaces).** When you explicitly allow it via the consent banner on web or the Settings toggle on mobile, NyxID collects anonymous usage events (pageviews, clicks, screen visits, uncaught errors) through a third-party analytics provider (PostHog, US region). No credentials, form content, tokens, or the body of any request you make are ever captured. Sensitive URL segments (reset tokens, OAuth callback codes, approval IDs) are dropped at the egress layer before any event leaves your browser or device.

**EU→US transfer basis.** PostHog Inc. is established in the United States, which does not benefit from an EU Commission adequacy decision at the time of this Privacy Policy's effective date. Where you are located in the European Economic Area, the United Kingdom, or another jurisdiction subject to cross-border transfer restrictions, your opt-in telemetry data is transferred to PostHog Inc. under the Standard Contractual Clauses (Module 2: Controller-to-Processor) approved by the European Commission in Implementing Decision (EU) 2021/914 of 4 June 2021, supplemented by encryption in transit (TLS 1.2 or higher), encryption at rest, scoped access controls, and the egress-scrubbing safeguards described above. A copy of the executed Standard Contractual Clauses is available on request to [contact@chrono-ai.fun](mailto:contact@chrono-ai.fun).

Events are keyed to your NyxID account UUID after you sign in, allowing us to understand product usage in aggregate without requiring your name or email. Raw events are retained for 90 days; aggregated metrics may be retained longer. If you delete your NyxID account, the backend enqueues a matching delete request to the analytics provider so your event history is removed.

You can change your telemetry choice at any time from the Settings page. We honor the browser Do-Not-Track signal.

**Per-surface scope.** Your telemetry choice is stored on the surface you set it on and does not sync between web dashboard, mobile app, and CLI. Each surface manages its own telemetry setting. The CLI uses `nyxid telemetry enable|disable` or the `DO_NOT_TRACK=1` environment variable. The mobile app exposes a matching toggle in its Settings screen.

Self-hosters of NyxID can run with analytics disabled by default, or point at their own analytics project.

## 9. Children's Privacy

NyxID is not intended for use by children under 16 (or the applicable minimum age in your jurisdiction). We do not knowingly collect personal information from children. If you believe a child has provided data to us, please contact us for immediate removal.

## 10. Changes to This Policy

We may update this Privacy Policy from time to time to reflect changes in our practices or legal requirements. Material changes will be indicated by a new effective date at the top of this document. Continued use of the Service after changes constitutes acceptance of the revised policy.

## 11. Contact Us

If you have any questions about this Privacy Policy or your data, please contact us at: [contact@chrono-ai.fun](mailto:contact@chrono-ai.fun)
