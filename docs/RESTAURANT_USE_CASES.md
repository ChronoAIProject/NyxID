# Restaurant use cases and customer connections

Status: proposed setup and acceptance definitions. No provider connection,
message, call, Notion write or application change has been made. The accompanying
SKILL.md files guide authenticated NyxBot through onboarding and specialist
creation. They are unpublished drafts; installing them and implementing missing
setup/runtime integrations remain separate work.

The initial catalog has exactly three use cases. Telegram and telephone are
channels shared by FAQ and reservations, not a fourth use case.

| Use case | Business outcome | Owner connects/configures | Customer contact |
| --- | --- | --- | --- |
| [FAQ agent](skills/setup-restaurant-faq/SKILL.md) | Answer approved business questions through Telegram and inbound telephone | Published FAQ, customer Telegram bot, business phone/voice integration, staff contact | Reply in the originating chat or live call |
| [Queue manager](skills/setup-restaurant-queue/SKILL.md) | Tell staff the current queue and let staff immediately notify an entry to return within X minutes | Authoritative queue source, authenticated NyxBot management, SMS sender, return window | SMS to that queue entry's verified, opted-in number |
| [Reservation manager](skills/setup-restaurant-reservations/SKILL.md) | Take requests from Telegram and phone and log them in Notion | Telegram bot, business phone/voice integration, Notion destination/property mapping | Acknowledge in the originating chat or call; retain chosen follow-up contact |

Working assumptions pending product confirmation: queue notifications are
staff-triggered, and reservations await staff confirmation. Automatic queue
advancement and automatic booking confirmation are separate extensions; neither
is implied by connecting SMS or Notion.

## Shared owner onboarding

Sign in, select use cases, enter shared business details once, connect each
required integration, select exact resources, review access, test and activate.
Reuse suitable personal connections and the same customer bot/number.

Authenticated NyxBot follows the selected onboarding skill to open real hosted
connection flows, verify completion, create/reuse the necessary specialist and
guide tests. Default to one specialist per use case; keep the queue specialist
private to staff. Skills stay on NyxBot for setup; runtime guidance is separate.
See the [tool mapping and specialist contract](USE_CASE_TEMPLATES.md).

| Connection card | Required selection and verification | Reused by |
| --- | --- | --- |
| Customer Telegram bot | Client-owned child bot, verified management/webhook, independent customer delivery | FAQ, reservations |
| Business telephone | Supported voice account, authorized inbound number or verified forwarding route, exact installation routing, intelligible spoken replies | FAQ, reservations |
| SMS sender | Messaging account, authorized sender/number or messaging service, supported recipient countries, signed status callbacks | Queue |
| Queue source | Supported authoritative source/backend, queue/location ID, readable state and supported staff transitions | Queue |
| Notion | Owner connection, exact accessible database/data source, property mapping and verified record creation/read-back | Reservations |

Voice and SMS capability must be verified independently even if they share a
provider account. Do not replace an existing number route or webhook without
owner review; preserve the native Telegram manager's existing webhook contract.
The external BotFather sees bounded drafts and status, not private connection
inventory. Signup establishes identity; connection consent is a separate step.

## Customer identity and reachability

Public customers need no NyxID account for FAQ or request submission. The owner
needs the NyxID account and integrations. Owner management through NyxBot is an
authenticated context separate from public customers.

| Contact | Establishes | Does not establish |
| --- | --- | --- |
| Verified Telegram event | Sender/chat on the exact customer bot and a bound reply destination | Customer phone number or access to another person's records |
| Verified phone event | Correct provider, business number and call/session route | Caller ID alone does not verify record ownership or SMS consent |
| Collected phone number | Possible callback/SMS destination, normalized with country code | Ownership/reachability until verified; consent until explicitly collected |
| Queue-entry contact | Verified number and notification consent scoped to that entry | General messaging or marketing permission |
| Reservation contact | Selected follow-up destination with verification/consent status | Automatic linking of a caller to a Telegram identity |

Scope contacts to owner, installation and business record. Preserve source
channel and sender/session references separately. Never merge customers by name
or caller ID. Cross-channel retrieval/changes need explicit verified linking or
recovery. FAQ does not require creating a customer profile or collecting a phone.

## FAQ agent

Start with owner-reviewed published FAQ facts: hours, location, menu/contact
links and policies. FAQ v1 does not require a private Notion connection. Both
channels use the same publication revision. Unknown answers offer staff contact;
a live phone transfer needs its own supported and tested adapter/target.

FAQ grants no record-writing or SMS access. On a shared bot/number, reservation
intent may route to the active reservation use case; otherwise offer staff
contact without claiming to take a reservation.

Acceptance: an independent Telegram customer and a real test caller receive
correct answers and unknown-fact fallback; private owner information is absent;
a publication update reaches both channels. Browser voice is not a telephone test.

## Queue manager

Initial surface: the signed-in owner asks NyxBot for current queue status. Read
the selected authoritative source, report actual state and freshness, then accept
an explicit staff action such as "Notify Q014 to return within 10 minutes."
Staff or an existing queue system populates entries; public self-service joining
is not required for v1. Missing/stale source data must be visible.

Minimum entry data: stable entry ID, display ticket, location, party size if
needed, status, join time, verified destination, notification consent and send
attempt/result. Map provider states explicitly. Suggested business states are
waiting, called, arrived, served, cancelled and no_show. SMS delivery is separate
from arrival/serving; delivered never means the guest has returned.

Example bounded message: "Your table at Example Restaurant is ready. Ticket
Q014: please return within 10 minutes, by 19:40. [Staff contact]"

The operation accepts an entry ID and return window. The backend resolves
recipient, sender and message template. Do not expose an arbitrary number/body
tool to the model. Clarify ambiguous tickets and recheck entry/version before
sending. Explicit authenticated staff intent can send immediately where existing
NyxID policy permits it; existing approval gates still apply. Public customers
cannot trigger staff sends.

Recommended v1 timing: persist a dispatch time and absolute deadline immediately
before a bounded send attempt. Include X and the local deadline; refuse already
stale sends. Report delayed/failed delivery to staff and require an explicit new
action for a revised notice. Do not claim the guest received the full window or
automatically mark a no-show based on uncertain delivery.

Use durable idempotent notification intents and signed provider receipts. Report
submitted/delivered/failed/unknown accurately; API acceptance is not handset
receipt. Reconcile ambiguous timeouts rather than blindly resending. Repeated
commands for the same intent cannot duplicate a send; intentional resend is a
new explicit action. Collect numbers through a bounded enrollment/verification
flow, such as customer-confirmed entry plus OTP or inbound SMS verification.
Record queue notification consent and support opt-out/cancellation before sends.

Acceptance: two entries have distinct verified numbers; notifying one sends only
to it with the chosen X/deadline; duplicate commands, wrong-owner entry, cancelled
entry, opt-out, failed delivery and unavailable source behave correctly. Use only
explicitly authorized test recipients. Public callers cannot send staff alerts.

## Reservation manager

Collect the same structured request by Telegram or inbound phone: customer name,
requested date/time, party size, bounded special requests, and chosen follow-up
contact. Resolve dates in business timezone, clarify ambiguity, read details back
and obtain confirmation to submit. Caller ID does not replace contact collection.

Select the exact Notion database/data source and map these properties:

| Property | Meaning |
| --- | --- |
| Request ID/title | Stable business request reference and display title |
| Customer name | Customer-supplied name |
| Requested time | Date/time with restaurant timezone preserved |
| Party size | Validated guest count |
| Contact/contact kind | Chosen follow-up destination; verification/consent metadata stays in the private request record |
| Source | Telegram or telephone |
| Status | Initially pending; staff confirms or rejects |
| Notes | Bounded special requests |
| Created at | Recorded request time |

Keep channel/event correlation and submission recovery in a private business
request record, not raw transcripts. Restrict Notion access to the chosen
destination and needed operations. Validate provider API version and property
types: database and data-source IDs are not interchangeable. Schema creation
requires a separately reviewed implemented step; otherwise guide owner repair.

Persist request identity/submission state before the Notion write and include
that identity in the row. Serialize local retries and reconcile uncertain writes
by identity. Query-before-create alone is not an atomic Notion uniqueness
guarantee; stop for recovery if outcomes cannot be determined. Deduplicate replayed
events, but do not automatically merge similar Telegram and telephone requests.

Say "request recorded, awaiting staff confirmation" only after destination write
verification. A locally queued write is "pending logging," not saved in Notion.
If a call ends after the write, preserve the request with acknowledgement
unobserved; do not repeat the write. Follow-up after a call/chat needs a reachable
authorized contact. Notion edits do not automatically notify customers: that is
a separate reviewed follow-up operation if added. Staff can contact customers
using the recorded contact outside this initial automated flow.

Notion is the request log, not a capacity engine. Initial scope does not promise
live availability or automatic booking confirmation. Automatic confirmation
would require an authoritative capacity service and concurrent-booking checks.

Acceptance: Telegram and phone test requests land in the same selected Notion
destination with correct source/status; customer read-back matches; duplicate
events do not duplicate rows; failures and uncertain writes never claim success;
another owner/customer cannot read the record.

## Shared routing and implementation evidence

Bind inbound Telegram bots and phone numbers to exact installations. Route only
to selected active capabilities and keep customer sessions separate. Preserve
intent while collecting a reservation; clarify ambiguous requests. Staff queue
management remains authenticated. Every effect rechecks live installation,
connection, resource, record, consent and budget authority. Shared connections
do not merge capability permissions.

This shared-channel design needs explicit specialist routing: today's platform
installation stores one agent ID. Repeatedly linking the bot to different agents
does not compose them. Extend the installation binding/runtime before exposing
multiple specialists through one bot/number; never use personal NyxBot as a
public fallback.

Local code/spec inspection, not live verification:

- Telegram relay and native child onboarding exist; preserve the current
  [BotFather contract](PLATFORM_BOTFATHER.md).
- The Twilio overlay has create/get/list messages and calls. This is not a
  complete inbound phone, sender-provisioning or SMS status adapter.
- The ElevenLabs overlay has conversation/signed-URL operations. It does not
  establish working business number provisioning or inbound call routing.
- The Notion overlay uses the `2022-06-28` database API contract. Compatibility
  with the selected destination/API must be verified during implementation.
- Browser [realtime voice](chat/11-voice.md) is not PSTN readiness. The public
  customer runtime is currently tool-free. No queue backend or Notion intake
  flow was verified live for this proposal.

Implement registered flows/adapters and bounded public operational execution,
then test with authorized accounts, numbers and data. Track readiness per channel:
Telegram-only FAQ is partial delivery of a template promising Telegram and phone.

See [onboarding skill format](USE_CASE_TEMPLATES.md) and
[implementation plan](plans/2026-10-15-use-case-bundles.md).
