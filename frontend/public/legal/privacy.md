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

Production deployments should use HTTPS; the deployment operator configures this protection. Tokens use RS256 digital signatures. NyxID supports changing signing keys with an overlap period during which tokens signed with retained keys can still be verified. It also supports changing encryption keys in stages and migrating stored encrypted data. Keys must be kept while they are needed to read stored data or verify existing audit and billing records. The service operator decides the key-rotation schedule and when old keys can be retired.

On mobile, authentication tokens use the OS-provided secure storage (iOS Keychain via Expo SecureStore, Android EncryptedSharedPreferences).

## 5. Data Sharing and Sub-processors

We do **not** sell, rent, or trade your personal information. We share data only in the following cases:

- **With your consent:** when you authorize a third-party service via OAuth/OIDC
- **Legal obligations:** when required by law, regulation, or valid legal process
- **Security:** to prevent fraud or protect the rights and safety of our users

**Service and recipient inventory**

Depending on enabled features and operator configuration, data can reach:

- Product analytics services: optional events, a pseudonymous account identifier and filtered event metadata, only with analytics permission as described in section 8. The shipped integration is PostHog, when enabled. Its built-in host is the US ingestion endpoint; self-hosters can configure a different host for their own analytics project.
- Email, SMS and push-delivery services: delivery addresses or device tokens and the messages or notifications needed for the enabled feature. Configured push integrations include Firebase Cloud Messaging and Apple Push Notification service.
- Providers you connect: account identifiers, authorization credentials, messages or request content needed for the connection or operation you initiate.
- Assistant, model and voice services: conversation content, tool context, prompts, audio and related content needed for the feature you choose to use. When enabled or configured, integrations include the NyxAgent assistant engine or Aevatar broker, and model/voice services such as OpenAI and xAI, including connections using an operator-configured key.
- Billing and payment services: billing identities, usage, checkout and payment references when billing is enabled. Configured integrations include Lago, which may be self-hosted, and payment integrations such as Stripe.
- Hosting, database, storage and key-management services managed by the operator: the application data and key-management requests needed to run the deployment.

The providers used depend on your deployment and enabled features. User-connected third parties have their own policies. The deployment's sub-processor register and processing regions are available from [contact@chrono-ai.fun](mailto:contact@chrono-ai.fun). You can also ask that address about the deployment's retention and cross-border transfer arrangements.

## 6. Data Retention

Account data is retained while the account is active. Account-deletion requests receive an immediate acceptance response once the required checks pass, and account access is revoked at acceptance. Local data removal continues in the background. If a request needs more preparation or another issue must be resolved first, access continues until the request can be accepted.

NyxID requests revocation of connected-provider authorizations where the provider supports it. This can also revoke your other connections that share the same provider authorization. The deletion status shows pending work and unsupported or unconfirmed revocations to the service operator. Some records and authorizations can remain until the operator resolves that work. Deletion does not yet cover every stored record or establish that every external copy has been removed. Contact us for the status of your request.

Audit and billing records, and some records of Machine actions, settlements and access restrictions, are retained for integrity and safety. These records are not automatically purged and have no fixed retention period in the software. They remain subject to access controls and the operator's retention arrangements. Organization-owned resources remain owned by the organization when a person's account is deleted.

NyxID tracks analytics deletion requests and any further steps needed from the provider or service operator. Accepting a request does not prove that the provider has completed deletion. Account deletion does not purge operator-managed backups; those backups expire under the operator's retention policy. Contact us for the applicable retention details.

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

Available settings cover profile changes, account deletion and connection/approval controls. For access/export and other requests, contact [contact@chrono-ai.fun](mailto:contact@chrono-ai.fun). Access and export requests are fulfilled manually after identity verification. We provide the expected timeline when we acknowledge the request. Personal-data exports are not currently available as a self-service download.

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
