# Channel Bot Relay Design

> **ADR-013 update (2026-04-09):** Channel Bot Relay is now a **pure passthrough gateway** per ADR-013. NyxID never stores message bodies, media bytes, outbound captions/filenames, or raw webhook payloads; inbound attachment routing metadata is retained in `channel_messages` for authorized downloads. Synchronous agent replies (HTTP 200 + body) are no longer supported; agents must return 202 and post replies via `POST /api/v1/channel-relay/reply`. See NyxID#221, ADR-013, and [`docs/CHANNEL_EVENT_GATEWAY.md`](./CHANNEL_EVENT_GATEWAY.md) for the full rationale. The earlier deprecation-in-favor-of-service-connections direction (NyxID#191) has been recalled: channel relay and the new HTTP Event Gateway are the first-class inbound paths.

## Overview

The passthrough storage rule applies to relay transport. When NyxBot is the
consumer, its separately authorized assistant-media path can store verified
owner Lark/Feishu files as encrypted assistant uploads under
`assistant:channel-media`; relay history still contains routing metadata only.
See [Lark/Feishu posts and NyxBot ingestion](#larkfeishu-posts-and-nyxbot-ingestion).

NyxID Channel Bot Relay turns NyxID into a **multi-platform messaging gateway**. Users register bots or connect accounts (Telegram, Discord, Lark, Feishu, Slack, WhatsApp, X). Adapter-owned webhooks or polling normalize inbound messages into a common format, route each message to the correct AI agent's callback URL, and relay the agent's asynchronous response back to the chat.

Combined with [Agent Isolation](./AGENT_ISOLATION.md), the same NyxID user can wire different messaging platforms (or even different conversations on the same platform) to different AI agents -- each with independent credentials, rate limits, and audit trails.

[NyxBot service adapters](NYXBOT_SERVICE_ADAPTERS.md) add typed phone, SMS and
external speech interfaces to the executable registry. Telnyx, Twilio and
ElevenLabs use dedicated bindings and guest specialist sessions. The versioned
`/api/v1/channel-adapters` catalog describes those interfaces alongside existing
messaging adapters; `/api/v1/channel-platforms` retains the channel-bot contract
documented here.

---

For Telegram's optional native bot creation, see [Telegram bot creation](TELEGRAM_NEW.md). The **Telegram** option uses the separate `telegram-new` adapter; **Telegram bot token** uses the existing `telegram` adapter and token setup. These are display labels; platform identifiers and existing connections remain unchanged.

Ordinary and native thread text replies accept one complete logical reply and
split it only when required by the destination's limit. HTTP 200 confirms
platform acceptance of every generated part; partial or uncertain delivery
returns an error. A single reply token covers the whole request. See the
[complete-reply contract](CHANNEL_COMPLETE_REPLIES.md) for the 2 MiB wire bound,
64-part preflight, metadata-only tracking and coordinated CMAEG rollout.

## Telegram manager channels

A configured Telegram bot-creation manager can also be registered using its existing token as a `telegram` channel. It reports `credential_source: "telegram_manager"` and retains the manager's single webhook. Manager setup events stay in the creation workflow; ordinary messages use the channel's exact or default conversation routes. See [Telegram manager setup and limitations](TELEGRAM_NEW.md#using-the-manager-as-a-channel-bot).

These callbacks deliberately omit `X-NyxID-User-Token` for every route: a public Telegram sender does not authorize access to the channel owner's services. The signed callback and message-bound reply token remain available, allowing the agent to send asynchronous replies as the manager. This exception does not change other channel bots' scoped owner-token behavior. The manager path retains its 32 concurrent deliveries per backend process, drop-on-full, transient delivery, and possible duplicates.

## Staged thread-follow support

The [thread-follow design](CHANNEL_THREAD_FOLLOW.md) is approved for staged
implementation. The initial contract unit adds optional metadata-only
`thread_context` to direct NyxBot callbacks behind the default-off
`nyxbot:thread-follow` flag, resolved for the linked person even on organization
bots. Other callbacks omit it. The adapter capability response adds
`thread_reply`, `thread_follow` and `thread_history`; absent fields mean
unsupported. Native bound replies are implemented for Slack, Telegram (including
managed bots), Discord, Lark and Feishu. History is supported for Slack and
Discord; Lark/Feishu offer explicitly partial known-message reads. Telegram uses
metadata fallback. Provider failures never request additional scopes. All
`thread_follow` capabilities remain false and channel-agent responses report
`follow_readiness=unavailable`. This unit does not enable thread follow.

T1 excludes X public-thread follow; existing addressed-post replies and X DMs
keep their current behavior. Agent Event Gateway transports remain legacy until
the external gateway implements and advertises the
[versioned facts and bound-reply contract](CHANNEL_THREAD_FOLLOW_GATEWAY_CONTRACT.md).
Native direct reply/history operations are dormant until the follow service
supplies an explicit bound target; all legacy reply paths remain unchanged.
Owner controls and email follow ship in later review units. History is ephemeral
and limited to 20 messages / 32 KiB / eight seconds, with metadata fallback.

## X accounts: DMs, mentions and replies

X also supports separately selected encrypted chat and own-post metadata notifications. See [Typed channel activity](CHANNEL_ACTIVITY.md) for activity counts, event selection, receiver capability declarations and the callback compatibility contract.

X is managed-only: users connect their own X account with OAuth and provide no developer credentials. NyxID uses the account's user-context token to subscribe to selected unencrypted DMs, explicit @mentions and direct replies to the account's posts through the X Activity API and deliver the routed agent's asynchronous reply. Existing polling connections can switch through Verify or Reconnect after an admin configures the webhook credentials. With `BILLING_ENABLED=false`, connections without those credentials retain the existing polling path. Billing-enabled X channels require webhooks and never fall back to paid DM polling. App-only bearer tokens cannot access private DMs. Although X also supports legacy OAuth 1.0a user-context credentials, a BYO path would require users to supply and maintain developer credentials, so this channel deliberately does not expose one.

### X Activity webhook delivery

NyxID uses its shared X developer app and pays the app's API charges. Users authorize this app through the existing OAuth flow; they do not supply developer credentials or need their own developer account. This creates a NyxID channel for an existing X account, not a new X Chat bot identity.

In **Admin > Platform Credentials > X (Twitter)**, configure the **App bearer token**, OAuth 2.0 **Client ID** and **Client Secret** from the same X app. The Client Secret verifies modern webhooks as well as OAuth. Keep the optional **API key secret** (OAuth 1.0 consumer secret) for legacy webhook signatures; it is distinct from the OAuth Client Secret. OAuth credentials remain in the shared `twitter` provider; the bearer token and legacy consumer secret are encrypted in `platform_credentials`. Clearing an individual webhook credential does not clear OAuth credentials. Clearing the entire provider clears both backings.

NyxID registers or reuses `{BASE_URL}/api/v1/webhooks/channel/x/platform` using `POST /2/webhooks`, then creates one subscription for each selected event (`dm.received`, `chat.received`, `post.mention.create`, `post.reply.create`, `post.create`) through `POST /2/activity/subscriptions` with the connected account's user-context token. Webhook management and subscription discovery/cleanup use the app bearer token. The callback must be publicly reachable HTTPS without an explicit port. X's GET CRC receives a JSON `response_token` computed using HMAC-SHA256 and the OAuth 2.0 Client Secret, with consumer-secret fallback for legacy-only configuration. POST verifies `X-Twitter-Webhooks-Signature-OAuth2` using the Client Secret when that header is present; otherwise it verifies `X-Twitter-Webhooks-Signature` using the consumer secret. A malformed or invalid modern signature never downgrades to legacy verification. Both schemes authenticate the exact raw body before routing. The app bearer token is never a signing key. CRC rejects tokens that parse as JSON objects so its response cannot be reused as a valid event signature. Webhook ingress shares the general global and per-IP limits. Size `RATE_LIMIT_PER_SECOND` and `RATE_LIMIT_BURST` for aggregate application traffic and X delivery bursts; X may share delivery IPs across subscriptions.

Each subscription is tagged with the NyxID channel UUID. Setup is serialized across replicas with a renewable platform lease and discovers existing subscriptions before creating one, allowing Verify to recover an interrupted setup without deliberately creating duplicate subscriptions. A successful setup sets `webhook_registered`, cancels the polling lease, and excludes the channel from polling. For DM-only channels with billing disabled, a completed read-only initial setup failure leaves polling available and a local error on the detail page. A subscription mutation with an uncertain outcome leaves the channel failed and cleanup-eligible instead of silently polling. With billing enabled or public events selected, failed setup marks the channel failed, and a maintenance sweep retries cleanup of any partially installed subscription; select **Verify** to retry setup. Reconnect also rechecks setup. Setup lease contention preserves an unchanged channel for all event selections and billing modes. An already registered active channel also retains delivery after a read-only provider setup failure, once live credentials and billing admission have passed. Missing credentials, failed billing admission and uncertain subscription mutations still fail closed. Initial paid and non-DM setup retains its stricter requirements. Changing event selections on a registered channel records setup as incomplete before the update handler immediately reconciles the subscriptions, including a change back to DM-only. All inbound delivery, DMs included, pauses until that reconciliation succeeds; if interrupted, select Verify to retry. Events in that pause are not replayed. A failed or interrupted reconciliation remains eligible for cleanup. Unregistered channels retain their safe polling fallback. A temporary OAuth refresh failure during Verify preserves an existing active channel; revoked credentials still stop it. There is no automatic background webhook repair. Clearing webhook credentials does not switch registered channels back to polling; restore the credentials and select Verify. The platform catalog retains `ingestion.mode: "poll"` as the fallback and advertises webhook support through `registration.webhook_ingestion`; each channel's `webhook_registered` and `webhook_ingestion` report its effective delivery mode.

Signed `data.filter.user_id` selects the registered account, and `data.tag` must match its NyxID channel UUID; each DM's recipient, public mention target or replied-to account must match that account. Only currently selected event types are admitted. Self-authored posts are ignored by mention/reply handling; explicitly selected `post.create` events are recorded as metadata notifications without reply authority. Every delivery revalidates the live owner-bound OAuth connection before routing. Inbound message IDs use the existing channel deduplication and metadata-only relay. NyxID acknowledges promptly and forwards once; it does not queue bodies or retry agent callbacks. Deleting a channel disables its routes and best-effort deletes only subscriptions tagged to that channel. It preserves the shared webhook, other subscriptions, and the OAuth credential. A failed remote cleanup is recorded in the deletion audit as `managed_webhook_cleanup: "failed"` and retains a cleanup marker on the deleted channel. The maintenance sweep retries removal until it succeeds, including when customer billing or polling is disabled. The deleted channel never resumes delivery.

Readable DM delivery consumes **`dm.received`**. Select **`chat.received`** separately to record encrypted X Chat activity and optionally notify a capable agent. These notifications contain metadata only; reading or replying to encrypted content requires a separate encryption integration. See [Channel activity](CHANNEL_ACTIVITY.md) for event selection and receiver opt-in. The documented legacy webhook payload addresses the recipient account; NyxID preserves an explicit conversation ID when supplied, otherwise derives the one-to-one conversation ID. **Webhook channels support recipient-addressed one-to-one DMs only; group-DM delivery is not supported.** Existing polling connections retain group-DM polling behavior only while billing is disabled and they have not switched to webhooks. There is no per-channel delivery-mode selector.

Webhook signature and event documentation checked 2026-09-28: [X Activity API](https://docs.x.com/x-api/activity/introduction), [webhook setup and verification](https://docs.x.com/x-api/webhooks/quickstart), and [event payloads](https://docs.x.com/x-api/activity/event-payloads). X Activity documents 1,500 self-serve event subscriptions. This is separate from the Chat bot allowance and from the older Account Activity API's Pay Per Use limit of three accounts. `dm.received` costs $0.010 per delivered event at the published rate; outbound operations are charged separately. X deduplicates billable resources within a 24-hour UTC window; provider usage reports and the Developer Console remain authoritative.

### Platform setup and pricing

Admin > Platform Credentials lists every registered adapter's provider descriptor. Meta uses encrypted `platform_credentials`; X's `ProviderOAuth { provider_slug: "twitter" }` backing reads and writes the **same encrypted Client ID and Client Secret** used by the existing `twitter` OAuth provider. There is no second copy of the OAuth credentials; the webhook API secret and app bearer token use the separate encrypted platform backing. Clearing either OAuth credential clears a shared provider field, retaining the provider record, and stops all of the `twitter` provider's OAuth connections and logins until restored. Clearing the whole provider or either OAuth field requires this impact confirmation; corresponding CLI clears require `--confirm-shared-provider`. Clearing only a webhook credential does not require the OAuth impact confirmation. Update and delete audits record `shared_provider_slug`. Rotating credentials affects all connections using the shared app. Secret values never appear in admin responses. The descriptor supplies fields, setup checklist, backing information and callback URL for the generic API, page and CLI.

Configure a confidential Web App with OAuth 2.0 user authentication and PKCE in the X Developer Console. Set the callback to `{BASE_URL}/api/v1/providers/callback`, exactly as displayed in Platform Credentials. Enable `tweet.read tweet.write users.read dm.read dm.write media.write offline.access`. New channel OAuth flows request all these scopes. Existing DM-only connections can continue without `tweet.write`; enabling mentions or replies requires it and instructs users to reconnect if it is missing. The channel OAuth flow explicitly requests its scopes; provider defaults do not grant new permissions to old connections. Completion rejects connections without every required scope and requires fresh consent.

**Pricing:** X charges NyxID's developer app. Published prices checked on 2026-09-19 are $0.010 per delivered `dm.received` event, $0.015 per DM send request, and $0.010 per user resource read. The `dm.sent`, `dm.read`, and `dm.indicate_typing` notifications are free; this does not make the send API free. See [X API pricing](https://docs.x.com/x-api/getting-started/pricing). NyxID's customer credit rate is configured separately as described below.

[Current rate limits](https://docs.x.com/x-api/fundamentals/rate-limits): `/2/dm_events` permits 15 reads per 15 minutes per user. Existing-conversation message creation permits 15 per 15 minutes and 1,440 per day per user, plus 1,440 per day per app. X therefore declares a 60-second minimum poll interval. The generic sweep defaults to 30 seconds (`CHANNEL_POLL_INTERVAL_SECS`; `0` disables). A quiet, healthy account normally receives new DMs within roughly 60-90 seconds, plus callback processing; pagination, provider backoff and sweep load can increase latency.

### NyxID channel billing

With `BILLING_ENABLED=true`, X channel activity uses the active `api-twitter` catalog service and the existing BillingService, Lago rate cache, entitlement checks, reservations, allowances, grants, wallet settlement and billing ledger. Enable the billing feature flag for the channel owners who should be charged. Owners outside the rollout remain metered without charges, and `BILLING_ENABLED=false` preserves uncharged operation.

Configure a **Requests** price on the **Your own key (BYOK)** lane in `billing.byok_pricing` and wait for it to become `synced`. Shared-app OAuth is classified as `NyxidPlatformOauthApp`, which uses the BYOK lane in the existing billing contract; it is not a `NyxidManagedMaster` platform key and does not use `platform_key_pricing` or a resale charge. A price on the **NyxID platform key** lane alone leaves X channels blocked. The setup error is: “The X service (api-twitter) has no synced price on the Your own key (BYOK) lane that shared-app X channels bill through; configure a Requests price on that lane”. Existing legacy platform request pricing is also supported when selected by the normal lane resolver. A missing selected billable lane, missing Lago provider/subscription, incompatible non-request metric, or unavailable rate fails closed for billing-enabled channels. An explicitly configured zero rate or a request allowance can make the customer operation free.

Each X activity can carry its own price. In the admin service editor for `api-twitter`, open **Billing lanes → Your own key → Operation prices** and enter a price for each row below, or send `billing.byok_pricing.operations` (`[{ "operation": "channel_dm_received", "credits_per_unit": "…" }]`) with `PUT /api/v1/services/{id}`. A synced operation price replaces the base request rate; pending, failed and unpriced operations use the base rate. Explicit zero makes an operation free. Upgrade every replica before authoring these prices. No prices are seeded automatically; after deployment, administrators should check [X's current published rates](https://docs.x.com/x-api/getting-started/pricing) and author the NyxID credit prices.

| Channel activity | Operation key | Provider call or event |
|---|---|---|
| Account verification | `get_me` | `GET /2/users/me` |
| Public post reply | `create_tweet` | `POST /2/tweets` |
| Outgoing DM | `channel_dm_send` | `POST /2/dm_conversations/{id}/messages` |
| Incoming DM | `channel_dm_received` | DM webhook event |
| Incoming Chat | `channel_chat_received` | Chat webhook event |
| Incoming post | `channel_post_received` | Post webhook event |

The account and post keys share their catalog operations; channel DM send is separate from catalog `send_dm`. Long replies consume one unit per accepted chunk, and media sends consume one unit per DM carrying media. The adapter does not call the separately priced media-metadata API. Existing request allowances and funding precedence are unchanged.

The persisted channel owner pays, including the organization wallet for an organization-owned channel. Replies keep the calling API-key attribution but do not transfer the charge to the agent's personal wallet. Grant- or allowance-funded paid channels still require a provisioned Lago wallet and subscription; grants do not bypass catalog pricing or provider configuration. Funding follows the existing order: request allowance, eligible credit grants, then wallet credits. An incoming event is metered before route lookup, because X has delivered a billable event even when no agent route exists. Billing identities bind the channel UUID and X message ID, so duplicate deliveries do not reserve or charge again; no message body is stored in usage or ledger rows.

Outgoing operations reserve before making the provider request. Successful responses persist settlement intent for the existing reconciliation worker; confirmed client rejections settle zero usage and release their reservation. Transport failures and server errors have unknown provider outcomes: their forwarded reservations remain held for operator reconciliation, and the integration does not automatically resend. X initiated-send idempotency claims remain pending after an error once a provider request has been attempted to prevent an automatic replay of earlier successful chunks; inspect the conversation before issuing a new send. An admission check before subscription setup reserves one received direct message at the `channel_dm_received` price (or the base rate) and releases it without consuming it.

Insufficient credits or a billing configuration/entitlement failure during inbound delivery, an initiated send, or an asynchronous reply marks the channel failed and removes its tagged X subscription. Cleanup failures retain a durable marker and retry on the channel maintenance sweep, which runs every 60 seconds when polling is disabled. Verify rechecks funding before reactivating the subscription; cleanup and recovery share the platform lease. Existing polling channels fail before their next paid DM read and need Verify after webhook credentials are configured. Creation and Reconnect do not initialize a DM polling cursor when billing is enabled.

Billing-provider outages also stop the subscription when funding cannot be reserved; restore billing and select Verify to recover. X charges for a push event before NyxID can reject it. The event that discovers exhausted credits, in-flight deliveries, and events delivered while remote cleanup is unavailable can therefore still cost the platform. The channel is stopped to bound continued costs; this integration does not create an unbacked customer debit for an event whose reservation failed. Webhook acknowledgments retain the channel relay's immediate-ACK behavior, so a process failure before persisting a meter can also lose accounting for that delivery. Provider usage reports remain authoritative for upstream costs. Keep `BILLING_RECONCILE_INTERVAL_SECS` enabled for durable settlement retries.

### Connect, route and recover

Use Add Channel Bot > X (Twitter), or `nyxid channel-bot register --platform x --managed`. The CLI checks the server's managed bootstrap and prints `/channel-bots?connect=x`; optional label and org scope are preserved. An unconfigured app shows an explicit unavailable state.

`GET /api/v1/channel-bots/managed-onboarding/x` returns `available`, `flow: "oauth_connection"`, `provider_slug`, `required_scopes` and `authorize_start_url`. `POST` to its `/start` endpoint accepts `{label, target_org_id?}` and creates an owner-bound OAuth connection through existing OAuth/PKCE machinery. Channel-started connection rows are tagged `source: "channel_onboarding"` with a unique `source_id`. The existing OAuth refresh sweep deletes pending rows older than one hour, retaining completed connections. Cleanup is disabled when `OAUTH_REFRESH_SWEEP_INTERVAL_SECS=0`. The existing popup completion protocol carries only a nonce and completion status. `POST` to `/complete` accepts `{connection_id, label, target_org_id?}`; this ID is the `user_api_keys` row UUID, not the OAuth protocol's internal connection UUID. Completion validates ownership, platform credential provenance and scopes, verifies `/2/users/me`, initializes a cursor only for legacy unbilled polling and applies the ordinary duplicate-account and bot-limit checks.

All managed bootstrap/start/complete and reconnect routes require an authenticated human. API keys, delegated tokens, service accounts and relay tokens are rejected by the existing human-only middleware. Writes share the existing per-user managed-onboarding rate limit. Org creation and reconnect require the same owning-org admin access as other channel bots; an actor's personal connection cannot back an org bot.

The bot starts `active` with `credential_source: "connection"`. `webhook_registered` indicates whether the X Activity subscription was installed; otherwise the account polls only when billing is disabled. Webhook registration is automatic, and the user does not configure a callback or secret. Configure conversation/default-agent routing exactly as for other channels. Detail and `channel-bot show` include the connected handle, connection UUID, cursor, last/next poll, error count and locally authored failure cause.

Polling, Verify Bot and async replies resolve the live owner-bound connection and reuse `refresh_user_api_key_in_place`. A deleted/revoked connection, insufficient scopes or permanent refresh failure marks the bot `failed` with a metadata-only audit event; there is no stale-token fallback. Transient polling errors back off exponentially, and five consecutive failures stop polling with an audit. Rate limits defer polling without counting as an error. Reconnect runs OAuth again and posts `{connection_id}` to `POST /api/v1/channel-bots/{id}/reconnect`; it requires the same X account, swaps the connection and preserves the committed cursor. For legacy unbilled polling, the next sweep processes DMs received during the failure through the normal bounded page walk and deduplication. Only a missing cursor is initialized to the newest event. Delete removes the bot and routes but **retains the OAuth connection**; revoke that connection separately when appropriate.

### Select mentions and replies

New and existing channels default to **Direct messages**. After connecting, open the X channel detail, select **Mentions** and/or **Replies to my posts** under **Events to receive**, and save. At least one event must remain selected. The same setting is available through the CLI and API:

```bash
nyxid channel-bot update <bot-id> --x-events dm,mentions,replies
nyxid channel-bot show <bot-id>
```

```http
PATCH /api/v1/channel-bots/{id}
Content-Type: application/json

{"x_events":["dm","mentions","replies"]}
```

`x_events` is returned on X bot detail responses. Omitting it preserves the selection; legacy rows without it mean `["dm"]`. Empty, duplicate or unknown selections and selections on non-X channels are rejected. The selection controls incoming subscriptions; existing DM initiated-send permissions still use the conversation's explicit human opt-in.

Saving validates webhook credentials and live OAuth scopes before changing the selection, then reconciles the subscriptions tagged to that channel. Other channels' subscriptions are untouched. If provider setup fails after saving, the saved selection remains visible and the channel reports the failure; restore access/configuration and select **Verify Bot**. Reconnect retains the selection. Public events require working webhooks even with billing disabled; there is no public polling fallback. Deploy all backend replicas before enabling public events so every webhook recipient understands them.

- `mentions` subscribes to `post.mention.create`: another user's explicit @mention in a post body. Implicit reply mentions alone do not trigger it.
- `replies` subscribes to `post.reply.create`: another user's direct reply to a post by the connected account. This does not monitor every nested comment in a thread.
- X does not deliver protected-account posts through Activity webhooks. Reposts, quotes, likes and encrypted X Chat are outside these selections.

Public conversations use `post:<conversation_id>`, separately from DM addresses, and preserve the root thread ID and parent-post reference in the callback. Configure a default agent route to handle all incoming posts, or an exact public conversation route to handle one thread. A mention and reply notification describing the same post produce one metadata row and at most one callback, even when delivered concurrently. They also share one NyxID incoming-post billing identity. X's own billing remains governed by its resource/event rules.

Agents acknowledge callbacks with HTTP 202 and use the existing asynchronous reply endpoint. The reply is **public** and targets the persisted incoming post, even if agent metadata supplies a different post/thread ID. Public callbacks omit the owner's `X-NyxID-User-Token`; the signed callback and message-bound reply token remain available. Public responses currently support text only, with X enforcing its account/post length and reply restrictions. Arbitrary initiated public posts, public media replies and edits are unsupported and are not advertised on public conversation capabilities. DM image/video replies retain their existing behavior.

Public receives and successful post replies use the `channel_post_received` and `create_tweet` operation prices when configured, otherwise the `api-twitter` base request price, with the usual funding rules. The billing operation IDs are `x-post-received` and `x-post-reply`, separate from DM operations. As with other channel traffic, callback failures are not queued or retried, and immediate webhook acknowledgement does not guarantee delivery across process failure.

Contract checked 2026-09-22 against [X Activity event payloads](https://docs.x.com/x-api/activity/event-payloads) and [Create Post](https://docs.x.com/x-api/posts/create-post). Local regression tests exercise the real NyxID verification, routing, reply authorization and billing code with MongoDB and mocked X/agent HTTP endpoints. A deployed check with an independent account mentioning and replying to the connected account is still required to validate the app's live X access.

### Polling and message semantics

This section applies to legacy X channels with `BILLING_ENABLED=false`. Billing-enabled channels use the webhook lifecycle above.

The adapter's `Ingestion::Poll` and `CredentialResolution::OAuthConnection` hooks drive generic sweeps. Up to eight due bots per adapter are processed concurrently, and a per-bot failure does not stop the other bots in that tick. Each due bot is atomically claimed by `find_one_and_update`, with a 90-second lease renewed every 20 seconds and a 15-minute work deadline. Claims, renewals and cursor commits are fenced; reconnect invalidates the previous claim. All outcomes attempt to release the lease. An abrupt process exit recovers at lease expiry. Backoff, cursors, timestamps and error counts are persisted with BSON datetime compatibility for legacy rows.

[The DM events endpoint](https://docs.x.com/x-api/direct-messages/get-dm-events) does **not** support `since_id`. The adapter requests 100 `MessageCreate` events per page, follows `pagination_token` newest-first until the saved event ID (or an older ID) is reached, and emits the batch chronologically. Own-account messages are skipped. At onboarding it saves the newest existing ID without emitting history; empty accounts use an explicit empty baseline. At most ten pages (about 1,000 events) are walked. When that limit is reached before the cursor, the fetched batch is emitted chronologically and the cursor advances to its newest event after processing. Older DMs are skipped, with a persistent, locally authored `last_poll_notice` on the detail page and CLI; this does not count as an error or fail the bot. A processing error or a 429 before the bounded window completes retains the old cursor. X exposes only the last 30 days of events.

The shared pipeline retains the existing webhook behavior, metadata-only message storage, routing, callback telemetry and touch semantics. X opts into the existing bot/platform/message-ID dedup hook. Overlapping completed batches are suppressed. Callback delivery retains the existing relay semantics: a stored message is not automatically redelivered after callback failure. This is not an exactly-once end-to-end delivery guarantee, and an interrupted provider reply may already have sent an earlier chunk.

X can return HTTP 200 with both DM `data` and an `errors` array ([partial errors](https://docs.x.com/x-api/fundamentals/response-codes-and-errors#partial-errors)). An empty error array is a successful response. When DM events are present, errors limited to optional `user` or `media` expansions do not block those events; missing sender details and unavailable media use the normal normalization fallbacks. Errors about DM events, unclassified errors, malformed error arrays, and nonempty errors without any DM events still fail the poll and retain the cursor.

For reports that only the first DM arrives, test follow-up delivery across separate sweeps: configure an active agent route, send the first DM after onboarding, then send at least six more DMs in the same conversation over successive polls. Check that every message reaches the callback once, the cursor advances, the bot remains active, and an overlapping poll does not redeliver old messages. The database-backed `followup_dms_keep_reaching_agent_across_poll_sweeps` regression exercises this sequence with clean responses, empty error arrays, and unavailable user expansions after the first success. Provider responses and callbacks are mocked; testing the affected X account is still required to establish the incident's cause.

Capture `status`, `error`, `poll_error_count`, `last_polled_at`, `next_poll_at`, `poll_cursor`, and `poll_backoff_until` before reconnecting a failing bot. A failed bot with five poll errors has stopped polling; a future backoff can explain delayed delivery. If polls and the cursor advance, inspect routing and stored message `callback_status`: no route skips delivery, and a failed callback is not retried automatically. Do not capture access tokens or DM bodies in diagnostic logs. After deploying a polling fix, already-failed bots still need reconnecting; reconnect preserves their committed cursor for backlog recovery within X's retention and the bounded polling window.

One-to-one IDs have `{smaller_user_id}-{larger_user_id}` form; group DMs have their own conversation ID. Sender display names and media are normalized from expansions; raw event data is forwarded to the agent and never logged or persisted as a message body. Media URLs are private and may need the connected user's bearer token; the callback never exposes that token. `referenced_tweets` contains shared posts, **not DM reply targets**. Current docs expose no DM reply-reference field; optional `referenced_events` reply references are preserved when supplied, without inventing references from shared posts.

Replies call `POST /2/dm_conversations/{dm_conversation_id}/messages`, supporting existing private/group conversations. Text is split into chunks of at most 10,000 Unicode characters. `attachments` uploads image/video bytes through OAuth2 `POST /2/media/upload` before creating the DM. GIF/video processing is polled for up to 30 seconds before dispatch. Existing `metadata.attachments` still accepts one `{media_id}` uploaded by that user and attaches it to the first text chunk. Existing connections lacking `media.write` must re-consent; the shared OAuth provider defaults include this scope. Editing is unsupported. 401, 403 and 429 errors use local messages and rate-limit retry delay, never upstream error prose. The current v2 schema does not publish a text maximum; the established 10,000-character DM limit could not be independently reconfirmed from the Help Center during implementation (HTTP 403).

### Automation policy and verified limits

Automated replies to inbound DMs are allowed under [X's automation rules](https://help.x.com/en/rules-and-policies/x-automation) subject to user intent, consent and opt-out requirements. Unsolicited automated outbound DMs are not permitted. NyxID's reply authorization binds an inbound message; its initiated-send interface requires human opt-in and addresses an existing X DM conversation. Native capability does not establish recipient consent. Operators remain responsible for consent, opt-out handling and compliant agent responses. X Activity webhooks are available on self-serve plans subject to app access and limits. The polling fallback needs no webhook subscription.

## Problem Statement

Today, connecting an AI agent to a messaging platform requires:

1. **Per-platform bot infrastructure** -- each agent team builds and hosts their own Telegram/Discord/Lark bot
2. **Platform-specific code** -- webhook verification, message parsing, reply formatting differs per platform
3. **No centralized credential management** -- bot tokens scattered across agent configs
4. **No unified audit trail** -- no visibility into which agent handled which message
5. **No agent routing** -- can't send Telegram DMs to Claude and Discord messages to GPT without separate bots

NyxID already solves the equivalent problem for API credentials (proxy gateway). Channel Bot Relay extends this to messaging.

---

## High-Level Architecture

```mermaid
graph TB
    subgraph Messaging Platforms
        TG[Telegram]
        DC[Discord]
        LK[Lark]
        FS[Feishu]
    end

    subgraph NyxID
        WH[Webhook Handlers<br/>per-platform endpoints]
        PA[Platform Adapters<br/>normalize + verify]
        RS[Routing Service<br/>conversation -> agent]
        RL[Relay Service<br/>callback + reply]
        DB[(MongoDB<br/>channel_bots<br/>channel_conversations<br/>channel_messages)]
    end

    subgraph AI Agents
        A1[Claude Code<br/>callback URL A]
        A2[GPT Agent<br/>callback URL B]
        A3[Custom Agent<br/>callback URL C]
    end

    TG -->|webhook| WH
    DC -->|webhook| WH
    LK -->|webhook| WH
    FS -->|webhook| WH

    WH --> PA
    PA --> RS
    RS --> RL
    RL -->|lookup/store| DB

    RL -->|POST normalized msg| A1
    RL -->|POST normalized msg| A2
    RL -->|POST normalized msg| A3

    A1 -->|reply body| RL
    A2 -->|reply body| RL
    A3 -->|reply body| RL

    RL -->|send_reply| PA
    PA -->|platform API| TG
    PA -->|platform API| DC
    PA -->|platform API| LK
    PA -->|platform API| FS
```

---

## Message Flow

### Inbound: Platform -> Agent

```mermaid
sequenceDiagram
    participant U as User (Telegram/Discord/etc.)
    participant P as Platform API
    participant W as NyxID Webhook Handler
    participant A as Platform Adapter
    participant R as Routing Service
    participant RL as Relay Service
    participant DB as MongoDB
    participant AG as AI Agent (callback URL)

    U->>P: Send message
    P->>W: POST /api/v1/webhooks/channel/{platform}/{bot_id}
    W->>A: verify_webhook(headers, body)
    A-->>W: OK (signature valid)
    W->>A: parse_inbound(body)
    A-->>W: Vec<InboundMessage>

    loop For each InboundMessage
        W->>R: resolve_agent(bot_id, conversation_id, sender_id)
        R->>DB: Lookup channel_conversations
        DB-->>R: (agent_api_key_id, callback_url)
        R-->>W: AgentRoute

        W->>DB: Insert channel_message (direction: inbound)

        W->>RL: forward_to_agent(message, callback_url)
        RL->>AG: POST callback_url<br/>X-NyxID-Callback-Token: JWT<br/>X-NyxID-Signature: HMAC<br/>X-NyxID-Message-Id: uuid

        alt Sync Reply (200 + body)
            AG-->>RL: { reply: { text: "..." } }
            RL->>A: send_reply(bot, conversation_id, reply)
            A->>P: Platform send message API
            P->>U: Display reply
            RL->>DB: Insert channel_message (direction: outbound)
        else Async Ack (202)
            AG-->>RL: 202 Accepted
            Note over RL: Agent will call /channel-relay/reply later
        else Error/Timeout
            RL->>DB: Update callback_status = "failed"
        end
    end

    W-->>P: 200 OK (always, to prevent platform retries)
```

### Async Reply: Agent -> Platform

```mermaid
sequenceDiagram
    participant AG as AI Agent
    participant H as NyxID Reply Handler
    participant DB as MongoDB
    participant A as Platform Adapter
    participant P as Platform API
    participant U as User

    AG->>H: POST /api/v1/channel-relay/reply<br/>Authorization: Bearer {api_key}
    H->>DB: Lookup channel_message by message_id
    H->>DB: Verify api_key_id matches conversation's agent
    H->>DB: Lookup channel_bot (get encrypted token)
    H->>A: send_reply(bot, conversation_id, reply)
    A->>P: Platform send message API
    P->>U: Display reply
    H->>DB: Insert channel_message (direction: outbound)
    H-->>AG: 200 OK { platform_message_id: "..." }
```

### Editing a sent reply

`POST /api/v1/channel-relay/reply/update` replaces a previously sent message. `message_id` is the `platform_message_id` returned by `/reply` or `/send`, and `outbound_message_id` is the NyxID `message_id` from the same response:

```json
{ "message_id": "<platform_message_id>", "outbound_message_id": "<message_id>", "reply": { "text": "Updated response", "metadata": null } }
```

NyxID looks for the target only among rows the caller may edit. An agent API key sees the rows it sent, and a reply token sees only the reply its key sent to its bound inbound message in its bound conversation. A platform message ID is not a global identifier: Telegram numbers messages per chat, so a key that serves several chats routinely has several rows with the same ID. When more than one row matches and `outbound_message_id` is absent, the edit returns `409 conflict` rather than choosing one. On Telegram and telegram-new (adapters whose `message_ids_are_chat_scoped()` is true), API-key edits must send `outbound_message_id` and otherwise return `400 validation_error`: even a unique match could be another chat's message once the intended row has expired or was never stored. Other platforms accept edits without it for compatibility, but clients should always send it. Reply-token edits are anchored to one chat and may omit it.

An agent API key can edit anchored replies and agent-initiated messages it sent, while it remains the conversation's assigned agent (otherwise `403`). A per-callback reply token can edit only replies anchored to its bound inbound message, and only after its JTI has been consumed by `/reply`; it cannot edit initiated rows. Rows outside either scope return `404`. After the target row is resolved, every edit also passes the per-message rate limit, which is keyed on that row. Identical platform message IDs in other chats or tenants therefore never share a bucket. Discovery exposes native support as `capabilities.edit`.

- **Telegram / telegram-new:** text messages use `editMessageText`; media captions use `editMessageCaption` after Telegram returns exactly `Bad Request: there is no text in the message to edit`. Both use the chat ID, numeric message ID, and `parse_mode: "Markdown"`, matching sends. Ordinary bot messages are subject to Telegram's 48-hour edit window. An identical edit (`message is not modified`) succeeds idempotently.
- **Discord:** `PATCH` edits `content` on any bot message, including media messages. NyxID always edits through `PATCH /channels/{channel_id}/messages/{message_id}` with the bot token and never through the interaction-webhook edit endpoint, so edits that Discord only permits via the interaction token (for example ephemeral interaction responses) are not supported and surface as a classified refusal.
- **Slack:** `chat.update` with `channel`, `ts`, and `text`; `reply.metadata.blocks` passes through just as it does on sends. File/image messages cannot be edited by `chat.update` and surface as classified refusals. Existing Slack rate-limit error handling is preserved.
- **Lark / Feishu:** text edits via `PUT /im/v1/messages/{id}` and card edits via `PATCH /im/v1/messages/{id}` using `reply.metadata.card`. These edits do not apply to file/image messages and surface as classified refusals.
- **WhatsApp / X / OpenClaw:** `501 edit_unsupported`; WhatsApp and X have no message edit API. Device channels return `400 device_channel_reply_not_allowed`.

The edit address comes first from the outbound row. For pre-0.21.0 rows without that address, NyxID reads the parent inbound row, then falls back to the conversation's concrete address. Wildcard (`"*"`) and empty addresses cannot reach a platform; no resolvable address returns `channel_conversation_not_addressable` before dispatch.

Known permanent Telegram, Discord, and Slack refusals (including missing messages, messages the bot cannot edit, and closed edit windows) return `channel_conversation_not_reachable`; other upstream diagnostics use the existing bounded platform-error response. Lark/Feishu retain their existing error classification. Edits update only the outbound timestamp and routing-only audit metadata; message bodies and metadata are never persisted (ADR-013). Initiated edit audits retain a null `inbound_message_id`.

### Agent-initiated messages

`POST /api/v1/channel-relay/send` sends to a configured conversation without an inbound message. Use it for an opted-in digest, alert, or job-completion notification. Existing and newly created conversations default to `allow_agent_initiated: false`. Only a human session can set this field through conversation create/update; the agent cannot grant itself permission. Organization sends require owner write access for human callers, or the exact assigned active agent key for agent callers.

```mermaid
sequenceDiagram
    participant C as Agent or human owner
    participant H as NyxID send handler
    participant L as Shared conversation limiter
    participant DB as MongoDB
    participant A as Platform adapter
    C->>H: POST /channel-relay/send (conversation_id, message, idempotency_key?)
    H->>L: Check channel_initiate bucket before authentication
    H->>H: Authenticate API key or human session
    H->>DB: Load active conversation, authorize owner/assigned live key
    H->>H: Require opt-in, reject device/wildcard/empty address
    H->>DB: Load active bot, verify owner and platform scope
    H->>H: Check adapter initiated_send capability, validate content
    opt Idempotency key supplied
        H->>DB: Insert pending claim with delivery fingerprint
        DB-->>H: Claimed, matching sent receipt, or 409 conflict
    end
    H->>A: send_reply(chat_id, reply_to=None, text, metadata)
    A-->>H: Platform acceptance receipt or classified refusal
    H->>DB: Store outbound routing metadata; mark claim sent
    H-->>C: 200 (message_id, platform_message_id?)
```

Request and response:

```http
POST /api/v1/channel-relay/send
Authorization: Bearer <assigned-agent-key-or-first-party-human-access-token>
Content-Type: application/json

{
  "conversation_id": "660e8400-e29b-41d4-a716-446655440000",
  "message": { "text": "Your report is ready.", "metadata": null },
  "idempotency_key": "report-2026-09-15"
}
```

```json
{ "message_id": "550e8400-e29b-41d4-a716-446655440000", "platform_message_id": "12345" }
```

Browser session cookies and the CLI's first-party human access JWTs are accepted. OAuth-app access tokens, relay access tokens, delegated tokens, service accounts, and per-callback reply tokens are not accepted. The new endpoint does not relax any existing router middleware. Request parsing and UUID syntax validation precede the per-conversation check; authentication and resource lookups follow it. The limiter itself uses MongoDB shared coordination, at `1` message/second with burst `5` by default. Every syntactically valid attempt, including a duplicate, consumes rate capacity.

Gate order is rate limit → authentication → active conversation → assigned-key liveness or human owner write access → opt-in → device guard → concrete address → active bot and matching owner/platform → adapter capability → content → idempotency claim → platform send → metadata persistence. Missing/inactive conversations are not-found-shaped. `"*"` and empty addresses are catch-all routes and cannot be used to initiate; create a specific chat route first. Device channels have no outbound transport. Platform-specific rich content is still adapter-validated (`metadata.card` is Lark/Feishu-only).

Threading accepts durable platform metadata such as Telegram `message_thread_id` or Slack `thread_ts`. There is no internal `message_id` reply anchor. `metadata.interaction_thread_id` is rejected here because Discord interaction follow-up credentials expire and can redirect dispatch away from the conversation. Other supported metadata passes directly to the adapter; it is never persisted.

**Discovery:** `GET /api/v1/channel-relay/conversations?page=1&per_page=50` requires an active API key and returns only its active assigned rows. The response is `{ conversations, total, page, per_page }`, capped at 100 items per page. Each item contains only `{ id, platform, platform_conversation_type, addressable, allow_agent_initiated, capabilities, last_message_at }`. Wildcard and device rows are not addressable; device capabilities are all false. Wildcards retain the adapter's capability values. Owner conversation responses expose the same capabilities plus the human-set opt-in.

**Idempotency:** optional keys contain 1–128 characters and are scoped to a conversation. A unique composite MongoDB `_id` (`conversation_id:idempotency_key`) is claimed *before* contacting the platform. `channel_send_claims` stores a SHA-256 fingerprint over recursively key-sorted JSON `{text, metadata}`, an attempt fence, `pending`/`sent` state, IDs, and a 24-hour BSON `expires_at`. It stores no message content. Reusing the key with different content returns 409. A matching `sent` claim returns the original IDs with 200; a matching `pending` claim returns 409 (send in progress or delivery outcome uncertain). Credential preparation failures delete the pending claim. WhatsApp also releases it on a confirmed, locally classified Graph refusal before any message component was accepted (for example an invalid token, unauthorized test recipient, outside-window message or rate limit). WhatsApp transport errors, 5xx responses, invalid/oversized responses, missing message IDs, and partial acceptance retain the pending claim. Other adapters retain their existing dispatch-error behavior. There is no automatic retry or queue.

A crash after claiming, a platform timeout after acceptance, partial delivery of a multi-chunk message, or a database failure after acceptance cannot provide an exactly-once guarantee. After successful platform acceptance, metadata/receipt persistence failures retain the pending claim so retries cannot silently duplicate the message. A crash can leave that claim pending until TTL cleanup. MongoDB's TTL monitor runs asynchronously: claims can live slightly longer than 24 hours; once removed, the same key may send again. WhatsApp retains the claim for ambiguous or partial sends, so the same key returns 409 without dispatching again. Sending with a new key can still duplicate accepted components. Other adapters keep their existing claim-release semantics. A platform message ID proves only that the platform accepted a send, not that any recipient received or read it.

**Refusals:** disabled opt-in returns `channel_agent_initiate_not_allowed` (403), non-addressable routes return `channel_conversation_not_addressable` (400), unsupported transports return `channel_platform_send_unsupported` (501), and known permanent target refusals return `channel_conversation_not_reachable` (400). Examples include blocked/kicked Telegram bots, missing Discord access, archived Slack chats, missing Lark membership, WhatsApp undeliverable recipients/outside-window text, and X recipient DM refusals. WhatsApp outside its 24-hour window requires an approved template message. Classification belongs to adapters and also improves ordinary replies: being kicked from a chat is equally non-retryable for both paths. Unmatched Telegram, Discord, Slack, and Lark/Feishu refusals remain 502 with the platform description limited to 200 characters so callers can correct malformed payloads; matched refusals expose only the adapter-owned marker. WhatsApp retains locally authored reasons and X retains its existing 403 handling; existing Slack rate-limit handling is unchanged.

Audit event `channel_message_initiated` contains only `conversation_id`, `platform`, `channel_bot_id`, and `agent_api_key_id`. `channel.reply_sent` telemetry uses `reply_mode: "initiated"` with the existing 10% conversation-hash sampling. Neither audit, claims, logs, nor message rows retain text, cards, or metadata bodies (ADR-013).

Human setup and test-send:

```bash
nyxid channel-bot route create --bot-id <BOT_ID> --agent-key-id <KEY_ID> \
  --conversation-id <PLATFORM_CHAT_ID> --allow-agent-initiated
nyxid channel-bot route update <ROUTE_ID> --allow-agent-initiated true
nyxid channel-bot send --conversation <ROUTE_ID> --text 'Test notification' --idempotency-key test-1
nyxid channel-bot route update <ROUTE_ID> --allow-agent-initiated false
```

The conversation detail page provides the same opt-in, explains unprompted messaging, disables unsupported/non-addressable routes, and offers an explicit test-send form.

#### Outbound capability model

This declaration-and-contract-test model follows OpenClaw's `ChannelOutboundAdapter`: outbound is target-addressed, and a reply is an optional anchor on the same native send operation.

| Adapter | initiated_send | reply_to | thread | edit | Media in | Media out | Thread metadata |
|---|---|---|---|---|---|---|---|
| telegram | true | true | true | true | image, file, audio, video | image, file, audio, video | `message_thread_id` |
| telegram-new | true | true | true | true | image, file, audio, video | image, file, audio, video | Delegates to Telegram |
| discord | true | false | false | true | image, file, audio, video | image, file, audio, video | — (interaction follow-up is a reply-only mechanism) |
| lark | true | false | false | true | image, file, audio, video | image, file, audio, video | — |
| feishu | true | false | false | true | image, file, audio, video | image, file, audio, video | — |
| slack | true | true | true | true | image, file, audio, video | image, file, audio, video | `thread_ts` |
| whatsapp | true | true | false | false | image, file, audio, video | image, file, audio, video | — |
| x | true | false | false | false | image, video | image, video | — |
| openclaw | false | false | false | false | none | none | — |

Every adapter must implement `outbound_capabilities` and `media_capabilities`; there is no trait default. Flags describe what the native transport preserves, not generic platform possibilities or permission to send to every recipient. Tests exercise the actual request builders/native edit implementation. The additive `send_reply_outcome` and `send_bound_reply_outcome` hooks carry WhatsApp component IDs and certainty. Their defaults project existing sends, including each adapter’s `send_bound_reply` override, so adapter-specific fences and behavior remain in place. OpenClaw explicitly returns `ChannelPlatformSendUnsupported` and performs no HTTP request; it cannot report a successful empty receipt for a no-op. X sends address existing DM conversation IDs; WhatsApp messaging-window and recipient restrictions still apply. API keys can edit initiated messages on adapters with native edit capability; the edit audit records a null inbound message ID. Reply tokens remain bound to an inbound message and cannot edit initiated rows.

### Bot Registration

```mermaid
sequenceDiagram
    participant U as User (authenticated)
    participant H as NyxID Bot Handler
    participant A as Platform Adapter
    participant P as Platform API
    participant DB as MongoDB

    U->>H: POST /api/v1/channel-bots<br/>{ platform: "telegram", bot_token: "123:ABC" }
    H->>A: verify_bot_token(bot_token)
    A->>P: GET /getMe (or equivalent)
    P-->>A: { id: "bot123", username: "MyBot" }
    A-->>H: BotIdentity

    H->>DB: Check max_bots_per_user limit
    H->>H: Encrypt bot_token (AES-256)
    H->>H: Generate webhook_secret (32 bytes)
    H->>DB: Insert channel_bot (status: pending_verification)

    H->>A: register_webhook(bot, webhook_url, secret)
    A->>P: POST /setWebhook (or equivalent)
    P-->>A: OK

    H->>DB: Update channel_bot (status: active, webhook_registered: true)
    H-->>U: 201 Created { id, platform, bot_username, status }
```

---

## Agent Routing & Isolation

### How Conversations Map to Agents

```mermaid
graph TD
    MSG[Inbound Message] --> R{Routing Service}

    R -->|Step 1| SS{Sender-specific match<br/>in this conversation?}
    SS -->|Yes| AGENT[Route to bound agent]
    SS -->|No| EC{Step 2: Exact conversation<br/>match without sender?}
    EC -->|Yes| AGENT
    EC -->|No| DA{Step 3: Default agent<br/>for this bot?}
    DA -->|Yes| AGENT
    DA -->|No| UNROUTED[Log as unrouted<br/>Optional: send 'not configured' reply]

    AGENT --> CB[POST agent callback_url]
```

### Integration with Agent Isolation (PR #132)

The callback URL lives on the **ApiKey** (the agent), not on individual conversation routes. When a user sets up an agent on NyxID (`nyxid api-key create --platform claude-code --callback-url "https://my-agent.example.com/webhook"`), they register the agent's callback URL as part of the agent configuration. Conversation routes then just say "send to this agent" -- NyxID already knows how to reach it.

This means:
- **`ApiKey.callback_url`** (new field) -- where NyxID sends channel messages for this agent
- **`ChannelConversation.agent_api_key_id`** -- which agent handles this conversation (callback URL resolved from the API key)
- No `agent_callback_url` on the conversation route -- the URL is a property of the agent, not the conversation

```mermaid
graph LR
    subgraph Channel Relay Layer
        BOT[Channel Bot<br/>Telegram / Discord / Lark]
        CONV[Channel Conversation<br/>agent_api_key_id]
    end

    subgraph Agent Isolation Layer
        AK[ApiKey<br/>platform, callback_url<br/>rate limits, scopes]
        ASB[AgentServiceBinding<br/>per-agent credential override]
    end

    subgraph Proxy Layer
        PS[Proxy Service<br/>credential injection<br/>scope enforcement]
    end

    CONV -->|references| AK
    AK -->|scopes| ASB
    ASB -->|overrides credentials at| PS

    BOT -->|receives messages for| CONV
    CONV -->|forwards to agent via| AK
```

The relay and proxy are **parallel paths, not nested**:

- **Relay path**: Platform -> NyxID webhook -> agent callback URL (message forwarding)
- **Proxy path**: Agent -> NyxID proxy -> downstream API (credential injection)

An agent receiving a message via the relay can then call external APIs through NyxID's proxy using its scoped API key. The agent isolation scope enforcement applies to the proxy call, not the relay.

---

## Data Model

### Entity Relationship

```mermaid
erDiagram
    User ||--o{ ChannelBot : registers
    User ||--o{ ApiKey : owns
    ChannelBot ||--o{ ChannelConversation : has
    ApiKey ||--o{ ChannelConversation : "routes to"
    ApiKey ||--o{ AgentServiceBinding : "binds credentials via"
    ChannelConversation ||--o{ ChannelMessage : contains
    ChannelBot ||--o{ ChannelMessage : "sent/received via"

    ApiKey {
        string id PK "existing model -- new field added"
        string name "human-readable agent name"
        string platform "claude-code | codex | openclaw | generic"
        string callback_url "NEW: where to POST channel messages"
        int rate_limit_per_second "per-agent rate limit"
        int rate_limit_burst "per-agent burst"
        array allowed_service_ids "proxy scope"
        array allowed_node_ids "proxy scope"
    }

    ChannelBot {
        string id PK
        string user_id FK
        string platform "telegram | discord | lark | feishu"
        string label
        bytes bot_token_encrypted
        string platform_bot_id
        string platform_bot_username
        bool webhook_registered
        string webhook_secret_hash
        string status "pending | active | failed | invalid"
        string app_id "Lark/Feishu only"
        bytes app_secret_encrypted "Lark/Feishu only"
        bytes lark_verification_token_encrypted "Lark/Feishu only"
        bytes lark_encrypt_key_encrypted "Lark/Feishu only, optional"
        string public_key "Discord only"
        bool is_active
        datetime created_at
        datetime updated_at
    }

    ChannelConversation {
        string id PK
        string user_id FK
        string channel_bot_id FK
        string platform
        string platform_conversation_id
        string platform_conversation_type "private | group | channel"
        string platform_sender_id "optional: per-sender routing in groups"
        string agent_api_key_id FK "which agent handles this"
        bool default_agent "fallback route for unmatched conversations"
        bool allow_agent_initiated "human opt-in, default false"
        bool is_active
        datetime last_message_at
        datetime created_at
        datetime updated_at
    }

    ChannelMessage {
        string id PK
        string channel_bot_id FK
        string conversation_id FK
        string user_id FK
        string direction "inbound | outbound"
        string platform
        string platform_message_id
        string sender_platform_id
        string sender_display_name
        string content_type "text | image | file | audio | video"
        array attachments "StoredAttachment[]: inbound routing metadata only"
        string agent_api_key_id FK
        string callback_status "pending | delivered | failed | timeout"
        string reply_to_message_id FK "for outbound: which inbound this replies to"
        string platform_reply_message_id
        datetime created_at "TTL: 30 days"
    }
```

### MongoDB Indexes

| Collection | Index | Type | Purpose |
|---|---|---|---|
| `channel_bots` | `{ user_id: 1, platform: 1 }` | Unique | One bot per platform per user |
| `channel_bots` | `{ platform: 1, platform_bot_id: 1 }` | Standard | Webhook bot lookup |
| `channel_conversations` | `{ channel_bot_id: 1, platform_conversation_id: 1 }` | Unique | One mapping per conversation |
| `channel_conversations` | `{ user_id: 1, platform: 1 }` | Standard | List user's routes |
| `channel_conversations` | `{ agent_api_key_id: 1 }` | Standard | Find routes for an agent |
| `channel_send_claims` | `{ _id: 1 }` | Unique | Conversation/key delivery identity |
| `channel_send_claims` | `{ expires_at: 1 }` | TTL (expireAfterSeconds 0) | Remove claims after 24h |
| `channel_messages` | `{ conversation_id: 1, created_at: -1 }` | Standard | Conversation history |
| `channel_messages` | `{ created_at: 1 }` | TTL (30d) | Auto-cleanup |
| `channel_delivery_receipts` | `{ identity_key: 1 }` | Unique | Exact bot/owner/phone/WABA/recipient/message identity |
| `channel_delivery_receipts` | `{ expires_at: 1 }` | TTL (expireAfterSeconds 0) | Remove receipt observations after 30d |
| `event_dedup_records` | Existing `_id` and `expires_at` indexes | Unique / TTL | WhatsApp admission claim and committed marker |

---

## Platform Adapter Trait

```mermaid
classDiagram
    class PlatformAdapter {
        <<trait>>
        +platform_id() str
        +display_name() str
        +ingestion() Ingestion
        +managed_onboarding() OptionalDescriptor
        +platform_credentials() OptionalDescriptor
        +outbound_capabilities() OutboundCapabilities
        +media_capabilities() MediaCapabilities
        +fetch_attachment(http, credentials, attachment, max_bytes) FetchedMedia
        +registration() RegistrationDescriptor
        +registration_token(fields) Secret
        +updated_token(current, fields) OptionalSecret
        +build_verify_secrets(keys, bot) PlatformVerifySecrets
        +webhook_policy(body) WebhookPolicy
        +subscription_handshake(bot, query) String
        +prepare_webhook(bot, secrets, headers, body) PreparedWebhook
        +verify_webhook(bot, secrets, headers, body) Result
        +parse_inbound(body) Result~Vec~InboundMessage~~
        +reply_context(thread_id, created_at, metadata)
        +supports_reply_metadata(metadata) bool
        +send_reply(http, credentials, conversation_id, reply) Result~String~
        +edit_reply(http, credentials, conversation_id, platform_message_id, edit) Result
        +register_webhook(http, bot, url, secret) Result
        +verify_bot_token(http, credentials) Result~BotIdentity~
    }

    class TelegramAdapter {
        +platform_id() "telegram"
        -Secret header verification
        -Reuses telegram_service.rs
        -No challenge needed
    }

    class DiscordAdapter {
        +platform_id() "discord"
        -Ed25519 signature verification
        -PING/PONG challenge
        -Interaction-based model
    }

    class LarkFamilyAdapter {
        -base_url: String
        +platform_id() "lark" or "feishu"
        -Verification Token check
        -Optional Encrypt Key signature + AES-256-CBC decrypt
        -url_verification handled after bot lookup
        -App access token caching
    }

    class OutboundReply {
        +text: OptionalString
        +attachments: MaterializedAttachment[]
        +metadata: OptionalJSON
    }
    class MaterializedAttachment {
        +kind: MediaKind
        +bytes: Bytes_ephemeral
        +filename: OptionalString
        +mime_type: OptionalString
        +caption: OptionalString
    }
    class FetchedMedia {
        +bytes: Bytes_ephemeral
        +mime_type: OptionalString
        +filename: OptionalString
    }
    OutboundReply *-- MaterializedAttachment
    PlatformAdapter ..> FetchedMedia
    PlatformAdapter <|.. TelegramNewAdapter
    PlatformAdapter <|.. XAdapter
    PlatformAdapter <|.. OpenClawAdapter
    PlatformAdapter <|.. TelegramAdapter
    PlatformAdapter <|.. DiscordAdapter
    PlatformAdapter <|.. LarkFamilyAdapter
    PlatformAdapter <|.. SlackAdapter
    PlatformAdapter <|.. WhatsAppAdapter

    class WhatsAppAdapter {
        +platform_id() "whatsapp"
        -Meta App Secret HMAC-SHA256
        -GET Verify Token handshake
        -Phone Number ID event filter
        -Immediate POST acknowledgment
        -Cloud API text, template, interactive replies
    }

    note for LarkFamilyAdapter "Single implementation,\nregistered twice:\nlark = larksuite.com\nfeishu = feishu.cn"
```

### Platform-Specific Notes

| Platform | Auth Model | Webhook Verification | Challenge | Send Reply API |
|---|---|---|---|---|
| **Telegram** | Bot token (`123:ABC...`) | `X-Telegram-Bot-Api-Secret-Token` header (constant-time) | None | `POST /bot{token}/sendMessage` |
| **Discord** | Bot token + Application ID | Ed25519 signature (`X-Signature-Ed25519` + `X-Signature-Timestamp`) | `PING` -> `PONG` interaction response | `POST /channels/{id}/messages` with `Authorization: Bot {token}` |
| **Lark** | App ID + App Secret for tenant access token; Verification Token for inbound webhook auth; optional Encrypt Key for signed/encrypted delivery | Always verify Verification Token. If Encrypt Key is configured, also require `X-Lark-Signature` with `hex(SHA256(timestamp + nonce + encrypt_key + raw_body))`, then decrypt `{\"encrypt\":\"...\"}` using AES-256-CBC, PKCS7, IV = first 16 bytes, key = `SHA256(encrypt_key)` | `url_verification` -> verify token first, then echo `challenge` | `POST /im/v1/messages` with tenant access token |
| **Feishu** | Same as Lark | Same as Lark | Same as Lark | Same as Lark, different base URL (`open.feishu.cn`) |
| **Slack** | Bot user OAuth token (`xoxb-`) + Signing Secret | HMAC-SHA256 over `v0:timestamp:body`, five-minute replay window | `url_verification` | `POST /api/chat.postMessage` |
| **WhatsApp** | API Setup token for testing or System User token for ongoing operation; Phone Number ID + Meta App Secret; optional WABA ID | `X-Hub-Signature-256: sha256=<HMAC-SHA256(app_secret, raw_body)>` | GET subscription: constant-time SHA-256 Verify Token check, raw challenge as `text/plain` | `POST /{version}/{phone_number_id}/messages` with Bearer auth |

For inbound attachment downloads, the channel bot app needs any one of `im:message`, `im:message:readonly`, or `im:message.history:readonly`, configured in the Lark/Feishu developer console. The official [Obtain resource files in messages](https://open.larksuite.com/document/uAjLw4CM/ukTMukTMukTM/reference/im-v1/message-resource/get) API (`/reference/im-v1/message-resource/get`) lists these alternatives. Missing permission causes `channel_media_fetch_failed`. These bot-app permissions are separate from the service connection's Base field/read/create and Drive upload permissions.

Lark/Feishu registration requires `app_id`, `app_secret`, and `verification_token`, with optional `encrypt_key`. The form and CLI do not require `bot_token`; the API accepts omission and continues accepting the ignored placeholder from older clients. The adapter constructs its stored credential from App ID and App Secret and exchanges it for a tenant access token.

For the Lark/Feishu platform family, `register_webhook()` remains a no-op. Configure the webhook URL and subscribe to both `im.message.receive_v1` and `card.action.trigger` only in the Lark/Feishu Developer Console. The console inputs map to NyxID fields as follows:

- **App ID** -> `ChannelBot.app_id`
- **App Secret** -> `ChannelBot.app_secret_encrypted`
- **Verification Token** -> `ChannelBot.lark_verification_token_encrypted`
- **Encrypt Key** -> `ChannelBot.lark_encrypt_key_encrypted` (optional)

### Adding New Platforms

Managed registration is optional. Adapters declare `platform_credentials()` (provider key, fields, masking, setup checklist), `managed_onboarding()` (safe bootstrap and accepted completion fields), and implement completion/setup/re-registration hooks. `platform_webhook()` opts into the shared callback; handshake and target extraction remain adapter-owned. The admin inventory comes from `registered_adapters()` in the same registry used by `resolve_adapter`. Generic handlers never branch on an IM name. Registration fields can declare `platform_fallback`; managed verification resolves missing bot secrets from encrypted platform credentials on demand. Existing adapters keep their default hooks.

Implement `PlatformAdapter` in `backend/src/services/channel_adapters/<platform>.rs`, then register its module and add it to `services/channel_adapters/mod.rs::resolve_adapter`, the single runtime registry. The handler module re-exports it for existing callers.

- `registration()` declares field names, human labels, required/secret/patchable/clearable flags, storage columns, and which secrets verify webhooks. It also declares fields that rebuild outbound tokens, automatic versus dashboard webhook setup, the one-time secret label, and setup instructions. Handler and service validation, encrypted persistence, safe configuration responses, and secret decryption follow this descriptor.
- `BotCredentials` carries the outbound token and platform bot identity separately. `registration_token` and `updated_token` own token construction; only the existing Lark/Feishu adapter retains its legacy composite format. Token rotation verifies the replacement before storing it. Label-only updates never decrypt credentials.
- `PlatformVerifySecrets` is a generic map of zeroizing secret values, with redacted Debug output. Its default builder decrypts only fields marked as webhook secrets; an adapter may override `build_verify_secrets`.
- `webhook_policy` selects inline processing, immediate acknowledgment with background processing, or the existing challenge-only response. Lark/Feishu challenges remain inside authenticated `prepare_webhook`. `subscription_handshake` defaults to denial and handles GET protocols when supported.
- `prepare_webhook` verifies and preprocesses raw payloads before normalization. WhatsApp filters app-wide events by the bot's Phone Number ID here. `reply_context` translates stored thread context into outbound metadata; `supports_reply_metadata` admits platform-native content without text.
- `dedup_inbound_by_platform_message_id` defaults to false and selects the existing metadata lookup used by X. WhatsApp also opts into `atomic_inbound_admission`, which claims the bot/owner/message identity and commits inbound metadata in one transaction before dispatch. Keep deduplication off for platforms such as Telegram where edited messages legitimately reuse an ID.
- The public `/api/v1/webhooks/channel/{platform}/{bot_id}` GET/POST handlers use these hooks. No new per-platform handler or generic pipeline branch is needed. Unknown platforms and OpenClaw return 404 on both methods; OpenClaw has its own integration path.
- Mirror the registration fields in `frontend/src/lib/channel-platforms.ts`, extend the frontend platform type/schema and CLI options, and add normalization, verification, lifecycle, and reply tests. Callback payloads stay platform-independent.

### WhatsApp Cloud API Setup

This integration supports **WhatsApp Business Platform through Meta's Cloud API**. The consumer **WhatsApp Business App has no API** and is not supported by this adapter. Twilio-hosted WhatsApp uses a different API and is out of scope.

1. For controlled tests before App Review, use the temporary access token from Meta's WhatsApp > API Setup and add and verify the intended test recipients there, following [Meta's getting-started guide](https://developers.facebook.com/documentation/business-messaging/whatsapp/get-started). For ongoing operation, create a System User in Meta Business Settings, assign the WhatsApp Business Account, and generate a token with `whatsapp_business_messaging` and `whatsapp_business_management` permissions and access to the phone number. A System User token is not required for the developer test flow. Obtain the **Phone Number ID** from WhatsApp > API Setup and the **Meta App Secret** from the app's Basic settings. The Phone Number ID is neither the display phone number nor the Meta App ID.
2. Register the bot:

   ```bash
   nyxid channel-bot register --platform whatsapp --label "WhatsApp Support" \
     --token-env WHATSAPP_ACCESS_TOKEN --phone-number-id 123456789 \
     --app-secret-env META_APP_SECRET --waba-id 987654321
   ```

   `--waba-id` is optional. API fields are `bot_token`, `phone_number_id`, `app_secret`, and optional `waba_id`. The Phone Number ID is stored in `platform_bot_id`, the encrypted access token in `bot_token_encrypted`, the encrypted App Secret in `app_secret_encrypted`, and WABA ID in the optional generalized `app_id` column. API responses expose the identifier as `waba_id`, never as an App ID.
3. Copy the returned **Callback URL** and **Verify Token** into Meta App Dashboard > WhatsApp > Configuration, then verify and save. The CLI prints both, and the web creation dialog shows them before navigation. The generated `webhook_secret` is shown once; only its SHA-256 hash is stored. Detail/list responses never return it. Keep it in a password manager; if lost, re-register the bot. The GET handshake returns the unquoted challenge with 200, or 403 for missing/invalid parameters or token.
4. Subscribe to the **messages** webhook field. Separately subscribe the app to the WABA with `POST /{version}/{WABA_ID}/subscribed_apps` and Bearer authentication, following Meta's dashboard/subscription instructions. NyxID's `register_webhook` is a no-op; supplying `waba_id` binds inbound events to that account but does not perform this API call.
5. Register the intended runtime with an agent key owned by the same person or organization as the bot, then add a conversation route to that key. The key must be active, unexpired, have a nonblank callback URL, and not be an assistant chat key. Route creation, updates, resolution, and the final key read before callback dispatch enforce these conditions. Setting a callback URL alone does not register a runtime. NyxID shows missing keys and routes in the bot owner's scope; it does not provision or widen keys. Send the business phone a message: `conversation_id` is the sender's WhatsApp ID (`wa_id`/`from`, digits without `+`), and conversation type is always `private`. Use these digits when creating a route. Outbound recipients also accept one leading `+` and spaces/dashes, which the adapter removes before sending.

Meta subscriptions are app-wide: each bot accepts only the `whatsapp_business_account` object, `messages` field, `whatsapp` product, its exact `metadata.phone_number_id`, and its configured WABA ID when present. The bot remains `pending_webhook` until a matching signed event contains a usable inbound message or a recognized status with message ID, recipient and timestamp. Empty or unrelated signed events do not activate it. Activation checks the current owner, credentials and lifecycle; a concurrent first message may continue when another event already activated the same bot under the same credentials. Managed activation also fences rotation of the shared Meta secret. A shared app callback does not relay the app's other numbers. Use separate apps or Meta's supported [webhook overrides](https://developers.facebook.com/documentation/business-messaging/whatsapp/webhooks/override) to route each number to its matching NyxID bot URL.

Rotate credentials with `nyxid channel-bot update <BOT_ID> --token-env WHATSAPP_ACCESS_TOKEN --app-secret-env META_APP_SECRET`, or PATCH `bot_token`/`app_secret`. Rotation preserves the Verify Token and phone identity. The Verify Bot action preserves WhatsApp's subscription secret; existing platforms retain their original verification lifecycle and response statuses.

**Verify Bot checks access to the configured phone through Graph.** Detail and successful verify responses include optional `last_verification` metadata: a check ID, `pending`/`verified`/`failed`/`incomplete` status, start/completion timestamps, and a locally authored error message. Failed checks are persisted before returning the API error. They remain separate from historical `status` and `webhook_registered`; a prior webhook observation does not prove that current credentials or routing work. Credential replacement invalidates the result, and a superseded attempt cannot overwrite a newer one. A pending check older than its 30-second deadline is displayed as incomplete without mutating credentials or the stored attempt. Managed checks use the exact loaded shared-secret snapshot; rotation invalidates the observation. Managed activation and verification use MongoDB transactions to fence shared-secret changes, requiring a replica set.

WhatsApp Graph calls have a 20-second request/body deadline and a 256 KiB JSON response limit, including streamed bodies. Credential checks additionally bound credential resolution and the adapter call to the check's 30-second deadline; database availability remains a separate dependency. Errors report safe local guidance and numeric Graph codes, excluding upstream prose, URLs, tokens and app-secret proofs. An opaque gateway error means no usable verification result was received; it does not establish whether the cause was the token, Graph, or server connectivity. Meta's developer test number and approved test recipients can support real conversations under Standard Access before App Review. Configure those external account inputs and the same-owner runtime separately, then validate an incoming message, callback acceptance, asynchronous reply and handset receipt.

Inbound text, button replies, and interactive button/list selections normalize to text (title preferred, ID fallback). Image and sticker attachments use the `image` category; audio, video, and documents use `audio`, `video`, and `file`. Captions become text. Each attachment carries its media ID as `file_key` and a versioned Graph lookup URL. Agents fetch the NyxID `download_url` with their assigned API key or callback reply token. NyxID performs the two Graph requests with the bot bearer; bot credentials never leave the relay. Location and contact cards normalize to readable text; full details remain in the individual message's `raw_platform_data`. Orders use `unknown`; reaction, system, unsupported, unknown, and unrecognized types are skipped. Status-only and error-only deliveries are acknowledged without dispatch. Every message in a batch is processed independently.

Replies support plain text, `metadata.template` objects, or `metadata.interactive` objects. Template and interactive content cannot be combined. Metadata cannot override the recipient, `recipient_type`, or `messaging_product`. `context.message_id` carries the original inbound message ID. Text splits into sequential messages of at most 4096 Unicode characters and returns the last `wamid`. A later chunk failure can leave earlier chunks delivered; there is no automatic outbound retry. Editing is unsupported. Outside the 24-hour customer service window, send an approved template; Graph code 131047 identifies that condition. Rate-limit and delivery errors use locally authored messages and numeric codes, never upstream text that could echo credentials.

Message history shows callback status on the **inbound** row: `delivered` means **Callback accepted** by the runtime. For outbound WhatsApp rows, `platform_message_id` preserves the final accepted component ID for existing callers. New rows also record every accepted component ID, the expected component count, whether the send completed, whether acceptance is uncertain, and a safe numeric NyxID failure code. Components include split text, attached media and separate audio captions; sends are capped at 64 components before dispatch. A failure after partial acceptance persists the known IDs before returning an error and does not complete an initiated-send claim. Neither acceptance nor the final ID alone proves recipient delivery.

Authenticated `messages` status events are filtered by object, product, Phone Number ID and configured WABA before observation. Only `sent`, `delivered`, `read`, `played` and `failed` are recognized. The metadata-only receipt collection stores UUID v4 IDs, a unique deterministic identity key, per-status provider times and up to eight nonnegative 32-bit provider error codes. No raw webhook, message content, pricing, opaque callback data or freeform error prose is stored. Message IDs are limited to 512 ASCII graphic bytes and phone/recipient IDs to 32 digits. Group and participant-scoped receipts are rejected because this adapter sends to individuals.

Receipt persistence errors are logged without document/error contents and do not prevent valid messages in a mixed webhook from reaching admission. Receipt processing remains best-effort after immediate provider acknowledgement. Receipts can arrive before the outbound row. Duplicate and reordered events converge by taking the latest provider timestamp for each status and a bounded, sorted set of codes; read or played implies delivery when a delivered timestamp is absent. Positive delivery evidence takes precedence over a failure when both are present, while both times remain visible. At most 128 valid receipts are processed per webhook. Receipt storage and admission markers expire after 30 days; receipts older than 30 days or more than one day in the future are ignored. Logical receipt expiry applies before asynchronous TTL cleanup.

Both authorized message-list endpoints enrich at most 100 rows with one bounded receipt lookup. Joins require exact bot, owner, sending phone, configured WABA, recipient and provider message ID matches. There is no unmatched receipt endpoint. The optional `delivery` DTO contains `status`, `complete` (all components accepted and recorded), `expected_components`, `recipient_only`, `failure_code`, and `components` with safe status/time/code metadata. Whole-reply sent/delivered/read/played status requires a complete component set and corresponding evidence for every component. Partial or unknown sends remain partial/unknown even when known parts were read. Older final-ID-only rows display `legacy_final_only`; group routes display recipient-only observations and never imply delivery to every participant.

WhatsApp POST webhooks return 200 immediately and process in the background; verification/parse failures are suppressed while logging diagnostic errors with bot/platform identifiers, never bodies or tokens. This remains best-effort delivery: an acknowledged event can be lost on process failure. Meta can retry for seven days. WhatsApp opts into atomic inbound admission through the existing `event_dedup_records` coordination store, scoped to bot, owner and `wamid`. A 30-second claim is conditionally committed together with the metadata insert in one MongoDB transaction; the transaction also rechecks older metadata rows. An expired or replaced holder cannot admit a row or callback. Committed markers persist for 30 days. Requires the existing replica-set/transaction-capable MongoDB deployment.

Concurrent copies produce one metadata row and one callback attempt. After admission there is no automatic callback redelivery, including on callback rejection, timeout, or a process crash before dispatch. Only a still-uncommitted claim can be released following pre-admission failure; an uncertain transaction commit cannot release committed metadata. This closes concurrent admission without promising exactly-once callback execution. X retains its existing lookup path and Telegram edited messages continue through their existing path. No durable message-content queue or automatic callback retry is introduced.

The adapter pins its Graph API version in the single `GRAPH_API_VERSION` constant. The `whatsapp-business` provider and `api-whatsapp-business` catalog counterpart inject a bearer token at the Graph root for separately authorized direct API calls; callers choose their versioned paths. There is no hosted WhatsApp OpenAPI overlay in the current registry.

### Managed WhatsApp onboarding (Embedded Signup)

NyxID can operate one Meta app as a Tech Provider. Users choose **Connect with Meta** in Channel Bots, enter a label and personal/organization scope, and complete Meta's popup. The popup offers an ordinary Cloud API number or an existing WhatsApp Business App number through coexistence. No user-entered access token, phone ID, or app secret is needed. **Advanced: use your own credentials** preserves the manual flow above. With no platform credentials configured, only the existing manual form is offered.

#### Administrator setup

1. Open **Admin > Platform Credentials** (`/admin/platform-credentials`). Create or choose a Meta Business app; the existing NyxID `facebook` provider app can be reused by adding the WhatsApp product. Enroll as a Tech Provider.
2. Add WhatsApp and Facebook Login for Business. Create a new Embedded Signup configuration, select the Cloud API product (this selects stable v4), use a business integration system-user token, and allow the NyxID frontend domain and JavaScript SDK login. Enter App ID, App Secret, and the new Embedded Signup Configuration ID in NyxID. An old v2 configuration must be replaced.
3. Complete Business Verification and App Review; request advanced access for `whatsapp_business_management` and `whatsapp_business_messaging`.
4. Copy the platform Callback URL and generated Verify Token from NyxID into Meta's app-level Webhooks settings. Subscribe to `messages`. For coexistence also enable Meta's Business App onboarding flow and subscribe to `smb_app_state_sync`, `smb_message_echoes`, and `history`.
5. Customers supply their own payment method and pay Meta directly for WhatsApp messaging. Sharing a credit line requires Solution Partner status. Meta's current pricing is message-based; consult its pricing page for applicable categories and rates.

The admin API lists adapter-declared providers even before configuration. Plain fields and encrypted secret fields live in `platform_credentials`, one UUID document per unique provider, with `updated_by` and BSON `updated_at`. PATCH/PUT `{ "fields": { "app_secret": "...", "app_id": "..." } }` sets or rotates fields; `null` clears one. DELETE removes the provider. App secrets are never returned. The generated encrypted platform Verify Token is intentionally readable by admins and can be regenerated with `{ "regenerate_verify_token": true }`; update Meta's dashboard after regeneration. Ordinary app-secret rotation preserves the Verify Token. Removing/clearing the app secret disables managed onboarding and stops managed signature verification/replies until restored. There is no decrypted credential cache in `AppState`, so changes apply on the next request across replicas. Audit events contain field names and provider identifiers only.

#### Protocol and recovery

`GET /api/v1/channel-bots/managed-onboarding/whatsapp` returns availability, public app/config IDs, the pinned Graph version, `signup_version`, supported feature types, and adapter-owned `signup_extras` keyed by feature type. Completion is `POST /api/v1/channel-bots/managed-onboarding/whatsapp/complete` with `{code, phone_number_id?, waba_id, business_id?, label, target_org_id?}`. Onboarding, re-registration, and repair use the same human-only router middleware as connect-link completion, rejecting API-key, delegated, service-account, and relay credentials. Completion, re-registration, and repair share five attempts per user per minute using the database limiter. JSON completion returns 201; `Accept: text/event-stream` returns progress (`exchanging`, `subscribing`, `registering`) followed by a result or the normal client-safe `ErrorResponse` (`error`, `message`, `error_code`). The browser uses the runtime API origin and the shared authenticated transport.

The server exchanges the one-use code without `redirect_uri`, debugs the token with the app token, and requires both granular permission target sets to include the WABA. It also verifies phone membership in that WABA and the exact phone identity. A WABA-only finish succeeds only when there is one unambiguous number. IDs are validated; phone-list pagination uses a bounded cursor walk on the pinned Graph host. App-secret proof (HMAC-SHA256 of the bearer token keyed by the app secret) is attached to managed Graph calls, including later verification and replies.

The ordinary registration persistence path performs duplicate checks, token encryption, and webhook-secret generation. The bot is saved as `credential_source: "platform"`, `pending_webhook` before webhook subscription so Meta's immediate verification callback can find it. Legacy rows default to `"user"`. Subscription, per-WABA webhook override, number registration, optional business ID, and coexistence sync outcomes are retained in `managed_setup`. A generated six-digit PIN is envelope-encrypted on the bot and never returned. The detail page's **Re-register number** calls `POST /api/v1/channel-bots/{id}/reregister`, reusing that PIN. A failed register request is accepted as already registered only after a separate Graph identity/status query confirms `CONNECTED`; upstream error prose is never interpreted or exposed.

Subscription/register failures remain visible on the saved bot. **Repair setup** on the detail page, or `nyxid channel-bot repair <id>`, calls owner-authorized `POST /api/v1/channel-bots/{id}/managed-setup/repair`. It repeats subscription, override, and registration/coexistence checks, retries failed sync requests, persists and returns the new setup state. Successful one-shot sync requests are preserved. Repair reuses the encrypted PIN and per-bot Verify Token (`webhook_secret_encrypted`); earlier managed rows with only a verify hash receive one atomically installed encrypted token and matching hash before the callback handshake. BYO rows keep their existing hash-only storage. A lost browser connection can leave a saved bot, so inspect its setup state before retrying signup. No Graph/database distributed transaction or durable setup job is claimed.

Deleting a managed bot best-effort removes the WABA callback override only when no other active managed bot shares that WABA. As specified by Meta, this is a bodyless `POST /{version}/{waba_id}/subscribed_apps`, which restores delivery to the platform callback. The number stays subscribed to the app in Meta. If another managed bot shares the WABA, the override is retained; a soft-deleted managed callback URL still forwards verified deliveries to active siblings. Cleanup outcomes (`removed`, `retained_shared`, `failed`, or `not_applicable`) are recorded in `channel_bot_deleted` audit metadata. Cleanup failure does not prevent local deletion.

The platform callback is `GET/POST /api/v1/webhooks/channel/whatsapp/platform`. GET rejects malformed subscription queries before accessing credentials, then compares the generated platform Verify Token in constant time. POST acknowledges immediately, then verifies the raw `X-Hub-Signature-256` with the platform app secret and resolves each `entry[].changes[].value.metadata.phone_number_id` to an active or pending active bot with `credential_source: "platform"`. Each target uses the existing per-bot verification, phone filtering, dedup, routing, and dispatch path; unknown and BYO-only numbers are dropped. Per-WABA `override_callback_uri` and `verify_token` are attempted after app subscription, but override failure is non-fatal because the platform callback is sufficient. Overrides apply to every phone in a WABA, so a managed per-bot POST URL dispatches sibling numbers too. BYO per-bot routes remain unchanged. Background acknowledgment and concurrent-dedup limitations are the same as the existing WhatsApp receiver.

Coexistence is verified server-side using `is_on_biz_app` and `platform_type = CLOUD_API`. Meta explicitly requires skipping `/register` for these numbers; re-registration checks that state without submitting a PIN. NyxID initiates Meta's one-shot contact/history sync requests immediately after subscription, within the documented 24-hour window, and records requested/failed states. These events do not import historical conversations or app-sent messages into agent conversations. A failed one-shot sync may require repeating Embedded Signup; follow Meta's recovery guidance.

Business integration system-user tokens default to **never expire** for offline server-to-server use, but the Facebook Login for Business configuration can choose an expiry and access can be revoked. NyxID rejects already-expired tokens at onboarding and stores the returned token encrypted; it does not exchange it for a short-lived user token or claim an automatic refresh flow. Reconnect through Embedded Signup after revocation/expiry. Managed token/app-secret rotation fields are hidden and rejected by the ordinary update API.

#### Browser hosting and Meta documentation

The SDK loads from `https://connect.facebook.net/en_US/sdk.js` only while the managed panel is open. Messages are accepted only from the exact origin `https://www.facebook.com`, with validated `WA_EMBEDDED_SIGNUP` finish/cancel/error shapes. SDK failures, cancellation, blocked popups, and timeouts have explicit UI states. The repository's `frontend/nginx.conf.template` does not set a CSP; the backend security-header CSP applies to API responses. Deployments that add a frontend CSP must permit the SDK in `script-src`, Facebook login frames in `frame-src`, and required Facebook/Graph requests in `connect-src`. Keep those allowances scoped to Meta origins and test against the deployed policy; do not weaken the API CSP.

Documentation checked September 8, 2026: [implementation](https://developers.facebook.com/documentation/business-messaging/whatsapp/embedded-signup/implementation), [Tech Provider onboarding](https://developers.facebook.com/documentation/business-messaging/whatsapp/embedded-signup/onboarding-customers-as-a-tech-provider), [coexistence](https://developers.facebook.com/documentation/business-messaging/whatsapp/embedded-signup/onboarding-business-app-users), [webhook overrides](https://developers.facebook.com/documentation/business-messaging/whatsapp/webhooks/override), [number registration](https://developers.facebook.com/documentation/business-messaging/whatsapp/reference/whatsapp-business-phone-number/register-api), [business token lifetime](https://developers.facebook.com/documentation/facebook-login/facebook-login-for-business), [secure requests](https://developers.facebook.com/docs/graph-api/guides/secure-requests/), and [debug token](https://developers.facebook.com/docs/graph-api/reference/debug_token/).

NyxID uses stable **Embedded Signup v4**, as recommended by [Meta's versions page](https://developers.facebook.com/documentation/business-messaging/whatsapp/embedded-signup/versions) and [v4 implementation](https://developers.facebook.com/documentation/business-messaging/whatsapp/embedded-signup/version-4). Meta's implementation page explicitly supports Graph `v25.0`, the existing `GRAPH_API_VERSION`. Stable v4 is selected by the product-enabled Facebook Login configuration, not an `extras.version` override. The default launch sends `extras: {}`; coexistence adds only `featureType: "whatsapp_business_app_onboarding"`, which Meta's Business App customization instructions retain for v4. No legacy `sessionInfoVersion` is sent. The adapter descriptor supplies the signup version and launch extras; the browser has no hardcoded version contract. Events support Cloud API, WABA-only, and Business App finishes (including a missing phone ID), optional v4 asset arrays, and cancel/error payloads. Multiple selected WABAs and unsupported migration/grant-only flows produce a local selection error. Validate the product-enabled configuration and both flows on a real Meta tenant before production.

Meta documents granular targets as strings/numeric IDs and permits absent targets for all-assets grants; NyxID intentionally requires explicit WABA targets in both scopes. No stable numeric "already registered" error was documented, hence the independent connected-state check.

Meta references checked for this implementation: [webhook overview](https://developers.facebook.com/documentation/business-messaging/whatsapp/webhooks/overview/), [endpoint verification](https://developers.facebook.com/documentation/business-messaging/whatsapp/webhooks/create-webhook-endpoint), [phone numbers](https://developers.facebook.com/documentation/business-messaging/whatsapp/business-phone-numbers/phone-numbers), [text messages](https://developers.facebook.com/documentation/business-messaging/whatsapp/messages/text-messages), [media](https://developers.facebook.com/documentation/business-messaging/whatsapp/business-phone-numbers/media), and [error codes](https://developers.facebook.com/documentation/business-messaging/whatsapp/support/error-codes).

### Why no generic/passthrough adapter

A generic adapter that skips normalization and forwards raw webhooks was considered and rejected:

- **Verification**: Each platform has a unique webhook signature scheme (HMAC, Ed25519, secret header). A generic adapter can't verify unknown platforms, so webhooks would either be unauthenticated (security risk) or require the user to implement verification logic somewhere.
- **Normalization**: With `content.text: null` and everything in `raw_platform_data`, the agent must parse every platform's JSON format itself -- defeating the purpose of the relay.
- **Replies**: NyxID needs to know how to call each platform's send API (different URLs, auth schemes, body formats). A generic adapter can't send replies, so the agent would need direct platform API access anyway.
- **User experience**: Setting up a "generic" bot would require the user to understand webhook verification, raw payload formats, and reply mechanics for their platform. At that point they're better off receiving webhooks directly without NyxID in the middle.

The `raw_platform_data` field on the callback payload serves the advanced use case: agents that need platform-specific features (Telegram inline keyboards, Discord embeds, Lark cards) can read it alongside the normalized fields. But the adapter still handles verification, parsing, and reply delivery.

**Bottom line**: Each supported platform gets a dedicated adapter (~300 lines). The cost is low, the user experience is complete (register bot, configure route, done), and agents get both normalized and raw data.

---

## Media

`PlatformAdapter::media_capabilities()` is required on every adapter. Conversation discovery and owner conversation detail expose `capabilities.media.inbound` and `.outbound` as arrays of `image`, `file`, `audio`, and `video`. Device conversations and OpenClaw have empty arrays. Capability declarations describe implemented transports; platform permissions, account limits, and WhatsApp's messaging window still apply.

### Inbound downloads

Callbacks preserve existing provider fields and add `content.attachments[i].download_url`. Agent history (`GET /channel-relay/messages/{conversation_id}`) returns the same attachment metadata and absolute download URLs. A document may carry `content.type = "file"` with no text: runtimes should process its attachments instead of dropping it as an unsupported text message.

`GET /api/v1/channel-relay/messages/{message_id}/attachments/{index}` accepts either the conversation's assigned, live API key or a valid reply token bound to that exact inbound message, conversation, platform and agent. Reading validates the signature and live bindings **without consuming the JTI**; the token may download repeatedly and still send its single reply. Relay, delegated, service-account, and ordinary user credentials are forbidden. Device conversations are refused because they have no bot. Inactive bots cannot fetch. API-key downloads also consume the key's configured per-agent rate-limit bucket before context resolution; exceeding it returns 429. Unknown indexes return `channel_attachment_not_found` (404).

NyxID resolves the live bot credential and calls the adapter's `fetch_attachment`. The response includes `Content-Type`, `Content-Length`, sanitized `Content-Disposition: attachment; filename="..."`, and `Cache-Control: private, no-store`. Downloads have a 30-second request timeout. `CHANNEL_MEDIA_MAX_BYTES` defaults to 20 MiB (Telegram's bot download limit). Content-Length is checked first; streamed chunks are counted before copying into the bounded buffer. The adapter returns ephemeral `FetchedMedia` bytes, then the handler writes them to the response. Nothing is written to disk or persisted as content. Provider URLs may expire; a retained metadata row is not an archival copy of a file.

| Adapter | Fetch path | Allowed media hosts |
|---|---|---|
| Telegram / telegram-new | `getFile(file_id)` → `/file/bot{token}/{file_path}` | `api.telegram.org` (adapter-owned base) |
| Lark / Feishu | `/open-apis/im/v1/messages/{message_id}/resources/{key}?type=image\|file`, tenant bearer | Adapter base host (`open.larksuite.com` / `open.feishu.cn`) |
| Slack | `url_private`, bot bearer | `files.slack.com`, `*.slack.com` |
| Discord | Attachment CDN URL | `cdn.discordapp.com`, `media.discordapp.net` |
| WhatsApp | Graph media-ID lookup → temporary URL, bearer on both | `graph.facebook.com`, `lookaside.fbsbx.com` |
| X | Media URL with user bearer | `pbs.twimg.com`, `video.twimg.com`, `ton.twitter.com` |

Webhook URLs are untrusted input even after parsing. Adapter allowlists, HTTPS/443, public DNS/IP checks, address pinning, and disabled redirects prevent SSRF. Credentials are never sent to a rejected host. Fetch errors use locally authored messages and never echo a URL. A debug record includes only message ID, index and byte count; downloads are not audited.

### Lark/Feishu posts and NyxBot ingestion

The adapter parses rich `post` title and content in document order: text, links,
mentions, emotion, code and horizontal rules become bounded text; image and
media elements become image/file selectors. Pinned own-bot mentions are removed.
Text is capped at 32 KiB with a cap marker, and at most ten embedded selectors
are retained with an omitted count. Malformed posts produce bounded output
without panicking. Embedded selectors live in additive
`ChannelMessage.embedded_attachments`; they do not change `CallbackPayload`
bytes or the existing indexed `/attachments/{index}` download contract.

With `assistant:channel-media` enabled for the linking person, verified owner
Lark/Feishu media on NyxID's relay or gateway becomes encrypted assistant uploads
at the start of the consuming run. Relay posts recompute text from the native
payload; supported gateway media uses `activity.kind.text` instead of CMA's
rendered media fallback. Audio/video activities retain their previous input.
Admission checks group addressing before any download, and every fetch rechecks
the stored owner/bot/route/chat/sender/thread authority. Guests get the fixed
note `attachments from guests are not shared with the assistant` with no download
or upload. Media prevents card/text decision parsing and question coalescing.

Native dedup is atomic with accepted media event insertion. Its key binds bot,
platform, chat and native message; a second relay UUID records
`duplicate_of_media_key` and starts no fetch, turn or reply. Media events retain
seven days of dedup evidence; text events retain 24 hours. Replays later than
that native-media horizon are outside the guarantee. Queued owner media converts
to a durable quoted-history expiry notice after six days. An accepted event with
no turn/queue binding after five minutes is `interrupted`, with a fixed note
delivered once on the next owner turn in the chat.

Gateway callback HTTP 202 bodies are parsed within 2 KiB into closed
`accepted`/`refused` admission codes; malformed, unknown or oversized responses
become `unknown`. Only status and a closed refusal reason are stored, never IDs
or response prose. Deliberate refusals such as `group_not_addressed` are skipped
by health. An accepted eligible owner-private image/post without a provider
event after the existing two-minute grace is lost, within the twelve-hour judge
window. Pending ingestion remains pending; per-item failures report
`media_failed` with a fixed reason (size, source/authority, type, rate limit,
fetch/store, timeout, stopped or expired). Interrupted events report
`interrupted`. Suppressed duplicates reuse the winning event's pending or final
outcome; an expired winner never becomes a lost-message resend recommendation.

Gateway support requires CMA ≥ `a570be21`, `CMAEG_MEDIA_ENABLED=true`, and NyxID
responding within `CMAEG_PROVIDER_REQUEST_TIMEOUT_MS` (default 30 seconds).
Downloads happen after admission in the run, outside that header budget. Upgrade
all replicas and drain old workers before enabling the default-off flag. See
[inbound channel media](chat/10-uploads.md#inbound-channel-media) for immutable
bind receipts, Stop/orphan cleanup, the 192 MiB shared byte budget, file/step
limits and the admin rollback drain.

### Outbound attachments

Both `/channel-relay/reply` (`reply.attachments`) and `/channel-relay/send` (`message.attachments`) accept up to ten entries:

```json
{
  "kind": "file",
  "source": { "type": "base64", "data": "aGVsbG8=" },
  "filename": "report.txt",
  "mime_type": "text/plain",
  "caption": "Your report"
}
```

Alternatively use `"source": { "type": "url", "url": "https://public.example/report.pdf" }`. URL sources require public HTTPS hosts and use the same DNS pinning, redirect prohibition, timeout and byte cap. Base64 is decoded with a length check before allocation. Filename, MIME type and caption are optional. Attachment-only sends are accepted for declared kinds; undeclared kinds produce a validation error listing supported kinds. `/reply/update` remains text/platform-card only and rejects nonempty attachments.

The service materializes every source before dispatch. At the adapter boundary, `OutboundReply.attachments` is `Vec<MaterializedAttachment>` (kind, bytes, filename, MIME type, caption), not the public source shape. Secret/content-bearing Debug implementations redact these values.

| Adapter | Native upload/send path |
|---|---|
| Telegram / telegram-new | Multipart `sendPhoto`, `sendDocument`, `sendAudio`, `sendVideo`; captions, reply anchor and topic ID preserved. A photo-specific parsed 400 can fall back once to `sendDocument` (see below). Text goes first for mixed public requests. |
| Discord | One multipart message with `files[n]` and `payload_json`; text/captions joined into message content. Deferred interaction follow-ups retain their existing routing. |
| Slack | `files.getUploadURLExternal` → PUT bytes → `files.completeUploadExternal` with channel/thread and initial comment; `files.info` resolves the share timestamp. Text goes first. |
| Lark / Feishu | `im/v1/images` or `im/v1/files`, then an image/file message. Audio/video use file upload (`file_type=stream`). Text/captions are preceding text messages. |
| WhatsApp | Multipart `/{phone_number_id}/media`, then image/document/audio/video message with media ID. Audio captions are separate text messages. |
| X | OAuth2 multipart `/2/media/upload` with `dm_image`, `dm_video`, or `dm_gif`, processing-status wait when needed, then DM `attachments: [{media_id}]`. |

Editing a media send targets the returned media message ID. Telegram uses `editMessageText` for text messages and falls back to `editMessageCaption` for media captions; Discord `PATCH` edits `content` on any bot message. Slack `chat.update` and Lark/Feishu message edits do not apply to file/image messages and surface as classified refusals. WhatsApp and X have no edit API. `/reply/update` does not replace attachment bytes.

The result is the last platform message ID; Slack may omit it if no share timestamp is available. Lark/Feishu need `im:resource`; Slack needs file read/write scopes; existing X OAuth connections need fresh consent to `media.write`. Platform size/type restrictions can be stricter than NyxID's cap.

Send idempotency fingerprints include attachment kind, source URL or SHA-256 of decoded bytes, filename, MIME type and caption. Changing any of them with the same key returns 409. The source URL identifies URL-based content; changing bytes behind the same URL cannot be detected on an already-completed replay. A multi-message send can partially succeed before a later upload fails; NyxID cannot roll back a platform send. Existing claim/retry semantics apply.

### Image outcomes and Telegram document fallback

Media senders distinguish definite refusals from uncertain delivery. A completed
HTTP non-2xx response, including **408 and 5xx**, or a parsed platform error is
definite. A missing/unreadable response, malformed successful JSON or missing
successful receipt is uncertain (`Media upload or delivery failed`). Definite
failures are never automatically retried. NyxBot records failed images and may
continue to the next image; an uncertain image stops the remaining batch and is
never resent. Shared WhatsApp Graph calls retain their specific timeout,
response-size, invalid-JSON and unknown-acceptance diagnostics. NyxBot's image
path recognizes those exact locally authored errors as uncertain without
changing text-send, verification or credential-check diagnostics. X limits
media-specific refusal parsing to upload/status responses without usable data;
partial `errors` alongside a successful `data` result remain usable.

The only retry is Telegram/telegram-new's image fallback: a parsed `ok:false`,
`error_code:400` photo response with HTTP 400 or 2xx and a photo-specific marker
(`PHOTO_INVALID_DIMENSIONS`, `PHOTO_SAVE_FILE_INVALID`, `PHOTO_EXT_INVALID`,
`PHOTO_CROP_SIZE_SMALL`, or `IMAGE_PROCESS_FAILED`) sends the same bytes
once with `sendDocument`. The caption, chat, reply anchor and forum topic are
preserved, and the document outcome is final. Chat/thread/reply-target and rights
errors, 403, 429, 5xx and transport failures never trigger fallback. Deny markers
take precedence when a description also contains a photo marker. NyxID always
materializes URL sources and uploads multipart bytes, so file-ID/URL-source
errors, padding errors and empty-file errors never trigger this fallback.

`/channel-relay/reply`, `/thread-reply` and `/send` can return additive
`delivered_as: ["file"]` for that substitution. The outbound row and audit record
`file`; NyxBot's image claim records `sent` with `as_document`, without a failure
line. Older adapters omit `delivered_as`.

NyxBot uses the existing route-key reply, bound-thread and initiated-send paths
to send tool images before text, one per message. It sends images itself on
gateway channels; no CMA attachment-block renderer is required. Chat-delivered
automations consume one initiate limiter token for their result text, with none
for its bounded image batch; public `/send` still consumes one per request.
Each X DM image is a separately billed request using the ordinary channel
billing context. A billing refusal suspends the channel through the existing
path and skips the remaining images; X public replies do not support images.

### Per-image delivery claims, audiences and topics (P15)

Property P15: **one send per source image and resolved audience, for the
image's whole lifetime, across every NyxBot path** (automatic replies on
direct and gateway transports, followed threads, event and duplicate-question
chats, chat-delivered automations and explicit `nyxid__post_to_chat
include_images` posts). Uncertain or in-progress sends are never retried and
never reported as delivered.

Release 0.71.0 includes #1889 and its v1 image claims. Before any v2 claim,
new replicas read `nyxbot_image_deliveries` for the same owner, conversation,
turn and NyxBot channel, across every destination alias (`inbound:`, `thread:`,
`chat:`, `initiated:`). A sent item reports `previously_delivered`; sending,
unknown, or a pending item in a sending batch reports `in_progress_or_unknown`.
Failures and unrecognized skips retain their old outcome. Only a pending batch
or a skip code v1 set before dispatch (`deadline`, `after_unknown`, `billing`)
permits a new v2 attempt. No new code rewrites the v1 rows; their existing
seven-day `expires_at` TTL stays. Existing conversation/user deletion still
purges their metadata.

This guard is deliberately conservative across all audiences of the same bot
for that turn: it can suppress a second chat's image during the seven-day
window. It covers old-replica sends completed before the new replica reads.
An old 0.71.0 replica and a new replica can still race between the read and
claim during rolling deployment; concurrent duplicates remain a rollout risk.
There is no mixed-fleet guarantee and no data repair is needed. Rollback to
0.71.0 restores its old behavior and ignores v2 claims, so it does not preserve
the new delivery invariant. Remove the v1 read path and collection/index one
release after this change ships, at least seven days after deployment; track
that sunset in the PR description.

**Claim (v2).** `nyxbot_image_deliveries_v2` holds one document per (source
image, audience): `_id = sha256("v2\0{owner}\0{attachment_id}\0{audience_key}")`
plus plain `user_id`, `conversation_id`, `attachment_id`, `audience_key`,
`state`, `attempt`, `code` and `created_at`, with cleanup indexes on
`attachment_id` and `(user_id, conversation_id)` and **no TTL**
(`services/channel_image_delivery.rs`, `models/nyxbot_image_delivery.rs`).
States move `pending` -> `sending{attempt}` -> `sent | failed{code} |
skipped{code} | unknown` through compare-and-set on the attempt token. Only the
winner of the `pending -> sending` race dispatches; success is reported only
after an acknowledged matching completion write, an ambiguous write settles
locally as `unknown`, and a claim found `sending` reports
`in_progress_or_unknown`. A later caller reports an existing claim's durable
state (`previously_delivered`, the same failure or skip code, in progress or
unknown) without resending and without repeating another path's failure line.
A definite refusal before any provider request (cancelled, deadline,
after-unknown, billing preflight or another pre-dispatch error) releases its
own `sending{attempt}` back to `pending` with an attempt-token CAS, clearing
attempt and code. This call reports the skip or failure; a later post or
automation may acquire the pending image. Unsupported audiences, missing
anchors and unavailable sources remain permanent refusals. Once provider I/O
begins, even a timeout or failed settlement retains the durable barrier and
never releases it for retry.
Claims live as long as their source image: they are deleted in the same
transactions that delete `assistant_attachments` (retention, conversation and
group deletion, user purge), a sender that finds its source gone after claiming
deletes the claim and reports `unavailable`, and a bounded orphan sweep on the
assistant upload-retention cadence removes only claims whose source attachment
no longer exists. Claims hold identifiers, states and codes only.

**Audience.** The audience is the outbound target actually sent to, resolved
by `services/channel_image_audience.rs` after the adapter's own normalization
and fallback, and serialized as
`"{channel_bot_id}\u{1f}{platform_chat_id}\u{1f}{native_thread_or_empty}"`
with the real `ChannelBot.id` (never a gateway partition or `NyxbotChannel`
id). Per adapter:

| Platform | Chat | Audience boundary | Not a boundary |
| --- | --- | --- | --- |
| Telegram, telegram-new | chat id | forum topic `message_thread_id` (normalized to a decimal string) | reply chains (`reply_to_message_id`) |
| Slack | channel id | a root thread (`thread_ts`, else the replied-to message, as the adapter posts it) is distinct from the top-level channel | — |
| Discord | channel id (threads are channels) | — | — |
| Lark, Feishu | chat id | — | thread replies share the chat audience |
| WhatsApp | normalized `wa_id` digits | — | — |
| X | DM conversation id | — | public replies carry no images |
| Aurinko, device, OpenClaw and other adapters | chat id or no image delivery, per the adapter's declared capability | — | — |

The Telegram and Slack adapters build their outbound `message_thread_id` /
`thread_ts` from the same `native_thread` function the audience uses, so the
key and the provider payload cannot drift.

Inbound Telegram thread keys and follow-scope classification remain unchanged.
Native `message_thread_id` and gateway `conversation.thread_id` continue to
select existing conversations, including reply chains. Separate additive
`telegram_topic` evidence selects the outbound target and image audience:
native messages require `is_topic_message: true`; gateway version 1 facts
must declare a topic and positive decimal native ID. Explicit reply-chain
evidence sends at chat level without changing its inbound partition. Legacy
anchors without evidence retain their old outbound target. Old replicas ignore
the additive field and reuse the same conversation keys; there is no migration.
The image guarantee applies only after every replica is upgraded.

**Topics.** Agent-initiated posts (`nyxid__post_to_chat` and chat-delivered
automation results) keep the thread's native topic
(the separate Telegram topic evidence, falling back to legacy
`platform_thread_id`, or a Slack root thread) for text and images through the adapter's `reply_context`; a topic is
therefore its own audience, and the parent chat is another.

**Explicit posts and automations.** `include_images` (default false) sends the
calling turn's own tool images (trusted `TurnAttachment` metadata of the exact
live turn, `chat_tool_image` only, deduplicated, at most 8) before the text,
each behind a turn-fence recheck and its claim; the text follows a final fence
recheck and nothing further is sent after Stop or replacement. Multi-turn
automation runs keep `TriggerRun.image_refs` (at most 16, oldest terminal
evicted against the run's configured audience, else `image_refs_truncated`)
and deliver first sends for refs whose claim is absent or `pending`, newest
first and at most eight, reporting the other claim states without consuming
that budget; an omitted-images line names the count. Unavailable image
metadata never blocks the final text: the text is sent and the image counts as
failed. The text/stream delivery barrier (`channel_turn_delivery.rs`) is
separate and unchanged.

### ADR-013 metadata boundary

Inbound `ChannelMessage.attachments` stores only `StoredAttachment { content_type, provider_ref, platform_message_id, file_key, image_key, filename, mime_type, size_bytes }`. Provider references are routing metadata needed to fetch an attachment; they are not its bytes. Old rows without the field deserialize as an empty list. The legacy cleanup retains new provider-reference metadata while removing historical content fields. Outbound rows store only the first attachment's `content_type` and an empty attachment list, never sources, bytes, captions or filenames. Audits contain only attachment kinds/count plus existing routing identifiers; idempotency rows store a fingerprint, never source content. No media is put in token claims and no new telemetry events are introduced.

## Platform catalog

`GET /api/v1/channel-platforms` is the authoritative inventory, derived from `registered_adapters()`, registration/managed/platform-credential descriptors, ingestion, display names and capability declarations. It includes disabled adapters (`enabled: false`). `platform_credentials.configured` is a non-secret boolean indicating stored or linked admin credentials, not a live provider health check. The metadata-only route is in `api_v1_delegated`, admits JWT sessions, API keys, service accounts and delegated `account:read`, and does not belong in the delegated deny list. The media GET does belong in that deny list.

Response shape (field lists and flow values vary by descriptor):

```jsonc
{
  "platforms": [{
    "platform": "telegram", "display_name": "Telegram bot token", "enabled": true,
    "managed_only": false, "managed_only_message": "This platform requires managed onboarding",
    "ingestion": { "mode": "webhook" }, // or { "mode": "poll", "min_interval_secs": 60 }
    "registration": {
      "documentation_url": "https://core.telegram.org/bots/api#setwebhook",
      "fields": [{ "name": "bot_token", "label": "Bot token", "secret": true,
        "hint": "Connect an existing Telegram bot using its BotFather token.",
        "required": true, "patchable": false, "clearable": false,
        "storage": "bot_token_encrypted", "webhook_secret": false, "platform_fallback": null }],
      "token_fields": ["bot_token"], "extra_fields": [], // extra_fields uses the same field shape
      "required_suffix": "", "automatic_webhook": true, "webhook_ingestion": true,
      "webhook_secret_label": null, "create_response_status": "active", "setup_instructions": []
    },
    "managed_onboarding": null, // or { "flow", "provider", "bootstrap_fields": [], "completion_fields": [] }
    "platform_credentials": null, // or { "provider": "meta", "configured": true }
    "capabilities": { "initiated_send": true, "reply_to": true, "thread": true, "edit": true,
      "media": { "inbound": ["image", "file", "audio", "video"], "outbound": ["image", "file", "audio", "video"] } },
    "webhook_path": "/api/v1/webhooks/channel/telegram/{bot_id}"
  }]
}
```

The frontend queries this inventory for labels, enabled choices, required/secret fields, and managed-flow dispatch. Only React flow components and presentation-specific mappings stay local. Telegram's native browser creation remains its dedicated component and route (`telegram-new` is managed-only, with no generic managed-onboarding descriptor). `nyxid channel-bot platforms [--output json]` exposes the same inventory to CLI integrations; `register --help` points to it.

### Standalone onboarding pages

`/channel-bots/connect` lists shareable setup links from the enabled catalog entries. `/channel-bots/connect/$platform` renders the selected platform outside the dashboard shell after authentication. Both use the same `ChannelBotSetup` form as the **Add Bot** dialog. The user walkthrough, all platform-specific steps, and the prefill field reference are in [Connect a channel bot](site/web/guides/channel-bots.md).

The page resolves the path against the fetched catalog. It does not maintain another platform allowlist. `registration.fields` supplies field order, names, labels, required/optional status, secret masking, and hints. `buildCreateChannelBotSchema` validates the selected enabled descriptor, and `channelBotRegistrationPayload` sends only that descriptor's fields plus `platform`, `label`, and optional `target_org_id`. `registration.extra_fields` covers legacy API inputs; those fields are deliberately not added to the new-user form.

Query prefilling uses the same declared field names. `field1` and `field2` are supported if declared by an adapter, not as positional aliases. `label` and `target_org_id` are shared form values; Telegram also uses a UUID `request_id` to resume its draft. `channelBotSetupRewrite` protects text values from TanStack Router's automatic JSON/number parsing, including long numeric IDs. Keep it on the router when changing navigation. The page removes descriptor-marked secret parameters with replacement navigation after consuming them; values stay in form state only. Before authentication, the original URL is preserved through `return_to`, so shared secret-bearing links can still expose credentials in access logs and the login history entry. The copy-link action emits a credential-free URL.

Manual forms require a valid form and an explicit submit. Managed entry points use the catalog's flow discriminator: `meta_embedded_signup` for Meta, `oauth_connection` for OAuth, plus the dedicated Telegram creation component. Prefilled manual credentials reveal the manual form when managed onboarding is also available. A flow marked `managed_only` never falls back to a manual credential form. The backend still enforces provider configuration, ownership, organization admin access, and credential validity.

The standalone completion screen keeps the user on the page until **Open channel bot** is selected. Callback URLs and returned instructions display independently of one-time secrets. A generated Verify Token is shown only when the create response includes one. Provider authorization windows remain separate steps; "one click" means opening the provider or submitting a completely prefilled credential form, not skipping consent or webhook configuration.

### Returning to an assistant chat

NyxBot setup links may carry one owner-scoped `return_to` identifier (`nyxa-…`
or `nyxg-…`). The setup page preserves it through request URL updates and
offers Back to chat on completion, cancellation, expiry and error. In-chat
Telegram setup keeps the request in component state and resumes the current
request on remount, retains its actor-scoped request ID, and polls that ID through
completion while preserving the chat URL and composer. Rendered setup anchors
preserve the same strictly validated raw target for new-tab navigation; modal
setup ignores it. Duplicates, encoded targets and surrounding whitespace are
refused. No arbitrary URL is accepted or assigned, and the assistant page performs its normal ownership check.

### Add or extend a form

1. Implement and register the backend adapter and its create/validation behavior. Declare its enabled platform ID, display name, registration fields, documentation URL, and clear setup instructions. Keep user-facing field names separate from encrypted storage names.
2. For a normal credential form, no new frontend route or per-platform field mapping is needed. Check that values fit the existing string-input contract and that the backend accepts the new fields. Field metadata currently describes text/password inputs, not arbitrary checkbox/select/file widgets.
3. For an existing managed protocol, declare its supported flow discriminator and implement the matching backend bootstrap/start/complete contract. For a new protocol, add a frontend implementation to `MANAGED_FLOW_COMPONENTS` and the `ManagedFlow` mapping/types; arbitrary provider SDKs cannot be generated from field descriptors alone. Telegram's creation protocol remains its own component.
4. Supply a platform icon when one exists. Missing icons use the shared service-icon fallback and do not prevent the form from loading.
5. Add catalog and backend contract coverage, then exercise the standalone page: signed-out return, required/optional fields, query prefill, exact numeric IDs, secret cleanup, personal/org ownership, explicit submit, completion instructions, and unknown/disabled states. For managed protocols, test provider success, denial, closed/blocked popups, unavailable configuration, and manual fallback where allowed.
6. Update the user guide's setup steps and parameter reference. Test the live provider separately from browser fixtures before claiming provider integration success.

Focused frontend checks, run from `frontend/`:

```sh
npm test -- src/pages/channel-bots.test.tsx src/schemas/channel-bot-setup.test.ts src/schemas/channels.test.ts src/lib/channel-platforms.test.ts src/router.test.ts src/hooks/use-channel-managed.test.ts src/hooks/use-channel-managed-oauth.test.ts
npx playwright test e2e/channel-setup-pages.spec.ts e2e/channel-x-managed.spec.ts e2e/channel-whatsapp-managed.spec.ts
npx tsc -b
```

The standalone browser suite covers every supported credential form, a synthetic catalog platform with unfamiliar field names, Telegram creation, X OAuth, both WhatsApp signup modes, and callback instructions without a generated secret. Provider endpoints are fixtures; these checks validate the browser flow without creating external accounts or bots. The local preview uses the real catalog and authenticated development account; unconfigured managed providers correctly show their configuration requirement or manual fallback.

## Callback Contract

### NyxID -> Agent (Webhook POST)

NyxID sends a normalized message to the agent's callback URL.

```json
{
  "message_id": "550e8400-e29b-41d4-a716-446655440000",
  "correlation_id": "77a1b4f1-f7a4-421e-9f89-7d0b1de9e0c6",
  "platform": "telegram",
  "agent": {
    "api_key_id": "880e8400-e29b-41d4-a716-446655440000",
    "name": "claude-support-bot"
  },
  "conversation": {
    "id": "660e8400-e29b-41d4-a716-446655440000",
    "platform_id": "12345678",
    "type": "private"
  },
  "sender": {
    "platform_id": "87654321",
    "display_name": "John Doe"
  },
  "content": {
    "type": "text",
    "text": "What is the weather in Tokyo?",
    "attachments": []
  },
  "reply_to_message_id": null,
  "thread_id": null,
  "timestamp": "2026-03-31T12:00:00Z",
  "reply_token": "eyJhbGciOiJSUzI1NiIsImtpZCI6...",
  "raw_platform_data": { "update_id": 123, "message": { "...": "full Telegram/Discord/Lark JSON" } }
}
```

**Design rationale:**

The payload normalizes messages into a common format so agents can handle all platforms with one code path. For agents that need platform-specific features (Telegram inline keyboards, Discord embeds/components, Lark interactive cards), the full original webhook JSON is included in `raw_platform_data`. Most agents ignore it; advanced integrations read it directly.

`conversation.platform_id` and `conversation.type` describe the actual inbound event after platform webhook verification and adapter parsing. They are independent of the selected route's configured conversation ID/type: a default route configured as `"*"` / `"private"` can receive a group message, whose callback must contain the actual chat ID and `"group"`. `conversation.id` continues to identify the selected NyxID routing rule for async replies. Consumers that previously classified groups using the route's configured type may observe a corrected conversation scope; downstream history partitioning should use the actual platform context.

Native mention metadata passes through unchanged in the signed `raw_platform_data`: Lark/Feishu `event.message.mentions` (including native `id.open_id` and placeholder `key`) and Telegram `message.entities` / `message.caption_entities` (including native UTF-16 offsets). NyxID does not resolve these mentions into specialist routes or infer targets from display names. Consumers can interpret native mentions within their own trusted bot/group context. The relay contract adds no normalized mention field and preserves Lark placeholders in `content.text`; NyxID's direct NyxBot consumer renders bounded, unambiguous placeholders as names for input text.

A sender needs no additional NyxID OAuth/channel binding merely to address a receiving bot. The receiving bot still needs platform registration, authenticated event delivery, and an agent callback route. Mentioning a bot or specialist does not establish the sender's NyxID identity, grant protected command/tool authority, prove another bot's group membership, or authorize another owner's credentials. External runtimes retain responsibility for specialist selection and execution authorization. For NyxID-hosted agents, the receiving NyxBot channel and its live chat override select the existing linked agent; native mention labels never select another owner's agent. Provider delivery rules apply before this relay; for example, Telegram documents specific [bot-to-bot communication settings and group delivery conditions](https://core.telegram.org/bots/features#bot-to-bot-communication). Raw mention passthrough cannot cause an undelivered provider event to reach NyxID.

- **`agent.api_key_id`** is the primary agent identifier. Same `ApiKey._id` from agent isolation (PR #132). A shared callback endpoint dispatches based on this value. Use for routing/authorization.
- **`agent.name`** is the human-readable label from key creation (e.g., `"claude-support-bot"`). For logging and display only -- never use for authorization.
- **`sender.platform_id`** is the platform-native user ID. The agent is responsible for mapping this to its own users. If the agent uses NyxID OAuth, it already has `nyxid_user_id` in its user table from login time -- it can match `sender.platform_id` against platform identities it collected during onboarding. NyxID doesn't need to do this lookup because the agent already has the data.
- **No NyxID user IDs for senders** -- the agent knows the bot owner (from its API key), and knows the sender (from its own user table or the optional resolve-sender API). NyxID's job is message transport, not identity resolution.
- **No PII** -- no emails, no NyxID-stored names. `sender.display_name` is platform-provided (Telegram `first_name`, Discord `username`).

**Field Reference:**

| Field | Type | Nullable | Description |
|---|---|---|---|
| `message_id` | UUID | No | NyxID's internal ID for this message record (stored in `channel_messages`). The agent uses this to send async replies via `POST /channel-relay/reply`. |
| `correlation_id` | UUID | No | Per-delivery correlation ID. This equals the callback JWT `jti`; retries mint a new value. |
| `platform` | string | No | Which messaging platform the message came from: `telegram`, `discord`, `lark`, or `feishu`. |
| `agent.api_key_id` | UUID | No | The `ApiKey._id` assigned to this conversation route. This is the agent's identity from agent isolation. A shared callback endpoint dispatches based on this. |
| `agent.name` | string | No | Human-readable name of the API key (e.g., `"claude-support-bot"`). For logging and display only. |
| `conversation.id` | UUID | No | NyxID's internal ID for the selected conversation route (from `channel_conversations`). A default route can serve several platform chats. |
| `conversation.platform_id` | string | No | The actual inbound event's native conversation identifier (Telegram `chat_id`, Discord `channel_id`, Lark `chat_id`), even when the selected route uses `"*"`. |
| `conversation.type` | string | No | The actual inbound event's conversation kind after adapter parsing: `private` (1:1 DM), `group` (multi-user chat), or `channel` (broadcast). The route's configured type does not override it. HTTP Event Gateway callbacks use their separate `device` contract. |
| `sender.platform_id` | string | No | The message author's ID on the platform. The agent maps this to its own users. |
| `sender.display_name` | string | Yes | Display name from the platform (Telegram `first_name`, Discord `username`). `null` if not provided. |
| `content.type` | string | No | Content kind: `text`, `image`, `file`, `audio`, `video`, `location`, `sticker`, or `unknown`. |
| `content.text` | string | Yes | Text body. Present for `text`; may contain caption for media. `null` for non-text without caption. |
| `content.attachments` | array | No | Non-text attachment metadata: `{ content_type, url, download_url, platform_message_id, file_key, image_key, filename, mime_type, size_bytes }`. Omitted when empty. Raw provider fields remain compatible; use `download_url` for authenticated, bounded downloads through NyxID. |
| `content.attachments[].download_url` | string | No | Absolute `{BASE_URL}/api/v1/channel-relay/messages/{message_id}/attachments/{index}`. Assigned agent API key or this callback's reply token required. No token is embedded in the URL. |
| `reply_to_message_id` | UUID | Yes | NyxID `message_id` of the message being replied to. `null` for standalone messages. |
| `thread_id` | string | Yes | Platform-native thread ID (Discord threads, Lark threads). `null` if not in a thread. |
| `timestamp` | ISO 8601 | No | When the message was sent on the platform (not when NyxID received it). |
| `reply_token` | string | Yes | Per-callback RS256 JWT the agent can send back as `Authorization: Bearer <reply_token>` on `POST /api/v1/channel-relay/reply` instead of its full API key. See [Reply Token](#reply-token). `null` only if token generation failed on NyxID — agents that receive `null` must fall back to API-key auth on the reply call. |
| `raw_platform_data` | object | Yes | The full original webhook payload from the platform (Telegram Update, Discord Interaction, Lark Event, etc.). Use this for platform-specific features like inline keyboards, embeds, interactive cards, or any data not captured by the normalized fields. `null` only if the raw data could not be preserved. |

**Headers:**

| Header | Description |
|---|---|
| `Content-Type` | `application/json` |
| `X-NyxID-Callback-Token` | RS256 JWT proving the callback came from NyxID. Verify with the JWKS at `/.well-known/jwks.json`. |
| `X-NyxID-Signature` | Transitional HMAC-SHA256 of request body, signed with the API key's hash. Dual-emitted during migration and will be removed later. |
| `X-NyxID-Message-Id` | UUID of the `channel_message` record |
| `X-NyxID-Timestamp` | ISO 8601 timestamp (for replay protection) |
| `X-NyxID-Platform` | Platform identifier (`telegram`, `discord`, `lark`, `feishu`) |
| `X-NyxID-User-Token` | Short-lived access token for the bot owner (`JWT_RELAY_ACCESS_TTL_SECS`, default `300`s). The agent uses it as `Authorization: Bearer <token>` to call NyxID's **proxy / LLM gateway / MCP** (and approval *status* polling) on behalf of the user. It is a relay token: it is **rejected** on account, admin, key-management, session, channel-reply, and other non-proxy endpoints; it inherits the originating agent key's service/node allowlist; on MCP it is stateless (no session is minted, so it re-authenticates every call); and it stops working the moment that agent key is revoked/deactivated. To *reply* to a conversation, use the agent API key or the per-callback `reply_token`, not this token. Absent if token generation fails. |

**NyxID-hosted native bot mentions.** A delivered Telegram/Lark/Feishu message
that positively and exactly mentions the receiving native bot may start a guest
turn for that bot's linked specialist or live chat override. The native group
must already have owner-enabled settings: explicit `members:everyone`, or the
ordinary default after the verified owner has talked there. Bot input never
creates a settings row, proves owner presence, redeems owner link codes, decides
cards or queues owner work. Private, self, unknown/system, reply-only, broadcast,
plain-name and unaddressed bot input stays closed, including with
`reply_mode:all`. A bot remains a guest if its ID overlaps a legacy owner ID.
Guest service, privacy, approval and machine restrictions continue through late
text/image delivery.

Automated turns atomically spend an allowance on the existing group/topic row
(`max_group_handoffs`, default six) and the owner's hourly
`max_group_handoffs_per_hour` handoff window. Only an admitted authenticated
human message resets the chat allowance; there is no timer refill. Replays and
uncertain/busy attempts are never refunded, and exhausted bot budgets do not
block human turns. Bots may use an existing active followed child but cannot
open or renew following. The budget is metadata in MongoDB and is shared across
replicas.

A human message that receives a proven pre-execution busy response may retry
after capacity returns. Its original request digest, deadline and reset claim
are retained with encrypted event context until the existing event expiry;
concurrent retries atomically claim one resumption and never refill bot hops
again. Changed requests and failed, dispatched or uncertain attempts cannot
use that retry path. Bot busy attempts remain consumed and silent.

For these NyxID-hosted native group messages, the event claim uses the stored
native message ID scoped to the receiving installation and actual chat. A
provider redelivery with a new NyxID relay UUID therefore cannot start another
bot turn or repeat a human turn/reset during the existing 24-hour event
retention window. The source
lookup checks the stored owner, receiving bot, route key, sender and chat;
unrelated source rows cannot supply that identity. Native gateway events retain
their existing provider event identity. The generic relay's human-message
delivery semantics are unchanged.

Gateway sources opt in through scoped `admission.bot_messages:native_mentions`
and pinned receiving identity only after authenticated capability discovery
advertises `native_bot_mentions_v1`. Existing connections reconcile through the
normal management sweep without sender OAuth or manual reconnect. Old gateways
keep their existing human-compatible source policy. Deploy CMA readers and
upgrade every NyxID replica before enabling
`CMAEG_NATIVE_BOT_MENTIONS_WRITES_ENABLED`; NyxID's sweep can then author
opt-ins. This keeps execution and delayed delivery on replicas that retain bot
guest identity and share the durable budget. Before an old-reader rollback,
remove opt-ins with the current
reader and clear gateway caches; disabling the write flag alone does not remove
stored policy. No separate thread-follow gateway upgrade is required for basic
mentions. Provider bot-to-bot delivery settings still apply before either server.

#### Callback Authentication (JWT)

NyxID signs every callback delivery with a dedicated RS256 JWT in `X-NyxID-Callback-Token`. The JWT header carries `kid`; fetch NyxID's public keys from `/.well-known/jwks.json` and select the matching key.

Claims:

| Claim | Value |
|---|---|
| `iss` | NyxID issuer (`JWT_ISSUER`) |
| `aud` | `channel-relay/callback` |
| `exp` | Expiration timestamp |
| `iat` | Issued-at timestamp |
| `jti` | Per-delivery callback token ID |
| `token_type` | `relay_callback` |
| `api_key_id` | Agent API key ID bound to the callback |
| `message_id` | Callback payload `message_id` |
| `platform` | Callback payload `platform` |
| `body_sha256` | Lowercase hex SHA-256 of the exact request body bytes |

Callback tokens have a 5-minute TTL (`JWT_RELAY_CALLBACK_TTL_SECS`, default `300`) and validation allows 60 seconds of clock skew. Compute `body_sha256` over the exact wire bytes received from the HTTP request; reformatting JSON, normalizing field order, changing whitespace, or adding a trailing newline changes the hash and must fail verification.

`payload.correlation_id` always equals the token `jti`. Each retry mints a fresh `jti` and `correlation_id`; idempotency is by `message_id`, not by `jti`.

During the transition, NyxID also dual-emits the legacy `X-NyxID-Signature` HMAC header over the same body bytes. That header remains for compatibility in this release and will be removed later.

### Identity Resolution (optional convenience API)

For agents that don't maintain their own user-to-platform mapping, NyxID provides a lookup endpoint. This is a convenience -- most agents integrated with NyxID OAuth already have this data from user onboarding.

```
GET /api/v1/channel-relay/resolve-sender?platform=telegram&platform_id=87654321
Authorization: Bearer nyxid_ag_xxxxx
```

**Response (linked):**
```json
{
  "platform": "telegram",
  "platform_id": "87654321",
  "nyxid_user_id": "770e8400-e29b-41d4-a716-446655440000",
  "linked": true
}
```

**Response (not linked):**
```json
{
  "platform": "telegram",
  "platform_id": "87654321",
  "nyxid_user_id": null,
  "linked": false
}
```

**Resolution checks** (in order):
1. `notification_channels` -- Telegram `telegram_chat_id` matched against `platform_id`
2. `user_provider_tokens` -- Telegram identity tokens with `telegram_user_id` metadata
3. Future: dedicated `channel_identity_links` collection for explicit cross-platform mapping

Scoped to the bot owner's account -- only resolves identities linked to the user who registered the bot.

### Callback acknowledgment

Return `202 Accepted` with an empty body, then POST the reply asynchronously. Inline HTTP 200 response bodies are not sent to the platform (ADR-013). `reply.text` is optional when declared media attachments or supported platform metadata carry the content.

### Agent -> NyxID (Async, HTTP 202 then POST later)

If the agent needs more time (LLM inference, tool calls, etc.), it returns `202 Accepted` with an empty body, then calls back when ready. The reply endpoint accepts either the agent's API key or the per-callback reply token minted with the inbound payload:

```
POST /api/v1/channel-relay/reply
Authorization: Bearer <nyxid_ag_xxxxx OR reply_token from callback>
Content-Type: application/json

{
  "message_id": "550e8400-e29b-41d4-a716-446655440000",
  "reply": {
    "text": "After checking multiple sources, the weather in Tokyo is 22C and sunny with 60% humidity.",
    "metadata": null
  }
}
```

**Async Reply Field Reference:**

| Field | Type | Nullable | Description |
|---|---|---|---|
| `message_id` | UUID | No | The `message_id` from the original inbound callback payload. Identifies which message this reply is for, so NyxID can resolve the correct conversation and platform to send the reply to. |
| `reply.text` | string | Yes | Text to send; optional for supported attachment-only replies. |
| `reply.attachments` | array | No | Up to ten outbound attachment sources; defaults to empty. See [Media](#media). |
| `reply.metadata` | object | Yes | Platform-specific extras supported by the adapter. |

<a id="reply-token"></a>
#### Reply Token

Each inbound callback carries a short-lived `reply_token` (RS256 JWT) that lets the agent post its async reply without holding the full agent API key. Intended for downstream runtimes (e.g. Aevatar) that would otherwise need to persist agent credentials and take on the associated secret-management burden.

| Property | Value |
|---|---|
| `aud` | `channel-relay/reply` (accepted only for reply, reply/update, and bound inbound media reads) |
| `token_type` | `relay_reply` |
| TTL | `JWT_RELAY_REPLY_TTL_SECS` (default 1800 = 30 min) |
| Max sends | `1` (duplicate `jti` → `401 "Reply token already used"`); attachment reads never consume it, and edits require prior consumption |
| Claim bindings | `api_key_id`, `conversation_id`, `inbound_message_id`, `platform` — all four must match the reply request; body's `message_id` must equal `inbound_message_id` |
| Revocation coupling | At reply time NyxID re-checks that the bound `api_key_id` is still active; a revoked key invalidates all outstanding tokens immediately |
| Replay store | MongoDB `reply_token_uses` collection, TTL-indexed on `exp_at` |
| Clock skew | 60s tolerance on both `iat` and `exp` |

The reply-token path skips the API-key branch's "caller must be the assigned agent for this conversation" check: the token was minted for a specific inbound message, so allowing the original callback recipient to complete its reply even after the conversation is reassigned is intentional. Narrowness is enforced by the four claim bindings above.

A leaked reply token authorizes one reply, later edits of that reply, and reads of the bound inbound message's attachments until expiry (default 30 minutes). Contrast with a leaked `full_key`, which grants unbounded access to everything the agent key can do until manually revoked.

### Callback Flow Decision

```mermaid
flowchart TD
    CB[Agent Callback POST] --> STATUS{Response Status?}

    STATUS -->|202| DISPOSITION[Parse bounded admission disposition]
    DISPOSITION --> ASYNC[Mark callback_status = delivered]
    STATUS -->|200 legacy, body ignored| ASYNC
    ASYNC --> WAIT[Agent calls /channel-relay/reply later]
    WAIT --> SEND[send_reply via adapter]
    SEND --> LOG_OUT[Store outbound routing metadata]

    STATUS -->|other HTTP status| ERR[Mark callback_status = failed]
    ERR --> OPT{Send error<br/>msg to chat?}
    OPT -->|configurable| ERRMSG[Platform: 'Agent unavailable']
    OPT -->|no| DONE[Done]

    STATUS -->|timeout| TO[Mark callback_status = failed, no HTTP status]
    TO --> OPT
```

---

## API Endpoints

### Platform Discovery (authenticated)

| Method | Path | Auth | Description |
|---|---|---|---|
| `GET` | `/api/v1/channel-platforms` | JWT/session, API key, service account, or delegated `account:read` token | All registered adapters, display names, registration/onboarding descriptors, configured booleans, ingestion and capabilities. |

### Bot Management (authenticated, human-only)

| Method | Path | Description |
|---|---|---|
| `POST` | `/api/v1/channel-bots` | Register a new bot. `label` is required except for `platform: "telegram"`, where a blank label uses the bot's Telegram `first_name`, then its username; a supplied label is stored unchanged. The response includes the saved `label`. |
| `POST` | `/api/v1/channel-bots/telegram/profile` | `{bot_token}` → `{username, display_name, label}` via Telegram `getMe`, so setup forms can suggest a name. Human-only, 20 requests per user per minute; the token is never stored, logged, or echoed. |
| `GET` | `/api/v1/channel-bots` | `?scope=user` lists personal bots; `?org_id=<id>` selects an administered org; `?scope=all` combines personal and administered-org bots |
| `GET` | `/api/v1/channel-bots/{id}` | Get bot details |
| `PATCH` | `/api/v1/channel-bots/{id}` | Update bot label or platform verification material |
| `DELETE` | `/api/v1/channel-bots/{id}` | Delete bot (deregisters webhook) |
| `POST` | `/api/v1/channel-bots/{id}/verify` | Re-verify bot token and webhook |

The list returns active (not deleted) bots, newest first, as `{ "bots": [...], "total": N }`.
Each bot's `user_id` identifies its personal or organization owner. `scope=all`
requires the same admin access as individual organization listings: member-only,
viewer-only and revoked memberships do not contribute bots. Memberships pointing
to missing organizations or personal accounts are excluded. `scope` and
`org_id` are mutually exclusive (HTTP 400); unknown `scope` values also return 400.
Omitting both parameters retains the personal listing. `scope=user` always uses
the authenticated caller's user ID; it does not accept a target user's ID.
Discover organization IDs through `GET /api/v1/orgs`, which returns `orgs` with
`id`, `slug`, `display_name`, and `your_role`. Pass an `id` whose `your_role` is
`admin` as `org_id` when listing its channel bots. A user may belong to multiple
organizations, so clients must select the relevant organization from this list.
Existing authentication requirements apply, including delegated `account:read`
for metadata reads; this does not grant bot-management access to Agent Keys.

### Conversation Routes (authenticated, human-only)

| Method | Path | Description |
|---|---|---|
| `POST` | `/api/v1/channel-conversations` | Create conversation -> agent route (callback URL resolved from `ApiKey.callback_url`) |
| `GET` | `/api/v1/channel-conversations` | List user's routes (filterable by bot, platform, agent) |
| `GET` | `/api/v1/channel-conversations/{id}` | Get route details |
| `PUT` | `/api/v1/channel-conversations/{id}` | Update route (change agent) |
| `DELETE` | `/api/v1/channel-conversations/{id}` | Delete route |

### Relay (agent-authenticated)

| Method | Path | Auth | Description |
|---|---|---|---|
| `POST` | `/api/v1/channel-relay/reply` | API key **or** reply token | Agent sends async reply to a message. See [Reply Token](#reply-token). |
| `POST` | `/api/v1/channel-relay/send` | Assigned API key or human owner | Initiate an opted-in target-addressed message; optional idempotency key. |
| `GET` | `/api/v1/channel-relay/conversations` | API key | Paginated active assignments, addressability, opt-in, and outbound capabilities. |
| `POST` | `/api/v1/channel-relay/reply/update` | API key **or** consumed reply token | Edit an anchored reply or API-key-authorized initiated message on Telegram, Discord, Slack, Lark, or Feishu. Send `outbound_message_id` (required for API-key edits on Telegram); ambiguous unpinned edits return 409. |
| `GET` | `/api/v1/channel-relay/messages/{message_id}/attachments/{index}` | Assigned API key **or** reply token | Download inbound media without consuming the reply token. |
| `GET` | `/api/v1/channel-relay/messages/{conversation_id}` | API key | Get conversation message history |
| `GET` | `/api/v1/channel-relay/resolve-sender` | API key | Resolve a platform sender to a NyxID user (query params: `platform`, `platform_id`) |

### Platform Webhooks (unauthenticated, signature-verified)

| Method | Path | Description |
|---|---|---|
| `POST` | `/api/v1/webhooks/channel/telegram/{bot_id}` | Telegram bot webhook |
| `POST` | `/api/v1/webhooks/channel/discord/{bot_id}` | Discord interaction webhook |
| `POST` | `/api/v1/webhooks/channel/lark/{bot_id}` | Lark event webhook |
| `POST` | `/api/v1/webhooks/channel/feishu/{bot_id}` | Feishu event webhook |
| `POST` | `/api/v1/webhooks/channel/slack/{bot_id}` | Slack Events API webhook |
| `GET`, `POST` | `/api/v1/webhooks/channel/whatsapp/{bot_id}` | Meta subscription verification and message webhook |

These share the generic adapter-driven route outside JWT authentication, with the same global rate limiting and body-limit posture as the existing channel webhooks. `delegated_read_denied_path` already denies the entire `webhooks` route class; the public subscription handler never extracts `AuthUser` or relies on delegated authorization.

---

## Security

### Threat Model

```mermaid
graph TD
    subgraph Threats
        T1[SSRF via callback URL]
        T2[Bot token leakage]
        T3[Webhook forgery]
        T4[Replay attacks]
        T5[Message injection in group chats]
        T6[Agent impersonation on async reply]
        T7[Unprompted message spam]
    end

    subgraph Mitigations
        M1[Block private IPs, HTTPS-only in prod]
        M2[AES-256 at rest, never in API responses]
        M3[Per-platform signature verification]
        M4[X-NyxID-Timestamp + replay window]
        M5[platform_sender_id scoping on routes]
        M6[api_key_id must match conversation agent]
        M7[Default-off human opt-in and per-conversation send limit]
    end

    T1 --> M1
    T2 --> M2
    T3 --> M3
    T4 --> M4
    T5 --> M5
    T6 --> M6
    T7 --> M7
```

| Concern | Mitigation |
|---|---|
| **SSRF** | Callback URLs validated: HTTPS-only in production, block RFC 1918/loopback ranges, optional domain allowlist |
| **Forged webhook attachment URLs** | Adapter-owned host allowlists, public-IP validation and DNS pinning, and no redirects. Credentials are never sent to a rejected host. |
| **Agent-supplied media URL sources** | HTTPS only, with the same public-host validation, IP pinning and redirect prohibition. |
| **Oversized media** | Content-Length pre-check plus streamed byte cap; base64 length checked before allocation. The raised JSON body cap applies only to `/reply` and `/send`. |
| **Download abuse** | Assigned-key or exact-reply-token binding and live authorization; API-key downloads use the per-agent limiter when configured. Reply-token reads do not consume the send JTI and remain subject to the group's global per-IP limiter. |
| **Large `/send` JSON before admission** | `/send` still parses the now-larger JSON body before its per-conversation limiter. The group's global per-IP limiter mitigates this work, and the route body cap remains bounded. |
| **Bot token storage** | AES-256 encrypted at rest (same pattern as `UserApiKey.credential_encrypted`). Never returned in API responses. Only `platform_bot_username` is exposed. |
| **Webhook forgery** | Per-platform verification: Telegram secret header, Discord Ed25519, Lark / Feishu Verification Token checks plus optional Encrypt Key signature verification and AES decryption. All comparisons use constant-time equality where applicable. |
| **WhatsApp verification** | POST HMAC verifies the exact raw body with the Meta App Secret before phone-number filtering. GET subscription checks SHA-256 of the one-time Verify Token in constant time. Bodies, secrets, and upstream free-form errors are never logged. |
| **Replay attacks** | Callbacks include `X-NyxID-Timestamp`; agents should reject messages older than 5 minutes. Callback JWTs also expire after 5 minutes with 60s skew tolerance. |
| **Callback authentication** | `X-NyxID-Callback-Token` is an RS256 JWT verifiable through `/.well-known/jwks.json`; its `body_sha256` claim binds the exact request bytes. `X-NyxID-Signature` HMAC is dual-emitted during transition and will be removed later. |
| **Agent impersonation** | Async reply endpoint accepts two auth paths: (a) the agent API key, which must match the conversation's `agent_api_key_id`; or (b) a per-callback reply token bound to a specific `inbound_message_id`, `conversation_id`, `api_key_id`, and `platform`, single-use, 30-min TTL, and revalidated against live `api_key.is_active` on every call. See [Reply Token](#reply-token). |
| **Proactive spam** | Default-off, human-only conversation opt-in; live assignment/key checks; concrete chat address and bot scope; shared per-conversation 1/s burst-5 admission before auth. A caller knowing a conversation UUID can consume its bucket, an intentional consequence of pre-auth limiting. |
| **Rate limiting** | Per-bot rate limiting on inbound webhooks. Per-agent rate limiting on callback dispatch (reuses `PerAgentRateLimiter` from agent isolation). |

### Migrating a stuck Lark / Feishu bot

Bots created before issue #455 may have the right App ID / App Secret but still remain in `pending_webhook` because the inbound Verification Token and optional Encrypt Key were never stored separately.

To self-heal an existing bot:

1. Read the bot's Event Subscriptions settings in the Lark / Feishu console.
2. Update the bot with its Verification Token and, if enabled in the console, its Encrypt Key.
3. Wait for the next verified inbound webhook or `url_verification` challenge. NyxID will auto-promote the bot from `pending_webhook` to `active` after successful verification.

You can update the bot either via the API or the CLI:

- `PATCH /api/v1/channel-bots/{id}` with `{ "verification_token": "...", "encrypt_key": "..." }`
- `nyxid channel-bot update <BOT_ID> --verification-token ... [--encrypt-key ...]`

---

## Tool Approval API (Aevatar Integration)

Aevatar agents use a **tool approval middleware** that pauses destructive tool calls and asks the user for permission before executing. The approval can be handled locally (in the chat UI) or remotely via NyxID. NyxID's `NyxIdToolApprovalHandler` creates an approval request on NyxID, then polls until the user decides (via Telegram push, mobile app, or web UI).

This is a **prerequisite** for the channel relay: when an agent receives a forwarded message and invokes tools on behalf of the user, destructive tools must go through the approval flow.

### Flow

```mermaid
sequenceDiagram
    participant A as Aevatar Agent
    participant M as Tool Approval Middleware
    participant H as NyxIdToolApprovalHandler
    participant N as NyxID API
    participant U as User (Telegram / Web / Mobile)

    A->>M: invoke tool (e.g. invoke_service)
    M->>M: Check approval mode + destructive flag
    M->>H: request_approval(tool_name, args, is_destructive)

    H->>N: POST /api/v1/approvals/requests
    Note over H,N: { tool_name, tool_call_id, arguments, is_destructive, approval_mode }
    N-->>H: { id, status: "pending", expires_at }

    N->>U: Push notification / Telegram message

    loop Poll (every 2s, up to 45s)
        H->>N: GET /api/v1/approvals/requests/{id}
        N-->>H: { status: "pending" }
    end

    U->>N: POST /api/v1/approvals/requests/{id}/decide { approved: true }

    H->>N: GET /api/v1/approvals/requests/{id}
    N-->>H: { status: "approved" }

    H-->>M: Approved
    M-->>A: Execute tool
```

### New Endpoint: Create Tool Approval Request

```
POST /api/v1/approvals/requests
Authorization: Bearer <user_access_token or api_key>
Content-Type: application/json

{
  "tool_name": "invoke_service",
  "tool_call_id": "call_abc123",
  "arguments": "{\"service_id\":\"...\",\"endpoint_id\":\"...\"}",
  "is_destructive": true,
  "approval_mode": "alwaysrequire"
}
```

**Response (201 Created):**
```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "status": "pending",
  "expires_at": "2026-04-01T12:05:00Z"
}
```

**Field Reference:**

| Field | Type | Required | Description |
|---|---|---|---|
| `tool_name` | string | Yes | Name of the tool requesting approval (max 256 chars) |
| `tool_call_id` | string | No | LLM-generated tool call ID for correlation |
| `arguments` | string | No | Serialized JSON of tool arguments (max 65536 chars) |
| `is_destructive` | bool | No | Whether the tool performs irreversible operations (default: false) |
| `approval_mode` | string | No | Aevatar's approval mode: `"alwaysrequire"`, `"auto"`, `"neverrequire"` (informational, stored but not enforced -- NyxID always creates the request) |

### Polling

Aevatar polls the existing `GET /api/v1/approvals/requests/{id}` endpoint. No dedicated tool-specific polling endpoint is needed — the existing endpoint returns the full `ApprovalRequestItem` which includes `status` and all tool fields.

NyxID uses `"rejected"` as the status value (not `"denied"`). Aevatar should check for `"rejected"` when determining if a request was denied.

### Rollback Compatibility

This design is fully rollback-safe:

| Concern | Design Decision | On Rollback |
|---|---|---|
| **New model fields** (`tool_name`, etc.) | Optional with `#[serde(default)]` | Old code ignores extra fields in MongoDB documents (no `deny_unknown_fields`) |
| **New endpoint** (`POST /requests`) | Additive only, no existing endpoint behavior changes | Old code returns 404; Aevatar times out (expected for new feature) |
| **Status values** | `"rejected"` used consistently everywhere -- no normalization layer | No change needed |
| **Sentinel `service_id: "tool_approval"`** | Tool approval documents appear in `list_requests` with `service_name` = tool name | Old frontend renders them as normal approval entries (odd `service_name` but functional) |
| **Frontend** | Additive: new tool approval rendering in approval list/detail pages. Guards on `tool_name != null` to distinguish tool vs proxy approvals | Old frontend ignores `tool_name` field (not in its type), renders tool approvals as normal entries with tool name as service name -- functional, just not pretty |

**Key rule**: existing endpoints (`GET /requests/{id}`, `GET /requests/{id}/status`, `POST /{id}/decide`) return the same response shapes and status values as before. All new behavior is on new paths.

### Model Changes

Four optional fields added to `ApprovalRequest` (existing collection, no migration needed):

```rust
// On ApprovalRequest model
pub tool_name: Option<String>,        // e.g. "invoke_service"
pub tool_call_id: Option<String>,     // LLM correlation ID
pub tool_arguments: Option<String>,   // serialized JSON
pub is_destructive: Option<bool>,     // destructive flag
```

Tool approval requests use sentinel values for proxy-oriented fields:
- `service_id: "tool_approval"`, `service_name: <tool_name>`, `service_slug: "tool"`
- `requester_type: "api_key"` or `"access_token"` (from auth context)
- `approval_mode: PerRequest` (tool approvals have no grant semantics)

### Relationship to Existing Approval System

The tool approval flow reuses the existing approval infrastructure:
- Same `approval_requests` MongoDB collection
- Same `process_decision` flow (web UI, Telegram, mobile push)
- Same notification pipeline (Telegram bot, FCM, APNs)
- Same `POST /{id}/decide` endpoint for user decisions

The only new surface is `POST /requests` for external creation and the response format alignment.

---

## Implementation Phases

### Phase 0: Tool Approval Endpoint

Prerequisite for agent tool execution. Backend + frontend changes to properly render tool approvals.

**Backend (modified):**
- `backend/src/models/approval_request.rs` -- add optional tool fields (`tool_name`, `tool_call_id`, `tool_arguments`, `is_destructive`), all `Option` with `#[serde(default)]`
- `backend/src/services/approval_service.rs` -- add `create_tool_approval_request()` function
- `backend/src/handlers/approvals.rs` -- add `create_request` handler (`POST /requests`), new request/response types (`CreateToolApprovalRequest`, `CreateApprovalResponse`). Existing `get_request_by_id` adds optional `tool_name`, `tool_call_id`, `tool_arguments`, `is_destructive` fields to `ApprovalRequestItem` (additive, no behavior change -- `null` for existing proxy approvals). Aevatar polls the existing `GET /requests/{id}` endpoint and checks for `"rejected"` status.
- `backend/src/routes.rs` -- add `POST /requests` to approval routes

**Frontend (modified):**
- `frontend/src/types/approvals.ts` -- add optional `tool_name`, `tool_call_id`, `tool_arguments`, `is_destructive` fields to approval request type
- `frontend/src/pages/approvals.tsx` -- distinguish tool vs proxy approvals in list: show "Tool: {tool_name}" badge when `tool_name` is present, skip service link for `service_id === "tool_approval"`
- `frontend/src/pages/approval-detail.tsx` (or equivalent decide page) -- show tool context: tool name, truncated arguments preview, destructive badge. Fall back to existing proxy display when `tool_name` is null

**Rollback**: old frontend ignores new fields, renders tool approvals as normal entries with tool name as service name.

### Phase 1: Foundation

Models, platform adapter trait, error codes, config. Frontend types and schemas (no UI yet).

```mermaid
gantt
    title Phase 1 - Foundation
    dateFormat  X
    axisFormat %s

    section Models
    channel_bot.rs           :a1, 0, 1
    channel_conversation.rs  :a2, 0, 1
    channel_message.rs       :a3, 0, 1
    Register in mod.rs       :a4, after a1, 1

    section Infrastructure
    Error variants (10000-10005)  :b1, 0, 1
    Config env vars               :b2, 0, 1
    DB indexes                    :b3, after a1, 1

    section Trait
    PlatformAdapter trait     :c1, 0, 1
    Normalized types          :c2, 0, 1
```

**Backend (new):**
- `backend/src/models/channel_bot.rs`
- `backend/src/models/channel_conversation.rs`
- `backend/src/models/channel_message.rs`
- `backend/src/services/channel_platform.rs` (trait + types)

**Backend (modified):**
- `backend/src/models/mod.rs` -- register modules
- `backend/src/services/mod.rs` -- register module
- `backend/src/errors/mod.rs` -- new error variants (10000-10005)
- `backend/src/config.rs` -- new env vars
- `backend/src/db.rs` -- new indexes

**Frontend (new):**
- `frontend/src/types/channels.ts` -- TypeScript types for `ChannelBot`, `ChannelConversation`, `ChannelMessage`, platform enum
- `frontend/src/schemas/channels.ts` -- Zod schemas for bot creation/update, conversation route creation/update

**Rollback**: no UI references these types yet; unused exports are harmless.

### Phase 2: Telegram Adapter

First platform adapter, reuses existing `telegram_service.rs`. No frontend changes (adapter internals).

**Backend (new):**
- `backend/src/services/channel_adapters/mod.rs`
- `backend/src/services/channel_adapters/telegram.rs`

### Phase 3: Core Services

Bot CRUD, conversation routing, relay orchestration. No frontend changes (service internals).

**Backend (new):**
- `backend/src/services/channel_bot_service.rs`
- `backend/src/services/channel_routing_service.rs`
- `backend/src/services/channel_relay_service.rs`

### Phase 4: Handlers, Routes & Bot Management UI

Wire up all backend endpoints. Ship the core frontend: bot management page, conversation route editor.

**Backend (new):**
- `backend/src/handlers/channel_bots.rs`
- `backend/src/handlers/channel_webhooks.rs`
- `backend/src/handlers/channel_relay.rs`

**Backend (modified):**
- `backend/src/handlers/mod.rs`
- `backend/src/routes.rs`
- `backend/src/main.rs` (webhook health check background task)

**Frontend (new):**
- `frontend/src/hooks/use-channel-bots.ts` -- TanStack Query hooks for bot CRUD
- `frontend/src/hooks/use-channel-conversations.ts` -- TanStack Query hooks for conversation routes
- `frontend/src/pages/channel-bots.tsx` -- bot list page: table with platform icon, bot username, status, active conversations count
- `frontend/src/pages/channel-bot-detail.tsx` -- bot detail: status, webhook health, conversation routes list, delete action
- `frontend/src/components/dashboard/add-channel-bot-dialog.tsx` -- create bot dialog: platform selector (Telegram only in Phase 4), bot token input, validation feedback
- `frontend/src/components/dashboard/channel-route-editor.tsx` -- conversation route CRUD: agent selector (from user's API keys), default agent toggle

**Frontend (modified):**
- `frontend/src/router.tsx` -- add `/channel-bots` and `/channel-bots/:id` routes
- `frontend/src/components/dashboard/sidebar.tsx` -- add "Channel Bots" nav item

**Rollback**: pages return 404 in router (removed routes), sidebar item disappears. No data loss -- bot records stay in MongoDB, webhook stays registered on Telegram (can be cleaned up manually or on re-deploy).

### Phase 5: Multi-Platform Adapters & Frontend

Discord, Lark, Feishu adapters. Frontend platform selector updated to support all platforms.

**Backend (new):**
- `backend/src/services/channel_adapters/discord.rs`
- `backend/src/services/channel_adapters/lark.rs`

**Backend (new dependency):**
- `ed25519-dalek` (Discord signature verification)

**Frontend (modified):**
- `frontend/src/components/dashboard/add-channel-bot-dialog.tsx` -- platform selector expands from Telegram-only to include Discord, Lark, Feishu. Each platform shows platform-specific setup instructions (e.g., Discord: Application ID + Bot Token; Lark: App ID + App Secret)
- `frontend/src/pages/channel-bot-detail.tsx` -- platform-specific status indicators and webhook health display
- `frontend/src/types/channels.ts` -- add Discord/Lark/Feishu-specific fields to `ChannelBot` type

**Rollback**: old frontend only shows Telegram in platform selector. Discord/Lark/Feishu bots created before rollback still appear in list (platform field renders as raw string) but can't be managed via the old create dialog.

### Phase 6: OpenClaw Bridge Migration

Migrate existing `openclaw_channel_mappings` to the generic relay. Backward-compatible dual-path lookup.

**Backend (new):**
- `backend/src/services/channel_adapters/openclaw.rs`

**Backend (modified):**
- `backend/src/handlers/openclaw_channel.rs` (dual-path lookup: check new `channel_conversations` first, fall back to legacy `openclaw_channel_mappings`)

**Frontend (modified):**
- `frontend/src/pages/channel-bots.tsx` -- show migrated OpenClaw bots alongside user-registered bots (read-only badge for auto-migrated entries)
- `frontend/src/hooks/use-channel-bots.ts` -- filter/display logic for `platform: "openclaw"` entries

**Rollback**: dual-path lookup means old code still reads from `openclaw_channel_mappings` directly. New entries written to `channel_conversations` won't be visible to old code, but old entries continue to work. No data loss.

### Phase 7: Message Log & Polish

Conversation message history viewer, delivery status dashboard, and UX polish.

**Frontend (new):**
- `frontend/src/pages/channel-conversation-detail.tsx` -- message log: inbound/outbound messages with timestamps, delivery status, sender info, content preview
- `frontend/src/hooks/use-channel-messages.ts` -- TanStack Query hooks for message history (paginated)
- `frontend/src/components/dashboard/channel-message-list.tsx` -- chat-style message list component with direction indicators

**Frontend (modified):**
- `frontend/src/pages/channel-bot-detail.tsx` -- add "View Messages" link per conversation route, delivery stats summary (delivered/failed/timeout counts)
- `frontend/src/router.tsx` -- add `/channel-conversations/:id/messages` route

**Rollback**: message log pages return 404. Bot management from Phase 4 continues to work.

---

## Phase Dependency Graph

```mermaid
graph LR
    P0[Phase 0<br/>Tool Approval<br/>BE + FE] --> P1[Phase 1<br/>Foundation<br/>BE + FE types]
    P1 --> P2[Phase 2<br/>Telegram Adapter<br/>BE only]
    P1 --> P3[Phase 3<br/>Core Services<br/>BE only]
    P2 --> P3
    P3 --> P4[Phase 4<br/>Handlers + Bot UI<br/>BE + FE]
    P4 --> P5[Phase 5<br/>Multi-Platform<br/>BE + FE]
    P4 --> P6[Phase 6<br/>OpenClaw Migration<br/>BE + FE]
    P4 --> P7[Phase 7<br/>Message Log<br/>FE only]

    style P0 fill:#fce4ec
    style P1 fill:#e1f5fe
    style P2 fill:#e1f5fe
    style P3 fill:#fff3e0
    style P4 fill:#fff3e0
    style P5 fill:#f3e5f5
    style P6 fill:#f3e5f5
    style P7 fill:#e8f5e9
```

---

## Environment Variables

| Variable | Default | Description |
|---|---|---|
| `JWT_RELAY_CALLBACK_TTL_SECS` | `300` | Lifetime for `X-NyxID-Callback-Token` JWTs |
| `CHANNEL_RELAY_INITIATE_RATE_LIMIT_PER_SECOND` | `1` | Shared per-conversation proactive send rate |
| `CHANNEL_RELAY_INITIATE_RATE_LIMIT_BURST` | `5` | Proactive send burst capacity |
| `CHANNEL_MEDIA_MAX_BYTES` | `20971520` | Per-attachment download/materialization cap (20 MiB); only reply/send body limits are raised to `ceil(cap / 3) * 4 + 65536`. |
| `CHANNEL_RELAY_CALLBACK_TIMEOUT_SECS` | `30` | HTTP timeout for agent callback requests |
| `CHANNEL_RELAY_MAX_BOTS_PER_USER` | `5` | Maximum bots a user can register |
| `CHANNEL_RELAY_MESSAGE_TTL_DAYS` | `30` | TTL for `channel_messages` auto-cleanup |
| `CHANNEL_POLL_INTERVAL_SECS` | `30` | Generic polling sweep interval; `0` disables. Each adapter's minimum interval, backoff and lease also apply. |

---

## Relationship to Existing Systems

| Existing System | Relationship | Migration Path |
|---|---|---|
| **Telegram approval bot** (system-level) | Completely separate. The admin's global bot for approval notifications is untouched. | None needed |
| **Telegram Login Widget** (identity provider) | Separate. Uses Telegram for authentication, not messaging. | None needed |
| **OpenClaw channel bridge** | Superseded. The new relay is a generalized version. | Phase 6: dual-path lookup, gradual migration |
| **Agent isolation** (PR #132) | Complementary. `ApiKey.id` is the `agent_api_key_id` reference. Proxy scope enforcement applies when agents make proxy calls. | Already integrated via shared `ApiKey` model |
| **Proxy gateway** | Parallel path. Relay forwards messages; proxy forwards API calls. Agents may use both. | None needed |
| **Approval system** | Extended. Tool approval creation endpoint (`POST /requests`) added for Aevatar agent tool execution. Reuses existing approval infrastructure. | Phase 0: add creation endpoint + response format alignment |

---

## Example: End-to-End Scenario

### Setup (one-time)

```mermaid
sequenceDiagram
    participant U as User
    participant N as NyxID
    participant TG as Telegram API

    Note over U,N: Step 1: Register agent with callback URL
    U->>N: nyxid ai-setup agent create<br/>--name claude-support<br/>--platform claude-code<br/>--callback-url https://my-claude.example.com/webhook
    N-->>U: API key: nyxid_ag_xxxxx (api_key_id: 880e...)

    Note over U,N: Step 2: Register Telegram bot
    U->>N: POST /api/v1/channel-bots<br/>{ platform: "telegram", bot_token: "123:ABC" }
    N->>TG: getMe (verify token)
    TG-->>N: { username: "MySupportBot" }
    N->>TG: setWebhook (register NyxID webhook URL)
    N-->>U: Bot registered (id: 660e...)

    Note over U,N: Step 3: Route conversations to agent
    U->>N: POST /api/v1/channel-conversations<br/>{ channel_bot_id: "660e...",<br/>  agent_api_key_id: "880e...",<br/>  default_agent: true,<br/>  resolve_sender_identity: true }
    N-->>U: Route created -- all DMs to MySupportBot go to claude-support
```

The callback URL is on the **agent** (API key), not the conversation route. If the user later creates a second agent ("gpt-research") with a different callback URL and routes a Discord bot to it, the same pattern applies.

### Runtime

```mermaid
sequenceDiagram
    participant Alice as Alice (Telegram)
    participant TG as Telegram API
    participant N as NyxID
    participant C as Claude Agent

    Alice->>TG: "Summarize my emails"
    TG->>N: Webhook POST (message from Alice in DM)
    N->>N: Verify Telegram signature
    N->>N: Parse message, resolve route -> claude-support (api_key_id: 880e...)
    N->>N: Resolve callback_url from ApiKey: https://my-claude.example.com/webhook
    N->>C: POST https://my-claude.example.com/webhook<br/>{ agent: { api_key_id: "880e...", name: "claude-support" },<br/>  sender: { platform_id: "87654321", nyxid_user_id: "770e..." },<br/>  content: { text: "Summarize my emails" } }
    C->>C: Match nyxid_user_id to internal user (from NyxID OAuth)
    C->>C: Process with LLM + tools
    C-->>N: 200 { reply: { text: "You have 3 unread..." } }
    N->>TG: sendMessage(chat_id, "You have 3 unread...")
    TG->>Alice: "You have 3 unread..."
```

**Meanwhile, on Discord (same user, different agent):**

```mermaid
sequenceDiagram
    participant Bob as Bob (Discord)
    participant DC as Discord API
    participant N as NyxID
    participant G as GPT Agent

    Bob->>DC: "Generate a report"
    DC->>N: Webhook POST (interaction from Bob)
    N->>N: Verify Ed25519 signature
    N->>N: Parse message, resolve route -> gpt-research (api_key_id: 990e...)
    N->>N: Resolve callback_url from ApiKey: https://my-gpt.example.com/webhook
    N->>G: POST https://my-gpt.example.com/webhook<br/>{ agent: { api_key_id: "990e...", name: "gpt-research" },<br/>  content: { text: "Generate a report" } }
    G-->>N: 202 Accepted (async, needs time)
    N-->>DC: Ack interaction

    Note over G: Agent processes for 30 seconds...

    G->>N: POST /channel-relay/reply<br/>Authorization: Bearer nyxid_ag_yyyyy<br/>{ message_id: "...", reply: { text: "Report: ..." } }
    N->>DC: Create message in channel
    DC->>Bob: "Report: ..."
```

## Aurinko email channel

Aurinko is an account-token email adapter with a separate application signing secret. Signed subscription challenges work during pending setup; account/subscription-bound notifications fetch mail and deliver it inline under producer-owned retries. Durable metadata claims distinguish active work from completion, preserve callback UUIDs across retries, and fence uncertain message-bound replies. This adapter returns retryable HTTP failures rather than the legacy always-ACK behavior. See [Aurinko integration](./AURINKO_INTEGRATION.md) for setup, CLI/API contracts, filtering, routing, credential lifecycle, and deletion outcomes.

## Tracked bot connection links

Reference: [bot-link HTTP API](API.md#bot-connection-links), [existing service connection links](API.md#third-party-connector-integration), and [CLI commands](site/cli/reference/others.md#channel-bot-connect-link).

Applications and agents can create a single-use setup request through
`POST /api/v1/channel-connect-links`. This uses the separate
`channel_connect_links` collection and `/connect/bot/{token}` page. Existing
service connector links (`/connect-links`, `/connect/{token}`) keep their current
request, response, storage, callback and webhook contracts. The reusable channel
setup URLs also remain available.

```json
{
  "platform": "discord",
  "label": "Support bot",
  "requested_by": "Support platform",
  "callback_url": "https://app.example.com/setup/return?state=your-request-id",
  "webhook_url": "https://app.example.com/events/nyxid",
  "expires_in": 900
}
```

The response contains `id`, `connect_url`, `expires_at`, and optional
`webhook_signing_secret` / `webhook_signing_key_id`. Share `connect_url` with the
human owner. `platform`, `label`, and the personal or organization owner
(`target_org_id`) are fixed when the request is created. Only a signed-in human
with write access to that owner can complete it; agent keys can create, inspect,
and cancel requests. Merely opening the link does not create a bot.

`callback_url` is an optional **browser return destination**. After setup, the
user clicks Continue after saving any one-time platform verification secret.
The return URL preserves caller query parameters and replaces `status`,
`channel_connect_link_id`, and `bot_id` with the actual result. The browser return
is useful for navigation; use the signed webhook as the trusted completion
signal. No bot credential or platform verification secret appears in either
callback.

`webhook_url` is a **server-to-server completion destination** for session/CLI/agent
callers. It must use HTTPS and resolve to public addresses. It is checked again
at delivery, with DNS addresses pinned and redirects disabled. The signing
secret is returned once and stored encrypted. OAuth app callers instead use
their existing registered connection webhook and its signing key; they cannot
supply `webhook_url`, and any browser callback must satisfy the app's registered
OAuth redirect URI matching policy. Existing app connection webhooks receive the new event types only
when that app creates bot connection requests.

The CLI requires `--webhook-signing-secret-file <new-path>` together with
`--webhook-url`. It saves the secret to that file (mode `0600` on Unix), refuses
existing files and symlinks before creating a request, and prints only the file
path and signing key ID. CLI JSON uses `webhook_signing_secret_file` instead of
returning the raw secret. The direct HTTP API response above is unchanged.

Terminal events are `channel_connect.completed`, `channel_connect.cancelled`,
and `channel_connect.expired`. Example:

```json
{
  "event_id": "stable-event-uuid",
  "event_type": "channel_connect.completed",
  "occurred_at": "2026-09-30T08:00:00Z",
  "data": {
    "user_id": "owner-uuid",
    "channel_connect_link_id": "request-uuid",
    "platform": "discord",
    "status": "completed",
    "bot_id": "bot-uuid",
    "bot_status": "pending_webhook",
    "webhook_registered": false,
    "is_active": true,
    "last_error": null,
    "expires_at": "2026-09-30T08:15:00Z"
  }
}
```

Completion means the bot was saved successfully. Some platforms still need
manual webhook configuration, so receivers must inspect `bot_status` and
`webhook_registered` before assuming inbound messages are ready. Agent routing
is configured separately. `user_id` is the bot owner, including an organization
UUID for org-owned bots.

Verify `X-NyxID-Signature` (`sha256=<hex digest>`) as HMAC-SHA256 over
`X-NyxID-Timestamp + "." + raw_request_body` using the signing secret. The other
headers are `X-NyxID-Event`, `X-NyxID-Delivery-Id`, and `X-NyxID-Key-ID`. Validate the
timestamp within your allowed replay window, compare signatures in constant
time, deduplicate by `event_id`, and respond with 2xx. The event ID, occurrence
time, and result snapshot remain stable across delivery cycles. Delivery is
at least once: a lost acknowledgement can cause a duplicate.

Terminal state and the pending notification are stored in one atomic write.
Retries use a five-minute delivery lease, up to five cycles of three bounded
HTTP attempts each. A separate bot-link sweep recovers interrupted delivery,
expired requests, and provider/Telegram completion without app polling. It uses
`CONNECT_LINK_EXPIRY_SWEEP_INTERVAL_SECS`; setting that to `0` disables background
recovery for both kinds of link. Bot notifications have a separate app rate
window so they do not consume the existing connector webhook delivery quota.

`GET /api/v1/channel-connect-links/{id}` exposes the result and `delivery_status`
(`pending`, `delivered`, `abandoned`, or `none`) for diagnostics. It never returns
the hosted token, webhook destination, or signing secret. A link defaults to 15
minutes and accepts a lifetime of 60–3600 seconds. A pinned setup gets 30 minutes
of recovery grace, including Telegram work in `ready` or `provisioning`. After
that window, incomplete links expire and notify the app. A late Telegram worker
can still finish the separately authorized bot, but it cannot change a terminal
link or send a second terminal event. The Telegram approval request has its own
15-minute lifetime; cancellation or expiry before bot creation ends this
single-use link too. Create a fresh link to start another attempt. For org-owned
Telegram setup, the administrator who started the approval must resume it;
other administrators can view link progress without taking over that consent.
Cancellation through
`POST /api/v1/channel-connect-links/{id}/cancel` is available before provider
setup starts. Once resources exist, resume the saved setup instead of creating
another bot. Manual and managed operations continue if the browser disconnects;
the hosted page offers recovery for saved bots and authorized OAuth connections.

### Bot-link implementation and compatibility

Bot links have their own collection and terminal state transitions. The existing service-link model, handlers, hosted page, and callback fields remain unchanged. The shared app lookup validates bot callbacks through the same OAuth redirect policy used by service links.

| Responsibility | Implementation |
|---|---|
| Stored link, token hash, encrypted direct receiver, and terminal event | [`models/channel_connect_link.rs`](../backend/src/models/channel_connect_link.rs) |
| Owner checks, renewable claims, atomic bot association, completion, recovery, and delivery | [`services/channel_connect_link_service.rs`](../backend/src/services/channel_connect_link_service.rs) |
| Request validation, public preview, human setup, and managed progress streaming | [`handlers/channel_connect_links.rs`](../backend/src/handlers/channel_connect_links.rs) |
| Bot creation with an optional link claim | [`services/channel_bot_service.rs`](../backend/src/services/channel_bot_service.rs) |
| Telegram request creation and worker completion | [`services/telegram_new_service.rs`](../backend/src/services/telegram_new_service.rs), [`services/telegram_new_connect.rs`](../backend/src/services/telegram_new_connect.rs) |
| Registered-app receiver and shared signature sender | [`services/developer_webhook_service.rs`](../backend/src/services/developer_webhook_service.rs), [`services/webhook_delivery_service.rs`](../backend/src/services/webhook_delivery_service.rs) |
| Routes, API discovery, indexes, and independent sweep task | [`routes.rs`](../backend/src/routes.rs), [`api_docs.rs`](../backend/src/api_docs.rs), [`db.rs`](../backend/src/db.rs), [`main.rs`](../backend/src/main.rs) |
| Hosted page and optional context for existing setup components | [`pages/channel-connect-link.tsx`](../frontend/src/pages/channel-connect-link.tsx), [`hooks/use-channel-connect-link.tsx`](../frontend/src/hooks/use-channel-connect-link.tsx) |
| Create, status, and cancellation commands | [`commands/channel_bot.rs`](../cli/src/commands/channel_bot.rs) |

Only one setup attempt can hold a link's renewable claim. Bot insertion and its link association commit together; Telegram request insertion and its link association also share a transaction. Completion checks the saved bot's readiness, then commits the terminal state and notification snapshot together. Delivery failure leaves the terminal result intact and schedules another attempt until the retry limit is reached.

Existing bot creation entry points pass no link claim, and existing frontend callers use their original endpoints outside the hosted context. Existing service callbacks retain their HTTP client, signature contract, timestamp behavior, and `developer_webhook` rate bucket. Bot callbacks opt into public-address validation, DNS pinning, redirects disabled, and the separate `developer_channel_webhook` bucket. Their sweep runs in a separate task while sharing the [configured sweep interval](ENV.md#hosted-connect-links).

Regression coverage lives beside the new service and handlers, in [`telegram_new_tests.rs`](../backend/src/services/telegram_new_tests.rs), and in the [hosted page tests](../frontend/src/pages/channel-connect-link.test.tsx). Existing `connect_link_service`, `connect_links`, `developer_webhook_service`, and CLI `commands::connect` tests cover the shared compatibility boundary.

### X channel workflow, health and recovery

**Supported workflow.** An X DM channel delivers through X Activity webhooks:

1. X posts `dm.received` to `POST /api/v1/webhooks/channel/x/platform`.
2. NyxID verifies the platform signature (`x-twitter-webhooks-signature[-oauth2]`) with the shared X app secret. It reads the target from `data.filter.user_id` and requires the subscription tag `nyxid:{bot_id}`.
3. The bot is found by `platform: "x"` and its X user id. Inbound billing admission runs, and the route (`channel_conversations`) forwards the message.
4. A bot linked to NyxBot over NyxID's direct relay calls back `/api/v1/nyxbot/relay/{channel_id}`. The owner's verified X user (`owner_sender_ids`, set with the link code) continues NyxBot's home thread. With private chats open to everyone, other X users get their own guest thread with no tools.
5. NyxBot's reply leaves through `POST /api/v1/channel-relay/reply` (`async_reply`) and becomes `POST /2/dm_conversations/{id}/messages` on X.

Polling remains a legacy DM-only fallback while `BILLING_ENABLED=false`. Paid deployments fail polling X bots with `webhook_required`. The Agent Event Gateway takes X only when `nyxbot:gateway-x` is on. Health then reports `recommended_action: "gateway"`, because delivery diagnostics live on the gateway.

**Platform limitations.**
- Mentions, replies and other public events need webhook credentials and `tweet.write`.
- Public replies are text-only.
- X rate limits are honoured: a poll backs off until the reset; a refused reply is not retried.
- `402` means the shared X app has run out of its own X API credits. Only an administrator can restore them. This is separate from NyxID credits; NyxID billing suspension is `billing_suspended`.

**Health endpoints.** All reads are metadata only:

| Method | Route | Access |
| --- | --- | --- |
| GET | `/api/v1/channel-bots/{id}/health` | Owner or org member who can read the bot; `can_write` says whether the caller may act |
| POST | `/api/v1/channel-bots/{id}/resubscribe` | Owner write access; X only. Re-runs subscription setup, then a live read-back. Audited as `channel_bot_resubscribed` |
| GET | `/api/v1/admin/channel-bots?platform=x&status=failed&limit=&offset=` | Admin; ordered by id |
| GET | `/api/v1/admin/channel-bots/{id}/health` | Admin |
| GET | `/api/v1/admin/channel-bots/webhook-rejections` | Admin; per-platform counters of unauthenticated rejections |

CLI: `nyxid channel-bot health <id>` prints the diagnostics and the recommended action, and `nyxid channel-bot show <id>` appends the same block.

**Diagnostics.** Only authenticated deliveries write the bot row:
- `last_inbound_at`/`last_inbound_source` and `last_webhook_at` are stamped after billing admission.
- `last_outbound_at`/`last_outbound_error` are written by every send path: NyxBot and agent replies through `/channel-relay/reply`, thread replies, and `/channel-relay/send`. The error has the numeric `code` and the stable `reason`.
- `last_webhook_error` and `failure_reason` hold stable codes, and `subscription_state` holds `{registered, checked_at, last_error}`.
- A successful authenticated delivery clears webhook errors and delivery-shaped failure reasons.

Forged or mis-targeted deliveries (`signature`, `target`, `platform_mismatch`) never touch a bot. They are counted per platform in `channel_webhook_rejections`, with at most one write per platform and reason every 30 seconds on each replica.

Verify and Resubscribe finish with a live read-only check: NyxID's callback must be a valid webhook in `GET /2/webhooks`, and every selected event must be subscribed with the bot's tag in `GET /2/activity/subscriptions`. A missing subscription persists `registered: false` with `subscription_missing` or `webhook_missing`. Missing platform webhook credentials on Resubscribe persist `webhook_unconfigured`.

**Failure codes and actions.** One recommendation function (`services/channel_bot_health.rs`) serves Health, the `channel_bot_failed` audit and the owner notice:

| `failure_reason` / reply `reason` | Meaning | `recommended_action` |
| --- | --- | --- |
| `reconnect_required`, `auth_revoked` (401), `scopes_missing` (403 scope), `access_denied` (other 403) | The account connection cannot be used | `reconnect` |
| `app_access_denied` (403 client-not-enrolled / code 453), `provider_credits` (402), `webhook_unconfigured` | The shared X app or platform credentials need an administrator | `contact_admin` |
| `billing_suspended` | NyxID billing stopped the channel | `enable_billing` |
| `webhook_required`, `webhook_setup_failed`, `webhook_missing`, `subscription_missing`, `unregistered` | The subscription is missing or incomplete | `resubscribe` |
| `rate_limited` (429) | Poll backoff is active (compared with now) or the last reply was limited | `wait_rate_limit` |
| `provider_unavailable` (5xx), `provider_error` | X failed; NyxID retries | `verify` |
| `recipient_unreachable` | The last reply's recipient does not accept DMs | `none` |

The first transition to `failed` sends one best-effort notice through the owner's Telegram and push channels. For an org-owned bot it goes to every org admin. A repeated failure sends nothing; a later failure after recovery, such as a billing suspension, sends one again. Notification failures never undo the status change.

**Reproduce locally** (MongoDB replica set plus wiremock; no X credentials):
- `handlers::nyxbot::tests::x_bus_tests::x_dm_reaches_nyxbot_and_the_reply_reaches_x_with_diagnostics` covers the full chain:
  - a signed DM is relayed over HTTP to NyxBot;
  - it lands in the home thread, and NyxBot's reply is posted to X;
  - a guest DM gets its own thread with no tools;
  - an X 403 and then a 429 on the reply are recorded with the Health action.
- `handlers::channel_webhooks::diagnostics_tests` shows that only authenticated deliveries write diagnostics.
- `services::channel_x_tests::webhooks::verify_live_check_persists_a_missing_subscription_and_resubscribe_restores_it` and `resubscribe_without_platform_webhook_credentials_asks_an_admin` cover the live check and Resubscribe.
- `services::channel_bot_health` holds the recommendation table, failure codes, rejection counters and org-admin notices.

**Staging verification** needs a staging X app with OAuth consent for the DM scopes, X API credits and Activity API access, and the platform webhook credentials (`app_bearer_token`, `consumer_secret`) in Admin > Platform Credentials.
1. Connect an account, then run Verify. Health should show `subscription_state.registered: true` and `recommended_action: "none"`.
2. Link the bot to NyxBot and send the link code from the owner's X account.
3. DM the bot. Confirm the reply arrives and Health shows `last_inbound_at` and `last_outbound_at`.
4. Delete the subscription in X, run Verify, and expect `resubscribe`.
5. Run Resubscribe and expect `none`.

Production credentials are never used in local tests.


## Platform-managed onboarding installations

Platform BotFather uses a dedicated native receiver rather than the gateway's
personal provider handshake. A durable platform manager or installation marker
closes ordinary conversation routes, NyxBot relinking, private/group defaults,
thread following and alternate agent setters. Missing setup/profile authority,
stale revisions and inactive installations fail closed; deleting or expiring setup
records never enables personal fallback. Legacy channels without an explicit marker
retain their existing behavior.

Public child effects require current reviewed publication, owned specialist,
manager/child identity, funding, quotas and activation. In-flight replies and
creation workers carry exact revision/digest/generation fences. Previous gateway
cleanup retains the captured remote version; admin recovery can preserve a changed
binding without deleting it. See [Platform BotFather](PLATFORM_BOTFATHER.md).

### Assistant multipart text receipts

NyxBot manager replies, explicit posts and automation results share the ordinary
relay's platform-aware boundary machinery and 64-message bound. Assistant Markdown
is planned into independently valid fragments; Telegram uses explicit UTF-16
entities. Long replies retain their text. Only explicit overflow, web-only action
review and unavailable media add app links. See [Full saved reply delivery](TELEGRAM_NEW.md#full-saved-reply-delivery)
for the send, attachment and rollout contract.

The separate metadata-only `channel_text_deliveries` journal binds an immutable
plan to its owner and logical source. Each part has a fresh CAS attempt, confirmed
provider receipt, and pending/sending/sent/refused state. An accepted prefix is
retained on failure; sending without a receipt is permanently uncertain. A
confirmed Telegram rate refusal returns only that part to pending. Every later
provider effect rechecks live authority and target. Proactive `text_delivery`
counts expose partial outcomes without exposing text. Stable automation run IDs
and authored turn-post identities prevent repeated calls from replaying accepted
parts; distinct authored posts in the same turn remain independent. Changed
content under the same automation run is rejected. These journals have no TTL
and are purged on owner deletion. They do not replace ordinary relay receipts or
the legacy/v2 image migration checks.

### Channel reply length

NyxBot and specialist turns destined for a chat receive turn-only length guidance
from the same per-platform limits as the delivery planner: Telegram 4,096 UTF-16
code units, Discord 2,000, and each other adapter's own budget. The model is asked
to fit one message unless the user explicitly requests long or complete output.
This is a preference: content is never truncated or discarded, and longer output
still uses the lossless multipart planner. The guidance follows direct/gateway
replies, followed threads, additional chat recipients, manager personal replies
and automation result destinations. Assigned specialists inherit their reporting
turn's chat guidance. `list_channel_chats` tool results supply destination-specific
`reply_length_guidance` before `post_to_chat` drafting; explicit posts accept up
to the existing 32,768-character turn-input bound and still split for delivery.
App-only turns receive no channel-length guidance. Stable instructions stay
independent of the destination.

### Telegram forwarded input and approval provenance

On direct NyxID delivery, NyxBot extracts visible text/captions, hidden link URLs
(using Telegram UTF-16 offsets), forward origin and quotes as third-party reference
data. Current `nyxid_relay` gateway delivery supplies only gateway plain text plus
authenticated original-message eligibility and own/quoted file handles; it loses
forward-origin names, quote/external-reply text and hidden link URLs. The product
owner accepted this demo limitation; no body persistence or CMA change is included. The relay envelope retains the exact provider text
for gateway verification. Opaque file handles resolve with the selected channel's
current token. Manager setup/proof material is scrubbed before model input.
`ChannelMessage.original_message` is optional boolean metadata. Only `true` from
the authenticated original event permits owner linking or action-card decisions;
missing legacy metadata fails closed. Followed-thread dispatch uses the same
stored eligibility, exact owner and route checks.

NyxID creates Telegram NyxBot gateway channels exclusively with `nyxid_relay`
sources. Relay callback text and attachments describe only the sending message's
own top-level fields, as required by the existing CMA verifier. Stored inbound
attachment metadata preserves the exact callback list as a prefix and appends
unique `external_reply` handles. Every gateway download index therefore still
selects the same stored file. NyxBot resolves the complete list and eligibility
using the exact message/channel/route-key/sender binding; no gateway change or
`telegram_input` envelope is required for that recovery. An external-reply-only
file leaves a text-only callback with no envelope attachments and remains
compatible with both existing gateway media modes. Top-level media still uses
the gateway's existing media-enabled setting.

For native gateway activity, the optional additive `telegram_input` contract is
`{"version":1,"original":false,"message":{...}}`. Its selected text, entities,
captions, forward origin, quotes and media handles must be derived at
authenticated ingress and bound into the activity digest/seal, without nested
setup messages or keyboards. NyxID bounds this envelope to 64 KiB. Native-source
events without it remain plain text and cannot decide actions, pair an owner or
run controls. NyxID does not provision native Telegram gateway sources. Richer
forwarding for both relay and native sources and gateway rendering of quoted
files are separate, optional [CMA follow-up work](plans/2026-10-21-cma-telegram-forwarding-followup.md).

The owner turn atomically binds encrypted inbound files with its message, checks
owner isolation and applies existing upload retention. Unsupported binary media
is retained without fabricated extracted text. Aggregate downloaded bytes are
bounded by the configured limit and 20 MiB; all new and retry-carried attachments
count against the ten-file turn limit.
