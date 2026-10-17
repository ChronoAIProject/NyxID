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

Public FAQ execution and the legacy guide use no personal assistant key,
connection, inventory, tools, private context or upstream session, including when
an administrator chats with them. Public FAQ execution uses a dedicated public
credential, three approved tools and the validated runtime's signed limits.

Personal chat uses the currently confirmed personal binding, its live owner and
binding generation. It reuses that person's NyxBot, ordinary account tools,
permissions, memories, conversation credentials and billing. The conversation is
available in NyxID for web continuation. A legacy onboarding binding, a website
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

## Local workflow

1. An admin converts an existing manager channel and chooses a dedicated sponsor
   organization they can administer. Conversion starts paused, revokes old route
   and assistant keys, and queues exact old-turn/transport cleanup. It leaves the
   shared native creation webhook intact.
2. Save a paused guide configuration in `/admin/platform-bots`, discover its live
   advertised models, then explicitly validate and enable the selected exact
   model. Readiness/discovery use sponsor funding and can consume sponsor credits.
3. Clients use `/login`, complete first-party signup/sign-in and account review,
   then send the provided confirmation in the original private chat. The same
   chat becomes their personal NyxBot. `/onboarding` offers account-bound native
   creation and guided setup; `/setups` lists their drafts. Legacy guide-only
   profiles retain their bounded onboarding flow and confirmation protocol.
4. The client publishes and reviews the exact saved facts, limits and resource
   plan. Approval binds the owner, revision, digest, profile/binding generations,
   planned stable IDs and zero-capability policy.
5. Open the native creation link. After Telegram creation is observed, review the
   exact child identity and management revision in NyxID. Observation alone does
   not authorize registration. Existing native workers enforce the setup scope,
   request generation and claimed-step lease inside resource transactions.
6. Once ready, an independent customer sends `/test` to the child and replies to
   the exact observed test message with its challenge. The owner's chat and
   `sendMessage` success alone cannot establish customer delivery.
7. The client explicitly activates the reviewed installation. Public execution
   remains fenced on installation/setup generations, live ownership, manager
   observation, child management, funding and approved publication.

Pending pages refresh at bounded intervals and have manual refresh. Metadata-only
status notices and interrupted-delivery notices use bounded durable retries;
browser status is authoritative if a Telegram notice is lost. Message bodies are
not a durable delivery queue, and interrupted model work is not replayed.

## Native chat entries and operator recovery

In `/admin/platform-bots`, an administrator can convert an existing channel into
a shared native entry. Select the channel, a sponsor organization you administer,
and an explicit delivery transport. The UI loads the supported choices from
`GET /api/v1/admin/platform-bots/native/transports`:

| Platform | Supported transports | Private chat surface |
| --- | --- | --- |
| Discord | Direct or gateway | Installed `/nyx` application command |
| Slack | Direct or gateway | Authenticated direct messages |
| WhatsApp | Direct or gateway | Authenticated private messages |
| Lark / Feishu | Direct | Authenticated private messages with raw event verification |
| X | Direct | Authenticated private messages |

An unsupported gateway choice is refused before setup or provider work. Select a
supported transport explicitly; losing a selected gateway makes the entry
unavailable and requires reviewed recovery. The entry never switches to direct
delivery automatically. Telegram retains its existing conversion and native bot
creation workflow described above.

Discord uses `/nyx help`, `/nyx login`, `/nyx status`, `/nyx logout`, `/nyx confirm`
and `/nyx ask`. Use `/nyx ask` for questions and personal action-card replies.
Other native private-message surfaces accept the supported text controls such as
`/help`, `/login`, `/status` and `/logout`. Follow the exact confirmation command
returned after browser account review in the original private chat; browser
approval alone does not sign the chat in. Forwarded, group and mismatched sender
events cannot establish personal authority. Telegram's `/onboarding`, `/setups`
and `/bots` management controls remain Telegram-only.

Conversion and updates start with both public and personal access paused. Finish
verification, review the runtime funding/model configuration, and explicitly
enable each intended audience. The runtime panel reports readiness and access
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
choose **Update reviewed native entry and pause**. A successor administrator must
have live platform-admin and sponsor access; review does not transfer the channel
owner or its transport payer. Reservation pauses the entry and revokes old chat
receipts, confirmations and turns before provider work. A failed verification
retains a recoverable reservation. A successful update also stays paused until
explicit re-enablement, and prior authority remains invalid afterward.

The corresponding authenticated API is `POST /api/v1/admin/platform-bots/native`
for conversion or a reviewed update, with the server-returned review from
`POST /api/v1/admin/platform-bots/native/{bot}/review`. Gateway inspection uses
`POST /api/v1/admin/platform-bots/native/{bot}/inspect`; retry submits the exact
returned recovery review. Retirement uses the existing platform-profile retire
control and retains routing/credential tombstones. Native repair uses this review
workflow; the Telegram repair endpoint refuses native profiles.

## Funding and limits

Guide, discovery and readiness use a purpose-confined catalog-platform inference
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
