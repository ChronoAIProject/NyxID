# Platform BotFather

Platform BotFather is an explicitly opted-in native Telegram onboarding manager.
It collects a static assistant or restaurant draft, links the private Telegram
sender to a first-party human account, and provisions only the client-reviewed
specialist and exact Telegram child. The restaurant template publishes facts,
FAQ and HTTPS request links; it never confirms a booking, takes payment or grants
connector/machine access.

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

The guide uses no personal assistant key, personal connection, inventory, tools,
private context or upstream session, including when an administrator chats with
it. A linked sender can access only narrowly projected, approved onboarding
setups. Web and Telegram edit the same encrypted draft; chat transcripts are not
merged. Anonymous drafts require explicit adoption after linking.

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
3. Clients enter `/onboarding` or the manager's bare `/start`, link their account
   using web approval plus same-private-chat confirmation, and resume or adopt
   their draft. Linking grants bounded onboarding authority only.
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

Required external validation remains explicit: signed native manager creation and
stale-form delivery, manager token rotation and shared webhook preservation,
two distinct client accounts, a distinct independent Telegram customer, observed
exact reply/challenge confirmation, and provider model/funding readiness against
the intended staging catalog. Local mocked provider success or HTTP acknowledgments
are not proof of A13 delivery. See the approved plan's A1–A16 table and
[implementation evidence](plans/2026-10-14-platform-botfather-implementation.md)
for actual commands, frozen snapshots and outstanding tests.
