# Telegram bot creation

`telegram-new` adds Telegram's native bot creation to **Channel Bots → Add Channel Bot**. The customer creates a bot in Telegram and approves its connection to a named NyxID destination. NyxID retrieves and encrypts the child bot token on the server. The display labels are **Telegram** for `telegram-new` (create a bot) and **Telegram bot token** for `telegram` (connect an existing bot). The platform identifiers, API routes, CLI arguments, and existing saved connections are unchanged.

This is a channel configuration capability. The wider onboarding wizard, onboarding drafts, Mini Apps, and Meta login flows are outside this change.

## What the customer sees

1. Open **Channel Bots → Add Channel Bot**, choose **Telegram**, select the personal or organization destination, and enter a label.
2. Click **Prepare Telegram bot**, then **Open Telegram**. Preparing saves the request before the external link becomes available.
3. In the platform's manager chat, tap **Start** if Telegram asks. The chat identifies the NyxID website, destination account, and initiating account by stable identifiers.
4. Tap **Create bot**. Telegram opens its native form with a suggested name and username derived from the NyxID label. Both remain editable; Telegram checks username availability. For example, `nyx_test_123456` suggests the username `nyx_test_123456_bot`. Username suggestions use ASCII letters, digits, and underscores, fit within 32 characters, and end in `bot`.
5. In the manager chat, review the exact bot username, numeric bot ID, and destination. Tap **Approve this bot**.
6. Use **Return to NyxID**, or reopen the existing browser page. Choose **Connect bot**. NyxID fetches the token, saves the connection, and registers the bot's message webhook.
7. Assign an agent using the existing channel-bot configuration and send a test message to the new bot.

The normal customer path does not use BotFather or require copying a token. The platform administrator still needs BotFather for the manager setup below.

## Administrator setup

Use a dedicated manager per environment. Do not reuse the NyxID notification/approval bot or a bot already registered as a channel.

1. In BotFather, create or select the platform's manager bot and enable management of other bots. Its Bot API `getMe` response must contain `can_manage_bots: true`.
2. Confirm that `BASE_URL` is the externally reachable HTTPS backend origin and `FRONTEND_URL` is the browser application origin. Telegram must be able to POST to the backend. The customer return link must reach the frontend.
3. Open **Admin → Platform Credentials → Telegram — bot creation**. Enter the manager bot token and save.
4. Saving validates the manager identity and existing webhook, encrypts its token, sets its webhook, and checks Telegram's reported webhook configuration. A failed save stays unavailable until a successful retry. The webhook callback is:

   ```text
   {BASE_URL}/api/v1/webhooks/channel/telegram-new/manager
   ```

5. Run the staging checks below before enabling this configuration for customers.

The manager webhook subscribes to `message`, `callback_query`, and `managed_bot`, with one concurrent delivery connection. Configuration does not discard pending Telegram updates. A manager pointing at a different webhook is rejected.

Rotating the manager token for the same bot preserves the webhook verification secret. This version rejects explicit webhook-secret regeneration. To change manager identity, delete the saved managed channel connections and finish or cancel active creation requests, then clear the configuration and save the new manager. Clearing also removes the manager's own webhook. It does not delete child bots from Telegram.

## Returning and recovering

An unfinished request is saved on the server for 15 minutes, bound to the signed-in NyxID actor and its original destination. Reopen **Add Channel Bot → Telegram** to continue. If the browser session was lost, sign in to the same account first. The Telegram return link is an ordinary page URL; it contains no sign-in token.

Once connection provisioning starts, the saved pending bot survives request expiry. **Retry connection** reuses its bot ID, encrypted token, and webhook secret. It cannot create a second saved bot or reactivate a deleted/suspended bot. Cancelling before provisioning releases the request. Cancellation or expiry does not delete a bot already created inside Telegram.

For up to 60 minutes after creation, to recover an unconnected bot whose creation NyxID recorded, start a new request, open the manager chat, and send:

```text
/recover @YourBotUsername
```

Recovery requires fresh consent naming that bot and destination. It does not enumerate or automatically attach the user's other bots. A creation event that cannot be matched to a waiting request produces this recovery instruction.

A subsequent Telegram management event conservatively suspends a connected bot. The event's user field identifies its creator and is not treated as proof of current ownership. Messages and replies stop; the bot continues reserving its remote identity until deleted. Neither Verify nor Retry can reactivate it. Suspension takes effect when NyxID observes the management event; a child token can remain valid after a transfer until Telegram resets it. To retain the bot, delete its NyxID connection and use the Telegram bot token option with its current token. Deletion also disables its NyxID conversation routes; recreate agent assignments afterward. Alternatively, create a different bot through the Telegram option. A bot with recorded management changes is ineligible for `/recover`. Once any NyxID connection has started provisioning for the bot, it cannot be claimed by a new creation request, including after deletion; retry uses the original saved connection.

## API and storage

All request routes require a person session or ordinary person access token; API keys, service accounts, relay tokens, and delegated auth are rejected. Destination write access is checked when beginning and connecting. An actor can cancel their own unstarted request even after losing organization access.

| Method | Route, relative to `/api/v1` | Result |
| --- | --- | --- |
| GET | `/channel-bots/telegram-new` | Availability and the actor's current request |
| POST | `/channel-bots/telegram-new` | Save `{label, target_org_id?}` and return a Telegram launch URL |
| GET | `/channel-bots/telegram-new/requests/{id}` | Read an actor-owned request with destination access |
| POST | `/channel-bots/telegram-new/requests/{id}/launch` | Replace the start challenge and return a fresh link |
| DELETE | `/channel-bots/telegram-new/requests/{id}` | Cancel before provisioning |
| POST | `/channel-bots/telegram-new/requests/{id}/connect` | Confirm `{telegram_bot_id, revision}` and finish/retry provisioning |
| POST | `/webhooks/channel/telegram-new/manager` | Authenticate manager updates with Telegram's secret header |

`telegram_bot_id` is a decimal string in JSON requests and responses, and `revision` is an integer. For example, the connect body is `{"telegram_bot_id":"900","revision":6}`.

CLI browser handoff:

```sh
nyxid channel-bot register --platform telegram-new --managed
```

`telegram_bot_requests` stores request status, actor/destination IDs, bot setup identity, hashed challenge/consent values, revision, and expiry. Only terminal requests that never started provisioning receive a TTL. Connected/deleted request provenance remains available for lifecycle handling. `telegram_managed_bot_events` stores the original provider creation time, bot setup provenance, the manager observation identifier, and management revisions. Provenance cannot be overwritten by a late creation event. No ordinary chat bodies or raw updates are stored.

Telegram private-message identity, exact named-bot consent, and browser confirmation are separate gates. Candidate identity is hidden from the browser until consent. Telegram API errors exclude raw responses and credential-bearing URLs. The bot token and webhook secret use the existing envelope encryption.

Registration atomically checks active identity and owner quota before inserting. Both Telegram platform IDs share the same remote identity check. Pending bot insertion and the request's provisioning transition happen in one MongoDB transaction. The transaction also checks the management revision approved in Telegram and permanently marks that creation as claimed. Before acquiring a token, NyxID verifies the creation is at most 60 minutes old, belongs to the current manager observation period, and has no pending manager updates or Telegram-reported delivery error since creation. The observation start is saved only after the manager webhook is verified. Clearing and reconfiguring the manager starts a new observation period, so old provenance does not become eligible again. Pending manager updates cause a retryable refusal. A Telegram delivery error after creation disqualifies that creation for automatic connection, even after delivery recovers; create a new bot or use the current token through the Telegram bot token option. An update received after an error does not prove that every earlier management event was delivered. Per-child renewable leases serialize setup and deletion. A separate identity lease coordinates manager configuration with channel registration without blocking unrelated manual Telegram bots.

## Deployment and staging validation

Use the repository's existing transaction-capable MongoDB topology. Deploy all backend replicas with this version and drain old writers before configuring Telegram bot creation: older replicas do not participate in the new registration transaction fence. Startup installs the request and event indexes; MongoDB TTL alone does not release active requests. Reads and new initiation perform logical expiry cleanup.

Official API documentation was checked during implementation:

- [Managed bots](https://core.telegram.org/api/bots/managed-bots)
- [Managed bot creation links](https://core.telegram.org/api/links#managed-bot-creation-request-links)
- [Bot API](https://core.telegram.org/bots/api), including `KeyboardButtonRequestManagedBot`, `ManagedBotCreated`, `ManagedBotUpdated`, and `getManagedBotToken` (whose parameter is `user_id`, the child bot ID).

Local verification uses real MongoDB transactions and a simulated Telegram API. It does not establish Telegram client compatibility or actual ownership-event semantics. Before customer use, the operator must check:

- Manager capability and successful webhook delivery with a dedicated staging bot.
- Native creation, exact approval, return, browser confirmation, inbound message, and agent reply on supported Telegram iOS, Android, and desktop clients.
- Whether creation emits exactly one initial management event and whether obtaining a token or configuring the child webhook emits another event. Additional events intentionally stop this first implementation.
- Confirm that the creation service message carries the original provider `date`, including on redelivery. Missing/old timestamps fail closed. Check creation-event timing and identity through a dropped browser session, cancelled/expired request, delayed webhook, and `/recover`.
- Token regeneration, management removal, and ownership transfer: observed changes must stop incoming messages and replies, including replies using old NyxID reply tokens.
- Interrupted manager save and interrupted child webhook setup; repeat saves/connection attempts must recover without a different bot or webhook secret.
- Existing webhook ownership checks, deletion, and recovery through the Telegram bot token option.

No real manager token was configured, no live Telegram bot was created, and no production deployment was performed as part of local implementation.

## Local verification record

Final integration validation on 2026-09-14, after rebasing onto `main` at `04c44f5b`:

- 17 focused backend tests passed with MongoDB 8.0 on an initialized local replica set and WireMock Telegram responses.
- 424 channel-related backend tests passed, including existing Telegram, managed WhatsApp, X OAuth/polling, channel handlers, relay replies, and registration behavior.
- 35 channel CLI tests passed, including the Telegram New browser link and refusal to prompt for a manual token. The committed CLI wizard bundle freshness test passed.
- The full frontend suite passed: 3,136 tests across 309 files. The subsequently added regression proving existing Telegram token setup remains separate also passed in the four-test platform suite.
- The frontend production build passed. ESLint completed with zero errors and 27 existing warnings outside the new flow. Workspace Clippy passed with warnings denied. Rust formatting and `git diff --check` passed.

Backend test builds used `CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0` because the full debug build exhausted local disk space. The database URI was supplied explicitly through `NYXID_TEST_DATABASE_URL`. No tests were skipped in the two backend runs above.

```sh
NYXID_TEST_DATABASE_URL='<local replica-set URI>' \
CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0 \
cargo test --manifest-path backend/Cargo.toml --bin nyxid-server telegram_new

NYXID_TEST_DATABASE_URL='<local replica-set URI>' \
CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0 \
cargo test --manifest-path backend/Cargo.toml --bin nyxid-server channel

CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0 \
cargo test -p nyxid-cli commands::channel_bot
cargo test -p nyxid-cli --test wizard_bundle_freshness
CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0 \
cargo clippy --workspace --all-targets -- -D warnings

npm test --prefix frontend
npm test --prefix frontend -- channel-platforms
npm run build --prefix frontend
npm run lint --prefix frontend
```
