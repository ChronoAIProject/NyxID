# Messenger and Instagram customer inboxes

NyxID supports customer-initiated, one-to-one **text DMs** on `messenger`
(Facebook Pages) and `instagram` (Instagram Login for professional accounts).
Both use the existing NyxID relay and signed agent callbacks. Each channel has
its own default-off enrollment flag, application configuration and release gate.
WhatsApp Embedded Signup and general Facebook sign-in retain their separate
configuration.

## Application administrator setup

The implementation pins **Graph v25.0**. Messenger uses `graph.facebook.com`;
Instagram uses `graph.instagram.com` and `api.instagram.com` for its initial
code exchange. Do not configure a customer-controlled Graph host.

Configure `messenger` and `instagram` independently in Admin → Platform
Credentials. The descriptor returned by the API is authoritative:

| Field | Value |
|---|---|
| `app_id` | Dedicated Facebook application ID or Instagram Login application ID |
| `client_secret` | Secret of the corresponding OAuth application |
| `webhook_signing_secret` | Secret proven to verify that platform's real raw-body `X-Hub-Signature-256` event signature |
| `verify_token` | Secret configured in Meta for the platform callback's GET verification handshake |

For Instagram, the login and webhook signing secrets may come from different
application settings. Record their sources and verify an actual test signature;
do not assume the secrets are interchangeable. NyxID encrypts stored secrets and
asset tokens and does not return them in setup progress or channel status.

Record the dedicated app's **Require App Secret** setting before testing. The
current adapters support apps where Graph accepts the access token without
an additional per-request `appsecret_proof`. If the app requires that proof,
leave its enrollment closed. Verify this setting against a native test inbox.

The equivalent administrator CLI is:

```sh
nyxid admin platform-credentials show messenger
nyxid admin platform-credentials set messenger \
  --field app_id=YOUR_APP_ID \
  --field-env client_secret=META_MESSENGER_CLIENT_SECRET \
  --field-env webhook_signing_secret=META_MESSENGER_SIGNING_SECRET \
  --field-env verify_token=META_MESSENGER_VERIFY_TOKEN
nyxid admin platform-credentials show instagram
```

Set the environment variables in your own secret-management environment before
running the command. Use the same field names with provider `instagram` for its
independent configuration.

Configure the following public HTTPS URLs using the server's configured
`BASE_URL` and `FRONTEND_URL`. Replace `{platform}` with `messenger` or
`instagram`:

| Meta setting | URL |
|---|---|
| Webhook callback | `{BASE_URL}/api/v1/webhooks/channel/{platform}/platform` |
| OAuth redirect | `{FRONTEND_URL}/channel-bots/meta-callback/{platform}` |
| Deauthorization callback | `{BASE_URL}/api/v1/webhooks/channel/{platform}/deauthorize` |
| Data-deletion callback | `{BASE_URL}/api/v1/webhooks/channel/{platform}/data-deletion` |

The deletion callback returns an opaque confirmation code and a public status
URL. It accepts Meta's signed form request, not ordinary browser credentials.
Pending cleanup/erasure obligations do not expire through TTL; only completed
requests have retention expiry. A status of `completed` means local erasure
finished after confirmed subscription removal or verified grant revocation.

### Messenger Page authority and routing

Request `pages_show_list`, `pages_manage_metadata` and `pages_messaging`.
The consenting person must have the selected Page's messaging task. NyxID
validates the actual granted permissions, enumerates eligible Pages and requires
explicit selection; a personal Facebook account token is not a Page inbox.
Page task evidence comes from `/me/accounts` under the consenting person's
token. Managed verification checks the selected Page's identity, uses
`/debug_token` to verify the issuing app, Page token and current messaging and
subscription scopes, and reads back that Page's subscription. The native
acceptance run records these provider responses and tests loss of access.
The selected Page token is retained independently from sibling Pages.

Subscribe the Page to `messages` and `messaging_handovers`. In the Page's
conversation routing configuration, configure the NyxID application as the
**default application and the application for organic/social conversations**.
This prerequisite enables the documented ownership read:

```http
GET /v25.0/{page-id}/thread_owner?recipient={PSID}
Authorization: Bearer {selected-page-token}
```

NyxID requires the returned `data[0].thread_owner.app_id` to identify its own
application before admitting a participant's turn and immediately before each
send. A missing owner, another app, or an unavailable read fails closed.
Business Suite and other inbox apps can take control by replying or moving a
conversation. Standby and control-loss events stop only their exact asset and
participant. An operator checkbox cannot reclaim another application's control.
Restore routing in Meta, then send a new qualifying DM so NyxID can reverify
ownership. This implementation does not send a handover/take-control request.

### Instagram professional identity

Use **Instagram Login**, including professional accounts without a linked
Facebook Page. Request `instagram_business_basic` and
`instagram_business_manage_messages`. NyxID validates the actual comma-separated
permissions returned by the documented short-token response:

```json
{"data":[{"access_token":"REDACTED","user_id":"APP_SCOPED_SUBJECT","permissions":"instagram_business_basic,instagram_business_manage_messages"}]}
```

The short-token response and `/v25.0/me` accept either one direct account
record or exactly one record in a `data` array. Direct tokens must include the
subject and permissions; direct identity responses must include `user_id` and
`account_type`. If `data` is present, it must have exactly one object and any
repeated direct account fields must agree with that object. Empty, multiple or
conflicting records are refused. `/me.user_id` is the professional inbox identity used by webhook
`entry.id`; it is independent of the short-token app-scoped `user_id` used for
deauthorization/deletion. Numeric JSON IDs are parsed without floating-point
conversion. `Business` and `Media_Creator` account types are accepted with ASCII
case normalization; personal accounts are refused.

Subscribe the selected professional account to `messages`. Launch requires
verified exclusive routing. Instagram has no ownership read implemented here;
a provider-observed control loss remains closed and cannot be cleared by merely
checking exclusive routing again. Resolve the provider routing condition, then
use **Reconnect** with fresh consent (delete and add again if reconnect is no
longer available). Story replies, mentions, comments, media and group
conversations are outside the supported ingress contract. Ordinary text quoting
a prior message ID is supported.

## Connect an inbox and open customer conversations

After all serving replicas understand both adapters, enroll the acting person
in `channels:messenger` or `channels:instagram`. Enrollment is checked in catalog
discovery, direct registration, OAuth start/exchange/completion, tracked setup
links and NyxBot onboarding. Organization ownership still requires a currently
authorized organization administrator; enrollment is evaluated for the person.

1. Open Add Bot, choose the enrolled channel and the personal or organization
   scope, and enter a label. Continue to Meta consent in the popup.
2. Grant the permissions. Messenger displays an explicit eligible Page picker;
   Instagram displays its verified professional inbox. Confirm the routing
   prerequisite and finish. NyxID verifies the asset and reads back its remote
   subscription before reporting an active inbox.
3. In **Set up customer conversations**, select a support specialist in the
   same scope and explicitly assign it. A personal owner receives a single-use
   owner code in the first-party UI. Send that code from the owner's own private
   account to the inbox. Keep the code private; an ordinary first DM never proves
   ownership. A fresh code replaces the previous code without changing routing,
   customer exposure or existing verified senders.
4. For a personal inbox, review the specialist's guest grants, select **Prepare
   guest model** when readiness requires it, and check again. Preparation is
   reachable while customer access is closed. The personal account funds guest
   model/tool usage under the existing guest rules.
5. For an organization inbox, use the member verification UI: issue a challenge,
   send it privately, wait for NyxID to observe it, then explicitly confirm it in
   the first-party UI. Enable `nyxbot:org-specialist-channels` and the existing
   org guest-grant rollout only after their readiness checks pass. Review the
   same-org specialist's Grants and provision/fund the organization wallet.
   Guest model preparation follows verified org guest funding at the first
   customer turn. Organization members retain their normal execution identity.
6. Explicitly check **Everyone for private DMs** and enable customer
   conversations. For organizations, guest exposure and private-DM access commit
   together. A failed enable leaves the committed access state intact. Readiness
   checks and repeated same-agent assignments preserve routing, owner codes and
   public access.
7. Send two successive private text DMs from an unrelated test account. Confirm
   each callback once and the corresponding ordered reply in the native inbox.
   Repeat with another sender and another inbox to verify isolation.

The existing channel access settings can close customer access again. Guest
threads cannot approve owner cards, use owner-only tools or receive the owner's
access-token header. **All** Messenger/Instagram relay callbacks suppress that
header, including personal and custom-agent routes. Their signed callback and
single-use, message-bound reply token remain available.

### Popup and setup recovery

The setup window persists its session ID under the signed-in actor and owner
scope and polls for consent. If the browser blocks the popup, allow popups
for NyxID and select **Sign in and grant inbox access** again. If the popup
closes before consent finishes, restart sign-in from the setup window.
A consent cancellation, revoked consent, expired session or
failed exchange is shown explicitly and can start a new consent flow.
React StrictMode observes one exchange promise rather than posting the code
twice. A completed installation is recoverable after interruption: finishing
again for the same selected asset retries subscription verification and completes
only the still-pinned, live setup link. A cancelled, expired or replaced tracked
link cannot complete its former session.

Progress responses expose `selected_asset_id` once installation has claimed an
asset and `subscription_state` from the committed bot. If setup is still
`installing`, or the completed session reports `pending`, choose **Retry
verification** for the selected asset. A repaired inbox marked `suspended`
requires explicit delivery resume and subscription verification. An inbox
marked `reconnect_required` requires a new consent flow.

Inbox detail exposes **Reconnect**. Reconnect requires fresh consent for the
same inbox and commits a new grant generation; queued old replies cannot adopt
it. Changing the application identity also retires prior NyxBot routing and
owner verification because participant IDs may change. Ownership transfer
requires the destination owner to reauthorize before dispatch resumes.

Application credential replacement and deletion are fenced by every retained
Meta obligation, including inactive managed grants, unexpired OAuth sessions,
pending signed deletion/deauthorization work and grant authority fences. Drain
those obligations before changing the configured app or secret; this keeps an
old app's signed callback verifiable until its cleanup is complete. A manual
registration can be purged by its owner through the normal **Delete bot** action
after the subscription is confirmed removed. That explicit purge removes the
manual bot's provider identifiers and local delivery records. A managed OAuth
installation remains retained until its provider cleanup request is confirmed.

The CLI can create a browser setup link and inspect its result:

```sh
nyxid channel-bot platforms --output json
nyxid channel-bot connect-link instagram --label 'Customer support'
nyxid channel-bot connect-link messenger --label 'Organization support' --org ORG
nyxid channel-bot connect-link-status LINK_ID
nyxid channel-bot show BOT_ID
nyxid channel-bot verify BOT_ID
nyxid channel-bot delete BOT_ID
```

For an isolated operator pilot, `channel-bot register` uses the adapter's
catalog-declared token, app ID/secret and `verification_token=exclusive` routing
confirmation. Its one-time response supplies the per-bot webhook URL and Verify
Token. Keep the app secret distinct from that Verify Token. Managed customer
setup uses the shared callbacks and does not expose customer asset tokens.

## Reply and delivery contract

A reply must bind to a stored inbound DM. NyxID derives the destination from that
record's bot, verified asset and participant. Arbitrary initiated sends, generic
test sends, edits, native quoted replies and media are unsupported. A reply may
be multipart: Messenger allows at most 2,000 UTF-16 units per part, Instagram
1,000 UTF-8 bytes. Splitting is lossless, including emoji and line breaks.

Every part rechecks live owner, asset, configured app identity, connection
generation, credential provenance, subscription, delivery gates and participant
control. The signed provider message must be younger than 24 hours and the
ordinary NyxID reply token must still be valid (default 30 minutes). Future or
missing provider times fail closed. NyxID intentionally applies no clock-skew
allowance to signed event timestamps; a future event indicates a clock or
fixture problem and is discarded. Extended ad windows and human-agent tags are
not supported. A newer qualifying DM can authorize a newer reply.

The existing reply journal records every accepted part and its provider ID.
Definitive refusals, throttles and ambiguous outcomes are distinct. A response
read failure, timeout, malformed success or uncertain 5xx is reported as unknown
acceptance. NyxID never automatically resends such a request. Inspect the native
inbox before taking any further action. A later refused part does not erase the
accepted prefix.

The webhook endpoint acknowledges promptly and dispatches in memory. Its 2xx
confirms receipt, not durable callback delivery. Admission and duplicate
suppression are transactional, but process loss before admission or before the
agent callback can lose a DM. Callback failure is recorded and is not queued for
automatic replay. Health separates failed callbacks from the last successful
callback, Meta-window closure, control loss, suspended
delivery, changed authority and unknown provider acceptance. An expired reply
JWT returns an authentication error to its caller before bot-level attribution. Relay history and
review fixtures contain metadata, not customer text or raw webhook batches.

## Lifecycle and incident controls

Instagram long-lived tokens are refreshed when within ten days of expiry, with
serialized refresh and a final generation/credential CAS. Expired or revoked
grants become visible reconnect states. Successful refreshes are identity
verified and encrypted before adoption. Active Messenger subscriptions are read
periodically; they are written only when missing or pending. A bounded separate
worker claims due work atomically across replicas, processes at most 16 bots per
pass with four concurrent jobs, and defers failures so other due rows progress.
It does not block the existing channel poll or billing cleanup.

Disconnect disables local dispatch and sends first, then removes only that
asset's subscription. Asset-level serialization protects successor installations
across bot IDs/owners. Decrypt or uncertain remote cleanup failures retain the
encrypted capability and pending cleanup state for retry. Sibling Pages sharing
a login keep their subscriptions. Verified deauth retires tokens while retaining
a reconnectable management identity. Verified deletion then works even after
token retirement; it erases the captured grant's retired bot, subject, asset,
username, OAuth credentials, relay metadata, control rows, journal/receipt IDs,
setup links and associated guest conversations after cleanup. Existing required
financial/audit records are retained. Saved-result text plans are included:
new saved-result plans on every platform carry the bot ID, and Meta plans are
created under the inbox erasure fence;
legacy plans are attributed in bounded batches by exact owner/source-message
pairs before their inbound records are deleted. Retained exact joins also erase historical
noncanonical logical keys; only expired-source recovery assumes the canonical
saved-result key. The receipt lookup has an owner/inbound-message index; new
parented plans require neither source nor channel-history scans. A non-sparse
owner/parent/ID index supports 100-plan pages, and attribution reads project
only the owner, source, key, ID and timestamp. Retained exact joins deliberately
have no timestamp cutoff: a required timestamp does not establish chronology
for historical or recovered metadata. This also erases precisely attributable
legacy rows whose stored timestamps precede the current bot record. Canonical
older plans also try the indexed exact saved handoff before a date comparison
can exclude genuinely unproven pre-bot history. Erasing one inbox preserves sibling inbox plans.
Personal account erasure removes the departing owner's rows through the existing
owner inventory and retires that owner's bot authority. Former owners' retained
routes and message history survive. Exact legacy journal joins are backfilled
on the original receipt's optional bot parent before TTL sources disappear;
account cleanup never guesses an inbox for an unproven journal. Independently
signed provider deletion is the authority for a wider captured-grant purge.

Account retirement and signed request capture serialize through the keyed grant
fence before the bot disappears. `channel_meta_erasure_fences` then retains only
the bot/grant UUIDs, keyed grant identity, platform and application ID, original creation/authorization
times and an opaque reference to the existing personal-erasure job. That job has
no terminal TTL and retains the opaque account UUID; existing ownership-transfer
records retain prior opaque owners. These existing records resolve the exact
journal owner after account removal. No raw provider subject, asset, credential,
participant ID, former-owner content or copied receipt/source list is retained
in this context. The context remains until independently verified provider
cleanup completes; the purge then removes it and writes provider completion.
Personal cleanup alone never writes that provider completion proof. Pending
requests and a first late signed deletion use the same captured grant and cutoff,
including crash/retry after bot removal. Former-owner and sibling records remain
outside ordinary personal deletion, and successor grants remain outside the
captured provider obligation.

Reconnect preserves the public bot ID and records each installation in
`channel_meta_grant_scopes`. The reconnect transaction seals the prior generation
range, opens the successor range and fences both keyed login subjects. These
constant-size scope rows retain bot/grant UUIDs, keyed subject and binding
fingerprints, platform and App ID, generation bounds, timestamps and cleanup
progress. They contain no raw subject, credential, participant address, copied
owner list or message/receipt content. Original inbound metadata, multipart
plans and reply receipts retain their admitted grant reference. A prior scope
remains discoverable independently of the current binding and account lifetime;
its outstanding app obligation is released only by exact provider completion.
After cleanup the scope drops its subject/binding fingerprints, App ID,
generation bounds and attribution timestamps. Only opaque bot/grant identity,
typed completion proof and replay state remain, without a terminal TTL.
Subjectless manual account retirement removes its unused scope. This additional bookkeeping is excluded from personal export and is registered as
erasure bookkeeping in the personal-data inventory.

The original relay event and turn context retain the admitted grant before
inbound metadata expires. Visitor conversations rotate internally when that
grant changes; the public chat, explicit specialist and chat policy remain on
the existing chat row. Owner homes retain turn/message provenance. Sources from
different grants on the same inbox are not coalesced into one owner turn.
Manual purge claims the exact scope before reading its inventory. Bound outbound
persistence also takes the message owner's personal-erasure barrier, including
settlement against a retired scope; another owner's retirement does not widen
that erasure authority.

Signed callback confirmation records the keyed subject and cutoff with a
durable scope cursor. Historical reconnects are captured in indexed pages of
100 scopes by the existing request worker. Initial compatibility capture holds
at most 200 targets (100 current and 100 distinct retired); later discovery
holds at most one page of 100 targets. Current-installation and retained-draft compatibility
capture is capped; an overflow uses indexed, restartable pages before the
request can complete. The request retains no raw subject. Every fresh grant
write shares the subject fence, so asynchronous capture does not permit writes
under an already accepted revocation. Erasure has its own cutoff: a later
deauthorization cannot enlarge an earlier deletion instruction.

Signed requests retain their authenticated authorization cutoff. A missing or
malformed authorization date keeps that exact target pending without revoking
its token, unsubscribing it or purging its data. Verified chronology repair is
checked against both sides of the original cutoff before any provider effect.
A later grant stays outside the request. For an older deletion job missing its
cutoff, replay the identical signed request to the same app and deletion endpoint
within 24 hours of its signed issue time. Check the original confirmation code
for completion. After that window, leave the old target pending if other
inboxes connected since the original request must survive. A new deletion
request from the granting administrator authorizes erasure of **all eligible
grants for that administrator and app through the new cutoff**, including
intervening installations. Use this option only when the administrator intends
that scope and Meta issues a new signed request. It has its own confirmation
code. Once it completes cleanup for the exact captured grant,
the worker can settle the older target using that grant’s durable completion
proof. Check both confirmation codes. An older target without exact proof
stays pending. Draft deauthorization-only jobs also require an identical signed
replay within 24 hours to recover a missing cutoff. After expiry, a cutoff-less
deauthorization job remains pending. Deletion proof applies only to deletion
jobs. Never substitute the time of the retry for the original cutoff.
Deauthorization transitions reread the live grant in their transaction and
preserve a concurrent deletion request.
Missing or malformed exact completion
proof cannot remove a retained obligation or acknowledge success.

Manual Delete acquires the existing asset lease before stopping the installation;
a busy asset returns the existing retry conflict with its bot and routes intact.
It captures the handler's authorized snapshot and accepts a concurrent successful
purge only from typed completion proof for that captured grant.
Manual Delete captures its actor, owner, credential source, grant and stopped
generation on the original bot in the stop transaction. HTTP cleanup, assistant
Delete retries and account retirement honor that same obligation. A reconnect
cannot inherit it. Earlier removed manual installations require a verified
Delete audit for the unchanged installation, or a fresh owner-authorized Delete;
account retirement alone does not authorize a wider purge. The captured fields
and optional audit reference disappear with the bot after confirmed cleanup.

A manual installation with an empty login subject creates no managed grant
authority or retired managed subject. Its explicit owner-driven Delete/purge
remains available before account closure; ordinary account erasure still follows
personal ownership. Managed retired grants retain their login verification
capability: changing/deleting their application ID or login secret is refused
until captured retired scope has completed provider cleanup. This check is
platform/application scoped, uses indexed bounded reads and does not block
unrelated webhook-field updates.
A domain-separated keyed revocation
fence and the opaque request status remain to prevent replay without retaining
the provider subject or asset. Callback timestamps cover their entire
second; a delayed callback cannot revoke a grant authorized in a later second.

Legacy saved-result plans can outlive the 30-day TTL on `channel_messages` and
`channel_reply_deliveries`. On dev, a Meta inbox reaches this path through a
saved private-chat result. Ordinary NyxBot also permits an organization-owned
bot linked by an administrator to that person's personal specialist. In that
case the text plan belongs to the organization, while the saved assistant
message and NyxBot channel belong to the linking person. Its logical key is
`saved-result:<turn ID>:<destination SHA-256>`; the saved assistant message keeps
`handoff_delivery.targets` after delivery. Erasure first uses exact owner/source
joins, then reproduces that destination hash from the saved target and resolves
its original NyxBot channel's bot parent. Bot-owner history (including retained
ownership transfers) selects eligible receipts; the indexed exact turn lookup locates the saved message, then verifies its
conversation owner against the saved destination's individual channel parent.
That read is bounded and runs only when source/receipt evidence is insufficient;
it does not enumerate channel-owner history or expand receipt ownership.
When personal conversation deletion removed the saved handoff, a retained native
event can still prove the exact journal owner, destination and channel parent.
The source batch reads those events and parents by indexed IDs and persists
positive sibling attribution on the original receipt before its source expires.
Native gateway saved replies retain their existing linking-person journal owner
and record their independently verified bot parent as well. It preserves positively identified sibling and other-platform
receipts and never purges an owner's unrelated journals. Organization guest
handoffs and initiated Meta posts do not authorize this saved-result path.

If a relevant legacy receipt has lost both its source and its saved destination
attribution, cleanup logs `legacy_saved_reply_attribution_required` and remains
pending. Local delivery stays disabled; a signed deletion obligation receives
no terminal TTL or success status. For recovery, an operator must restore the
exact original owner/source metadata or saved `handoff_delivery.targets` and
channel-parent metadata from independently verified retained records or a
backup. Verify the owner, turn, destination hash and original bot association;
do not guess an association, delete all same-owner receipts or use a routing
checkbox as proof. An ownerless scoped source is not sufficient by itself;
independent owner-bearing reply receipts or the verified saved handoff can
resolve it without deleting that source. Contradictory parent evidence, including ownerless receipts naming a sibling,
keeps the obligation pending in either owner/source arrangement. An exact saved
handoff with inconsistent or incomplete parent ownership also remains pending,
including when its timestamp predates the bot. A saved
handoff with a retained channel preserves that channel's concrete bot identity;
verified different-platform native handoffs can still preserve unrelated org
receipts owned by their linking person. Restore metadata only, keeping credentials and message text
out of diagnostics. The existing pending-cleanup worker retries; a manual
installation can also repeat its ordinary Delete action. If exact attribution
cannot be recovered, retain the unresolved obligation and escalate it for
operator handling rather than reporting completed erasure.

After derived cleanup finishes, one transaction records a typed, exact-grant
completion timestamp and removes the retired bot. Missing or malformed proof
retains the obligation for verified repair; a matching stale retired obligation
is cleared atomically during replay or credential drain. This makes crash retry
idempotent without retaining participant or credential data.

Assigning a specialist to a fresh Meta organization inbox keeps guest exposure
disabled and private DMs owner-only, including when the org wallet is funded.
The administrator must explicitly enable customer conversations after readiness
checks. Repair preserves the last committed guest exposure choice; it refuses
if the checked connection changes or the acting administrator loses authority
before retirement, including delivery-health or route recovery. Direct Meta
customer setup checks its dedicated callback key and default together under the
bot routing fence. Ordinary Connect, assistant Connect, organization restoration
and preview use the same direct Meta default/key/callback proof. Activation
rechecks that proof inside its transaction, including the single eligible
fallback default. Setup refuses competing defaults and active unscoped wildcard
routes before retiring an assignment. Reactivating a Meta default reconciles
other defaults under the bot fence; Check refuses ambiguous historical defaults.
The ordinary route API still supports explicit default replacement, and explicit
per-conversation overrides retain their behavior. Route creation preserves
storage, credential and routing error categories after failed-setup cleanup.
Moving a specialist and issuing or rotating an owner challenge recheck the live
Meta default and dedicated callback under the same write fences. Route drift
returns a repair conflict before changing the assignment or challenge. Repairing
the same organization specialist retains its deliberately saved guest policy;
preview describes that retained policy.
Combined specialist/audience PATCH and ordinary or assistant Connect on an
existing Meta assignment commit the relink, inherited-thread changes and requested
audience or owner challenge atomically. They reload the confirmed assignment and
delivery health inside that transaction. Personal and organization guest enabling
use the same live route proof before opening access. Explicitly closing access
remains available on a failing route, subject to the existing owner/admin checks.
After the first organization Grants activation, the service list and permission
digests on a direct Meta binding are a ceiling. During a temporary fallback,
administrators can remove services, edit spending limits or close guest access.
Expanding that ceiling requires the reviewed Grants adoption flow. Upgrade all
serving writers and drain earlier versions before enabling these new channels.
Hosted assistant setup stores only its server-computed expected callback in the
trusted wake reference. Wake admission verifies that callback and the live route
before minting an owner code. An older Meta wake without that proof gives repair
guidance and issues no new code; ordinary-channel wakes retain their behavior.
Marked credentials are
authenticated under their old owner and rebound to the destination in the same
fenced ownership transfer, following the existing staged/strict record writer
mode; fresh Meta consent is still required for delivery. A failed storage read is retryable setup failure and never
proof that an existing route or key should be rebuilt.

Turning off enrollment blocks new setup and preserves management/disconnect.
For an incident, suspend delivery independently:

```sh
nyxid channel-bot delivery BOT_ID --suspend
nyxid channel-bot platform-delivery instagram --suspend
# Once provider configuration and authority have been repaired:
nyxid channel-bot platform-delivery instagram --resume
nyxid channel-bot delivery BOT_ID --resume
```

Platform delivery requires a platform administrator. During a platform suspension,
owners can reconnect existing inboxes to repair their credentials. Delivery stays
on hold. Restoring the platform gate
requires affected inboxes to be verified individually. Suspension increments
binding generations and preserves identities and journals; old in-flight
replies remain invalid after resume. Do not replay unknown sends. Disabling one
Meta platform does not suspend the other platform or WhatsApp.

## Live evidence and App Review handoff

Local tests use MongoDB transactions, signed fixture webhooks, the real relay
and reply handlers, mocked Graph/OAuth endpoints, and browser popup/asset-selection
flows. They establish NyxID behavior against the supplied wire contracts.
**No live Meta application, permission submission, dashboard configuration or
external audience was changed during this implementation.** Public provider
approval and native-inbox verification remain external release dependencies.

For each platform, the app administrator must supply and record:

- Actual App ID, product/login configuration, app mode, app-role test senders,
  eligible Page or Business/Creator inbox, separate tester account and Page
  messaging-task evidence.
- Current App Review → Requests state, requested/granted Advanced Access,
  Business Verification state and any permission dependencies shown in that
  app's dashboard. Review state is per App ID and cannot be inferred from a
  different Meta product or app.
- Graph v25.0 availability, successful real token exchange/identity/subscription
  readback, refresh eligibility, actual scope responses and app-scoped subject
  versus webhook inbox IDs. Verify both Instagram single-account envelopes.
- Public trusted redirect/callback/deauthorization/deletion URLs, their handshake,
  the actual signing secret source, and a redacted real event whose raw bytes
  verify correctly. Retain no token or customer text in review artifacts.
- A redacted managed Messenger Page token inspection result confirming the issuing App ID, token type, Page profile ID, granted scopes and any asset-specific scope restrictions.
- Messenger default-app and organic/social routing settings, the actual
  `thread_owner?recipient=PSID` own-app result, and behavior when Business Suite
  or another inbox app takes control. Verify Instagram exclusive routing and its
  control-loss behavior on the intended product configuration.

Then record one reviewer journey per platform: first-party sign-in → consent →
verified asset selection → specialist assignment → explicit guest enable →
unrelated tester's two successive DMs → visible matching native replies.
Include long emoji text, two assets/senders, duplicate/echo suppression, failed
callback health, revocation/reconnect, and suspend/resume. Verify guest execution
and funding using the existing personal/org billing evidence.

### Permission descriptions for App Review

| Requested permission | Description for the reviewer | What to show in the recording |
|---|---|---|
| `pages_show_list` | A business administrator chooses the Facebook Page that NyxID should connect. NyxID lists the Pages the administrator can manage and displays eligible Pages in a picker. | Sign in as the Page administrator and show the Page picker before selecting the test Page. |
| `pages_manage_metadata` | After the administrator selects a Page, NyxID subscribes the app to messaging events for that Page. NyxID checks the subscription and removes it when the administrator disconnects that inbox. | Connect the selected Page, open its health details, verify the subscription, then demonstrate disconnect after the messaging test. |
| `pages_messaging` | A customer starts a private Messenger conversation with the connected Page. NyxID routes that message to the support specialist selected by the business and returns the specialist's text answer to that same customer. | From a separate Messenger account, send two different test questions. Show each corresponding reply in the original Page conversation. |
| `instagram_business_basic` | A business authorizes NyxID through Instagram Login. NyxID reads the professional account's identity and username so the administrator can confirm which inbox to connect. | Sign in as the test Business or Creator account and show its name in the NyxID account-selection screen. |
| `instagram_business_manage_messages` | A customer messages the connected professional account. NyxID receives the DM and sends the assigned specialist's text reply to the customer within the permitted reply window. | From a separate Instagram account, send two different test questions and show the matching replies in Instagram Direct. |

Supply the working staging URL, reviewer access, test account identifiers and
recordings through the matching App Review request. Include the deployed
privacy policy, retention policy and data-deletion instructions. Request
Advanced Access for the permissions demonstrated by that app.
If the dashboard permits combining products on the same eligible app, its admin
may group them; otherwise wait for the existing request's decision or submit
through the corresponding distinct App ID. This implementation does not assert
that concurrent submissions are available. Enable public enrollment for each
platform only after that platform's approval and native-inbox matrix pass.

Provider references: [Messenger routing](https://developers.facebook.com/documentation/business-messaging/messenger-platform/conversation-routing),
[Messenger policy](https://developers.facebook.com/documentation/business-messaging/messenger-platform/policy),
[Instagram Login](https://developers.facebook.com/documentation/instagram-platform/instagram-api-with-instagram-login/get-started),
[App Review](https://developers.facebook.com/documentation/resp-plat-initiatives/individual-processes/app-review/submission-guide).

## Review hardening notes

Messenger Page credentials do not inherit the Facebook Login user-token
`expires_in`; NyxID stores no synthetic Page expiry. Page-token revocation or a
Graph error 190 is the authority that moves an inbox to reconnect-required.
Instagram long-lived credentials retain their provider expiry and are refreshed
before expiry.

A signed Meta deletion callback requests erasure but does not prove remote grant
revocation. Deauthorization is the separate revocation proof. The cleanup job
keeps the encrypted capability and pending target until the asset subscription
is confirmed removed or revocation is independently verified. Pending requests
have no TTL; only terminal completed requests expire from the status collection.
Erasure and revocation bits are monotonic per grant across either callback order.

Verification never clears suspension, reconnect-required, provider-revoked or
unverified routing state. A platform suspension preserves those per-inbox
states and the worker does not verify them while the gate is closed. Messenger
control gain is accepted only for an exact `pass_thread_control` app identity
and current grant timestamp, then live `thread_owner` proof is still required
before forwarding or sending. Instagram provider control loss remains closed;
repair the Instagram routing configuration and use Reconnect with fresh consent
to establish a new grant.

Future provider timestamps are rejected without a clock-skew window. Graph
ownership reads use bounded retries only for idempotent transport/5xx failures;
other transient failures are recorded as `meta_control_unavailable` with an
operator action to check Graph and routing. The provider request is never
silently retried after a possible send acknowledgement.

Before replacing or clearing a managed App ID or login secret, close new
enrollment for that platform, finish or cancel open setup sessions, and have
the granting administrators revoke access and request deletion in Meta for
retained grants and inboxes. With disposable review assets, verify whether Meta
issues a distinct signed request when the administrator requests deletion again
after app removal. Wait for the deletion confirmation statuses to
complete. A disconnected managed inbox can still hold data subject to a later
deletion request. NyxID keeps the login secret available while that app has
outstanding obligations. Updating the webhook fields or saving the same
credentials remains possible. Each platform has its own credential record.

For a manually registered inbox, the owner can run
`nyxid channel-bot delete BOT_ID` to disconnect the asset and remove its
retained local identifiers. If subscription removal is pending, retry after
the provider recovers. This installation uses the owner's supplied
credentials and has no managed OAuth grant subject.
