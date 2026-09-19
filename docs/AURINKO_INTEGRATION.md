# Aurinko email integration

NyxID uses one platform-owned Aurinko application for mailbox authorization and upstream billing. A person or organization connects its own mailbox, and the resulting encrypted account token can serve both **AI Services** and an **Aurinko email channel bot**. The application Client ID and Client Secret authorize the account-code exchange; only the resulting account token is used to read or send mail. The bot also uses the application's separate webhook signing secret.

Existing manual account-token connections remain supported. Those connections keep their original independent AI Service and bot credentials until users explicitly create a managed mailbox connection.

This document supersedes the proposed connector/polling architecture in the historical [feasibility assessment](./assistant/AURINKO_EMAIL_CHANNEL_FEASIBILITY.md). The implemented channel is a native adapter with bounded inline handling and producer-owned retries; NyxID does not poll mailboxes.

## Configure application credentials

Admins can store the Aurinko application credentials in **Admin → Platform Credentials → Aurinko Email**. The three password fields are **Application Client ID**, **Application Client Secret**, and **Application webhook signing secret**. The signing secret comes from separate webhook settings; it is not the Client Secret or an account token. Values are encrypted and never prefilled or returned. The existing Aurinko provider retains the app ID and client secret; the platform credential row stores only the signing secret. Combined changes are atomic and configuration reads use one database snapshot.

The CLI uses the same descriptor and storage:

```sh
nyxid admin platform-credentials set aurinko \
  --field-env client_id=AURINKO_CLIENT_ID \
  --field-env client_secret=AURINKO_CLIENT_SECRET \
  --field-env signing_secret=AURINKO_SIGNING_SECRET
```

The application pair enables managed mailbox authorization when the Aurinko provider is active. Managed channel bots also require the signing secret. Saving the same values again is unnecessary after an upgrade; values entered through the existing provider form are reused. A code deployment alone does not require rotating the app pair or any existing account tokens.

Configure the following URLs before connecting a mailbox. Replace `https://nyxid.example.com` with the public HTTPS `BASE_URL` of this installation, without a path or query:

| Setting | Value | Where to enter it |
|---|---|---|
| Final callback (`returnUrl`) | `https://nyxid.example.com/api/v1/providers/aurinko/mailboxes/callback` | Aurinko application's authorized return URLs |
| Optional intermediate provider redirect | `https://nyxid.example.com/api/v1/providers/aurinko/intermediate` | Provider app's redirect URI and the matching Google, Office 365, or Zoho settings in Aurinko |

These URLs have different purposes. The final callback delivers the Aurinko account authorization to NyxID. The intermediate redirect forwards provider callback parameters to the fixed `https://api.aurinko.io/v1/auth/callback` endpoint; it never exchanges an account code. Use it when the provider requires a redirect on your own verified domain. If using Aurinko's default intermediate callback instead, register that exact URL with the provider. Never register a wildcard callback.

Configure the upstream provider applications in Aurinko as required for production. For native Zoho Mail, create a Zoho server application, add its credentials under **Aurinko app Settings → ZOHO**, configure the mail scopes, and enable the same OAuth credentials across the required data centers. These provider-specific credentials belong in Aurinko; the NyxID form receives the Aurinko application's credentials.

On installations without this Aurinko form, the existing provider admin API can already store the application pair. Authenticate as a NyxID admin, call `GET /api/v1/providers`, and find the `id` of the entry whose `slug` is `aurinko`. Send `PUT /api/v1/providers/{id}` with only these fields:

```json
{
  "client_id": "YOUR_AURINKO_APPLICATION_CLIENT_ID",
  "client_secret": "YOUR_AURINKO_APPLICATION_CLIENT_SECRET"
}
```

The response reports `has_client_id: true` and `has_client_secret: true`; it never returns their values. This uses the same encrypted provider storage and requires no secret seed or provider-type change. An older NyxID version can store this configuration but needs the managed-mailbox implementation before users can authorize their accounts.

## Connect a managed mailbox

Choose Aurinko Email from AI Services or the Aurinko managed connection option in Channel Bots. Select the mailbox provider and the personal or organization owner, then complete the hosted connection in the browser. Organization administration and resource scope are checked when authorization starts and again when the callback commits. The browser must retain the original signed-in NyxID session.

| Mailbox provider | Aurinko `serviceType` | Setup |
|---|---|---|
| Google / Google Workspace | `Google` | Hosted OAuth consent; configure the production Google application in Aurinko |
| Microsoft 365 | `Office365` | Hosted OAuth consent; configure the production Microsoft application in Aurinko |
| Zoho Mail | `Zoho` | Native mail OAuth; configure the Zoho server application in Aurinko |
| Enterprise IMAP/SMTP | `IMAP` | Enter the mailbox and server settings on Aurinko's hosted form |
| Exchange | `EWS` | Enter the Exchange login and server settings on Aurinko's hosted form |
| iCloud | `iCloud` | Complete the provider-specific hosted connection |
| POP3 only | Unsupported | Enable IMAP or use a supported mail provider |

NyxID does not accept or store IMAP, SMTP, Exchange, or mailbox app passwords. Users enter them only on Aurinko's hosted form. Use the server names and TLS settings supplied by the mailbox administrator. For Zoho over IMAP, enable IMAP access first; new free-plan accounts do not provide it, and accounts with two-factor authentication may need an app password. Zoho server names depend on the account and data center. See [Zoho IMAP access](https://www.zoho.com/mail/help/imap-access.html) and [Zoho SMTP settings](https://www.zoho.com/mail/help/zoho-smtp.html).

Successful authorization creates or completes the normal owner-scoped `UserEndpoint`, encrypted `UserApiKey`, and `UserService`. A managed bot references that same key; it stores no duplicate mailbox token or signing secret. NyxID records the verified Aurinko account ID, provider type, mailbox address, and application identity with the credential. The platform application's Aurinko billing applies to upstream usage; NyxID retains its existing platform OAuth provenance and owner billing rules.

The browser API is `POST /api/v1/providers/aurinko/mailboxes/authorize` with `provider`, `label`, optional `owner_id`, optional custom service `slug`, and optional existing key UUID as `connection_id`. It requires a first-party human session and returns `connection_id`, `service_id`, `authorization_url`, and `attempt_nonce`; it also sets the HttpOnly browser binding cookie. List reusable connections through `GET /api/v1/providers/aurinko/mailboxes?owner_id=OWNER_UUID`; the response contains connection/service IDs, provider, mailbox address, credential status and service activity, with no token. Cancellation uses `DELETE /api/v1/providers/aurinko/mailboxes/attempts/{attempt_nonce}`. Application clients must use the hosted browser connection flow instead of exchanging credentials or copying browser cookies.

Reconnect uses the same mailbox provider, key, and Aurinko account. It replaces the encrypted account token only after all callback checks pass and never automatically revokes the previous token: Aurinko does not document whether that revocation would invalidate the newly authorized provider grant. Use a separate connection for a different mailbox.

## Connect an AI Service

For managed setup, open **AI Services → Add service → Aurinko Email**, select the personal or organization owner, and continue to **Connect your mailbox**. Choose the email provider and click **Connect mailbox**. An existing managed service exposes **Reconnect mailbox** on its detail page. Channel Bots can reuse this connection without another sign-in.

From the CLI, `nyxid service add api-aurinko --oauth` opens the hosted mailbox flow. Application clients can create a hosted connect link for `api-aurinko`; the user signs in and selects their provider on NyxID. Local wizard pages cannot exchange a browser-bound mailbox authorization through a CLI bearer token.

For manual setup, in **AI Services → Add service**, choose **Aurinko Email** and enter an existing Aurinko account access token in the API credential form. This creates the normal owner-scoped endpoint, encrypted credential, and service records. Use **Test Agent Key** to probe authenticated `GET /v1/account`; the probe separates NyxID agent-key authorization from the upstream credential result.

The equivalent CLI flow is:

```sh
nyxid service add api-aurinko --credential-env AURINKO_ACCOUNT_TOKEN --label 'My mailbox'
nyxid catalog show api-aurinko
nyxid catalog endpoints api-aurinko
```

Set `AURINKO_ACCOUNT_TOKEN` through your usual secret-management environment. The token must represent the mailbox account, not an Aurinko application client secret. Manual AI Service and manual bot credentials are independent encrypted copies; rotate each copy if replacing the token. Managed connections instead share the one live key described above. Agent service bindings and Disable/Enable/Delete retain their normal AI Services behavior.

The catalog uses `https://api.aurinko.io` with Bearer authorization and a curated OpenAPI overlay. Fifteen concrete MCP operations cover account identity; email list/search; message, thread, and attachment reads; send and reply; draft create/get/update/delete/send; and email sync start/updated/deleted. Attachment reads return Aurinko's JSON representation. Writes carry write-risk/approval annotations and do not advertise provider idempotency. Scope the agent's NyxID API key to the intended service; normal owner, active-service, service-scope, and approval checks apply. A read-only agent should receive only read permissions and appropriate operation approvals. The general AI Service send operation supports user-authorized email composition; the channel reply endpoint has the narrower recipient policy below.

Managed authorization requests `Mail.Read`, `Mail.Send`, and `Mail.Drafts` for the exposed API operations. `Mail.ReadWrite` implies read access; `Mail.Drafts` includes sending messages and drafts; `Mail.All` is broader. The channel requires read and send access. NyxID verifies the account, active token status, scopes, and mailbox identity using `/v1/account`; it adds `pingProvider=true` only for Google, Office365, and IMAP, where Aurinko documents that check.

## Synchronize a mailbox through the API

The connected service exposes `POST /v1/email/sync`, `GET /v1/email/sync/updated`, and `GET /v1/email/sync/deleted` through its normal NyxID proxy path and MCP discovery. Start accepts `daysWithin` and `bodyType`; wait for `ready`, then use `syncUpdatedToken` and `syncDeletedToken` for the corresponding delta feeds. Follow `nextPageToken` while paging, and retain `nextDeltaToken` for the next incremental request. A 410 means the sync cursor expired and the caller must start again. Sync needs `Mail.Read`; starting it carries the normal write/approval annotation. The caller stores cursors and any synchronized messages. NyxID injects the account token and does not return it or persist message bodies.

## Register an email channel bot

Enable the existing channel relay and configure NyxID's public HTTPS `BASE_URL`. Aurinko must be able to reach the callback during registration. Prepare an agent API key owned by the same person or organization and set its approved callback URL. The runtime must acknowledge callbacks and deduplicate their stable `message_id` before doing work.

For managed setup, select an existing Aurinko mailbox connection owned by the bot's person or organization, or connect a new mailbox. Complete bot registration and create a route to the agent. The platform signing secret is resolved from the admin configuration whenever it is needed.

For manual setup, in **Channel Bots → Register bot**, choose **Aurinko** and enter the **Account access token** and **Aurinko signing secret**. The signing secret comes from the Aurinko application's webhook settings. It is not the account bearer token and is not a per-bot secret generated by NyxID. Manual registration remains available through the CLI:

```sh
nyxid channel-bot register --platform aurinko --label 'Support mailbox' \
  --token-env AURINKO_ACCOUNT_TOKEN --app-secret-env AURINKO_SIGNING_SECRET
nyxid channel-bot route create --bot-id BOT_UUID --agent-key-id AGENT_KEY_UUID --default-agent
nyxid channel-bot verify BOT_UUID
```

Use `--org` consistently for organization-owned bots, routes, and agent keys. Only one active Aurinko bot can claim an account, enforced by a unique database index even during concurrent registration. Different mailbox accounts can have separate bots.

The management API uses the existing endpoints:

| Action | Request |
|---|---|
| Register managed | `POST /api/v1/channel-bots/managed-onboarding/aurinko/complete` with `label`, `connection_id`, and optional `target_org_id` |
| Register manual | `POST /api/v1/channel-bots` with `platform: "aurinko"`, `label`, `bot_token`, `app_secret`, and optional `target_org_id` |
| Rotate manual credentials | `PATCH /api/v1/channel-bots/{id}` with `bot_token` and/or `app_secret` |
| Verify or repair | `POST /api/v1/channel-bots/{id}/verify` |
| Delete | `DELETE /api/v1/channel-bots/{id}` |
| Receive notification | `POST /api/v1/webhooks/channel/aurinko/{id}` |
| Reply | `POST /api/v1/channel-relay/reply` |

Registration stores the verified account before creating an Aurinko `/email/messages` subscription with `detailLevel: "status"` and `filters: ["withoutDrafts"]`. The signed POST validation challenge works while the bot is pending and returns the exact `validationToken` as `text/plain`. Normal delivery stays disabled until the subscription ID is persisted and setup succeeds.

Verify repairs missing/failed setup and reconciles subscriptions at this bot's exact callback URL after an uncertain creation. It adopts only compatible active subscriptions; signal-only subscriptions are replaced. It never deletes subscriptions with another callback URL. Changing the account token to a different mailbox is rejected. Rotating a manual bot's signing secret puts that bot into pending setup. Managed bots resolve the platform signing secret live. In either case, Verify requires evidence of a successful challenge using the current secret; an existing subscription alone is insufficient. Lifecycle operations serialize across replicas and never reactivate deleted bots.

For a managed mailbox, **Disable** on the AI Service prevents the linked bot from receiving, sending, or setting up subscriptions. **Enable** restores service availability; use Verify if the bot needs subscription repair. Deleting a bot retains its AI Service connection and removes only subscriptions for that bot's callback URL. Deleting the mailbox connection while active bots still reference it returns a conflict instructing the user to delete those bots first. This also includes pending or failed bots that have not been deleted. Final connection deletion follows the explicit upstream grant-revocation flow; other mailboxes are not treated as consumers of that grant.

Deletion deactivates the bot and its routes locally even when upstream authorization has been revoked. Aurinko deletion responds with `{"webhook_cleanup":"removed"}` or `{"webhook_cleanup":"failed"}`. CLI JSON adds `ok: true` and preserves `webhook_cleanup`; the UI and CLI warn on failed cleanup. In that case remove the subscription for the displayed callback URL in the Aurinko dashboard, or repeat Delete through the API/CLI after restoring access. Cleanup never deletes the mailbox account or another consumer's subscriptions. Ordinary legacy bot deletion still returns HTTP 204.

Owner hard deletion removes the owner's Aurinko bot credentials and email channel metadata. This integration does not expand personal deletion to legacy channel records. Active email ingress/reply work makes self-deletion retryable **before** the user is deactivated, so the user can retry. Once deletion finishes, the callback URL is inert. Hard owner deletion is local cleanup and does not call Aurinko; remove any remaining subscription in the Aurinko dashboard. Organization deletion retains its existing requirement to delete active resources first.

## Incoming mail and routing

NyxID verifies the raw request bytes using HMAC-SHA256 over `v0:{timestamp}:{body}` and constant-time comparison of `X-Aurinko-Signature`, with `X-Aurinko-Request-Timestamp`. Signatures can be up to 30 days old to allow retries carrying the original signature, but cannot predate bot creation by more than five minutes or be over five minutes in the future. Durable per-message completion metadata prevents replay within and beyond this window. Signed challenges are authenticated too.

Normal notifications must match the bot's persisted account and subscription. They contain IDs/change types, not mail bodies. NyxID fetches each eligible created/updated message with `bodyType=text`, `stripQuoted=true`, `loadInlines=false`, and `requireThreadId=true`. Lifecycle, tracking, deleted, and unrelated-resource events never become user messages. A missing thread or HTTP 408 can be transient; the producer must redeliver it.

Aurinko may subscribe to upstream changes or poll the mail provider before delivering its unified webhook. This also applies to IMAP; webhook delivery is not a promise of instantaneous arrival. NyxID itself adds no mailbox polling worker.

A conversation ID is `{accountId}:{sha256(threadId)}`, using lowercase hex, and fits the 256-character route limit. The original thread ID remains in callback routing metadata. Use `platform_conversation_id` from the callback when creating a per-thread route. Callbacks contain `Subject: …` followed by the text body, and transient `raw_platform_data` with account/message/thread/subject information. Attachments are not automatically fetched or forwarded. Agents can read them through an independently authorized AI Service connection.

The channel ignores mail received before bot creation; sent, draft, junk, and trash labels; mail from the mailbox itself; common bounce/no-reply senders; mailing-list/bulk traffic; meeting messages; and automation/report headers including `Auto-Submitted` and `X-NyxID-Auto-Reply`. This policy applies again when replying. Oversized content and missing/deleted upstream messages are permanently ignored. Incomplete fetches remain retryable.

Incoming notifications are capped at 256 KiB and 1,000 payload entries. Upstream responses are capped at 2 MiB, message text at 256 KiB, and subjects at 4,096 bytes. Provider requests have a three-second connect and ten-second total timeout, never follow redirects, and use a fixed origin. A batch has a bounded inline processing budget; a metadata-only rotating offset gives later items progress when earlier fetches repeatedly time out. No configurable upstream URL exists in production.

Successful or deliberately ignored notifications receive HTTP 200. Signature/binding failures receive 401, malformed input 400, and recoverable fetch/callback/concurrency failures 503 with `Retry-After: 10`. NyxID never uses 422 for Aurinko because Aurinko removes subscriptions on that response. Completed items in partially failed batches stay deduplicated. In-flight claims are retryable, not acknowledged as completed.

ADR-013 is preserved: no email bodies, attachments, raw notifications, or reply text are persisted; no queue or periodic retry worker is added. `channel_email_subscriptions` holds lifecycle bindings; `channel_email_batches` holds only a batch digest/cursor with a 31-day TTL; `channel_email_receipts` holds a stable UUID-v4 and completion marker; `channel_email_sends` holds the irreversible submission barrier. Receipts and sends last for the bot's lifetime, preventing an old message update from triggering fresh agent work after the normal 30-day channel-message log expires. Bot/owner cleanup removes this metadata.

Callback retries reuse the same UUID-v4 independently of route changes. Pending retries follow the current approved route and current API-key callback URL/scopes. Route reassignment revokes the old Aurinko reply authority, including reassignment of the same route to another key. The receiving runtime must also deduplicate `message_id`: a lost callback acknowledgement cannot prove whether the runtime started an action. Completed messages do not reroute. Existing relay rate limiting, callback status, last-message time, and metadata auditing apply.

## Reply behavior

The runtime acknowledges the callback, then posts a text reply using its scoped NyxID API key or the callback's message-bound reply token:

```json
{"message_id":"INBOUND_UUID","reply":{"text":"Your reply text"}}
```

NyxID checks the current bot, original message, owner, key, and routing authority, verifies the upstream account, and fetches the original message again before any send. It calls only `POST /v1/email/messages/{messageId}/reply?bodyType=text&returnIds=true`. The original message supplies the subject and recipient: a single `Reply-To` is honored, otherwise the sender is used. Multiple Reply-To addresses require manual mailbox handling. CC/BCC are empty; caller metadata cannot add recipients or attachments. Outbound custom headers include `X-NyxID-Auto-Reply: true` and `X-Auto-Response-Suppress: All`; the inbound filter recognizes the NyxID marker to prevent two NyxID bots from replying to each other.

Both authentication paths share one durable send barrier per bot/original message. Preflight errors are retryable with the same reply token. Once a provider POST is attempted, NyxID does not blindly resend it. A successful Aurinko `status: "Ok"` with `processingStatus: "Incomplete"` counts as submitted even without a returned ID. Repeated requests return the recorded optional ID without another POST. A timeout, connection loss, or unsuccessful/ambiguous POST leaves an uncertain barrier and returns a conflict requiring mailbox inspection. There is no documented Aurinko idempotency key, so NyxID cannot safely offer automatic recovery of an uncertain submission. Metadata auditing does not prove final recipient delivery.

## OAuth decision and validation boundary

Managed onboarding uses Aurinko's documented confidential-client account authorization-code protocol. Manual account-token entry remains available. The official [Account OAuth Flow](https://docs.aurinko.io/authentication/oauth-flow/account-oauth-flow) and [OpenAPI specification](https://apirefs.aurinko.io/assets/swagger.json) specify `GET /v1/auth/authorize` with `clientId`, `serviceType`, `scopes`, `responseType=code`, `returnUrl`, and `state`, followed by Basic-authenticated `POST /v1/auth/token/{code}` returning `accountId` and `accessToken`. No refresh token, token expiry, or PKCE support is assumed.

CLAUDE.md permits this named Aurinko exception without PKCE. Safeguards are fixed Aurinko endpoints with no redirects, an exact registered same-origin callback, server-side confidential-client exchange, ten-minute single-use state bound to a browser nonce, live actor session and owner authorization, app-configuration revalidation, and verified mailbox identity and mail scopes before persistence. Application credentials never serve as mailbox bearers. These controls reduce but do not eliminate authorization-code interception risk and must not be described as equivalent to PKCE. NyxID's own OAuth server and supported upstream flows retain PKCE.

## Live mailbox acceptance

Mock HTTP and MongoDB replica-set tests verify NyxID's protocol and lifecycle behavior. A real provider test also requires the configured Aurinko application, a reachable deployment, and a test mailbox. Run the following for an enterprise IMAP/SMTP mailbox and, separately, native Zoho Mail:

1. Complete the browser connection with the chosen owner. For IMAP, enter the account-specific server settings and mailbox/app password only on Aurinko's hosted form. Confirm the connected address and provider.
2. Read account identity through the AI Service and start email sync. Fetch updated and deleted feeds with their separate cursors. Use a test message to confirm that results belong to this mailbox.
3. Register a managed bot using that same connection. Verify subscription setup and route it to an agent key under the same owner.
4. Send a new plain-text message from a second test mailbox. Confirm one routed inbound event and a reply to the authorized test recipient. For IMAP, allow time for Aurinko's upstream polling and thread discovery; transient thread-not-ready responses must remain retryable.
5. Disable the AI Service and confirm bot effects stop. Enable it, reconnect the same mailbox, and use Verify if needed. Confirm the same connection is reused.
6. Confirm connection deletion reports its bot dependency. Delete the bot, verify the AI Service still works, then delete the test connection if desired. Verify cleanup affects only this bot's callback subscription and this mailbox's grant.

A POP3-only account cannot pass this flow because Aurinko exposes no documented POP3 service type. Verify it is rejected as unsupported; enable IMAP before testing that account. Successful mock tests do not establish live provider consent or recipient delivery. Native Zoho setup is documented in [Zoho OAuth setup](https://docs.aurinko.io/authentication/zoho-oauth-setup).

Official references: [documentation index](https://docs.aurinko.io/llms.txt), [webhooks](https://docs.aurinko.io/unified-apis/webhooks-api), [webhook authentication](https://docs.aurinko.io/unified-apis/webhooks-api/authentication), [authentication scopes](https://docs.aurinko.io/authentication/authentication-scopes), and the OpenAPI specification above. The Aurinko overlay participates in the existing spec-drift map. Local validation uses mock Aurinko HTTP and a real MongoDB replica set; it does not establish production delivery latency, provider app verification, Google approval, or actual recipient delivery.
