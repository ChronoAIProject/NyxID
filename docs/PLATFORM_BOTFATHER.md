# Platform BotFather

Platform BotFather is an explicitly opted-in public entry to NyxID and a person's
NyxBot. Visitors can ask NyxID FAQs or request a signup/sign-in link. A browser
account review followed by confirmation in the original private chat links that
sender to their own NyxBot. Telegram also provides native bot creation and
management for the linked account.

The separate legacy onboarding guide collects a static assistant or restaurant
draft and provisions only the client-reviewed specialist and exact Telegram child.
The restaurant template publishes facts, FAQ and HTTPS request links; it never
confirms a booking, takes payment or grants connector/machine access. Its bounded
onboarding binding does not select the account used for personal NyxBot chat.

## Authority and ownership

Only live NyxID admins using a first-party human session can discover, convert,
configure, pause, verify, repair or retire platform profiles. Operators, org-only
admins and former platform admins cannot use these controls. Ordinary channel
bot lifecycle and conversation/default setters refuse converted managers.

Conversion is durable: paused, retired, missing or unknown claimed profiles stay
closed to personal routing. Existing `telegram_manager` channels without explicit
opt-in retain their personal behavior. Converted manager and installation markers
survive setup/profile retention or deletion. There is no personal-route fallback
for an opted-in manager or child.

The default public answer source is the approved, deterministic FAQ corpus in
`backend/content/platform-public-faq.json`. It makes no inference call and uses no
personal assistant key, connection, inventory, tool, memory or upstream session,
including for a linked person or administrator. It needs no model or sponsor.

An explicitly selected public AI configuration uses the working public runtime
contract from #1914/#1925: a dedicated public credential, approved public tools,
validated signed limits and an authorized organization sponsor. Existing saved
AI policies retain that selection; choosing built-in FAQ explicitly clears the
policy. Public AI never borrows a visitor's personal credentials or context.

Personal chat uses the currently confirmed personal binding, its live owner and
binding generation. It reuses that person's NyxBot, ordinary account tools,
permissions, memories, conversation credentials and billing. The conversation is
available in NyxID for web continuation as a separate saved channel thread. The
old home transcript is not automatically replayed into that thread. Each bot and
provider needs its own browser consent and same-conversation proof, even for the
same owner; disconnecting one leaves other verified channels usable. A legacy onboarding binding, a website
login or signup alone never establishes personal chat authority. Web and the
legacy Telegram guide edit the same encrypted onboarding draft; anonymous drafts
require explicit adoption.

With the public/personal runtime configured, `/start` and `/help` explain the
entry flow. `/login` (also `/link`) creates a short-lived account-review link;
`/status` reports live chat login state; `/logout` (also `/unlink`) disconnects
the exact chat authority and cancels its pending proofs. An already-issued
legacy onboarding confirmation remains a separate bounded compatibility flow.
Telegram `/onboarding`, `/setups`, `/setup <id>`, `/bots` and `/bot <id>` use the
current personal account, even if the legacy guide is disabled. Bot creation
links require the same account in the browser. A selected setup supplies context
to the next personal turn and grants no approval.

The client owns the specialist, child and published facts. A platform manager
operator does not acquire client-resource ownership. Public child inference
ignores the specialist's private grants, persona, skills, keys and home context;
it separately checks that the owned specialist still exists.

## Shared FAQ and personal setup

1. Open **Admin → Platform bots**. Existing bots and their access controls appear
   first. Open **Prepare another shared bot** only to create another entry.
2. Select an eligible connected bot. Telegram uses the existing `telegram_manager`
   channel; the other providers use one NativeV2 conversion path. Native setup
   reviews the exact bot and delivery transport. No funding organization, model
   discovery or AI readiness check is required for basic FAQ/personal access.
3. After preparation, the new bot stays visible even if it is outside the current
   profile page. Its heading shows the saved label and provider. Select **Public
   FAQ**, **Personal NyxBot**, or both, then choose **Enable bot**. A live transport
   is required. Optional public AI configuration is under its own disclosure.
4. In the bot's private conversation, request public help or use the provider's
   login control below. An ordinary first-party NyxID sign-in/signup (including
   email verification and MFA when configured) is followed by explicit consent.
   Review the exact provider, bot, external account and private conversation.
5. Return to that same bot conversation and submit the one-time confirmation.
   Browser login and consent alone do not link it. On Discord choose **/nyx
   confirm**, then enter the displayed `P-…` value in its **code** option. Other
   native providers accept `confirm P-…` as a plain private message; Telegram
   accepts `/confirm P-…`. Lost or expired proof requires a fresh review/login.
6. Once linked, ordinary private text uses the person's existing NyxBot; Discord
   uses **/nyx ask** and its **question** option. Permissions, memory, tools,
   account credits and existing organization billing rules are the normal personal
   ones. Public FAQ/help remains a public answer even for a linked person.

**Pause bot** stops FAQ/AI and personal work, including legacy confirmation
handoffs. Safe status, help or disconnect notices may still be sent. Pause retains
chosen modes. **Resume bot** makes one conditional update restoring the selected
modes; there is no separate clear-stop step. If both are off, select at least one.
A status refresh can show a connection blocker while the remembered choices stay
intact. Repair that blocker first, refresh, then Resume.

An expired native OAuth connection without a refresh credential shows **Repair
required** and its saved personal binding is shown as disconnected. Reconnect
that account through the reviewed repair flow. A refreshable connection retains
its entry identity. Atomic Resume validates the saved owner, connection and
refresh authority without contacting the provider. Authenticated ingress or
reviewed repair uses the existing OAuth refresh producer before admitting work.
Reading status or Linked channels does not refresh tokens or contact the
provider. Message admission and delivery still require a live token.

A public request commits metadata-only article IDs, live mode guidance and its
settled receipt in one transaction. No raw FAQ question/body is retained in that
queue. Sender (20/minute) and profile (200/minute) server-clock windows bound FAQ
admission before a receipt is recorded. Rejected events can retry after the window
resets; there is no lifetime visitor cap. Duplicate events retain their original
audience. Pending FAQ deliveries recover through the normal sweeper, public pacing
and exact receipt/transport fences. A control change after admission refuses the
old send. Partial or uncertain provider effects are not automatically replayed.

Telegram's optional child-bot workflow remains separate. `/onboarding`, `/setups`
and `/bots` use the linked account and existing reviewed setup/provisioning
contracts. The legacy guide and public child inference retain their own funding,
model, owner review and independent-customer activation requirements. Those are
not prerequisites for a shared FAQ entry.

## Native chat entries and operator recovery

In `/admin/platform-bots`, an administrator can convert an existing channel into
a shared native entry. Select the channel and an explicit delivery transport.
An optional setup-funding organization is available for existing advanced policies;
basic setup defaults its administrative metadata to the acting platform admin.
This does not change the existing channel owner or transport payer. The UI loads the supported choices from
`GET /api/v1/admin/platform-bots/native/transports`:

| Platform | Supported transports | Private chat surface |
| --- | --- | --- |
| Discord | Direct or gateway | Installed `/nyx` application command |
| Slack | Direct or gateway | Authenticated direct messages |
| WhatsApp | Direct or gateway | Authenticated private messages |
| Lark / Feishu | Direct | Authenticated private messages with raw event verification |
| X | Direct | Signed activity webhook private messages; paid X transport policies apply |

An unsupported gateway choice is refused before setup or provider work. Select a
supported transport explicitly; losing a selected gateway makes the entry
unavailable and requires reviewed recovery. The entry never switches to direct
delivery automatically. Telegram retains its existing conversion and native bot
creation workflow described above.

Discord uses `/nyx help`, `/nyx login`, `/nyx status`, `/nyx logout`, `/nyx confirm`
and `/nyx ask`. Use `/nyx ask` for questions and personal action-card replies.
Other native private-message surfaces accept exact plain-text controls `help`,
`faq [topic]`, `login`, `status`, `logout` and `confirm P-…`. Slack uses those plain
messages because its slash commands are a different integration. Discord ordinary
DM text does not reach this integration: choose the installed `/nyx` command and
its named options. Conversion installs/reviews the exact known `/nyx` command
without replacing unrelated application commands. Follow the exact confirmation command
returned after browser account review in the original private chat; browser
approval alone does not sign the chat in. Forwarded, group and mismatched sender
events cannot establish personal authority. Telegram's `/onboarding`, `/setups`
and `/bots` management controls remain Telegram-only.

Conversion and updates start with both public and personal access paused. Finish
verification and explicitly enable each intended audience. Built-in FAQ is the
default; only an explicitly selected public AI deployment needs sponsor/model
readiness. The runtime panel reports readiness and access
state; the legacy guide's enabled flag does not describe native availability.

If setup fails or its outcome is uncertain, the channel keeps a durable pending
marker and refuses ordinary owner routing. Use **Continue reserved native setup**
for the retained attempt. For a gateway, **Inspect retained gateway setup** reads
the exact reserved remote record; review the result and choose **Retry reviewed
native setup**. Inspection and retry bind the retained attempt, request and
credentials. Do not remove markers, rotate dedicated transport keys through
ordinary key APIs, or create a substitute owner route to work around a failure.

Use **Review failed or changed native setup**, or **Review current native entry**
on an existing profile, when credentials, the configuring administrator, funding
authorization or selected transport need a new review. Select replacement
credentials belonging to the channel's existing owner, confirm the review, then
choose **Update reviewed native entry and pause**. A successor administrator needs live platform-admin access and can use their own
basic setup authority without sponsor membership. An explicitly selected setup
organization still requires its live administration rights. Public AI separately
requires an authorized organization sponsor. Review does not transfer the channel
owner or its transport payer. Reservation pauses the entry and revokes old chat
receipts, confirmations and turns before provider work. A failed verification
retains a recoverable reservation. A successful update also stays paused until
explicit Resume, and prior authority remains invalid afterward. The Linked
channels view shows old bindings as disconnected and requiring reconnection.
A fresh proof creates current authority; deferred cleanup is bound to old
revisions and cannot stop a new turn admitted after Resume.

The corresponding authenticated API is `POST /api/v1/admin/platform-bots/native`
for conversion or a reviewed update, with the server-returned review from
`POST /api/v1/admin/platform-bots/native/{bot}/review`. Gateway inspection uses
`POST /api/v1/admin/platform-bots/native/{bot}/inspect`; retry submits the exact
returned recovery review. Retirement uses the existing platform-profile retire
control and retains routing/credential tombstones. Native repair uses this review
workflow; the Telegram repair endpoint refuses native profiles.

Ordinary token-based Telegram channels and `telegram-new` managed children keep
their existing owner/child workflows; they are not a second shared-entry setup.
Aurinko email's claimed From address is insufficient same-channel account proof.
OpenClaw has no usable reply transport. Neither is offered as a shared entry.
The ordinary X adapter can poll DMs, but NativeV2 shared entry uses authenticated
activity webhook provenance; polling is not an alternative shared login route.
Groups, bots/self, edits and forwarded controls cannot establish personal proof.
Slack human uploads and control-shaped Discord question arguments receive fixed
supported-input help and admit no model, FAQ queue or personal media work.

Native delivery retains main's short receipt-bound deadline and encrypted reply
handles. A result that outlives that authority is refused for the chat and remains
available in its saved NyxID thread; it cannot adopt a later credential or newer
receipt. Inspect the actual runtime and delivery state when recovering a failure.
No live provider setup or production activation is implied by local tests.

## Funding and limits

Optional public AI, the legacy guide, paid model discovery and readiness use a purpose-confined catalog-platform inference
path billed to the explicitly authorized sponsor organization. The configured
admin is never supplied as a personal payer. Ordinary platform-key execution
continues to use `BillingOwnerResolver::resolve_for_execution` and bill the acting
person; that global policy is unchanged. Child customer inference uses the client
owner's personal execution payer. Neither path consults personal connections as
a fallback.

Only a live permitted HTTP inference catalog service with supported model
listing can be selected. Discovery resolves GET `models` operation pricing and
protocol headers. Readiness requires an advertised exact ID and an actual tiny
response reporting that exact model. Aliases with a different reported model are
refused. Missing service/model/grant/sponsor readiness fails closed.

Admission uses Mongo server-clock rate windows and leases, atomic maximum-cost
reservations, sender/platform concurrency and exact daily-credit caps. The whole
inference lifetime is bounded below the lease duration. Successful calls await
lease release; cancellation has idempotent cleanup. Daily caps conservatively
count the admitted maximum for forwarded calls while ledger settlement records
actual usage. Before-forward holds are refundable. An independent database-only sweep settles expired holds once, using the original UTC reservation day; dispatch-admitted calls retain conservative spend after crashes. Capacity/funding error replies
consume bounded reply admission and do not dispatch another model call.

## Expiry, revocation and recovery

Draft expiry ends draft authority; TTL deletion cannot restore personal routing.
Edits, pause/cancel, unlink/replacement and profile changes invalidate prior
approval/test authority immediately, including resumed worker and late reply
admission. Fresh human approval preserves planned resource identities rather than
creating replacement bots. Native creation provenance supports reviewed recovery
after native request retention, but management drift is refused. Provisioning
stops after five failed or crashed attempts and requires a fresh owner review
without replacing planned resource IDs. The setup page explains detected runtime
unavailability instead of presenting an unavailable installation as serving.

Activated installations clear draft retention expiry. A same-owner onboarding
binding renewal does not transfer client ownership or revive old onboarding
requests. Onboarding expiry alone does not end active owned consent. Explicit
unlink/replacement, account disable, specialist destruction/ownership change,
manager/child management changes and platform pause fence public effects.

Conversion cleanup captures exact old turn, async operations, machine jobs and
remote gateway identity/version. Claim expiry does not authorize cancelling a
new owner turn. Exact machine cancellation requires advertised node capability;
there is no broad-cancel fallback for old nodes. Gateway version drift is refused
rather than deleting an unreviewed binding. Admins can inspect previous gateway
connections and explicitly retry the captured version, preserve a changed binding,
or close cleanup after confirming the remote connection is absent. Each resolution
rechecks live admin authority and the exact reviewed remote version; it never adopts
a newer binding into a deletion request. Runtime evidence is tracked in the checklist. Account disable pauses setups and revokes
bindings; deletion clears encrypted/public setup content while retaining child
installation tombstones. Durable retirement progress lets interrupted cleanup
advance in bounded batches, and ordinary accounts with no platform state keep
standalone Mongo compatibility. Runtime validation is tracked separately.

## Deployment and external acceptance

No deployment or live enablement was performed for this implementation. Before a
rollout, all replicas handling manager/child traffic must enforce the new markers,
worker scopes and admission checks. Rolling back to a binary that ignores those
markers is unsafe; pause/deactivate opted-in traffic using a compatible enforcing
binary and retain tombstones. Do not remove markers to simulate rollback.
Upgrade all ingress, callback, command and delivery replicas to understand opaque
v2 native identities before the first native conversion or update writes v2
records. Paused and pending setup already persist those records and routing
constraints. After that point, keep v1-only binaries out of serving and automatic
restart paths. Retained Telegram v1 proofs and saved deliveries remain compatible,
but a v1-only binary cannot safely recover a native v2 entry. Preserve the selected
transport, installation ownership and retirement markers throughout rollback or
recovery.

Startup also upgrades the shared `platform_public_turns` index so legacy
deduplication and multiple runtime transcripts can coexist. Before starting an
upgraded replica, prevent older binaries from entering startup, including through
automatic restarts; keep them out of restart paths throughout the rolling upgrade
and afterward. An older startup can recreate the unrestricted index or fail once
multiple runtime transcripts exist. This requirement also applies to deployments
using only the legacy Telegram public runtime, before any native conversion.
Use compatible binaries for rollback or recovery and retain both legacy rows and
runtime transcripts.

Required external validation remains explicit: signed native manager creation and
stale-form delivery, manager token rotation and shared webhook preservation,
two distinct client accounts, a distinct independent Telegram customer, observed
exact reply/challenge confirmation, and provider model/funding readiness against
the intended staging catalog. Local mocked provider success or HTTP acknowledgments
are not proof of A13 delivery. See the approved plan's A1–A16 table and
[implementation evidence](plans/2026-10-14-platform-botfather-implementation.md)
for actual commands, frozen snapshots and outstanding tests.
