---
title: Privacy Policy
effective_date: 2026-10-19
---

# Privacy Policy

**Effective date: 2026-10-19**

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

- Session token hashes and refresh-token JTI/revocation metadata on the server; authentication tokens in platform-protected Expo SecureStore on mobile. Some short-lived delivery protocols temporarily retain encrypted credentials for one-time delivery.
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

Servers receive technical metadata with HTTPS requests for security and audit. Optional product analytics is a separate data flow described in section 8.

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

NyxID stores account data in MongoDB. Dedicated OAuth-token, MFA-secret and API-credential fields use application-layer AES-256 encryption. Passwords use Argon2id with per-user salts; session tokens are hashed rather than reversibly encrypted. Newly saved configured default request-header values are encrypted at rest. Values saved before this protection was enabled are protected when the operator migrates them. Database/storage encryption and backup protections depend on the deployment.

Production deployments should use HTTPS; the deployment operator configures this protection. Tokens are digitally signed. NyxID supports changing signing keys while continuing to verify tokens signed with retained keys. The keys used to check existing audit and billing records must be preserved when signing or encryption keys change. The deployment operator manages key rotation and retirement. Ask the deployment operator about its actual key-rotation schedule.

On mobile, authentication tokens use the OS-provided secure storage (iOS Keychain via Expo SecureStore, Android EncryptedSharedPreferences).

## 5. Data Sharing and Sub-processors

We do **not** sell, rent, or trade your personal information. We share data only in the following cases:

- **With your consent:** when you authorize a third-party service via OAuth/OIDC
- **Legal obligations:** when required by law, regulation, or valid legal process
- **Security:** to prevent fraud or protect the rights and safety of our users

**Service and recipient inventory**

Depending on enabled features and operator configuration, data can reach:

- PostHog: product analytics events, pseudonymous account UUID and scrubbed event metadata. The built-in host is the US ingestion endpoint; self-hosters can override it.
- Firebase Cloud Messaging and Apple Push Notification service: device push tokens and notification payloads when push is enabled.
- Hosting, database, storage and transactional email services: stored account/application data, email addresses and transactional messages as configured.
- Lago and payment integrations such as Stripe: billing identities, usage, checkout and payment references when billing is enabled. Lago may be self-hosted; the implementation alone does not establish an external processor.
- NyxAgent (`llm-nyx`) or the configured Aevatar broker: conversation content and tool context for the selected assistant engine.
- Configured model/voice services, including OpenAI and xAI platform-key paths: prompts, audio and related request content needed for the requested feature.
- Connected OAuth providers, messaging platforms/channel bots, credential-proxy destinations and Oracle workers: the account identifiers, messages or request content needed for the connection or operation you initiate.

The providers used depend on your deployment and enabled features. User-connected third parties have their own policies. For the applicable service-provider register, processing locations, retention and cross-border transfer arrangements, ask [contact@chrono-ai.fun](mailto:contact@chrono-ai.fun) for the applicable deployment information.

## 6. Data Retention

Account data is retained while the account is active. When an account-deletion request is accepted, access ends immediately and removal continues in the background. If something must be resolved before deletion can begin, the request is not accepted and access continues. The deletion status shows what is pending and which steps need help from the service operator. Some records and connected-service authorizations may remain until that work is finished; deletion does not yet cover every stored record or confirm that every connected service has revoked its authorization. Contact us for deletion status; removal of all copies and external records within 30 days is not guaranteed.

Audit and billing records, and some records of Machine actions and access restrictions, are retained for integrity and safety. Audit records are not automatically deleted, including after 90 days. These records remain subject to access controls and the applicable retention arrangements. Organization-owned resources remain owned by the organization when a person's account is deleted.

NyxID tracks analytics deletion requests and any further steps needed from the provider or service operator. Accepting a request does not prove that the provider has completed deletion. Backup retention, legally required evidence retention, other external-provider deletion and actual fulfillment timelines require operator confirmation. Contact us for the status and applicable retention details of a deletion request.

## 7. Your Rights

You have the right to:

- Access and export your personal data
- Correct inaccurate information in your profile
- Request account deletion, subject to the retention and implementation limits in section 6
- Revoke consent for third-party service connections at any time
- Revoke any active approval grants
- Disconnect third-party sign-in providers
- Disable push notifications through your device settings
- Opt out of non-essential communications

Available settings cover profile changes, account deletion and connection/approval controls. For access/export and other requests, contact [contact@chrono-ai.fun](mailto:contact@chrono-ai.fun). Personal-data exports are requested through that contact channel; they are not currently available as a self-service download.

## 8. Cookies, Local Storage, and Telemetry

**Web:** NyxID uses HTTP-only secure cookies for session management and browser local storage to persist authentication state.

**Mobile:** The app stores authentication tokens and push token references using Expo SecureStore and platform-protected local storage. The app does not use tracking cookies, advertising identifiers, or cross-app tracking.

**Optional product analytics.** Analytics requires account permission, which defaults off, and an account-bound choice on each device. Web and mobile Settings provide separate account and device controls. The CLI provides `nyxid telemetry enable|disable|status` for the selected account/device and `account-enable|account-disable` for account permission. Turning account permission off applies to future analytics dispatch across devices. Turning a device choice off stops that device's optional events and does not change another device's choice. Browser Do-Not-Track/Global Privacy Control and CLI `DO_NOT_TRACK` or `NYXID_TELEMETRY=off` further restrict collection.

Client events pass through authenticated NyxID checks. Pending events lose permission after withdrawal, regrant or an account switch. Events already sent cannot be recalled. Pre-login analytics, session replay, automatic screen/DOM capture and raw exception collection are disabled. Security audit and essential operational records are separate from product analytics.

Events can be keyed to the NyxID account UUID, which is a pseudonymous personal identifier. Explicit event categories and bounded metadata exclude raw credentials, request bodies, messages, form contents and URLs. Raw-event and aggregate retention depend on the analytics project settings; ask the deployment operator for the applicable period. Account deletion requests provider erasure; queue acceptance alone does not guarantee provider completion. Contact us for confirmed status.

Self-hosters of NyxID can run with analytics disabled by default, or point at their own analytics project.

## 9. Children's Privacy

NyxID is not intended for use by children under 16 (or the applicable minimum age in your jurisdiction). We do not knowingly collect personal information from children. If you believe a child has provided data to us, please contact us for immediate removal.

## 10. Changes to This Policy

We may update this Privacy Policy from time to time to reflect changes in our practices or legal requirements. Material changes will be indicated by a new effective date at the top of this document. Continued use of the Service after changes constitutes acceptance of the revised policy.

## 11. Contact Us

If you have any questions about this Privacy Policy or your data, please contact us at: [contact@chrono-ai.fun](mailto:contact@chrono-ai.fun)
