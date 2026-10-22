---
title: Privacy Policy
effective_date: 2026-10-23
---

# Privacy Policy

**Effective date: 2026-10-23**

## 1. Introduction

NyxID ("the Service"), operated by ChronoAI Pte. Ltd. ("we", "our"), provides identity and access management, credential proxying and hosted AI assistant features, including NyxBot. This Privacy Policy describes personal information handled through our hosted web service, mobile app, CLI and connected features, including assistant conversations, automations and linked messaging channels.

If you use a self-hosted deployment, its operator determines the deployment's configuration, providers and operational data handling; ask that operator for its applicable privacy information. Third-party services you connect also have their own terms and privacy policies. This notice explains processing; optional analytics permission and service or channel authorisations are separate choices. See also our [Terms of Use](/terms).

## 2. Information We Collect

The information processed depends on the features you use and their configuration.

**Account information**

- Email address (for registration and communication)
- Display name (optional, user-provided)
- Password (stored as a salted Argon2id hash, never in plaintext) when using email/password sign-in

**Authentication data**

- Session token hashes and refresh-token JTI/revocation metadata on the server; authentication tokens in platform-protected Expo SecureStore on mobile. Some short-lived delivery protocols temporarily retain encrypted credentials for one-time delivery.
- Multi-factor authentication (MFA) secrets (encrypted at rest)
- Social sign-in profile information and provider identifiers; OAuth tokens, API keys and other credentials when you connect services
- Service and organisation memberships, agent permissions, consent and channel-link records needed to determine authorised access

**Device information (mobile)**

- Push notification token (FCM on Android, APNs on iOS), device platform, and app identifier — used to deliver approval challenges

**Usage data**

- Approval decisions (approve/deny/revoke), timestamps, and idempotency keys for security audit trails
- Service, model and operation usage, metered quantities, credit movements, billing identities and payment or checkout references when billing is enabled

**Assistant and automation content**

- Messages, replies, conversation titles, group messages, saved agent instructions and memory notes, and working-context summaries used to continue conversations
- Uploaded files and images, display filenames, extracted document text and generated tool images; enabled channel-media features can import supported files from verified owner messages
- Tool activity, action confirmations, automation instructions and execution status, and results retained for asynchronous work
- Voice audio processed for an enabled voice feature, transcripts saved in the conversation, and voice session or usage metadata
- Reply feedback, including optional comments, for review by authorised service administrators, operators and users with permission to read administrative usage reports, including Usage Viewers

**Linked channels and other hosted features**

- Messaging-platform, bot, sender, conversation and message identifiers; delivery and account-link status; bot credentials and verification material where needed
- Message content and supported attachments when routed to an assistant or another connected runtime. NyxID's basic channel relay stores routing and delivery metadata; assistant features can separately retain the conversation content
- Oracle task prompts, responses and generated files when you use a browser-model pool, and encrypted webhook payloads when retained webhook replay is configured

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
- Run requested assistant tasks, retain conversation history and memory, retrieve relevant content from permitted services, and execute configured automations or machine actions
- Carry messages and supported media between NyxID, linked messaging platforms and the assistant or runtime used for the conversation
- Process voice interactions and save their transcripts where supported
- Measure billable usage, apply allowances and credits, and manage checkout, payment and billing records
- Review submitted reply feedback and investigate failures

## 4. Data Storage & Security

NyxID stores account data in MongoDB. Dedicated OAuth-token, MFA-secret and API-credential fields use application-layer AES-256 encryption. Passwords use Argon2id with per-user salts; session tokens are hashed rather than reversibly encrypted. Newly saved configured default request-header values are encrypted at rest. Values saved before this protection was enabled are protected when the operator migrates them. Database/storage encryption and backup protections depend on the deployment.

Assistant file payloads, extracted upload text, tool-image payloads and stored per-conversation agent credentials use application-layer encryption. This does not mean that every assistant record is encrypted at that layer: conversation text, titles, memory notes, working-context summaries and feedback are stored as application data in the database. Content may contain personal information you or a connected service supply. Authorisation controls restrict access; these features do not provide end-to-end encryption that prevents the hosted service from processing the content.

Production deployments should use HTTPS; the deployment operator configures this protection. NyxID-issued JWTs use RS256 digital signatures. NyxID supports changing signing keys with an overlap period during which tokens signed with retained keys can still be verified. It also supports changing encryption keys in stages and migrating stored encrypted data. Keys must be kept while they are needed to read stored data or verify existing audit and billing records. The service operator decides the key-rotation schedule and when old keys can be retired.

On mobile, authentication tokens use the OS-provided secure storage (iOS Keychain via Expo SecureStore, Android EncryptedSharedPreferences).

## 5. Data Sharing and Sub-processors

We do **not** sell, rent, or trade your personal information. Data can be shared in the following circumstances:

- **Providing the Service:** with the infrastructure, assistant, model, communications and billing services needed for an enabled feature
- **Your instructions and authorisations:** with connected services, agent runtimes and messaging recipients used by your requests, configured automations or authorised agents, including OAuth/OIDC connections
- **Legal obligations:** when required by law, regulation, or valid legal process
- **Security:** to prevent fraud or protect the rights and safety of our users

**Service and recipient inventory**

Depending on enabled features and operator configuration, data can reach:

- Product analytics services: optional events, a pseudonymous account identifier and filtered event metadata, only with analytics permission as described in section 8. The shipped integration is PostHog, when enabled. Its built-in host is the US ingestion endpoint; self-hosters can configure a different host for their own analytics project.
- Email, SMS and push-delivery services: delivery addresses, phone numbers or device tokens and the messages or notifications needed for the enabled feature. Supported integrations include Firebase Cloud Messaging, Apple Push Notification service and Twilio SMS; email delivery uses the operator's configured mail service.
- Providers you connect: account identifiers, authorization credentials, messages or request content needed for the connection or operation you initiate.
- Assistant, model and voice services: prompts, relevant conversation history, instructions, memory, tool context, attachments or extracted text, audio and related content needed for the feature. Depending on the configured path, this can include the NyxAgent assistant engine, Aevatar broker, and model/voice providers such as OpenAI and xAI. Some services use your connection; others use an operator-configured platform connection. Model and tool responses can become part of a saved NyxID conversation.
- Messaging platforms and channel gateways: sender and chat identifiers, message text, supported media and routing information for linked bots or conversations. Supported paths include Telegram, Discord, Slack, Lark/Feishu, WhatsApp and X, and an Agent Event Gateway where configured. Recipients in a shared conversation can see messages sent there. Linking personal access to a shared bot requires browser account review and confirmation in that same private chat; a website sign-in alone does not link it. Disconnecting a channel ends that link's authority but does not recall messages already delivered.
- Billing and payment services: billing identities, usage, checkout and payment references when billing is enabled. Configured integrations include Lago, which may be self-hosted, and payment integrations such as Stripe.
- Hosting, database, storage and key-management services managed by the operator: the application data and key-management requests needed to run the deployment.

This inventory describes supported data flows, not a claim that every listed service is enabled or acts as our sub-processor. A provider engaged to process hosted-service data on our behalf may be a sub-processor; a service you connect or a messaging platform may process data under its own policies. The software's integration list does not establish a provider's contractual role, current processing region, retention period or training practices. Contact [contact@chrono-ai.fun](mailto:contact@chrono-ai.fun) for the hosted deployment's current provider and sub-processor information, processing regions, retention and cross-border transfer arrangements. For a self-hosted deployment, request those details from its operator.

## 6. Data Retention

**Conversations and feature content.** Saved assistant messages, memory and working context are not governed by the upload expiry period. Conversation deletion removes that conversation's locally stored messages and attachments through its cleanup path; an agent's saved memory is separate from a single conversation and can be managed separately. Upload retention is configurable by the operator. Software defaults are 24 hours for unbound uploads, 30 days for bound user images and documents, and the conversation lifetime for tool-generated images. Operators can change these periods or select deletion of user images after a turn. Expiry makes a payload unavailable and background cleanup removes stored payloads; message text, attachment descriptions or expiry markers can remain. These defaults are not a promise of the hosted deployment's current settings or an exact physical-deletion deadline.

Oracle task content and retained webhook payloads use their configured feature retention periods. Content already sent to a model, messaging platform, connected service or other recipient is subject to that recipient's handling and retention. Deleting NyxID history, disconnecting a channel or stopping a task does not establish deletion of those external copies or undo an action already performed.

**Account deletion and retained records.**

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

**Web:** NyxID uses HTTP-only session cookies and browser storage for authentication state and settings. Secure-cookie and HTTPS protection depend on deployment configuration. Some public pages load fonts from Google Fonts, which causes the browser to contact Google and disclose the technical request metadata needed to fetch them. These font requests are separate from optional product analytics.

**Mobile:** The app stores authentication tokens and push token references using Expo SecureStore and platform-protected local storage. The app does not use tracking cookies, advertising identifiers, or cross-app tracking.

**Optional product analytics.** Analytics requires account permission, which defaults off, and an account-bound choice on each device. Web and mobile Settings provide separate account and device controls. The CLI provides `nyxid telemetry enable|disable|status` for the selected account/device and `account-enable|account-disable` for account permission. Turning account permission off applies to future analytics dispatch across devices. Turning a device choice off stops that device's optional events and does not change another device's choice. Browser Do-Not-Track/Global Privacy Control and CLI `DO_NOT_TRACK` or `NYXID_TELEMETRY=off` further restrict collection.

Client events pass through authenticated NyxID checks. Pending events lose permission after withdrawal, regrant or an account switch. Events already sent cannot be recalled. Pre-login analytics, session replay, automatic screen/DOM capture and raw exception collection are disabled. Security audit and essential operational records are separate from product analytics.

Events can be keyed to the NyxID account UUID, which is a pseudonymous personal identifier. Explicit event categories and bounded metadata exclude raw credentials, request bodies, messages, form contents and URLs. Raw-event and aggregate retention depend on the analytics project settings; ask the deployment operator for the applicable period. Account deletion requests provider erasure; queue acceptance alone does not guarantee provider completion. Contact us for confirmed status.

Self-hosters of NyxID can run with analytics disabled by default, or point at their own analytics project.

## 9. Children's Privacy

The Terms of Use require users to be at least 18 years old. NyxID is not intended for children. If you believe a child has provided personal information to us, contact us to request review and removal, subject to the retention and deletion limits in section 6.

## 10. Changes to This Policy

We may update this Privacy Policy from time to time to reflect changes in our practices or legal requirements. Material changes will be indicated by a new effective date at the top of this document. Continued use of the Service after changes constitutes acceptance of the revised policy.

## 11. Contact Us

If you have any questions about this Privacy Policy or your data, please contact us at: [contact@chrono-ai.fun](mailto:contact@chrono-ai.fun)
