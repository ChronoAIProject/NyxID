# Platform BotFather implementation review

Primary reviewer: main session. The sole implementation worker is Heca agent
`01a136d8-23ed-7cc1-907c-de7c08807589` (GPT-6.1-Sol, xhigh, full access).
No additional reviewer agents. Implementation and review are in progress;
unchecked entries below are not verification claims.

## Review gates

- [ ] Platform control reads/writes require live NyxID admin membership and
  first-party human authentication; operators and ordinary users are rejected.
- [ ] Public onboarding remains usable without admin authority; client setup
  controls enforce their own authenticated owner, never a supplied owner ID.
- [ ] Existing personal bots retain absent-field defaults and existing owner
  discovery/approval semantics.
- [ ] Platform messages cannot select personal home threads, keys, prompts,
  memory, discovery, machines, permission cards or broad account tools.
- [ ] Direct, gateway, route changes, key changes, reconnect and recovery cannot
  bypass platform authority or installation activation.
- [ ] Identity binding has private numeric sender binding, explicit human
  review, bounded attempts/expiry, one-time redemption and revocation fencing.
- [ ] Setup records, typed facts and published FAQ are tenant-scoped; public
  transcript text cannot become privileged instructions.
- [ ] Native Telegram creation stays serialized; exact child review prevents
  stale form ambiguity and assignment to the wrong setup.
- [ ] Existing Telegram worker/connect internals enforce live approval, setup
  revision and profile/binding generations before effects and registration.
- [ ] Provisioning is retry-safe; uncertain remote outcomes cannot duplicate
  resources; cancellation fences new work and reconciles started work.
- [ ] Installation activation gates all private/group/channel/follow/reply
  paths, including owner-seen auto-admission and ordinary access setters.
- [ ] Limits coordinate across replicas; explicit funding does not inherit
  the operator wallet; exact accounting is reused where money is involved.
- [ ] Pause/unlink/delete/account disable/profile changes invalidate live and
  recovered execution, with complete cascades and bounded retention.
- [ ] Frontend uses existing form/auth/router patterns; registration respects
  invitation/MFA rules and preserves only safe handoff data.
- [ ] Restaurant FAQ publication is explicitly reviewed; reservation form links
  do not claim confirmed bookings.
- [ ] Relevant tests, formatting, lint/type checks and builds pass; runtime
  evidence is distinguished from unperformed external staging checks.

## Test environment

The existing MongoDB on port 27017 is standalone and was left untouched.
A dedicated loopback-only replica set was started for this task:
`mongodb://127.0.0.1:27119/?replicaSet=nyxidBotfatherTest`.
An actual session transaction inserted a probe and aborted it; subsequent count
was zero. Use `NYXID_TEST_DATABASE_URL` for transaction-dependent test runs.
The primary session owns process cleanup (PID 70903); ephemeral data path is
recorded in `/tmp/nyxid-botfather-test-mongo-path`.

## Findings and resolutions

R1 — Source revised, tests pending: initial `platform_bot_authority::personal_routes_allowed` and the
route insertion transaction reject all `credential_source=telegram_manager`
bots, including existing installations with no platform opt-in. This contradicts
default-off compatibility. Requested an explicit persisted platform marker,
transactional conversion fence, fail-closed tombstone and a legacy manager
regression test. Sent to the implementation worker.

R2 — Open: conversion directly nulls `active_turn`, bypassing stop/settlement
and cancellation of asynchronous work. Requested preservation of turn cleanup
and billing lifecycle with durable fencing, plus an in-flight conversion test.
Conversion must also handle legacy standalone routes and pending reconnects;
gateway cleanup must preserve the manager's shared creation webhook.

R3 — Open: conversion accepts any active organization as payer, without
checking the configuring person's authority over that wallet or an explicit
platform funding policy. Requested authorized destination validation and live
admission rechecks, with unrelated-organization rejection tests.

R4 — Open: ordinary channel-bot lifecycle handlers still authorize by bot
ownership. Requested coverage of update/delete/verify/recovery/transfer for
converted bots, including a former platform admin and an organization-only
admin. Platform config endpoints alone cannot establish admin-only management.

R5 — Open: lifecycle restrictions also block normal admin channel deletion,
while platform-profile deletion initially only tombstones its config. This
would prevent manager replacement, whose existing prerequisite is retiring the
channel. Requested dedicated admin repair/retirement paths preserving tombstones
and the shared webhook contract.

Source follow-up: R2 now retains a stopping turn, uses a durable cleanup queue,
gates new turns and schedules cleanup independently of connector sweep settings.
Connection publication writes the bot fence transactionally; conversion refuses
pending or unresolved gateway attempts. R3 now checks and fences funding-org
membership. R4 has ordinary lifecycle/transfer restrictions; R5 has admin
repair/verification and retirement that deactivates the channel. All remain
pending final regression/race evidence and complete-feature review.

Setup review requirements communicated: anonymous idempotency is sender/profile
scoped; repeated create/cancel cannot bypass retained-data/creation limits;
edits/expiry/cancel invalidate running child authority; missing/purged setup
must never restore personal routing; delivery proof binds the exact approved
generation and is stronger than a successful send API response.

R6 — Open: binding decision rate admission was inside the decision transaction,
so invalid-code failures roll it back. Fixed 750 ms windows also do not enforce
minimum spacing across a boundary. Requested durable rejected-attempt accounting,
actor/IP limits and invalid-guess/boundary tests.

R7 — Open: unlink increments binding generation but leaves previously approved
chat-confirmation requests live; they can resurrect a revoked binding. Requested
request-to-binding generation fencing or atomic cancellation, with late-confirm
after unlink/replacement tests.

R8 — Open: `/setups` prepares client status using a binding but rechecks only
the profile generation before sending. Requested exact binding/user validation
at send admission, properly scoped status projection, public-preview rate limits
and metadata-only update deduplication for repeated `/link` deliveries.

R9 — Open: expected linking-command errors are logged and acknowledged without
a Telegram reply. Requested bounded, safe recovery guidance for expired/wrong
codes and throttling, while retaining reply authority and webhook ACK behavior.

Early verification: backend `cargo check -p nyxid --bin nyxid-server` passed
(2m46s), but source edits overlapped the build, so it is not a pinned baseline
or final-diff check. Unchanged frontend channel-bots/detail tests passed
(55 tests, two files). Neither result verifies the completed new feature.

The primary early `cargo nextest ... test(platform_onboarding)` run was stopped
during compilation because free disk fell to approximately 4 GiB. It produced
no test result. This worktree's generated `target` occupied 13 GiB (7.5 GiB
incremental); primary cleared only that cache. Subsequent builds use
`CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0`
and run against coherent checkpoints to avoid repeatedly compiling partial edits.

Lean `cargo check -p nyxid --bin nyxid-server --tests` first found a
`Zeroizing<String>` Display formatting error in the Telegram link response.
After correction, the second run passed in 2m58s with one unused
`approval_digest` warning (provisioning not yet wired). Log:
`/tmp/nyxid-botfather-lean-check-2.log`. This is a typecheck snapshot only.

Source review has identified the alternate
conversation setters, group owner-presence auto-admission, and the independent
Telegram connection worker as required enforcement boundaries. These are
already communicated to the implementation worker and covered by the plan.

Frozen foundation checkpoint: backend manifest aggregate
`f5961f3702acff1cd096e153d8c7ee513ff7889f768ad3e96ecf64e87056a326`.
Primary started lean `cargo nextest run -p nyxid --bin nyxid-server
-E 'test(platform_onboarding)' --test-threads 2` against the isolated replica
set. Log: `/tmp/nyxid-botfather-frozen-runtime.log`. Compilation in progress;
no result yet. Worker is continuing frontend work while backend remains frozen.

R6–R9 source follow-up: durable DB-clock spacing, prior-binding generation
comparison, pending-confirmation cancellation, exact status-reply admission,
safe command error replies and regression tests are present. Runtime verification
is still pending. Admin profile discovery and client-owned binding list/revoke
endpoints have also been added.

Open review question: `permits_personal_routes` now permits a missing bot row
when no platform profile references that channel id. Requested a compatibility
justification and all-caller trace, plus fail-closed child installation handling
in the upcoming runtime stage. Existing tombstone tests cover a missing converted
manager row, not a wholly orphaned installation.

R10 — Open: cleanup leases fence completion but not cancellation effects.
A stalled worker can outlive its claim while a retry completes cleanup and
unblocks the conversation. If the owner starts a new private turn, the old
worker's conversation-wide request_stop/async/machine cancellation and fresh
active-turn lookup can cancel that unrelated new work. Requested captured
conversion turn/operation identities and effect fencing, with a stale-claim
versus retry/new-turn regression. Gateway cleanup also needs durable remote
identity/recovery if the local channel row disappears before remote deletion.

Frozen foundation runtime result: 17 onboarding tests passed, zero failed
(nextest run `c8877d4e-017c-4fa8-a395-fcb8d8a26847`, 4.483s after 6m36s compile).
This verifies the listed identity/control/setup cases, not the unfinished guide,
provisioning/runtime stages or R10. Started a separate compatibility run covering
existing channel routing, channel bot service and Telegram creation suites;
log `/tmp/nyxid-botfather-frozen-compatibility.log`.

Compatibility result: the cargo invocation unexpectedly started recompiling
unchanged sources, so primary stopped it before tests and ran the existing
checkpoint binary `target/debug/deps/nyxid_server-ffcf344f44c98ff7` directly.
Manifest verification matched every backend file. Routing: 12 passed;
channel-bot service: 18 passed; Telegram creation: 68 passed; zero failures
or ignored tests in those suites. Logs:
`/tmp/nyxid-botfather-{routing,channelbot,telegram}-compat.log`.
Backend freeze is released for the next implementation stage.

Frontend review, pending fixes/tests: local form/mutation/confirmation state
must be reset on actor/resource changes, and a lost create response must retain
its idempotency reference across refresh. Retirement currently captures a
profile for review but submits the latest mapped profile; binding revocation
stores only the id. Both must bind the reviewed generation and require review
again after a refresh changes it. Invalid setup UUIDs must show an unavailable
state rather than a permanently pending disabled query. Sent to the worker.

Primary frontend verification: the four new page/schema/idempotency test files
passed (11 tests, 1.32s) at the current frontend snapshot. Log:
`/tmp/nyxid-botfather-primary-frontend.log`. This is component-level evidence;
browser end-to-end and final full-feature checks remain pending.

R11 — Open, confirmed by worker: missing-bot routing allowance only preserves
incomplete test fixtures; no valid production compatibility reason exists.
`resolve_agent`, cached callback loading and NyxBot link checks can reach the
predicate without otherwise proving bot existence. Worker will deny absent
bot rows, seed actual bots in legacy fixtures, and add orphan regressions.

R12 — Open: `/onboarding/link` is absent from telemetry sensitive-path
exclusions, and its code input lacks sensitive marking. API response referrer
policy does not set the HTML document policy. Requested page-level no-referrer,
sensitive input annotation, and telemetry suppression tests including SPA
navigation from an already opted-in page and URL/referrer event properties.

Primary frontend typecheck passed (`npx tsc -b --pretty false`). Local Chromium
smoke used mocked `/api/v1` responses on Vite port 4827 for starter selection,
link review, setup list, setup detail and admin profiles at 1440px and 390px.
All ten page/viewport checks rendered expected headings with no page errors or
horizontal overflow. Mobile admin screenshot was visually inspected. Script,
log and screenshots are `/tmp/nyxid-botfather-browser-review.mjs`,
`/tmp/nyxid-botfather-browser-review.log`, and
`/tmp/nyxid-botfather-admin-{1440,390}.png`. These are smoke/layout checks, not
backend integration or live Telegram evidence. Primary owns Vite session 23397.

R12 source resolution verified locally: linking route joins telemetry exclusions,
matching-code input is sensitive, and page emits no-referrer metadata. Primary
telemetry suite passed 32 tests, including the added onboarding privacy cases;
log `/tmp/nyxid-botfather-primary-privacy.log`.

Checkpoint finding status: R1's explicit opt-in compatibility is verified by
the foundation and legacy Telegram suites. R6–R9's specified binding/throttling/
reply regressions passed in the 17-test foundation run. R2 remains open through
the later R10 stale-worker finding; R3's conversion destination check passed,
but runtime funding admission is not implemented yet. R4/R5 broader lifecycle
and recovery coverage remains part of final acceptance. R11 source is being
corrected with updated fixtures; its new runtime evidence is pending.

Extended local browser run passed at both widths: matched-code approval sent
the expected request without an owner override and advanced to chat confirmation;
editing hours sent the reviewed setup revision, saved, and rendered revision 2.
No page errors/overflow across the ten page/viewport checks. API responses remain
mocked; log `/tmp/nyxid-botfather-browser-actions.log`.

Primary R10 node regression: `cargo test -p nyxid-cli --bin nyxid
exact_cleanup_job_ids_do_not_cancel_a_new_turn_in_the_same_conversation`
passed one actual-process test (23.38s compilation, 0.01s test). CLI/machine
source manifest remained unchanged. Log:
`/tmp/nyxid-botfather-primary-exact-jobs-bin.log`. An earlier `--lib` invocation
selected a helper target and ran zero tests; it provides no verification.
The backend stale-claim and gateway recovery tests still await runtime execution.

Related CLI regression run on that same binary passed all three machine-job
tests and the cancellation-scope test (four total, including the exact-job test).
Logs `/tmp/nyxid-botfather-primary-{jobs,cancel}-compat.log`.

Small follow-ups for the next review batch: the conversation cleanup branch
currently calls `admit_cleanup_effect` twice consecutively; remove the redundant
write. Frontend daily-credit validation accepts zero and values above the
server's 1000-credit maximum; align its exact-decimal validation and displayed
constraints with the server without floating-point money conversion.

Primary shared machine protocol regression: all 29 `nyxid-machine` tests passed,
including signing, replay/downgrade fences, operation capabilities and serialization.
Log `/tmp/nyxid-botfather-primary-machine-protocol.log`.

Next-stage review targets: ensure setup-bound native creation challenges check
the binding's exact Telegram sender before mutating their first-start/consent
state, not only at connection; ensure generated resource IDs and observed child
review do not accidentally invalidate or silently widen the approved digest.

Funding design constraint sent to worker: `BillingOwnerResolver::resolve_for_execution`
charges `NyxidManagedMaster` to the acting person's wallet even for an org-owned
resource. Native guide funding must use an explicit, purpose-confined path and
must not accidentally charge `configured_by` or relax the ordinary platform-key
rule. Verify selected model/binding/org/funding failure has no personal fallback.

Local browser server cleanup: primary stopped the task-owned Vite listener on
port 4827 after browser verification. The isolated test MongoDB remains running
for subsequent backend checks.

Provisioning source review in progress: `fence_request` must validate the live
request's sender, observed child and management revision, not just its supplied
snapshot. Check that all relevant native management events increment
`connection_generation` atomically or match those live fields explicitly. Also
check retry/status behavior after profile pause so stale stored approval is not
presented as current operational readiness. These are review targets while the
integration is still being written, not completed-stage failure claims.

Additional installation boundary to verify: manual token registration of an
observed setup child before exact-child approval. Its installation may not yet
have a remote id; registration must still recognize the setup-linked native
creation request/provenance and cannot turn it into an ordinary active bot.

Provisioning review follow-up: expired `waiting_consent` requests could be approved
before the expiry sweep, because direct child review bypassed the ordinary `get`
expiry path. The live request fence now requires active/unexpired pre-provisioning
statuses while retaining admitted provisioning/connected recovery. Source corrected;
expiry-race and delayed-review runtime tests remain pending.

New step lease finding: resource creation in `begin_approved` and
`create_approved_specialist` checks approval authority but currently omits the
claim/generation/DB-clock lease fence on its write transaction. Later auto-connect
fences its job, which does not cover preceding resource creation. Worker asked
to carry the exact claimed-step authority into both resource transactions and
prove expired claimants cannot write after reclaim. Purpose-confined specialist
creation should also reject every grant encoding (`slugs`, `ids_by_request` included).

Installation observation/registration race sent to worker: ordinary registration
can read an absent remote installation before `observe_child` commits, then
publish an unmarked bot because the two transactions write different rows.
Use a common identity/registration fence and handle already-connected observed
identities explicitly. Required regression interleaves the transactions rather
than testing only sequential registration after observation.

Primary started an intermediate lean `cargo check -p nyxid --tests` while source
remains in development; log `/tmp/nyxid-botfather-provisioning-intermediate-check.log`.
This cannot be treated as a frozen final checkpoint.

Native inference review: shared HTTP client has only a connect timeout. New
guide calls initially had 120-second concurrency leases but no total deadline,
allowing stalled responses to outlive capacity admission. Require a bounded
lifetime shorter than the lease and final live-turn admission after billing
preparation, with slow-header/chunk and preparation-delay regressions. Per-sender
quota rows also need retention that preserves current spend and live leases.
Daily quota currently holds maximum reservation even on pre-dispatch failure;
worker asked to make refund/settlement policy explicit and test it against exact
billing reservations and actual usage. These are source-review findings pending
correction and runtime proof.

Intermediate compile passed in 2m34s (16 warnings, largely unwired inference).
This overlapped edits and is not pinned evidence. Source now includes 90-second
guide deadline, final live concurrency fence and seven-day quota TTL. Pre-dispatch
reservations refund atomically; dispatched calls retain conservative cap usage.
Remaining review: actual settlement/recovery tests and documented cap semantics.
Provisioning claim now reaches specialist creation; follow-up sent because the
Telegram request insert cloned the claim but initially did not call `fence_step`.

Source fixes observed: Telegram request insertion now calls `fence_step` in its
transaction. `observe_child` now writes the shared channel registration revision
and refuses an already active connection, preventing the installation/registration
write skew. Both still require runtime race evidence.

Reapproval lifecycle finding sent: edits and pause preserve stable resource IDs,
but initial `approve_owned` unconditionally inserts installation/job rows and
starts BeginTelegram. A previously provisioned setup cannot resume or republish
through that path. Require exact reviewed reuse of existing resources with new
approval/authority generations and test edit, pause, binding refresh and profile
generation recovery without new bot identities.

Native guide review targets sent: closed action set must update the captured
current draft with revision fencing, not create another draft for each follow-up.
Multiple setups require explicit current selection. Error/quota replies must
use durable reply throttling and duplicate admission. Detached webhook spawn
currently ACKs before durable processing, and inference turn dedup can suppress
retries before action/reply completion; require recoverable persisted progress
and crash-boundary tests. Final manager credential reload must match the captured
manager identity and observation before sending client-specific output.

Current integration source (not runtime evidence): dedicated native inference
uses `PublicPurpose` to distinguish organization-sponsored guide from client-paid
customer execution; the ordinary platform-key billing resolver is unchanged.
Customer context captures exact installation/setup/binding generations and
checks active/readiness state and webhook registration. Delivery proof captures
installation generation, setup revision, digest, independent numeric sender,
reply message ID and hashed challenge; confirmation must arrive as a reply from
that customer account. Activation, runtime ingress, crash recovery and frontend
integration remain under construction at this review checkpoint.

Reapproval stage matrix sent to worker: installation only; native request
waiting; observed child; child approved but no bot row; registered pending;
connected/active; expired or purged native request. Fresh reviewed authority
must reuse stable resource identities and never silently resurrect old approval.

Guide command review: bare `/start` was consumed by native creation before the
platform guide and offered unreviewed creation. Worker asked to route opted-in
platform welcome through its paused/enabled guide state while preserving valid
creation challenges and legacy manager behavior. Selection follow-up: use the
explicit current setup for ReviewCurrent, and index/expire guide selections.

Customer dedup finding: native reply dedup keyed by profile/sender/update/kind
collides across children because Telegram update IDs are per bot. Require exact
source bot/installation identity and a two-child same-sender/same-update test.

Primary started a second intermediate lean backend check covering the new guide
and customer modules. Log `/tmp/nyxid-botfather-runtime-intermediate-check.log`;
source remains mutable, so this is integration diagnostics, not acceptance proof.

Second intermediate compile completed exit 101: missing ReturnDocument import in
configure_guide and Send lifetime failure at platform_guide's spawned future;
two unused-import warnings. No tests ran. Diagnostics sent to worker.

Additional integration review targets sent: live specialist destruction/ownership
must fence customer runtime, and an owner's ineligible customer test must fail
before paid inference. New guide/customer business orchestration belongs in
services per AGENTS; webhook handlers should delegate after transport/auth checks.

Lifetime review sent: active installations must not silently inherit a draft's
30-day retention deadline. Clarify separate active runtime authority and the
intended binding-expiry/unlink behavior, preserving A11 and showing any required
re-review deadline in UI/status. Test active expiry and fresh reviewed renewal.

Source progress: native guide/customer orchestration moved to services; config
import fixed; guide handle now has an explicit boxed future boundary. New
metadata-only native receipts are persisted before webhook ACK and use leased
generation fences. Follow-up: guide draft/update/selection writes also need the
receipt claim in their own transactions, not only inference and send. Customer
receipt recovery initially returns without a notice; its durable failure status
or fixed recovery reply must be made explicit and tested.

Runtime authority source now separates active owned runtime from temporary
onboarding expiry: activation clears draft expiry, and active runtime uses a
binding revocation epoch rather than its ordinary session-expiry deadline.
Explicit unlink/replacement must advance that epoch; same-account renewal must
not invalidate a still-approved active installation. Verify recovery notices use
the same authority, rather than the older expiring onboarding-binding check.

Model readiness target sent: current selection accepts a syntactically valid ID
but reply extraction requires exact returned model identity. Validate the
configured model contract before enabling and provide a usable model selector
with no personal-service fallback; test unsupported IDs and returned identity.

Primary rechecked the CLI/machine source manifest during runtime integration:
all 154 recorded files remain unchanged from the successful exact-job and shared
protocol tests. Those results still apply; no redundant rerun is needed without
further CLI/machine changes. Backend cleanup tests remain pending on the new
backend checkpoint.

Anonymous multi-draft usability target sent: `/setup` currently selects only for
a bound client, but the guide can create several anonymous drafts and asks for
selection. Either implement exact native-sender selection or explicitly hand
off to linking/adoption; test multiple anonymous drafts and sender isolation.

Third intermediate backend check finished exit 101 with seven E0308 argument
mismatches after the reply helper switched to owned values: coordination calls
in platform_binding_service need borrowed values, and native receipt call sites
need owned arguments. No Send error was reported in this snapshot. No tests ran.
Log `/tmp/nyxid-botfather-native-integration-check.log`; diagnostics sent.

Additional readiness findings sent: model discovery must apply GET/models
operation pricing and protocol-specific headers. Successful discovery must await
its concurrency lease release before sequential model validation starts; relying
on spawned Drop cleanup races the readiness concurrency-one gate. Tests must
prove actual sponsor/client/admin payer isolation with distinct accounts, not
only the shared fixture's operator-as-client case. Shared fixtures should use all
production indexes and real claim paths.

Held for coherent-checkpoint review: customer `TEST <proof>` confirmation currently
returns through `confirm_test` before native receipt/ingress admission. Verify a
durable per-sender/per-installation attempt limit and duplicate handling precede
its repeated authority-write transactions; it must not bypass native admission
simply because it avoids inference. Invalid proofs must not consume model credits.

Other remaining full-flow checks: safe effective status after profile pause or
revocation (stored approved/active is not necessarily current readiness), actionable
provisioning failure/recovery status and notices, account deletion cleanup and
retained non-replayable installation tombstones, complete UI review/activation
flow, and stage-specific reapproval/retention regressions.

### Native UI integration review continuation

Primary reran the four focused frontend page/schema/idempotency files: 11/11
passed in 1.16s. Log `/tmp/nyxid-botfather-ui-integration-review.log`. These are
existing tests; they do not establish new provisioning/activation coverage.

Consolidated feedback sent after initial UI integration: add bounded background
progress refresh; display the captured approval facts/limits/resource effects,
not only the model/bot beside an editable form; clear stale creation launch URLs
on revision/state/account changes. The pending customer proof admission finding
was also sent. Effective readiness, recovery notices, deletion/tombstones,
anonymous adoption and distinct-actor billing remain open.

Primary ran the newly added provisioning component file: 3/3 failed during
render. `launch?.actor === actor` evaluates true when both are undefined, then
dereferences null `launch.revision`. This can occur with missing authentication
state. Sent the exact failure and requested a retained missing-actor regression.
Log `/tmp/nyxid-botfather-primary-provisioning-ui.log`.

After the explicit null guard, primary reran provisioning UI tests: 3/3 passed
(stale child review refusal, captured exact approval submission, activation
disabled before independent receipt). Log
`/tmp/nyxid-botfather-primary-provisioning-ui-2.log`. This is component evidence
only; backend lifecycle and full page account-switch coverage remain pending.

### Integrated native backend checkpoint

Worker confirmed Rust/Cargo freeze and no Cargo process. Primary recorded 1,043
source/manifests in `/tmp/nyxid-botfather-native-checkpoint.json`, aggregate
`cf3ba2eba823d6e36fbccb1fa7ed06da04437adadcc6d33e5c07d40cddc31b71`.
The nextest build/run is in progress in
`/tmp/nyxid-botfather-native-checkpoint-tests.log`, filtering platform_onboarding
plus inference/runtime/native-receipt/setup-notice modules. This checkpoint adds
source beyond the last passed typecheck; no runtime success claimed yet.

Follow-up proof admission review: `confirm_test` now has durable proof rate limits
and native receipt dedup. However `capture` (including authority-write transaction)
precedes rate admission, and receipt is completed before confirmation transaction.
Check bounded pre-capture ingress and crash recovery between completion and proof
commit. These are pending review targets, not yet verified fixes.

Frozen native checkpoint compiled in 6m00s, nextest run
`66dda8ba-fd46-4f32-8a65-2967c6c95f4f`: 27 tests, 25 passed (one LEAK),
2 failed. Failures: conversion cleanup remains incomplete and stale-cleanup
regression fails at complete_cleanup. Personal source inspection identifies
raw `platform_cleanup_pending/id` being discarded by typed conversation
replace_one in finish_turn_inner (~2618). Sent request to persist markers and
keep the exact completion fence; do not weaken the test. LEAK reported for
legacy_manager_is_opt_in_and_tombstone_is_closed also needs cleanup.

Same frozen binary compatibility: routing 13/13, channelbot 18/18, Telegram
68/68 passed. Logs `/tmp/nyxid-botfather-native-{routing,channelbot,telegram}-compat.log`.
Checkpoint files unchanged after suites. Backend edits released for fixes.

Handler fault review sent: manager and managed-child dispatch errors currently
ACK 200 even if receipt admission failed on a transient DB error. Ordinary
manager-channel lookup also swallows errors. Require retryable pre-admission
infrastructure failures to preserve Telegram redelivery; intentional closed or
invalid inputs may ACK. Verify handler-level failed admission followed by one
redelivery, and no inference replay after a persisted receipt.

Two further lifecycle findings sent: final guide-configuration commit must
revalidate the actor's live admin authority after potentially long model readiness;
and invalidate_setups currently filters only current binding_generation even for
explicit revoke. A preserved active g1 installation followed by renewal g2 then
unlink is execution-blocked by revocation epoch but remains stored active/approved.
Explicit revoke/replacement must invalidate affected older generations as well,
while same-account renewal continues preserving active installations. Add runtime
and projected-status regressions for the complete sequence.

Guide mutation review sent: anonymous guide_update_current must independently
compare the captured draft scope to the verified sender's HMAC scope. Current
selection is scoped, but the mutation cannot rely on a caller-provided capture
when both owners/bindings are None. Existing native draft idempotency results
must also match current owner/binding/profile after renewal or replacement.
Anonymous ReviewCurrent should direct linking/adoption, not an owner-only detail
page that cannot display the unowned draft.

Admin picker review: new eligible catalog listing batches OwnerGrants and provider
snapshots and uses available_with_grants. Candidate discovery works before a saved
model. Initial configuration flow tests requested. Primary inspected mobile
provisioning screenshot; layout fits. Worker subsequently reports mocked refresh
and pause coverage; final independent UI verification remains pending.

Worker cleanup/proof rerun compilation failed before tests: E0532 tuple variants
ChannelBotNotFound/ChannelBotInactive in native_dispatch_status, plus unused mut
warning. Diagnostics sent. Log `/tmp/nyxid-botfather-cleanup-proof-runtime.log`.
The earlier nextest LEAK report's underlying cause is not yet established; do not
label it a production memory leak.

Primary frontend lifecycle run passed 20/20 tests across six current files
(admin, provisioning, anonymous adoption, public onboarding, schema and request
idempotency). Log `/tmp/nyxid-botfather-primary-ui-lifecycle.log`, duration6.08s.
This is component/schema evidence; no backend/provider result is inferred.
The worker's corrected backend build is now running in
`/tmp/nyxid-botfather-cleanup-proof-runtime-2.log`.

Primary frontend TypeScript build also passed exit0; log
`/tmp/nyxid-botfather-primary-ui-lifecycle-typecheck.log` (empty diagnostics).
The 20-test run includes the newly added empty-guide bootstrap through named
service discovery, paused save and explicit validation.

Worker corrected runtime2 passed9/9, build9m22s, run10.418s, nextest
`4be36e99-6416-41ca-b72b-84dbaace7ab1`; no LEAK recurrence. Fix coverage:
cleanup persistence/exact completion, final admin fence, atomic proof dedup,
renewal then unlink/replacement, anonymous scope/replay/review handoff.

Primary used the same binary (SHA recorded and rechecked at
`/tmp/nyxid-botfather-cleanup-proof-binary.sha256`): existing personal-conversation
engine40/40 passed; full platform_onboarding32 ran31passed/1failed. Failure:
`platform_onboarding_native_command_errors_reply_safely_and_provider_failure_is_acked`
expects200 for failed delivery of an intentional invalid-command error response;
new classifier yields503. Requested distinction from transient pre-admission
DB/infrastructure failure, plus handler redelivery tests. Logs:
`/tmp/nyxid-botfather-primary-cleanup-conversation-compat.log` and
`/tmp/nyxid-botfather-primary-cleanup-platform-regression.log`. Primary processes
complete; backend edits/builds released again.

Upcoming billing-phase finding sent: native TurnGuard Drop only spawns release
and discards release errors. No durable abandoned-turn budget sweep is evident.
A crash/DB outage can strand funded pre-forward spent after its lease expires.
Require bounded recovery of stale admitted/funded turns, one-time refunds,
conservative retention for dispatch_admitted/uncertain calls, and stale-worker
fences. Test failed cleanup/process loss, duplicate recovery, UTC rollover and
never refunding possibly forwarded work.

Retirement integration findings sent: unconditional new transactions affect
ordinary no-platform account disable/delete; retirement runs before established
session/key revocation and can short-circuit it; sweep repeatedly selects already
paused inactive owners with limit100, risking starvation. Requested indexed
no-platform fast path, preserved core revocation, and durable unfinished recovery.

Primary established standalone baseline using a separate task-owned Mongo on
127.0.0.1:27120 (PID94859, path pointer
`/tmp/nyxid-botfather-standalone-path`), with the cleanup-proof2 binary before the
new retirement integration: ordinary disable/revoke1/1 and delete cascade1/1
passed,3.14s each, no skipped fixture. Logs
`/tmp/nyxid-botfather-standalone-disable-baseline.log` and
`/tmp/nyxid-botfather-standalone-delete-baseline.log`. Rerun after the fix to prove
this supported legacy path remains intact. Replica set27119 untouched.

Latest lifecycle source review: full serialized DownstreamService equality in
readiness includes defaulted fields absent on legacy rows (topics, service type,
streaming flags) and credential ciphertext. Requested secret-free transaction
projection/revision fence, legacy missing-default and concurrent mutation tests.
Retirement recovery still selects cancelled/expired setups that disabled-retirement
updates exclude, potentially starving later owners; requested aligned terminal
progress and >100 terminal-owner regression. Effective runtime notice/UI must
reflect child/manager readiness and use an unavailable label when stored Active
cannot execute. Funded payer tests need exact durable charge/ledger evidence in
addition to usage-row attribution. All findings sent to sole implementation worker.

Primary independently reran six frontend files after effective status changes:
20/20 passed,1.84s, `/tmp/nyxid-botfather-primary-effective-ui.log`. Full
frontend/src manifest remained unchanged throughout this run
(`/tmp/nyxid-botfather-primary-effective-ui-source.json`). Source now shares
assess_owned_runtime between active admission and status, preserving transactional
write fences; exact charge/ledger/wallet assertions added to payer tests; terminal
retirement progress regression now places100 cancelled owners ahead of live work.
Backend runtime verification remains pending for these fixes.

Lifecycle funded checkpoint:1048 files, source manifest
`/tmp/nyxid-botfather-lifecycle-funded-source.json`, aggregate
`f936746014cd4eeef0b0cb4f1eb45f0abb912d8aedd7ebfeafaa60af5b234c6b`.
Worker broad nextest12003b37-f4e7-437c-b87d-02869aa84e56: build6m22s;
fail-fast24/46 executed,22pass/2fail,22unrun. Failures: Decimal128 zero
encoding assertion in abandoned budget; provider-not-reached readiness race timeout.
Requested complete no-fail-fast feedback before edits.
Primary same-binary checks after retirement integration: standalone disable1/1,
delete1/1 on27120; billing route_context8/8 and reservation5/5; personal
conversation40/40; routing13/13; channelbot18/18. Telegram67/68 with a real
legacy ACK regression: corrupt ordinary manager channel.label now produces500
instead of200 (telegram_manager_e2e_tests.rs689). Sent for correction while
retaining retryable transient platform admission failures. Logs use
`/tmp/nyxid-botfather-lifecycle-{billing-context,billing-reservation,personal-conversation,routing,channelbot,telegram}.log`;
standalone logs `/tmp/nyxid-botfather-standalone-{disable,delete}-after.log`.
Source manifest and `/tmp/nyxid-botfather-lifecycle-binary.sha256` unchanged
through primary tests. All primary test sessions complete; worker may release
freeze after collecting remaining platform test outcomes.

Environment correction: full non-fail-fast primary run on27119 finished36pass/8fail
(`/tmp/nyxid-botfather-primary-lifecycle-all-failures.log`). Funded failures exposed
Mongo7.0.37/wire21, which supports transactions but not production client bulkWrite.
Primary should have verified major version earlier. No production logic/test bypass
was introduced. Downloaded official macOS arm64 Mongo8.0.16 tarball and verified
published SHA256; unpacked in task-only temp path, no Homebrew/global changes.
New primary-owned replica set: mongodb://127.0.0.1:27121/?replicaSet=nyxidBotfather8,
PID1868, path pointer `/tmp/nyxid-botfather-mongo8-path`. Version8.0.16/wire25,
transaction insert+abort and client bulkWrite probes passed. Official installation
reference: https://www.mongodb.com/docs/v8.0/tutorial/install-mongodb-on-os-x-tarball/ .
All future funded runtime runs use27121. Old7.x transaction evidence remains
valid for non-bulk paths; unchanged-binary full platform rerun on8.0 in progress
at `/tmp/nyxid-botfather-primary-lifecycle-mongo8.log`.

Same-binary Mongo8 run completed38/44pass,6fail,28.15s
(`/tmp/nyxid-botfather-primary-lifecycle-mongo8.log`). Readiness role-demotion
and destination-change tests now pass. Remaining: two Decimal128 zero byte-level
comparisons; three deferred-settlement assertions before production worker drain;
active customer capture conflicts immediately after successful activation
(platform_inference_service.rs1428). Sent root-cause directions, preserving exact
value assertions and production deferred billing. Legacy malformed-channel ACK
regression remains separate. All primary processes done; freeze released for fixes.

Activation root cause confirmed in source: channel_bot_service registration
consumes the managed creation record with retired=true (line776). New assessment
incorrectly required retired!=true. Worker fix requires consumed true plus exact
manager observation/revision, owned live channel/specialist and existing setup
authority; it does not remove those fences. New funded helpers explicitly require
Mongo8 and run production recover_retryable_settlements before ledger assertions;
semantic monetary comparison still requires Decimal128 storage and exact Credits.
Runtime validation of these fixes remains pending.

Primary browser validation of mounted gateway recovery UI passed1440/390:
inspection required; preserve-changed resolution disabled until checkbox; request
binds profilegeneration4/captured7/observed8/action; completed task disappears;
zero page errors/overflow. Mocked API only, not gateway/server authorization proof.
Script/log `/tmp/nyxid-botfather-primary-gateway-browser.mjs`/`.log`, screenshots
`/tmp/nyxid-botfather-primary-gateway-{1440,390}.png`; mobile visually inspected.
Reviewed API source final admin/profile/task fences and explicit remote reinspection.
Sent small singular-credit copy correction with results.

Primary full frontend `npm run build` passed exit0 with recovery controls mounted:
`/tmp/nyxid-botfather-primary-frontend-build.log`. Main and credential-accept
bundles plus mock scenario footprint assertion passed. Existing assistant chunk
size warnings remain outside this feature; no platform bundle error. Build outputs
are local only. Primary process61337 complete.

Primary changed-scope ESLint22files exits0: no errors;11 warnings in existing
sidebar/layout/command-palette export/effect code. Reviewed diffs: additions are
navigation entries, admin visibility exclusion and onboarding route handling;
warning-producing exports/effect are unchanged baseline. No platform-file warnings.
Log `/tmp/nyxid-botfather-primary-eslint.log`. Rust fmt check currently fails only
new customer-send assertion wrapping; worker notified to fix after runtime freeze.
Log `/tmp/nyxid-botfather-primary-gateway-format.log`.

Gateway/token frozen runtime completed50 tests:48pass,2fail,57.801s; compile6m02s,
nextest0655019a-64f6-43bf-8ab5-05a1a940d346. Gateway3, token-priced admission,
abandoned/refund, model discovery, readiness races and legacy invalid command
notice pass. Customer expanded future SIGABRT stack overflow; guide exact checkpoint
assertion omits opening10000 balance (actual9999.999 vs expected-0.001). Sent fixes
without raising stack or weakening monetary equality. Runtime log
`/tmp/nyxid-botfather-gateway-token-runtime.log`.
Primary same-binary full Telegram suite68/68pass42.19s Mongo8, including formerly
failing malformed ordinary-manager lookup ACK.
`/tmp/nyxid-botfather-primary-gateway-telegram.log`. Primary1050file source manifest
unchanged; freeze released after primary finished. Remaining local acceptance
cases still required before completion.

New source-review finding sent: guide inference captures service/target before
billing preparation, then calls selected_service again but discards the returned
service before dispatching the original target. A concurrent eligible base URL/auth
configuration change can therefore leave a stale captured target in use. Requested
secret-free captured/live projection comparison and equivalent model-discovery
check, plus barriered preparation mutation test proving no stale provider dispatch.
Runtime exploit/reproduction not yet claimed; pending worker investigation/fix.

Primary navigation compatibility: existing sidebar/public-path/router suites31/31
pass1.82s (`/tmp/nyxid-botfather-primary-navigation-tests.log`). Source review
confirms new public exceptions are exact /onboarding and /onboarding/link only;
setup pages remain under authenticated dashboard; admin route and page retain
admin-write/live actor guards. This suite is compatibility evidence, not additional
backend permission proof.

Reviewed prepared-target correction: both text and models paths revalidate after
preparation; captured/live public projection, in-memory encrypted credential and
default headers, and effective auth must match. Shared master resolver uses the
service credential itself. Barriered FakeLago wallet-preparation test mutates URL
(text) or encrypted credential (models), requires no provider call/no forwarded
usage. Runtime pending. Reviewed new customer webhook helper: production wrapper
fixes Telegram endpoint; injected endpoint remains Rust-only test seam. Missing
webhook secret ->401; injected admission DB failure ->503; duplicate valid update
checked against exact charges. Sent correction for stale final2-vs3 count and
requested explicit provider-call count so dispatch dedup is directly evidenced.

New registration-race test review: initial managed_bot webhook itself calls
observe_child, apparently completing the intended race before spawning manual
registration. Sent correction to establish waiting_bot/unclaimed installation
preconditions and race only after event provenance is present. Pending worker
correction/runtime; no race coverage pass claimed.

Registration race fixture corrected: seeds ManagedBotEvents only, asserts current
request waiting_bot/no Telegram ID and installation unclaimed/no active child row,
then releases competing observation/manual registration from shared DB fence.
Asserts exactly one winner and matching persisted resources. Runtime pending.

Compatibility candidate sent for assessment (not yet confirmed supported-use
regression): ordinary bot route update/delete now use transactions, unlike baseline
updates. Bot route creation already required transactions and ENV default is a
replica set. Asked worker to establish support contract and preserve supported
permission reductions; no blanket transaction-error fallback authorized.

Signed/preparation frozen runtime:57run56pass1fail, build13m49s/run56.107s.
Stack overflow and funded checkpoint errors are corrected. Gateway/recovery,
preparation URL+credential drift, empty wallet, concurrency/budget, slow response
pause, group fixed handoff, registration race and notice claim/unlink pass.
Failure customer fixture expected503 got200 at2022. Primary source diagnosis:
from.id800 omits is_bot:false; private_sender requires exact false and deliberately
ACKs ignored messages. Sent realistic fixture+precondition assertion, preserving
strict production parser. Customer full signed delivery/recovery remains unproven
until corrected test completes. Log `/tmp/nyxid-botfather-signed-preparation-runtime.log`.
Primary same-binary auth/invitation27/27pass1.00s Mongo8
(`/tmp/nyxid-botfather-primary-signed-auth.log`); this verifies existing delegated
policy and global invitation behavior, not the still-missing complete A2 journey.

Primary same-binary Telegram68/68pass37.10s Mongo8,
`/tmp/nyxid-botfather-primary-signed-telegram.log`. Frozen1082file manifest
unchanged through checks. All primary processes complete; worker freeze released
for fixture correction and remaining acceptance work.

Closed standalone-route compatibility candidate after primary verification:
baseline HEAD backend/src/db.rs90 already calls require_transactions before
indexes/migrations; docs/SERVICE_HISTORY.md64 explicitly rejects standalone
startup. New transactional route-update/delete fencing adds no deployment
prerequisite for supported NyxID servers. Direct library calls on standalone
do change; no unsupported transaction fallback added. Existing standalone
account-disable/delete utility behavior was independently preserved.

Primary cleanup during continued work: gracefully stopped obsolete task-owned
Mongo7 replica27119/PID70903 and standalone27120/PID94859 after checking exact
dbpaths; both exited. Kept logs/data directories. Mongo8 replica27121/PID1868
remains active for all remaining tests; user database27017 untouched. Worker
notified. Latest runtime binary checksum remains05856552ecf233f23d78c0abb970873f1abf5dd7f11dbbb069df4135f0f2b30d.

Correction to earlier registration-race diagnosis: full dispatch distinguishes
top-level managed_bot metadata (returns before observe_child) from private
message.managed_bot_created. The original managed_bot-only fixture lacked
created_by/observation/created_at provenance; it did NOT pre-complete observation.
Primary's earlier suspected cause was incorrect. Explicit provenance seed and
waiting_bot/nochild preconditions remain valid, and corrected race passed in
57test run. Worker notified to correct evidence wording; no production change
is needed for this diagnostic correction.

User-authorized parallel verification (2026-10-14): user explicitly requested
two GPT-6.1-Sol subagents for independent remaining issues. Both were started
through Heca with xhigh effort/full access in this exact worktree; no Loose
placement. Provisioning worker 01a13893-aeb5-74b1-af35-f9f583f66bd2 owns
platform_provisioning_service.rs and its new dedicated acceptance test file
(A7/A14/A16). Lifecycle worker 01a13893-af4d-73b0-8426-ac87625b3ceb owns a new
dedicated lifecycle acceptance file (A12/A15); shared production edits require
coordination. Existing implementation worker retains inference/setup/signup,
shared support and module registration. Cargo/builds are serialized through that
worker; no concurrent global formatting. Primary remains sole final reviewer.
No new runtime result or deployment is implied by delegation.

Primary reviewed the new second-client fixture before execution: required
restaurant public hours/address were initially omitted; worker notified. The
getManagedBotToken user_id matcher and original customer is_bot:false fixes
are present. New admin poison fixture uses syntactically valid closed-action
JSON and a real encrypted private connector credential; signup journey invokes
real registration/login and mounted authenticated setup routes. These cases
remain pending execution and do not yet close acceptance items.

Provisioning/lifecycle worker handoff: four tests each written, not compiled or
runtime-proven. Primary inspected execute_step saved-provisioning resume change:
ready|provisioning status guard, matched-count stale rejection, retry marker
clear and connection-generation increment. Main notified to fix finish_request
unconditional installation ready_for_test mutation on an active native retry.
Primary additionally identified analogous execute_step Connected branch: its
fence permits Active but writes setup and installation ready_for_test without
a provisioning-state guard. Sent provisioning worker a follow-up to prove
same-live-claim replay after activation before finish_step and fix if confirmed.
Do not close either idempotence finding until combined runtime verifies it.
Lifecycle tests explicitly use synthetic lifecycle states for some fences; they
do not prove the real activation ceremony. Primary retains final review.

Primary source review of replay fix completed: execute_step obtains the fenced
setup from fence_request and transitions both setup/installation only when
Provisioning, requiring both state-guarded updates to match inside the transaction.
ReadyForTest/Active are preserved. Regression proves original running claim and
generation are still live using Mongo $$NOW before replay, then checks both
states plus setup revision/authority generation and installation generation/
authority unchanged. Native finish_request now transitions installation/queues
notice only after actual provisioning setup transition, with installation
transition mismatch aborting transaction. Both acceptance modules are registered.
Source review agrees with intended semantics; runtime remains pending. Main
worker notified to include both regression paths in frozen combined run.

## Compatibility pass before merge (2026-10-14)

CI first completed with 23 failures once the backend swap reserve was raised.
An independent breaking-change review and a GPT-6-Astra sign-off review found
the following; each is fixed with a regression or an existing test:

- Channel-bot ownership transfer always returned 409: the route fence bumped
  `route_revision` before `inspect()` hashed the bot. The fence now follows the
  version check; a missing bot keeps its 404 (`refuse_platform_bot`).
- Route deletion refused orphaned, transferred and already inactive routes,
  breaking account deletion and NyxBot disconnect. Deleting an inactive route is
  a no-op; orphaned/transferred routes keep main's soft-delete; only active
  routes on platform bots stay refused.
- Narrowing `private_chats` to `owner` keeps main's behaviour; only widening is
  fenced.
- Route creation validates its key before platform checks, restoring main's
  error precedence.
- Manager conversion refuses route keys shared with other bots instead of
  revoking them.
- Admin Platform Credentials require a first-party human only for a converted
  Telegram manager; other providers keep existing admin authentication.
- Remote Telegram IDs match installations only for Telegram bots.
- Cancelling a setup releases its native creation request (also on read).
- Test fixes: NyxBot fixture seeds the linked bot row (R11 fails closed),
  signup test uses the kebab-case template, the distinct-customer journey runs on
  a 16 MiB stack, and the signup password is generated at runtime (CodeQL).

Astra convergence round (same day) found three remaining concurrency and
retention gaps, now closed:

- Route creation, key rebinding and re-enabling write-fence the agent key in the
  route transaction (`route_binding_revision`), so they conflict with conversion's
  key revocation instead of leaving an active route on a revoked key
  (`platform_onboarding_route_creation_conflicts_with_concurrent_key_revocation`).
- A purged setup counts as terminal; the setup expiry sweep releases native
  requests of cancelled, expired or purged setups.
- The first-party requirement for a converted manager is enforced inside the
  manager operation (`configure_manager_for`), closing the race with the first
  conversion.
- Re-enabling a route without rebinding pins the fenced key in the update
  predicate, so a stale reactivation cannot enable a route that was concurrently
  rebound to a revoked key
  (`platform_onboarding_stale_reactivation_cannot_enable_a_rebound_revoked_key`).
