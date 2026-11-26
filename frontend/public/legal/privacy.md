---
title: Privacy Policy
effective_date: 2026-11-17
---

# Privacy Policy

**Effective date: 2026-11-17**

## 1. Introduction

NyxID ("the Service"), operated by ChronoAI Pte. Ltd. ("we", "our"), provides identity and access management, credential proxying and hosted AI assistant features, including NyxBot. This Privacy Policy describes personal information handled through our hosted web service, mobile app, CLI and connected features, including assistant conversations, automations and linked messaging channels.

If you use a self-hosted deployment, its operator determines the deployment's configuration, providers and operational data handling; ask that operator for its applicable privacy information. Third-party services you connect also have their own terms and privacy policies. This notice explains processing; optional analytics permission and service or channel authorisations are separate choices. See also our [Terms of Use](/terms).

## 2. Information We Collect

The information processed depends on the features you use and their configuration.

**Account information**

- Email address (the account identifier used for sign-in)
- Display name (optional, user-provided)
- Password (stored as a salted Argon2id hash, never in plaintext) when using email/password sign-in

**Authentication data**

- Session token hashes and refresh-token JTI/revocation metadata on the server; authentication tokens in platform-protected Expo SecureStore on mobile. Some short-lived delivery protocols temporarily retain encrypted credentials for one-time delivery.
- Multi-factor authentication (MFA) secrets (encrypted at rest)
- Social sign-in profile information and provider identifiers; OAuth tokens, API keys and other credentials when you connect services
- Service and organisation memberships, agent permissions, consent and channel-link records needed to determine authorised access

**Device information (mobile)**

- Push notification token (FCM on Android, APNs on iOS), device platform, and app identifier — used to deliver enabled approval challenges and optional NyxBot notifications

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
- Optional crash diagnostics: app version/build, coarse runtime and system version, error categories and sanitized code locations, only with the updated analytics permission described in section 8

Servers receive technical metadata with HTTPS requests for security and audit. Optional product analytics is a separate data flow described in section 8.

## 3. How We Use Your Information

- Authenticate your identity and manage your sessions
- Provide single sign-on (SSO) to connected services
- Deliver enabled push notifications for time-sensitive approval challenges and optional NyxBot notifications (mobile)
- Process your approval, denial, and revocation decisions
- Register and manage your device for push delivery
- Enforce security policies (rate limiting, anomaly detection)
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
- Crash diagnostics services: sanitized diagnostic reports may be exported by the server to operator-configured Sentry web/mobile projects. These events contain no account/user identifiers, IP address, user agent, cookies or message content. Local reports are linked to the account; vendor reports expire within 30 days.
- Push-delivery services: device tokens and the notifications needed for enabled approval and NyxBot notification features. Supported integrations include Firebase Cloud Messaging and Apple Push Notification service. NyxID itself does not send account email. When billing is enabled, billing and payment providers may receive your email address as a billing contact and may send invoices or receipts under the operator's configuration.
- SMS and voice-call providers: when you choose a phone channel, Twilio receives the phone number, message content and delivery metadata needed for Twilio Messaging SMS, or the call metadata and conversation audio needed for Twilio ConversationRelay voice calls. These paths are optional and are used only when enabled for the feature.
- Providers you connect: account identifiers, authorization credentials, messages or request content needed for the connection or operation you initiate.
- Assistant, model and voice services: prompts, relevant conversation history, instructions, memory, tool context, attachments or extracted text, audio and related content needed for the feature. Depending on the configured path, this can include the NyxAgent assistant engine, Aevatar broker, and model/voice providers such as OpenAI and xAI. Some services use your connection; others use an operator-configured platform connection. Model and tool responses can become part of a saved NyxID conversation.
- Messaging platforms and channel gateways: sender and chat identifiers, message text, supported media and routing information for linked bots or conversations. Supported paths include Telegram, Discord, Slack, Lark/Feishu, WhatsApp and X, and an Agent Event Gateway where configured. Recipients in a shared conversation can see messages sent there. Linking personal access to a shared bot requires browser account review and confirmation in that same private chat; a website sign-in alone does not link it. Disconnecting a channel ends that link's authority but does not recall messages already delivered.
- Billing and payment services: billing identities, usage, checkout and payment references when billing is enabled. Configured integrations include Lago, which may be self-hosted, and payment integrations such as Stripe.
- Hosting, database, storage and key-management services managed by the operator: the application data and key-management requests needed to run the deployment.

This inventory describes supported data flows, not a claim that every listed service is enabled or acts as our sub-processor. A provider engaged to process hosted-service data on our behalf may be a sub-processor; a service you connect or a messaging platform may process data under its own policies. The software's integration list does not establish a provider's contractual role, current processing region, retention period or training practices. Contact [contact@chrono-ai.fun](mailto:contact@chrono-ai.fun) for the hosted deployment's current provider and sub-processor information, processing regions, retention and cross-border transfer arrangements. For a self-hosted deployment, request those details from its operator.

### Google user data

NyxID's use and transfer to any other app of information received from Google APIs will adhere to the [Google API Services User Data Policy](https://developers.google.com/terms/api-services-user-data-policy), including the Limited Use requirements.

**What we access.** If you sign in with Google, we receive your name, email address and profile picture to create and authenticate your NyxID account. If you connect a Google service (Google Workspace, Gmail, Google Calendar, Google Drive, Docs, Sheets or Slides), we receive OAuth tokens for the permissions you approve on Google's consent screen. Depending on those permissions, NyxID can read your Gmail messages and settings and send email on your behalf; see, create and edit your calendars and events; and see, create and edit your Google Drive files, including Docs, Sheets and Slides. NyxID does not request permission to delete, archive or relabel Gmail messages.

**How we use it.** Google user data is used only to provide the features you request: actions you take in NyxID, tasks you ask the assistant (NyxBot or your other agents) to perform, automations you configure and agents you authorise. For example, NyxBot can summarise your inbox, check your availability, add a meeting to your calendar, reply to an email or edit a document when you ask it to. Depending on your settings, NyxID asks you to confirm actions that change your data before they are sent to Google.

**How we store and protect it.** OAuth tokens are stored with application-layer AES-256 encryption. NyxID does not keep a separate copy of your mailbox, calendar or files. Google data that the assistant retrieves while answering you becomes part of that conversation and is stored with it until you delete the conversation or your account (see sections 4 and 6).

**Who receives it.** To carry out a request, the relevant Google data is sent to the assistant and model services that process that request, as described in the inventory above. We do not sell Google user data, use it for advertising, or transfer it to data brokers or information resellers. We do not use Google user data to develop, improve or train generalised or non-personalised AI or machine-learning models, and we do not transfer it to third parties for that purpose. People at NyxID do not read your Google user data unless you give us permission for specific data, it is necessary for security purposes such as investigating abuse, it is required to comply with applicable law, or it has been aggregated and anonymised for internal operations.

**Your controls.** You can disable or delete a Google connection at any time in AI Services. Deleting a connection removes its stored tokens and asks Google to revoke NyxID's access. You can also remove NyxID's access from your [Google Account permissions](https://myaccount.google.com/permissions) page. Deleting a conversation removes the Google data saved in it. Notes an agent saved to its memory are separate and can be removed from that agent's memory. Account deletion is described in section 6.

### Meta and WhatsApp data

**What we access.** When a business connects a WhatsApp Business Account to NyxID, through Meta's Embedded Signup ("Connect with Meta") or with its own credentials, we receive an access token for that account with the WhatsApp permissions the business approves, and identifiers such as the WhatsApp Business Account ID, phone number ID and display phone number. When people message the business's number, Meta sends NyxID the sender's WhatsApp ID (phone number) and profile name, the message content and supported media, and delivery and read status for messages the business sends.

**How we use it.** Only to provide the messaging features the business configures: receiving its customers' messages, delivering them to the agent, service or people it chooses, sending its replies and approved template messages, showing delivery status, and keeping the connection set up (phone-number registration, webhook subscription and status checks).

**How we store and protect it.** Access tokens use application-layer AES-256 encryption. NyxID's message relay stores message metadata (identifiers, direction, content type, timestamps, attachment details and delivery status), not message text. If the business links the channel to a NyxID assistant, those conversations are stored in that assistant's history.

**Who receives it.** Message content and sender identifiers go to the destination the business configures for the channel. Replies go to Meta, which delivers them under WhatsApp's terms and policies. We do not sell data received from Meta or use it for advertising.

**Retention and deletion.** Ordinary message records are deleted after 30 days, and WhatsApp delivery receipts and duplicate-message markers are kept for 30 days. Conversation routes and some delivery-tracking records (message IDs, delivery states and timestamps, without message text) have no fixed retention period and are removed when the channel owner's account is deleted. Deleting a channel stops messaging through it immediately; its encrypted credentials stay deactivated until account deletion or a deletion request removes them. See our [Data Deletion Instructions](https://nyxid.ai/data-deletion).

## 6. Data Retention

**Conversations and feature content.** Saved assistant messages, memory and working context are not governed by the upload expiry period. Conversation deletion removes that conversation's locally stored messages and attachments through its cleanup path; an agent's saved memory is separate from a single conversation and can be managed separately. Upload retention is configurable by the operator. Software defaults are 24 hours for unbound uploads, 30 days for bound user images and documents, and the conversation lifetime for tool-generated images and files. Operators can change these periods or select deletion of user images after a turn. Expiry makes a payload unavailable and background cleanup removes stored payloads; message text, attachment descriptions or expiry markers can remain. The account export includes retained uploaded and generated files as metadata only. While a file is retained, you can download it individually from its attachment controls in the web or mobile app. These defaults are not a promise of the hosted deployment's current settings or an exact physical-deletion deadline.

Oracle task prompts, responses and generated files use the configured Oracle task retention period; enrolled browser workers and their manager commands are retired or expire through the Oracle lifecycle, and encrypted login profiles and snapshots are removed with the retired pool. Content already sent to a model, messaging platform, connected service or other recipient is subject to that recipient's handling and retention. Deleting NyxID history, disconnecting a channel or stopping a task does not establish deletion of those external copies or undo an action already performed.

**Account deletion and retained records.**

Account data is retained while the account is active. Account-deletion requests receive an immediate acceptance response once the required checks pass, and account access is revoked at acceptance. Local data removal continues in the background. If a request needs more preparation or another issue must be resolved first, access continues until the request can be accepted.

NyxID requests revocation of connected-provider authorizations where the provider supports it. This can also revoke your other connections that share the same provider authorization. The deletion status shows pending work and unsupported or unconfirmed revocations to the service operator. Personal local account, authentication, service, assistant, channel, device and related data is erased after acceptance, subject to unsettled external effects and the bounded background cleanup process. Shared organization records and records owned by other people are preserved; shared human-readable names and email addresses are redacted, while a shared-resource attribution with an opaque account identifier may remain. External provider copies and actions already delivered to a provider, model, messaging platform, connected service or recipient are outside NyxID's control.

For integrity, safety and legal or operational accountability, NyxID retains hash-chained audit records, financial and billing records, machine lifecycle records, erasure and access-restriction bookkeeping, and shared-resource attribution that keeps only an opaque account identifier after human names and email addresses are redacted. These retained records are not automatically purged and have no fixed retention period in the software; they remain subject to access controls and the operator's retention arrangements. Individual usage records are an exception: usage billed to a wallet expires automatically 30 days after the billing provider acknowledges it, and other usage (for example free services or your own keys) expires 30 days after it is counted in daily usage summaries. The daily usage summaries, billing ledger, wallet balances and credit records remain retained as described above. Organization-owned resources remain owned by the organization when a person's account is deleted.

This bookkeeping includes records of unresolved problems found while changing encryption keys. NyxBot keeps records of received messages, delivery attempts and revoked access on linked messaging platforms to prevent duplicate processing and finish interrupted work. These records contain platform message and sender identifiers, account identifiers where needed, and status information; those records do not store message text. Public chat sessions offered by an organisation expire 24 hours after creation. Their encrypted transcripts are scheduled for automatic deletion at that same 24-hour deadline, though background deletion can run later. Encrypted connection details are cleared after any required remote cleanup is confirmed; separate records of revocation and recovery may remain.

NyxID tracks analytics deletion requests and any further steps needed from the provider or service operator. Accepting a request does not prove that the provider has completed deletion. Account deletion does not purge operator-managed backups; those backups expire under the operator's retention policy. Contact us for the applicable retention details.

## 7. Your Rights

You have the right to:

- Access and export your personal data
- Correct inaccurate information in your profile (web Settings lets you edit your display name; the mobile app lets you edit your display name and avatar URL; contact us for other corrections)
- Request account deletion, subject to the retention and implementation limits in section 6
- Remove connected service authorizations in AI Services where the provider supports revocation
- Revoke any active approval grants
- Disable push notifications through your device settings
- Manage optional NyxBot push notifications, where available, in the web notification settings page or assistant notifications dialog, and in mobile Settings → Notifications

You can download a self-service JSON export from **Settings → Privacy → Download your data** on the web or **Settings → Privacy → Export my data** in the mobile app. The export is in `nyxid.account-export` version 1 format and includes account profile and organisation memberships, service connections, security/session metadata, your own assistant inputs, assistant replies to web or mobile turns in your private conversations that were not routed to a connected messaging platform, and channel metadata, account activity, crash and error reports you chose to send, as metadata only (app and system versions, error category and timestamps — no stack frames), machine metadata, audit event metadata and your billing wallet, grants and usage rollups where available. Shared or team memory, system-injected instructions, content authored by other people, replies in shared or guest conversations, and shared conversation titles are excluded. Explicitly marked inputs you authored in shared conversations remain included. Each kind of record is listed separately with a total count and a truncation flag. Records are listed newest first. Uploaded and generated files are represented by metadata (name, MIME type, size, creation time and expiry), never by raw file payloads. Every URL-valued field, including connection and webhook addresses, is exported as `scheme://host[:port]` only because URL paths can contain access tokens; every reduced URL has a sibling `<field>_path_omitted: true` marker. Full endpoint, webhook, trigger-destination, redirect-URI, and profile avatar addresses remain visible in the web or mobile controls where those values are managed; operational Oracle browser/worker addresses are not presented as full-address account data. Exports exclude passwords, hashes, tokens, keys, secrets, ciphertext, signatures, raw file payloads, transient coordination and lease records, erasure bookkeeping, and raw retained ledger or integrity records; audit event metadata and billing summaries remain included. The serialized export is limited to 32 MiB. Each kind of record initially receives an equal share of the space; unused space is then shared equally among kinds still limited by size, with up to three further reads. Each kind is limited to 10,000 rows. The top-level `truncated_sections` list identifies kinds still capped after this allocation. A kind that cannot be read, including one not read within the export's 60-second preparation limit, is marked unavailable and listed in `unavailable_sections`. You may request up to three exports per hour. Contact [contact@chrono-ai.fun](mailto:contact@chrono-ai.fun) for other access or correction requests.

## 8. Cookies, Local Storage, and Telemetry

**Web:** NyxID uses HTTP-only session cookies and browser storage for authentication state and settings. Secure-cookie and HTTPS protection depend on deployment configuration. The application shell loads fonts from Google Fonts on every page, which causes the browser to contact Google and disclose the technical request metadata needed to fetch them. These font requests are separate from optional product analytics.

**Mobile:** The app stores authentication tokens and push token references using Expo SecureStore and platform-protected local storage. The app does not use tracking cookies, advertising identifiers, or cross-app tracking.

**Optional product analytics.** Analytics requires account permission, which defaults off, and an account-bound choice on each device. The first-run prompt acknowledgement is retained in the account profile so the prompt is shown once; the account and device controls remain available in Settings. Web and mobile Settings provide separate account and device controls. The CLI provides `nyxid telemetry enable|disable|status` for the selected account/device and `account-enable|account-disable` for account permission. Turning account permission off applies to future analytics dispatch across devices. Turning a device choice off stops that device's optional events and does not change another device's choice. Browser Do-Not-Track/Global Privacy Control and CLI `DO_NOT_TRACK` or `NYXID_TELEMETRY=off` further restrict collection.

Client events pass through authenticated NyxID checks. Pending events lose permission after withdrawal, regrant or an account switch. Events already sent cannot be recalled. Pre-login analytics, session replay and automatic screen/DOM capture are disabled. Security audit and essential operational records are separate from product analytics.

Events can be keyed to the NyxID account UUID, which is a pseudonymous personal identifier. Explicit event categories and bounded metadata exclude raw credentials, request bodies, messages, form contents and URLs. Raw-event and aggregate retention depend on the analytics project settings; ask the deployment operator for the applicable period. Account deletion requests provider erasure; queue acceptance alone does not guarantee provider completion. Contact us for confirmed status.

Withdrawing permission stops new reports and pending exports; reports NyxID has already received remain for up to 30 days unless the account is deleted, and exported vendor copies expire under vendor retention.

Updated analytics permission (policy 2) optionally includes automatic crash and error diagnostics. It requires a new account acknowledgement and a policy-2 device choice; existing policy-1 permission continues to cover categorical usage analytics only. Choosing Not now keeps the existing choice. Diagnostics include app version/build, coarse system/runtime versions, finite error categories and sanitized code locations. They exclude messages, raw stacks, URLs, function names, credentials, email, device names, IP and user agent. Reports remain linked to the account only in NyxID and are retained for up to 30 days. Account deletion removes that local link. Optional Sentry exports contain no account/user/IP/UA/cookie identifiers and expire under project retention of 30 days or less.

Web covers JavaScript errors, unhandled rejections, render and resource failures while the page is running. iOS also captures native signal crashes; Android captures only JavaScript errors and JavaScript fatal errors. Android native JVM/NDK crashes are not captured. Counts reflect consented reports and do not represent every crash. Mobile may retain up to three sanitized records on-device for up to 24 hours for the same account and consent revision. The iOS protected, backup-excluded crash file can contain register values between a crash and the next launch. An eligible file is sanitized in memory and deleted; files without capture authority, with a missing or mismatched arming token, or older than 24 hours are deleted at module initialization on the next launch. The PLCrashReporter signal handler cannot be unregistered within a process, so a crash after pause or disarm still writes a raw file in the protected app sandbox. That file is deleted unread at the next launch because capture authority was cleared; if the app is never launched again, it remains on the device. Registers and exception reasons never enter reports. Logout, withdrawal, account switch or a definitive availability denial purges local diagnostics. Unknown availability, rollout readiness or network failures pause capture and preserve the bounded sanitized records.

Self-hosters of NyxID can run with analytics disabled by default, or point at their own analytics project.

## 9. Children's Privacy

The Terms of Use require users to be at least 18 years old. NyxID is not intended for children. If you believe a child has provided personal information to us, contact us to request review and removal, subject to the retention and deletion limits in section 6.

## 10. Changes to This Policy

We may update this Privacy Policy from time to time to reflect changes in our practices or legal requirements. Material changes will be indicated by a new effective date at the top of this document. Continued use of the Service after changes constitutes acceptance of the revised policy.

## 11. Contact Us

If you have any questions about this Privacy Policy or your data, please contact us at: [contact@chrono-ai.fun](mailto:contact@chrono-ai.fun)
