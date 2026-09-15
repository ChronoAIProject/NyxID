# Telegram bot creation

`telegram-new` adds Telegram's native bot creation to **Channel Bots → Add Channel Bot**. Customers can start in NyxID or in the Telegram manager chat. A website request saves the destination before creation and connects automatically. A Telegram-first creation receives a private claim code so the customer can choose the destination in NyxID. NyxID retrieves and encrypts the child bot token on the server. The display labels are **Telegram** for `telegram-new` (create a bot) and **Telegram bot token** for `telegram` (connect an existing bot). The platform identifiers, API routes, CLI arguments, and existing saved connections are unchanged.

This is a channel configuration capability. The wider onboarding wizard, onboarding drafts, Mini Apps, and Meta login flows are outside this change.

## What the customer sees

Starting from the website takes two application steps:

1. **Continue in Telegram.** Choose a personal or organization destination and enter a label on `/channel-bots?connect=telegram-new`. One click saves the authenticated creation request with `auto_connect: true` and opens the manager chat. This action authorizes NyxID to connect one fresh bot created through that private handoff.
2. **Create your bot.** The manager chat names the destination and website and explains that creating the bot connects it automatically. Tap **Start** if Telegram asks, then **Create bot** and finish Telegram’s native name/username form. Suggestions come from the saved label and remain editable. The verified creation webhook records the bot; a server worker retrieves and encrypts the token and installs the message webhook. No separate **Approve this bot** or **Connect bot** action is required for new website requests.

The page updates automatically and opens the connected bot’s settings. The manager also sends a best-effort completion message with a settings link. Connection continues if the user closes the browser. Choosing an AI agent for replies remains part of the existing bot settings; connecting the channel alone does not configure an agent.

These are two application steps, not a guarantee of two physical taps. Telegram controls its Start prompt and creation form. The direct `t.me/newbot/...` form link cannot carry an arbitrary setup identifier, and `ManagedBotCreated` does not echo the keyboard’s `request_id`. The private `/start` handoff is retained to correlate the Telegram sender with the saved NyxID request. It does not establish a verified Telegram login identity for the NyxID account.

Starting from the Telegram manager chat also has two application steps:

1. **Create your bot.** Send `/start` without a payload, tap **Create bot**, and finish Telegram’s native form. When no active website request is bound to that creator, the manager sends the bot username, a private claim code, and a **Connect in NyxID** button. The code is valid for up to 15 minutes, within the original creation’s 60-minute recovery window. It is sent immediately, even if Telegram’s initial management confirmation is still arriving.
2. **Connect in NyxID.** Open the link and sign in if needed. The page previews the bot, fills its label, and lets you choose Personal or an organization you manage. Click **Connect** once. NyxID saves the destination and connection job, then completes setup on the server. If Telegram is still confirming creation, the page offers **Retry** without consuming the code. Nothing connects on page load.

The private Telegram message also includes a URL without a code and the code itself. If Telegram’s in-app browser does not share your NyxID session, open that URL in your usual browser and enter the code. Share the code only with NyxID: anyone holding it can claim the bot into an account they control, just as someone holding a BotFather token can register that bot. The child token itself is never sent to the customer.

The normal customer path does not use BotFather or require copying a token. The platform administrator still needs BotFather for the manager setup below. Requests saved with the previous flow retain its Telegram consent and browser completion; cancel and restart to use automatic connection.

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

Rotating the manager token for the same bot preserves the webhook verification secret. This version rejects explicit webhook-secret regeneration. To change manager identity, delete the saved managed channel connections and finish or cancel active creation requests, then clear the configuration and save the new manager. Clearing permanently deletes NyxID's saved manager token, identity, webhook secret, and observation period. It also attempts to remove the manager's own webhook, preserving any webhook belonging to another application. A Telegram API failure is logged and does not prevent local deletion, including when the manager was already deleted in BotFather or its token was revoked. It does not delete child bots from Telegram.

Deleting and recreating a manager in BotFather gives it a new numeric bot ID and token, even when its username is reused. Updating its display name or icon does not update NyxID's saved credentials. To replace it:

1. Cancel unfinished Telegram creation requests and delete saved managed channel connections tied to the old manager. Deleting a channel connection also disables its conversation routes.
2. Open **Admin → Platform Credentials → Telegram — bot creation** and choose **Clear provider**.
3. Enable bot management for the replacement in BotFather, then save its new manager token in NyxID.
4. Start a fresh creation request and use its new **Open Telegram** link. Previous setup requests and links do not transfer to the replacement manager.

This clears NyxID's saved configuration. If the public Telegram profile shows the replacement but a Telegram client still opens "Deleted Account", check the username in another Telegram client; clearing NyxID cannot reset Telegram's own username resolution or chat history.

## Returning and recovering

An unfinished request is saved on the server for 15 minutes, bound to the signed-in NyxID actor and its original destination. **Channel Bots** displays **Resume Telegram setup** while a request is active. If the browser session was lost, sign in to the same account first.

The selected flow, draft label, and draft destination are retained in the page URL. Once a request exists, its server-saved label and destination take precedence and the fields are locked until cancellation. The nonsecret `request_id` is also retained so that reloading the setup page can find an already completed request. Reading it still requires the initiating actor and current destination access. Launch challenges and bot tokens are never stored in browser storage or the NyxID page URL. A claim arrives in the link’s `claim` query parameter, which is removed with `history.replaceState` before router construction, authentication requests, or telemetry boot. The nonsecret `claim_entry=true` flag identifies the manual-entry screen. A signed-in visitor’s code stays in page memory. For sign-in redirects only, the code and a bounded expiry temporarily use `sessionStorage`; the authenticated setup page reads them once and removes them. The form waits for the actual user identity before consuming this handoff. Codes never enter `localStorage`, cookies, `return_to`, query keys, or mutation variables. Redemption, expiry, leaving the setup page, and an account switch discard the code; reloading after the page consumed it requires the original link or manual entry.

The page polls active requests every two seconds while visible, refreshes on focus, and offers **Check progress**. **Reopen Telegram** issues a fresh launch link for the same saved request. A blocked popup has an **Open Telegram** fallback link. Neither returning nor polling creates a new request.

After a fresh creation event, a database-backed worker completes the connection independently of the browser. Every backend replica scans due requests; atomic scheduling claims and the existing per-bot renewable lease coordinate attempts. Connection work is deferred from the webhook response so Telegram can acknowledge the creation delivery before the worker checks for pending updates. Temporary failures retry with backoff. Setup retains its 15-minute expiry until a pending channel bot is saved; provisioning then survives expiry and reuses the saved bot ID, encrypted token, and webhook secret. It cannot reactivate a deleted/suspended bot. Cancellation before provisioning releases the request, but does not delete a bot already created inside Telegram.

New automatic requests accept only a fresh `managed_bot_created` event from the private sender bound by their start challenge. They reject recovery commands and creation updates preceding that handoff. An expired or cancelled request must be restarted to create a new bot; an existing bot can be connected through **Telegram bot token**. With no bound website request, `/recover @YourBotUsername` issues a fresh claim for the creator’s unconsumed creation and invalidates the previous code. Duplicate creation deliveries do not rotate a successfully delivered code. An unstarted redeemed request must be cancelled before a fresh claim can be issued. Once a pending channel bot has been saved, the creation is permanently retired: deleting that bot does not reopen it. Retry the saved connection or use the manual token path. `/recover` reports when there is already a saved connection.

Legacy manual requests retain `/recover @YourBotUsername` for recorded unconnected creations up to 60 minutes old, followed by fresh named-bot consent and browser confirmation.

A subsequent Telegram management event conservatively suspends a connected bot. The event's user field identifies its creator and is not treated as proof of current ownership. Messages and replies stop; the bot continues reserving its remote identity until deleted. Neither Verify nor Retry can reactivate it. Suspension takes effect when NyxID observes the management event; a child token can remain valid after a transfer until Telegram resets it. To retain the bot, delete its NyxID connection and use the Telegram bot token option with its current token. Deletion also disables its NyxID conversation routes; recreate agent assignments afterward. Alternatively, create a different bot through the Telegram option. A bot with recorded management changes is ineligible for `/recover`. Once any NyxID connection has started provisioning for the bot, it cannot be claimed by a new creation request, including after deletion; retry uses the original saved connection.

## API and storage

All request routes require a person session or ordinary person access token; API keys, service accounts, relay tokens, and delegated auth are rejected. Destination write access is checked when beginning and connecting. An actor can cancel their own unstarted request even after losing organization access.

| Method | Route, relative to `/api/v1` | Result |
| --- | --- | --- |
| GET | `/channel-bots/telegram-new` | Availability and the actor’s current request; optional `?request_id={uuid}` reads a specific request, including a completed one |
| POST | `/channel-bots/telegram-new` | Save `{label, target_org_id?, auto_connect?}` and return a Telegram launch URL; the website sends `auto_connect: true` |
| GET | `/channel-bots/telegram-new/requests/{id}` | Read an actor-owned request with destination access |
| POST | `/channel-bots/telegram-new/requests/{id}/launch` | Replace the start challenge and return a fresh link |
| DELETE | `/channel-bots/telegram-new/requests/{id}` | Cancel before provisioning |
| POST | `/channel-bots/telegram-new/claims/preview` | Preview `{code}` without mutation or Telegram API calls; return `{bot_username, expires_at}` |
| POST | `/channel-bots/telegram-new/claims/redeem` | Redeem `{code, label, target_org_id?}`; return HTTP 202 with the saved safe request response |
| POST | `/channel-bots/telegram-new/requests/{id}/connect` | Legacy browser completion: confirm `{telegram_bot_id, revision}` and finish/retry provisioning |
| POST | `/webhooks/channel/telegram-new/manager` | Authenticate manager updates with Telegram's secret header |

`auto_connect` defaults to false for existing API clients and older stored rows. Request responses include `auto_connect` and nullable `connection_error` (safe retry text). Website status progression is `waiting_telegram → waiting_bot → ready → provisioning → connected`; a redeemed claim enters at `ready`. Legacy requests include `waiting_consent` before `ready`. Preview has a separate per-actor limit of 30 requests per minute; redemption shares the existing five-per-minute creation limit.

`telegram_bot_id` is a decimal string in JSON requests and responses, and `revision` is an integer. For example, the connect body is `{"telegram_bot_id":"900","revision":6}`.

CLI browser handoff:

```sh
nyxid channel-bot register --platform telegram-new --managed
```

`telegram_bot_requests` stores request status, actor/destination IDs, bot setup identity, hashed challenge/consent values, revision, and expiry. Automatic requests additionally store the bound start update ID, attempt count, next retry time, and safe retry message. Only terminal requests that never started provisioning receive a TTL. Connected/deleted request provenance remains available for lifecycle handling. `telegram_managed_bot_events` stores the original provider creation time, bot setup provenance, the manager observation identifier, and management revisions. Provenance cannot be overwritten by a late creation event. No ordinary chat bodies or raw updates are stored.

`telegram_bot_claims` stores only the SHA-256 hash of a random 100-bit code (20 characters, grouped for manual entry), the manager/bot/creator IDs, observation identifier, expected initial management revision, creation and expiry times, delivery state, and redemption state. Secret-bearing models redact Debug output; handler responses project dedicated safe structs. Claims and requests use UUID string IDs and BSON dates. Pending claims have a cleanup TTL; redeemed claims retain their pin and request reference. No code, token, or chat body is logged or audited. Successful redemption emits metadata-only `telegram_bot_claim_redeemed`; successful connection uses the existing `channel_bot_created` audit.

The Telegram-first trust model is equivalent to pasting a BotFather token: the claim is bearer proof of the recorded creation, and the human NyxID session selects the account. No persistent Telegram-to-NyxID login identity is stored or consulted. Preview and initial redemption require matching creator, bot, manager observation, unretired provenance, and exactly the initial management revision. A missing initial confirmation returns a retryable conflict with no provider calls and no consumption. The first successful redemption transaction permanently pins actor, owner, and label and inserts the ready request before any token acquisition. A different actor gets a not-found-shaped response; a same-actor retry with the same destination and label recovers the saved in-flight request until claim expiry. A completed redemption conflicts. Expired or unknown codes have the same not-found response. Another active setup belonging to the actor conflicts without consuming the claim, and the page offers explicit resume/cancel actions. An active setup belonging to the Telegram creator in another account produces a separate explanation.

A redeemed request gets its own 15-minute retry window, bounded by creation plus 60 minutes, so a code redeemed near expiry can survive a transient provider failure. After pending bot insertion, the existing worker resumes saved credentials even after request/claim expiry. The worker still performs manager delivery checks before token acquisition and the registration transaction still enforces live identity, quota, provenance, and owner access. A claim is never reopened after provider provisioning begins.

For website-first automatic requests, the initiating authenticated action authorizes creation and connection together. The private start URL is a bearer handoff: its first private Telegram sender binds the request, and later senders cannot replace that identity. Completing creation after the named destination is shown accepts that connection. This is an intentional authorization change from the legacy flow and does not prove that the NyxID actor and Telegram sender share a preverified login identity. A creation must follow the bound start update, occur no earlier than request creation, and pass the same immutable creation-provenance and management checks as before. The worker rechecks the initiating person’s live destination access. Legacy rows retain separate private-sender, exact named-bot consent, and browser-confirmation gates; candidate identity remains hidden until ready. Telegram API errors exclude raw responses and credential-bearing URLs. The bot token and webhook secret use the existing envelope encryption.

Registration atomically checks active identity and owner quota before inserting. Both Telegram platform IDs share the same remote identity check. Pending bot insertion and the request's provisioning transition happen in one MongoDB transaction. The transaction also checks the expected initial management revision (or the revision approved in Telegram for a legacy request) and permanently marks that creation as claimed. Before acquiring a token, NyxID verifies the creation is at most 60 minutes old, belongs to the current manager observation period, and has no pending manager updates or Telegram-reported delivery error since creation. The observation start is saved only after the manager webhook is verified. Clearing and reconfiguring the manager starts a new observation period, so old provenance does not become eligible again. Pending manager updates cause a retryable refusal. A Telegram delivery error after creation disqualifies that creation for automatic connection, even after delivery recovers; create a new bot or use the current token through the Telegram bot token option. An update received after an error does not prove that every earlier management event was delivered. Per-child renewable leases serialize setup and deletion. A separate identity lease coordinates manager configuration with channel registration without blocking unrelated manual Telegram bots.

## Deployment and staging validation

Use the repository's existing transaction-capable MongoDB topology. Deploy all backend replicas with this version and drain old writers before configuring Telegram bot creation: older replicas do not participate in the new registration transaction fence. Startup installs the request, event, and claim indexes; MongoDB TTL alone does not release active requests. Reads, new initiation, and the automatic worker perform logical expiry cleanup. Upgrade backend replicas before releasing the new frontend: older backends reject the new request flag and cannot run the automatic worker. No migration upgrades old requests into automatic authorization. There are no new environment variables.

Deploy the updated frontend server configuration with the app: claim-link document responses need `Referrer-Policy: no-referrer` before any asset request, and request/referrer access logs must redact claim query values. The included Nginx template applies these rules even after SPA fallback; ordinary pages retain their existing referrer policy. Apply the same redaction at any upstream ingress/CDN or alternative hosting layer, because the initial document request necessarily carries the query parameter. The Vite development server applies the claim document header too.

Official API documentation was checked during implementation:

- [Managed bots](https://core.telegram.org/api/bots/managed-bots)
- [Managed bot creation links](https://core.telegram.org/api/links#managed-bot-creation-request-links)
- [Bot API](https://core.telegram.org/bots/api), including `KeyboardButtonRequestManagedBot`, `ManagedBotCreated`, `ManagedBotUpdated`, and `getManagedBotToken` (whose parameter is `user_id`, the child bot ID).

Local verification uses real MongoDB transactions and a simulated Telegram API. It does not establish Telegram client compatibility or actual ownership-event semantics. Before customer use, the operator must check:

- Manager capability and successful webhook delivery with a dedicated staging bot.
- Website-first native creation and automatic connection; Telegram-first `/start`, claim delivery, sign-in return, and one explicit Connect; inbound messages and agent replies on supported Telegram iOS, Android, and desktop clients.
- Whether creation emits exactly one initial management event and whether obtaining a token or configuring the child webhook emits another event. Additional events intentionally stop this first implementation.
- Confirm that the creation service message carries the original provider `date`, including on redelivery. Missing/old timestamps fail closed. Check creation-event timing and identity through a dropped browser session, cancelled/expired request, delayed webhook, duplicate event, and old-form completion after a new handoff. Verify legacy `/recover` separately.
- Token regeneration, management removal, and ownership transfer: observed changes must stop incoming messages and replies, including replies using old NyxID reply tokens.
- Interrupted manager save and interrupted child webhook setup; repeat saves/connection attempts must recover without a different bot or webhook secret.
- Existing webhook ownership checks, deletion, and recovery through the Telegram bot token option.

No real manager token was configured, no live Telegram bot was created, and no production deployment was performed as part of local implementation.

## Local verification record

Telegram-first claim validation on 2026-09-15:

- All 40 Telegram-focused backend tests passed against an isolated MongoDB replica set and WireMock; none were skipped. Claim coverage includes immediate issuance before management confirmation, no provider calls on preview or failed provenance, code normalization and hashed storage, another human account selecting its destination, single-use pinning, concurrent redemption, both active-setup conflict cases, org write access and revocation, expiry, management/observation changes, recovery/rotation, saved-credential retries, and no claim for a bound website request. The near-expiry test proves a ready request can retry a pre-insertion provider failure after the claim expires.
- All 115 channel/Telegram frontend tests passed, including sign-in handoff consumption only after the user identity exists, StrictMode preview, manual entry, explicit redemption, unchanged draft on an uncertain redemption response, conflict cancellation, expiry, and account switching.
- The 26 existing auth, router, and public-path tests also passed after the login redirect change.
- Five Playwright checks passed: website-first and Telegram-first flows at 390px and 1440px, plus an actual sign-in redirect with API fixtures. Claim checks assert URL/history/storage removal, no secret referrers, preview before one Connect, and no auto-redemption. Both claim layouts were visually inspected.
- The production frontend build, targeted ESLint, Rust formatting, and diff checks passed. A real isolated Nginx process validated the production template’s syntax, claim-only document referrer policy through SPA fallback, ordinary-page policy, and request/referrer access-log redaction.

Telegram APIs and authentication were simulated for these local checks. No live bot was created and no deployment was performed.

Automatic connection validation on 2026-09-15:

- All 30 Telegram-focused backend tests passed against an isolated MongoDB replica set and WireMock; none were skipped. The run covers webhook-driven connection without consent/browser completion, concurrent workers, duplicate events, delayed management delivery, retry with the same saved credentials after expiry, revoked destination access, earlier/unbound creation events, legacy authorization, and suspended provisioning.
- All 104 channel-related frontend tests passed, including automatic status completion, popup fallback, saved-request reload, and discarding an outstanding handoff when the account changes.
- Browser checks passed at 390px and 1440px. They cover the single website action, two displayed steps, destination locking, launch renewal, closing/reopening the page, and server completion while the page is closed. The test rejects any browser `/connect` call in the new flow. Both layouts were visually inspected.
- TypeScript, the frontend production build, targeted ESLint, Rust formatting, and `git diff --check` passed.

These checks used simulated Telegram responses. No live Telegram bot was created and no deployment was performed. Earlier records below refer to the previous approval flow and do not certify the new authorization contract.

Copy and connected-step validation on 2026-09-14: all 23 Telegram-focused backend tests passed against an isolated MongoDB replica set and WireMock, including readable personal/org identity, HTML escaping, consent, and the return action. All 100 channel-related frontend tests passed. Telegram browser checks passed at 390px and 1440px in light and dark mode, including reload and reopening; the connected steps were visually reviewed. TypeScript, targeted ESLint, Rust formatting, and the frontend production build passed. These checks did not send messages to a live Telegram chat.

Setup-page persistence validation on 2026-09-14: 100 channel-related frontend tests passed, along with 11 browser checks covering Telegram, X, and WhatsApp. Telegram was checked at 390px and 1440px: draft reload, saved destination locking, fresh launch for the same request, closing and reopening the browser page, resuming from Channel Bots, and returning after consent. TypeScript, targeted ESLint, and the frontend production build passed. Browser checks use simulated API responses and do not establish live Telegram client behavior.

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
