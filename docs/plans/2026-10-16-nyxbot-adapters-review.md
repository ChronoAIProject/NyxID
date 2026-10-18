# NyxBot adapters: implementation review

Opened: 2026-10-16. Final review: 2026-10-17. Status: implementation and local
acceptance complete for the documented supported profiles. R1–R23 are resolved.
PR integration validation is in progress; the CI findings below remain open.
The separate credential-exposure incident below remains open pending rotation.

The primary agent reviews the implementation personally. There is one Heca
implementation worker and no delegated reviewer. The worker's effective Codex
turn context was checked: `gpt-6.1-sol`, `xhigh`, `danger-full-access`, approval
policy `never`, in the `wild-sage` worktree on `telnyx-nyxbot-integration`.
Heca agent: `01a14557-28ff-7800-ab15-3a7dd1c01124`.

The design is in `2026-10-16-nyxbot-service-adapters.md`. The implementer maintains
`2026-10-16-nyxbot-adapters-implementation.md` with changes and check results.

## Acceptance coverage

The primary reviewed the implementation and final corrections personally. All
items below passed within the supported profiles in the operator guide. Provider
fixtures establish local protocol behavior, not live account eligibility.

- [x] Compiled, typed, versioned capabilities derive from the executable registry.
- [x] Existing channel discovery and implementations remain compatible.
- [x] Binding availability uses current credentials, resource configuration,
      grants, account support and billing readiness; execution rechecks access.
- [x] Telnyx SMS and Conversation Relay work through the existing NyxBot engine.
- [x] Twilio SMS and ConversationRelay exercise the same shared coordinator.
- [x] One usable ElevenLabs integration has an explicit carrier composition.
- [x] Owner setup, discovery, configuration and troubleshooting are usable.
- [x] Notion FAQ reads execute through the existing authorized tool path, and
      subsequent reads observe fixture edits during the same call.
- [x] Trusted instruction updates take effect between turns without mutating an
      active turn's snapshot or granting permissions.
- [x] Public callers cannot inherit owner authority, private history or memory.
- [x] Signatures, account/resource binding, cross-tenant isolation, bounded
      ingress and replay prevention are enforced before effects.
- [x] Durable session claims and binding revisions fence concurrent/stale workers.
- [x] Interruption prevents stale output and never repeats an ambiguous effect.
- [x] Revocation, socket loss, process loss and closure have tested cleanup.
- [x] Usage evidence, configured platform prices, reservations and settlement are
      correct for each supported composition and credential mode.
- [x] Credentials, webhook bodies and raw audio stay out of relay diagnostics.
- [x] Tests exercise the actual wiring, including failure paths, rather than only
      parsers and capability declarations.
- [x] Relevant Rust/frontend checks pass; final diff and docs match behavior.

## Findings

All 23 implementation findings are resolved. The descriptions below preserve
the original findings and requested evidence; their imperative wording records
review history. The closure table and final validation supersede the earlier
progress notes.

- **R1 — resolved, capability accuracy.** The initial
  `services/channel_capability.rs::describe` unconditionally advertises inbound
  and outbound text plus `SendMessage` for every legacy adapter. OpenClaw's
  registered channel adapter explicitly refuses outbound transport. Derive
  these capabilities from executable hooks or explicit adapter declarations,
  and cover disabled/unsupported adapters. Unknown limits must be represented
  explicitly rather than an unexplained zero. Recheck final code and tests.
- **R2 — resolved, executable interfaces.** The initial
  `call_channels/profiles.rs::Adapter` registers five `PlatformAdapter` objects
  whose ingress/effect methods all return unsupported, while a separate enum
  descriptor claims messaging/voice support. Use typed executable interfaces or
  registry variants that the coordinator actually invokes. Keep existing chat
  registry callers compatible without representing a call interface as an inert
  chat implementation. Verify capability-to-handler contract coverage.
- **R3 — resolved, platform webhook authority.** Initial `binding::create` accepts
  a tenant-supplied Telnyx webhook public key even for a platform credential,
  without proving its account association. A tenant can sign an event naming
  its allocated number/application but another tenant's call-control ID, which
  shared provider credentials could then act upon. Platform verification keys
  must come from trusted account configuration or an authenticated documented
  provider lookup, and call resource correlation must be enforced. Test forged
  ingress and prove that no cross-resource provider command is dispatched.
- **R4 — resolved, catalog execution policy.** Initial credential resolution and
  direct provider HTTP helpers do not apply the selected service's
  `proxy_operation_policy`. Resolve an exact canonical method/path and call the
  shared policy guard before provider operations, including live revalidation.
  Carrier transport authorization remains the owner's explicit binding; this
  must not require broadening the FAQ specialist's tool grants. Test a catalog
  policy that permits resource reads but refuses SMS/call effects.

- **R5 — resolved, live session authority.** `session::fence` initially compares
  the stored session with its snapshot but does not compare the current binding
  revision or credential identity with the session. Require live binding/session
  revision and identity agreement, plus valid claim/lease/state, before effects.
  Test a binding change after utterance admission and before egress.
- **R6 — resolved, trusted-account rotation.** `binding::live` initially does not
  revalidate the platform account trust row/key. Rotating or deleting that trust
  must revoke old webhook authority, with a test proving rejection before effects.
- **R7 — resolved, diagnostic redaction.** Derived `Debug` on Binding and CallSession
  exposes credential identity strings containing API-key identifiers. Redact those
  fields and cover the intended diagnostic contract.
- **R8 — resolved, provider redirects.** Carrier, resource verification and SMS use
  the shared HTTP client, which follows redirects. Use the existing no-redirect
  credential transport and test rejection before a redirected provider effect.
- **R9 — resolved, Telnyx membership pagination.** `prove_call` checks only the first
  active-call page. The official API is cursor-paginated; later-page callers must
  work within a bounded scan and a nonmember must fail before any command.
- **R10 — resolved, settlement identity.** Call close and SMS receipts reopen the
  original reserved billing ID. The meter rejects that duplicate when funded.
  Finalize the durable existing meter with its admitted rates/funding, including
  after credentials, allocation or current pricing becomes unavailable.
  Follow-up coverage includes frozen terminal duration, closed-but-unsettled
  recovery, multipart unused-hold release, receipt-before-acceptance ordering,
  and delivery status updates after billing has already finalized.
- **R11 — resolved, carrier closure and revocation.** Relay-ended and carrier-hangup
  events are initially conflated. A relay end cannot establish that carrier
  charging stopped. Live binding checks also block cleanup after revocation.
  Separate lifecycle evidence, preserve bounded cleanup, and enforce a provider
  time limit to bound liability through process/database outages.
- **R12 — resolved, SMS answer integrity.** `one_sms` silently truncates answers to
  70 UTF-16 units. Preserve complete bounded replies with correct part admission
  and settlement or explicit overflow behavior; never silently cut FAQ content.
- **R13 — resolved, interruption evidence.** The coordinator ignores provider heard
  prefix/duration while advertising interrupted-prefix evidence. Keep generated
  text separate from heard evidence, and prevent subsequent turns from treating
  the discarded suffix as something already heard.
- **R14 — resolved, utterance ordering.** A second final utterance during a live turn
  currently closes the entire call with AssistantTurnActive. Add bounded ordering
  or intentional interruption semantics and test this ordinary caller behavior.
- **R15 — resolved, callback authority.** NyxAgent egress fencing alone does not fence
  later tool callbacks through ChatAuthority/AssistantTurnFence. A binding can
  change after turn dispatch and before a tool effect. Bind live adapter session
  authority into the ordinary callback fence, without adding reads to unrelated
  turns, and exercise this race with an actual authorized tool callback.
  Follow-up: the first `fence_tool` queries a nonexistent UserApiKey `is_active`
  field (`status` is authoritative), which rejects valid BYOK callbacks; it also
  omits live org access and platform credential identity checks. The shared
  authority contract must remain consistent across admission, egress and tools.
- **R16 — resolved, owner lifecycle.** New binding/session/event/allocation records
  need account, agent and conversation deletion behavior. Stop execution before
  removal; retain only necessary cleanup/usage evidence until finalization, then
  purge owner data. Test deletion with an active call and pending settlement.
- **R17 — resolved, Custom LLM request identity.** ElevenLabs receipt keys initially
  combine binding/revision with transcript length and final user text, omitting
  session identity. Two callers asking the same question collide. Namespace by
  durable session and stable request evidence; test cross-call identical text,
  duplicate requests and provider history truncation.
- **R18 — resolved, stream outcomes and disconnect.** Broadcast lag/error currently
  becomes a successful final frame; failed/unconfirmed engine settlement is not
  checked before Custom LLM `finish_reason=stop`. Propagate actual outcomes.
  Watch downstream closure while waiting for model/tool output, cancel promptly,
  and verify failure/lag/disconnect through the HTTP stream.
- **R19 — resolved, socket claim loser cleanup.** Two upgrades can authenticate before
  the claim. The loser currently invokes unconditional cleanup and hangs up the
  winner's call. Only the winning claim may stop its call; test concurrent upgrade.
- **R20 — resolved, ticket logging.** Socket tickets are bearer query parameters and
  the global default TraceLayer includes full request URIs. Use redacted/path-only
  spans and prove authentication material is absent from captured diagnostics.
- **R21 — resolved, final NyxAgent dispatch fence.** The channel egress path initially
  checks the live turn, stop flag, bound credential and guest partition. After
  policy and billing awaits, it rechecks only the call session. Reuse the full
  turn/credential authority at the final dispatch boundary, and test cancellation
  or credential replacement during admission so no stopped turn reaches dispatch.
- **R22 — resolved, readiness policy fidelity.** `operation_paths` probes invented
  call IDs, while Twilio ReceiveCall has no policy prerequisites. A probe cannot
  prove a later resource-specific permission, and inbound Twilio uses both a call
  read and TimeLimit write. Model unresolved resource policy honestly and test
  discovery against exact runtime prerequisites.
- **R23 — resolved, frontend discovery contract.** The initial frontend descriptor
  schema requires a documentation URL, while the real registry includes disabled
  OpenClaw with no documentation (`describe` emits an empty string). Parsing the
  whole catalog fails before any profile filtering. Test the full backend catalog
  against its consumer schema, including legacy/disabled entries, and preserve
  the planned forward compatibility for unknown additive operation values.

Historical follow-ups to R11 and R17 (resolved): Twilio failed/busy/unanswered terminal records
may lack `end_time` and `duration`, so settlement must handle authenticated
zero-usage terminal states. Verify provider support for setting TimeLimit before
answering an inbound call. Custom LLM currently hashes only final user text;
session scoping alone still collides on a later legitimate repeated utterance.
Cover retry, repeated text and history trimming through the HTTP endpoint.

Historical R16/R10 lifecycle follow-up (resolved): successful SMS sets only `state=closed`,
leaving no terminal timestamp and `billing_finalized=false`; finalization then
returns without progress, permanently blocking deletion. Failed SMS admission
and expired Custom LLM sessions are also ignored by the relay-only sweep. Require
profile-specific finalization without manufacturing call meters for noncalls.
The final SMS dispatch must also recheck session, credential and operation policy
after its per-part billing awaits (R4/R5).

R12 provider-limit follow-up: Twilio documents 66 UCS-2 units per multipart
segment for US/Canada toll-free messaging (152 GSM-7), so a universal divisor of
67 under-reserves some valid messages. Its current SMS limit table lists 700
concatenated UCS-2 characters. Test provider-specific limits and preserve explicit
overflow behavior. Source: https://www.twilio.com/docs/glossary/what-sms-character-limit.

R10 pricing follow-up: adapter validation accepts operation-priced lanes while
its billing context never selects an operation. Honor the applicable authored
price or reject unsupported lane configurations explicitly; never silently use
the base rate. Require funded-meter tests, not only billing-disabled fixtures.
R16 setup follow-up: disabling preserves unique resource rows/claims, which
currently prevents correcting a binding or reassigning a revoked allocation.
Provide safe explicit management after active work and pending usage settle.

## Validation boundary

Provider capability finding: Twilio documents a REST `TimeLimit` update;
Telnyx's inbound Answer and Conversation Relay APIs do not expose an equivalent
call-leg limit. The implementation gates platform Telnyx voice with an actionable
unavailable reason while retaining BYOK Telnyx voice and platform SMS. The user
has been informed. Discovery and UI must reflect this; no platform Telnyx voice
completion claim is acceptable.

Local controlled provider fixtures are in scope. Live provider calls, SMS,
account changes, deployment and publishing have not been performed. Record
exact verification evidence; do not label a fixture result as live validation.

An earlier broad worker configuration read exposed a developer-platform bearer
credential in local tool logs. The primary activity read repeated that log before
the issue was caught. The user was notified and broad configuration reads were
stopped; subsequent inspection projects non-secret settings only. No evidence of
external use was observed. Credential rotation has not been performed because it
affects a separate connected service. Do not include credential material here or
claim this process issue is resolved.

## Historical implementation and validation notes

These entries preserve the progression of the review. Statements about open
work, running processes and pending validation describe their original
checkpoints; the final validation and cleanup record below is authoritative.

### Initial local test setup

Frontend dependencies were installed with `npm ci --no-audit --no-fund`.
An isolated MongoDB replica set, `nyxbotAdaptersTest`, is listening on loopback
port 27116. Its primary status was verified. Tests should set
`NYXID_TEST_DATABASE_URL=mongodb://127.0.0.1:27116/?replicaSet=nyxbotAdaptersTest&directConnection=true`.
The existing server on port 27017 is standalone and is not suitable for these
transaction tests. Test process files are under
`/tmp/nyxbot-adapters-mongo-20261016`; stop that test server after validation.

Existing frontend channel setup coverage passed: six files, 122 tests (pages,
setup/channel schemas, channel-platform library, managed and managed-OAuth
hooks). This was a baseline before new frontend implementation. An early
`cargo check -p nyxid --locked` warmed dependencies but overlapped the addition
of call modules and failed on six not-yet-created modules; it is not a final
build result or a baseline regression.

An interim `cargo check -p nyxid --locked` passed in 2m11s before the newest
TurnAuthority changes. It emitted 106 warnings, primarily unreferenced adapter
code awaiting route wiring. This establishes an intermediate compile checkpoint,
not final validation. `scripts/check-rci-backend-boundary.sh` also passed during
implementation.

A second primary check was cancelled while waiting for the shared target lock,
after confirming the implementer's check was already compiling in this worktree.
It produced no compile result. The worker check logs to
`/tmp/nyxbot-adapter-check4.log`. Disk availability fell to about 1.6 GiB while
other worktrees were building; their processes and artifacts were left untouched.
`git diff --check` passed at this intermediate point.

When free space fell to about 250 MiB, the primary reclaimed one completed
incremental compiler cache, `nyxid_server-2iq48us2tnwzc`, in the shared target.
It was over 16 hours old, no active rustc referenced it, and every session lock
was acquired exclusively/nonblocking before removal. Two candidates containing
`-working` entries were skipped. Source, executables, dependency outputs and
active compilation caches were preserved. Free space returned to about 6–7 GiB.

The implementer's fourth `cargo check` passed in 13m43s with five warnings;
the primary inspected `/tmp/nyxbot-adapter-check4.log`. The primary reran
`scripts/check-rci-backend-boundary.sh` successfully after the authority changes.
These are intermediate results, before the acceptance tests and final UI work.

The primary's `npx tsc -b --pretty false` passed for the initial owner setup UI
(`/tmp/nyxbot-adapters-review-tsc.log`, exit 0). The isolated MongoDB replica set
was rechecked and remains PRIMARY. Later UI/admin/discovery changes still require
final validation.

## Authority review notes

The existing guest contract in `docs/chat/09-nyxbot-orchestrator.md`, section
12a, denies tools to orchestrator guests. A public FAQ call must bind to an
owner-configured specialist with the appropriate Notion grant and guest read
access. Capability descriptors and caller ID must not widen that authority.


Continuation review follow-ups (2026-10-17):

- R16/R10: `sms_turn` still sets `state=closed` before `finish_noncall`,
  whose nonclosed predicate skips the missing terminal timestamp/finalization.
  Require an actual successful-SMS lifecycle test.
- R16: allocating versus removing a trusted account and reusing versus releasing
  a global resource claim need shared transaction write fences, not snapshot
  absence checks alone. Revision-filtered deletion counts now are checked.
- R4/R5: call admission performs resource/billing awaits after resolving authority.
  New answer/relay/cap effects must revalidate live binding/session/credential and
  current exact operation policy immediately before provider dispatch. Cleanup
  remains restricted to the durable admitted call after disable.
- R1: legacy lifecycle unknown limits still serialize as zero; model them explicitly.
- Settings tests: the primary independently ran the existing NyxBot settings
  suite (10 passed), but its stderr includes localhost:3000 ECONNREFUSED network
  attempts introduced by the mounted adapter hooks. Fix fixture isolation.
- ResourceClaim now contains a distinct UUID `_id` and unique `identity_key`;
  the identity-key query/deletion concern from the previous draft no longer applies.
- The new discovery URL `/assistant?panel=nyxbot` matches `useAccountPanel`.


Primary frontend verification now adds seven actual setup interaction tests:
owner-only specialist selection; unsupported platform Telnyx calling;
valid submission; credential clearing on provider/mode switch; platform-service
deduplication; cleanup conflict visibility; and Custom LLM secret removal on
modal close/account change. Together with the ten existing settings tests, all
17 pass in `/tmp/nyxbot-adapters-review-ui-tests-2.log` with no accidental
network attempts. The schema contract tests await the generated full catalog
fixture. The first renewed TypeScript run found only an invalid test query option;
that option was removed and final type validation is pending.


The primary's renewed frontend type check and lint of the new test files pass
(`/tmp/nyxbot-adapters-review-tsc-3.log`,
`/tmp/nyxbot-adapters-review-test-lint.log`, both exit 0).

Further R10/R16 recovery requirements: an SMS accepted by the provider whose
HTTP response is lost still needs durable callback-to-send correlation, and
Twilio callbacks without segment counts need authenticated message-read recovery.
The initial binding/admin lists also hide records beyond 100 without a cursor;
creation and discovery must have compatible, usable bounds.


R10 evidence trust follow-up: a BYOK Telnyx webhook public key is supplied by its
owner and is not established as Telnyx's account signing key by an authenticated
lookup. Resource API proof protects routing, but self-signed duration/part fields
cannot alone establish a fee-bearing quantity. Priced BYOK usage needs provider
API evidence before settlement. Shared-platform Telnyx keys instead come from
administrator trust configuration. Also revalidate Twilio authority after its
cap request returns before issuing relay TwiML/tickets (R5).


Additional verified code review requirements:

- R16: `prepare` initially inserted outside the removal transaction fence.
  It now writes the live Binding while inserting the session; transient conflicts
  still need the standard transaction retry helper and concurrency coverage.
- R18: terminal output used ignored `try_send`; a full queue could lose the
  final frame after a successful turn. Bounded terminal delivery and queue-boundary
  behavior need verification alongside interruption/slow-consumer behavior.
- R10/R11: recover known SMS IDs without depending on receipt arrival, prevent
  one provider message from joining multiple sends, and fairly schedule recovery
  beyond the first 100 persistently pending rows.
- R16 traceability: management mutations and Custom LLM secret/session issuance
  need metadata-only events through the existing audit append service. Initial
  implementation logs only trusted-account configuration and allocation.

Independent UI verification: 22 tests pass across new owner adapter setup,
new admin resource setup and existing NyxBot settings, including one additional
rollout-flag visibility test. Log: `/tmp/nyxbot-adapters-review-ui-tests-3.log`.
The existing administrator credentials suite separately passes 15 tests
(`/tmp/nyxbot-adapters-review-admin-tests.log`). Implementation-file frontend
lint and the renewed backend boundary script both pass. These are intermediate
checks while backend recovery changes are still being completed.


Continuation verification: `prepare` now uses the retryable transaction helper,
and SMS/call recovery has ordered progress markers and supporting indexes. The
primary frontend type check passes again (`review-tsc-4.log`, exit 0).

R16 new race: `disable` ignores its conditional update matched count. A concurrent
reverify can win, after which disable returns success with the binding still
enabled. Require a checked CAS and behavior coverage. R11 also needs actual
ringing-to-active Twilio bootstrap tests, including duplicate admission while a
relay is active: returning Hangup for every duplicate must not unexpectedly
terminate an already admitted call on an ordinary retry.


Primary production frontend build passed (`/tmp/nyxbot-adapters-review-build.log`,
exit 0), including credential-accept bundle and mock-footprint validation.
The primary owns the final `docs/NYXBOT_SERVICE_ADAPTERS.md` and the Telnyx
integration corrections; the implementation worker owns its evidence checklist.

The worker's finish notification on 17 October reflected provider stream retry
exhaustion, not successful completion. The same worker was resumed. Tests4 failed
to compile on a moved `hash` in reproducible relay-ticket issuance; this is not
a passing backend validation. The disable CAS now checks its matched count,
and Custom LLM terminal/error sends are bounded. Behavior tests remain pending.


Primary transcript/schema/feature-flag regression run passed: six suites,
197 tests (`/tmp/nyxbot-adapters-review-transcript-tests.log`). The run emitted
localhost connection-refused diagnostics; no failing assertions. Lint of new
implementation files plus changed transcript consumers completed with zero
errors and one existing React Compiler/TanStack Virtual compatibility warning
at `chat-message.tsx`'s unchanged `useVirtualizer` call.


R5/R6 final SMS transport follow-up: the post-billing dispatch block resolves
a fresh credential but discards it, then sends with the earlier `r.key`. OAuth
refresh intentionally preserves credential epoch, so identity equality does not
prove the earlier token remains valid. Send with the credential returned by the
final live-authority resolution. Management allocation also needs an active
person lookup rather than UUID syntax alone for its person-owned contract.


R16 cascade concurrency: user deletion bulk-disables existing bindings and
checks pending usage before applying user security state. Concurrent binding
creation/reverification can reopen admission and insert a call after that
check, then cascade erases its cleanup record. A durable per-owner admission
fence must participate in creation/reverification/session admission and deletion.
SMS send-record insertion also needs a live-session write fence so disable cannot
close the session and permit deletion before a late dispatching receipt is
inserted. Require gated deletion/admission coverage.

Primary updated UI run passes 23 tests in three suites, including ending an
issued ElevenLabs guest session after the secret dialog closes
(`/tmp/nyxbot-adapters-review-ui-tests-4.log`).


With shared build disk space falling to 3.3 GiB, the primary reclaimed two
completed incremental caches over 24 hours old (`nyxid_server-3m71ueoscyg5k`
and `nyxid_server-1i7f7nsvcac79`). Each had no working session, and every
existing session lock was acquired exclusively/nonblocking before removal.
Active caches, source, dependency outputs and executables were preserved.
Free space was 5.4 GiB afterward while unrelated builds continued.

Continuation personal review verified that the owner admission fence joins
binding creation/reverification, session preparation/admission, allocation and
SMS effect-record insertion. The deletion regression calls the actual admin
cascade, retains authentication while cleanup is pending, rejects subsequent
admission and retries deletion after finalization. The rollout readiness branch
now checks the adapter flag itself. These are source-review results; the new
backend tests still require a passing run.

Further concrete R16/R18 findings were sent to the existing implementer:
Custom LLM activation writes by session ID alone and can reopen a just-closed
session; require a checked prepared-state/generation transition. SMS activation
must check its conditional update count too. WebSocket terminal send and close
must be bounded so backpressure cannot indefinitely delay carrier cleanup.
Binding disable must invalidate all active session leases before its bounded
immediate cleanup, including Custom LLM sessions beyond the first 100 rows.

The primary's renewed production frontend build, including the End guest session
action, passes (`/tmp/nyxbot-adapters-review-build-final.log`, exit 0). It includes
TypeScript, both production bundles and the mock-footprint assertion. Vite reports
the existing large-chunk advisory; it does not fail the build. The full registry
schema fixture is still awaiting backend test execution.

Disk pressure recurred while unrelated worktrees compiled. The primary removed
two completed incremental caches approximately 19–21 hours old
(`nyxid_server-0frakpc8jqnqu`, `nyxid_server-1l6yqwv8ykeq6`) only after confirming
no working session and holding every existing session flock exclusively and
nonblocking. Source, executables, dependency outputs and active builds were
preserved; available space returned to 7.4 GiB.

The primary has now reviewed the current registry/capability projection,
binding creation/live/read-only authority and policy checks, owner admission,
session preparation/activation/admission, coordinator and socket paths, Custom
LLM authentication/history/SSE, carrier proof/dispatch, SMS evidence settlement,
management/removal, HTTP DTO/auth routing, and the changes to shared NyxAgent
egress, callback gates, recap and billing-meter settlement. The compile snapshot
is older than several of these fixes; this source review does not close the
behavioral acceptance items.

One additional concrete R16 retention race remains under implementation:
an authenticated callback may load its binding before deletion, then insert a
call/SMS receipt after cascade has purged the owner collections. Receipt insertion
must transactionally write-fence the existing binding while permitting disabled
bindings for already-admitted cleanup. Cascade deletes bindings before events,
so this orders insertion against deletion without reopening owner admission.
Require a late-receipt versus actual removal/deletion regression.

The receipt-retention fix now joins call/SMS receipt insertion to a transaction
write on the existing binding. Disabled bindings remain eligible for admitted
cleanup. The added regression covers actual binding removal and concurrent
owner cascade versus both receipt paths, including late attempts after purge.
The primary reviewed this implementation, the noncall activation CAS, bulk lease
invalidation, bounded socket shutdown, and final SMS authority after meter
awaits. No further concrete source finding is currently open beyond obtaining
behavioral validation. The implementer was asked to freeze source except for
compiler, test and lint fixes. The renewed primary backend boundary check passes
(`/tmp/nyxbot-adapters-review-boundary-final.log`, exit 0).

Check6 was cancelled by the implementer after about 18 minutes because its
snapshot predated the final activation and receipt fixes. Only its task-owned
cargo/rustc processes were terminated; it produced no passing result. The
coherent replacement run is `/tmp/nyxbot-adapter-tests6.log`, using the shared
target and isolated replica set, `NYXID_UPDATE_CONTRACT_FIXTURES=1`, and
`cargo test -p nyxid --locked -- services::call_channels::tests
services::admin_user_service::tests --test-threads=1`. It is still compiling.

Final security-convention follow-up for the next required fix/lint pass: wrap
minted Custom LLM session tokens and raw relay-ticket HMAC bearers in
`Zeroizing<String>`, matching the repository rule and the existing Custom LLM
setup-secret implementation. Serialize only at the intentional response/URL
boundary. This does not justify cancelling the current test build.

The primary added guide crosslinks in `docs/chat/README.md` and
`docs/CHANNEL_BOT_RELAY.md`. Two further completed >19-hour incremental caches
(`nyxid_server-3rxvag8yx8aa5`, `nyxid_server-0lzmetpv0vv9o`) were reclaimed with
all existing session locks held exclusively/nonblocking and no working sessions;
free space returned to 5.1 GiB. Active builds and other artifacts were preserved.

The implementer identified an existing native-tool inventory assertion that
still expects 22 tools, and its audited tool fixture has not yet invoked the new
`list_channel_adapters` entry. Queue the count update and real discovery call
for the next required fix pass, then run `assistant_authority_tests` as part of
the affected regressions. The frozen adapter/account-lifecycle run remains
unchanged while it compiles.

At roughly 34 minutes into tests6, available disk fell to about 451 MiB while
the compiler remained active. The primary reclaimed completed incremental cache
`nyxid_server-2s79a8bn44rkj` (about 7.7 hours old), with both session locks held
exclusively/nonblocking and no working session. Space returned to 3.5 GiB.
Source, active builds, executables and dependency outputs were preserved.

To stabilize space during sustained swapping, the primary also checked older
caches carrying stale `-working` markers. Two were abandoned:
`nyxid_server-31gpti5vkhzmz` and `nyxid_server-3gvoovcq440gs`. Every recursive
file timestamp was over 12 hours old (session ages about 20 and 30 hours), every
session lock was held exclusively/nonblocking, and `lsof` found no other process
with an open file in either directory. Only these abandoned compiler caches
were removed; free space returned to 11.3 GiB. The frozen test build continued.

Twilio WebSocket interoperability follow-up is queued after frozen tests6:
signature verification should accept either the exact configured WS/WSS URL or
its scheme-only HTTP/HTTPS upgrade representation, preserving host, port, path,
query and ticket. Neither forwarded headers nor caller-selected URLs may expand
the set. Require helper rejection tests for changed authority/path/query and an
actual upgrade signed with the HTTPS form. This is defensive interoperability
coverage, not a claim of live Twilio validation.

Final evidence mapping found two remaining behavioral coverage gaps in the
existing findings: R8 has source use of the no-redirect transport but no actual
redirect-to-second-server refusal assertion; R20 has the path-only span change
but no captured request diagnostic proving a bearer query is absent. Both were
queued for the next batched fix pass, without interrupting tests6. The regression
should exercise the production span factory rather than duplicate it.

R21 evidence mapping exposed a remaining source boundary: the full channel
egress fence still precedes the awaited `billing.mark_forwarded` call. Recheck
channel turn/key/session authority after that final admission await, immediately
before transport, and preserve correct known-not-dispatched meter handling on
refusal. The existing gate-after-NyxAgent-acceptance test covers tool callbacks,
not this pre-dispatch boundary. A deterministic admission revocation regression
was requested in the same batched patch.

The primary recorded source hashes and review baselines in
`/tmp/nyxbot-adapters-review-tests6-source.json` and
`/tmp/nyxbot-adapters-review-tests6-snapshot` before allowing queued edits while
tests6 continues. The current compiler had already parsed its source and was
sampled in MIR borrow checking, with a 13 GiB footprint under heavy swapping.
No running build was cancelled or restarted. Tests6 will be treated as its
recorded earlier snapshot; final source requires a separate acceptance run.

The primary reviewed the new Zeroizing bearer lifetimes, scheme-only Twilio
upgrade verifier and controlled HTTP-form signature upgrade, redirect refusal
regression, production request span/capture regression and actual audited adapter
discovery invocation. These are source-reviewed, not yet executed. The operator
guide now describes the exact two URL representations accepted for WebSocket
signatures. Readiness tests were asked to assert the actual per-operation reason,
so an unrelated disabled/billing failure cannot satisfy the policy regression.

The R21 patch now performs the full channel fence after the forwarded billing
marker, immediately before transport, and persists zero usage on known
non-dispatch. Its per-instance test gate exercises stopped/replaced turns and
revoked execution keys, asserting no upstream request or FAQ callback and a
finalized zero-quantity meter. The primary caught and the implementer corrected
an AppConfig dereference error in that new fixture before compiling it. R22
readiness coverage now asserts ResourcePolicyUnresolved per call operation for
both carriers. These source-reviewed changes still require execution.

With free disk at 2.8 GiB, the primary removed 257 abandoned compiler `.rcgu.o`
intermediates for `nyxid_server-312abb2676d3047c` (2.11 GiB, newest related
artifact over 31 hours old). Immediately before removal, no live rustc/clippy/
linker command referred to that hash and no process had an open file in that
artifact group. Only generated object intermediates were removed; source, final
executables, metadata, libraries and all active builds were preserved. An exact
path/size/mtime manifest is in `/tmp/nyxbot-adapters-stale-objects-cleanup.json`.

At that continuation checkpoint, tests6 remained active (Cargo 83900,
rustc 84129), around 95 minutes elapsed and 13 minutes CPU. Its only diagnostic
is the unused-mut warning already corrected in newer source; no tests have run.
A second sample advanced to dead-code/live-symbol analysis with a 14.5 GiB
compiler footprint. The host has 16 GiB RAM and roughly 30.6 GiB swap in use.
The user has an optional pending choice about pausing unrelated compiler jobs;
no answer/authorization was received and none was paused. The existing worker
continues validation with notify-on-finish. No implementation approval or
passing backend acceptance is claimed.

## Finding closure

The primary reviewed the complete adapter path and the final corrective deltas.
The worker's acceptance fixture map names the executed cases for each behavior.

| Finding | Reviewed resolution and evidence |
| --- | --- |
| R1 | Registry capabilities reflect executable hooks, disabled legacy profiles and explicit unknown limits; all 15 descriptors pass the backend/frontend contract checks. |
| R2 | Typed messaging, conversation and Custom LLM registry variants dispatch through the coordinator; actual transport fixtures exercise each profile. |
| R3 | Platform verification keys come from trusted account records; resource membership and exclusive ownership precede effects. Forgery/cross-resource fixtures pass. |
| R4 | Provider effects use the shared exact canonical operation policy at final dispatch. Signed Telnyx transport covers encoded call IDs and shared canonical-path checks pass. |
| R5 | Live binding revision, session generation, lease, credential and owner authority fence effects after awaited admission work. Revocation and deletion races pass. |
| R6 | Trust rotation/deletion invalidates old authority; verification-key retrieval checks the current account revision and key. The rotation fixture rejects forged ingress. |
| R7 | Diagnostic implementations redact credential identity; minted session and relay bearers use Zeroizing wrappers. |
| R8 | Credential transport rejects redirects; a real second-server fixture confirms no redirected credential or effect. |
| R9 | Telnyx membership proof scans bounded cursor pages and refuses nonmembers before commands. |
| R10 | Durable admitted meters retain rates/funding and finalize once after revocation; funded call/multipart SMS, frozen duration, receipt ordering and remainder-release fixtures pass. Unsupported pricing lanes are rejected explicitly. |
| R11 | Relay and carrier terminal evidence are distinct; admitted cleanup survives revocation. Twilio TimeLimit is confirmed before relay release. Platform Telnyx voice is explicitly unavailable because the provider cannot enforce the required bound. |
| R12 | Complete bounded SMS replies reserve conservative multipart capacity and use provider evidence; overflow requests a shorter answer explicitly. |
| R13 | Provider-correlated heard-prefix evidence is retained separately from generated text and used by guest recaps. Actual interruption fixtures pass. |
| R14 | Bounded utterance ordering supports a second final utterance during a live turn; real signed carrier/WebSocket FIFO fixtures pass. |
| R15 | Ordinary tool callbacks revalidate adapter authority and the specialist's existing grants; disabling after NyxAgent acceptance rejects an actual Notion callback. |
| R16 | Transactional owner/binding/session write fences order admission, disable, activation, receipts and cascade deletion; bounded fair recovery, terminal noncall handling, resource release and late-receipt races pass. Management actions use metadata-only audit events. |
| R17 | Custom LLM identity binds a durable guest session and advancing history; real HTTP fixtures distinguish repeated text, independent callers, duplicates and ambiguous resets. |
| R18 | Failure/lag cannot become successful SSE completion; bounded terminal delivery and downstream cancellation pass full-queue, disconnect and slow-tool fixtures. |
| R19 | Only the winning WebSocket claim owns cleanup; a concurrent loser cannot hang up the winning call. |
| R20 | The production trace factory records method/path without bearer queries; captured diagnostics verify the contract. |
| R21 | Full turn/key/session authority is checked after the final billing await and before NyxAgent dispatch; stopped/replaced turns and revoked keys make no upstream request and settle known non-dispatch to zero. |
| R22 | Readiness reports ResourcePolicyUnresolved for future call resources and checks both carriers' prerequisites; runtime enforces the actual call's policy. |
| R23 | The generated complete registry parses in the normal frontend environment; nullable documentation and additive operations remain compatible. TypeScript, lint, formatting and all three schema assertions pass. |

## Final validation

The final selected backend run passed **881 tests**, with zero failures and zero
ignored tests; 8,172 tests were outside the selected filter. It exercises signed
HTTP/WebSocket transport, real authenticated FAQ proxy callbacks, Custom LLM SSE,
live instructions, interruption, concurrency, owner deletion, funded usage and
the affected shared authority/proxy/registry paths. It does not represent the
entire backend suite or live provider testing.

The primary verified all 53 backend source/configuration hashes against
`/tmp/nyxbot-adapter-tests13-source.json`, including coverage of every changed
backend Rust/configuration file. No backend source changed afterward. Manifest
canonical JSON SHA-256:
`5c3f558372f84303d00a0940aa2676b278536f1d9a23ad82f0f4ec0c54c0b267`.

| Check | Final result | Evidence |
| --- | --- | --- |
| Selected backend acceptance/affected regressions | 881 passed; compilation 1m57s, tests 672.92s | `/tmp/nyxbot-adapter-tests13-final.log` |
| Focused signed carrier and canonical authorization | 20 passed | `/tmp/nyxbot-adapter-tests13-carrier.log` |
| Full workspace Clippy, all targets, locked, warnings denied | Passed | `/tmp/nyxbot-adapter-clippy13.log` |
| Rust formatting and backend RCI boundary | Passed | `/tmp/nyxbot-adapter-fmt-final13.log`, `/tmp/nyxbot-adapter-boundary-final13.log` |
| Frontend owner/settings/admin/catalog, normal happy-dom environment | 26 passed in four suites | `/tmp/nyxbot-adapters-review-final-ui15.log` |
| Final TypeScript build check | Passed | `/tmp/nyxbot-adapters-review-tsc15.log` |
| Final catalog test lint and formatting | Passed | `/tmp/nyxbot-adapters-review-catalog-lint15.log`, `/tmp/nyxbot-adapters-review-catalog-format15.log` |
| Worker admin/feature-flag regression | 10 passed | `/tmp/nyxbot-adapter-frontend-final13.log` |
| Earlier affected transcript/schema/flag regressions | 197 passed in six suites | `/tmp/nyxbot-adapters-review-transcript-tests.log` |
| Existing administrator credentials regression | 15 passed | `/tmp/nyxbot-adapters-review-admin-tests.log` |
| Production frontend build including both bundles and mock-footprint check | Passed; product implementation unchanged afterward | `/tmp/nyxbot-adapters-review-build-final.log` |
| Final whitespace check | Passed | `/tmp/nyxbot-adapters-review-diff15.log` |

These runs overlap and must not be summed as a distinct-test count. The earlier
122-test legacy channel baseline also passed. Existing frontend build/lint
advisories about bundle size and the unchanged useVirtualizer call remain; the
latest test command emits the environment's module.register deprecation warning.
They are not adapter test failures or new adapter findings.

The primary's final test-only correction imports the generated JSON fixture
statically so happy-dom does not reinterpret its file URL, clones it per test,
and narrows the first descriptor before constructing the additive-field case.
This removes the Node-only test workaround and passes strict TypeScript.

Final runtime corrections reviewed personally include the coordinator's existing
Metered(Proxy) route classification, current platform trust verification, safely
canonicalized encoded Telnyx call IDs, and awaited zero-usage funding release on
a separate Tokio task to avoid the existing deep proxy poll-stack overflow. No
authorization check was bypassed and the final tests use the default thread stack.
Earlier failed or incomplete runs remain historical evidence in the worker log.

The feature remains default-off. Every API replica must be upgraded before
`nyxbot:service-adapters` is enabled because old replicas do not enforce the
additive tool-callback authority. There was no live call/SMS, account change,
commit, push or deployment. The separate credential-exposure incident described
above remains unresolved pending rotation; this review does not label it closed.

## Final local cleanup

Both task-owned MongoDB instances were stopped with graceful SIGTERM after their
PID, command, data path, port and replica identity were verified. Process absence
was then checked: PID 78712 on 27116 (MongoDB 7 initial fixture) and PID 54322 on
27117 (MongoDB 8 final fixture). Data and logs were retained. The record is
`/tmp/nyxbot-adapters-review-mongo-cleanup15.json`. Reproduction requires starting
the MongoDB 8 replica again. Unrelated services and compiler processes were not
stopped.

## PR integration check (2026-10-17)

The user authorized PR publication, CI monitoring, squash merge and cleanup.
The branch is now `feature/nyxbot-service-adapters`, rebased onto main
`64116aace` (six upstream commits after the original validation baseline).
The 881-test result above belongs to the pre-rebase source; fresh combined-source
checks and GitHub CI are required before merge.

Conflicts in the proxy handler and model registry were resolved without restoring
the old proxy implementation over main's service-execution changes. Main now
carries transport-level HttpEgressGuard/OwnedHttpEgressGuard checks through
credential preparation to provider dispatch. Those revalidate ChatAuthority via
the shared ensure_live_turn/enforce_turn_gate path, which includes the adapter's
session authority. The earlier handler-only checks are superseded by that shared
transport gate. Both model modules are registered. The final review also checked
additive serde/frontend fields, unchanged legacy registry projection, optional
credential-resolution keys, owner cleanup and durable billing release. No
unrelated API or data-contract removal was found.

The contributing guide's API reference, architecture and README updates are now
included. The wizard source-closure hash matches its committed bundle and none
of this PR's files are in that closure. Main's GitHub ruleset requires one
approving review; no bypass is planned. Live provider validation and the separate
credential-rotation incident remain outside these local/CI results.

### First PR CI run

[CI run 39013759876](https://github.com/ChronoAIProject/NyxID/actions/runs/39013759876)
validated rebased head `fb97669f1e64c8c0a96ab516ff5472a9b3d5e216`. The full
backend nextest run completed: **9,224 passed, four failed, eight skipped**. All
four failures were new adapter tests; no existing backend test failed.

- **CI1 — open:** Linux stack overflows in
  `concurrent_duplicate_websocket_loser_cannot_cleanup_winner`,
  `signed_carrier_ingress_websocket_finals_and_fifo_reach_real_faq_tools`, and
  `twilio_rejected_or_uncertain_cap_never_releases_relay_and_never_replays_cap`.
  Coverage independently aborted on the first of these, before producing a
  coverage percentage. No lowered threshold or increased stack is accepted as
  a resolution.
- **CI2 — open:** `websocket_disconnect_stops_accepted_turn_before_tool_callback`
  timed out waiting for fixture callback completion after disconnect. Preserve
  the actual cancellation/zero-tool-effect contract and remove timing ambiguity.
- **CI3 — open:** CodeQL alerts 517 and 518 trace Twilio account identifiers into
  required REST URL paths. Verify the public routing identity separately from
  credential material and preserve fixed HTTPS production destinations. The
  scanner finding is under investigation, not dismissed as a passed check.
- **CI4 — open:** CodeQL alert 516 flags the test's formatted settlement error.
  Diagnostics must expose only a safe error classification.

The full frontend suite passed locally after rebase: **6,106 tests in 575 files**;
lint and the production build also passed. Remote Rust format/Clippy, all three
KMS feature builds, production image inputs, billing smoke, CLI, Machine Container
E2E, frontend, wizard freshness and CLI/frontend coverage passed. Individual
CodeQL analysis jobs succeeded, but the aggregate correctly remains red for the
three findings. These passing jobs do not override the failing gates.

The machine restarted during this phase; previous `/tmp` logs and local processes
were lost. GitHub job logs and the uploaded JUnit/resource artifacts are retained
as durable evidence. Before the restart, the task's stopped MongoDB data and
frontend dependencies/build output were removed and MongoDB logs compressed.
Remaining generated frontend documentation was removed afterward. No unrelated
server or shared source workspace was stopped or deleted.

### CI correction review in progress

The primary personally reviewed the proposed public Twilio routing identity,
fixed-provider URL validation and cancellation-completion assertions. One review
correction requires canonical lowercase SID digits so the path authorized by
catalog policy is byte-identical to the dispatched path; uppercase and mixed-case
SIDs now have rejection cases. The public SID is parsed independently of the
auth token, which never becomes part of a provider URL. Test diagnostics retain
only the fixed error code and stage rather than formatting a full settlement
error.

The disconnect fixture now signals when the accepted upstream handler finishes
or is dropped. The assertion waits for that signal, requires exactly one accepted
request, permits only cancellation or a rejected callback, and still requires
zero FAQ reads. This establishes completion before asserting absence of effects.
CI1–CI4 remain open until the final correction and renewed validation complete.

The additional scope review checked shared turn fencing, credential-resolution
callers, funding release, additive message fields and frontend parsing against
main `64116aace`. No unrelated API/data-contract removal or dependency version
upgrade was found. Compilation resource pressure is being contained to this
worktree; other worktrees' compilers and shared target artifacts are not cleanup
targets.

The proposed CI1 correction moves `ingress_correlated` and `close` behind
functions returning `BoxFuture`, leaving their existing async bodies and await
ordering intact. Callers now retain the boxed handle instead of embedding those
large admission/cleanup state machines and their construction temporaries. The
primary reviewed the exact diff: no new task, detached operation, retry, billing
decision, authority check or cancellation branch is introduced. Workspace
formatting and whitespace checks pass; runtime validation is still pending.

Two full local diagnostic compilation attempts were interrupted while protecting
disk headroom under concurrent worktree memory pressure. They are not passing
builds or source-regression evidence. A temporary copy excludes unrelated test
functions for focused diagnosis only; repository tests, CI configuration, thread
stack limits and coverage thresholds remain unchanged. The next published
revision must still pass the complete Linux test and coverage jobs.
