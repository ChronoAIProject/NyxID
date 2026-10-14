# Platform BotFather implementation and evidence

Date: 2026-10-14. Approved scope: all stages of
[the plan](2026-10-13-platform-botfather.md) and
[review](2026-10-13-platform-botfather-review.md). One implementation worker;
primary reviews directly. No deployment, production writes, push or live
activation. The two pre-existing untracked plan documents are retained.

## Contract decisions

- The platform guide uses a separate native manager receiver. The existing
  gateway provider authenticates a person-bound `NyxbotChannel`; that contract
  is unsuitable for platform authority. Personal children may continue using
  the gateway, subject to installation gates.
- Manager credentials and creation webhook remain owned by Platform Credentials.
  Platform guide configuration references the exact manager identity and
  observation period, rather than copy its token or install another webhook.
- After explicit platform conversion, ordinary conversation routes and personal
  NyxBot links cannot configure the manager, including exact/sender/default
  routes. A legacy `telegram_manager` without that opt-in retains its supported
  personal behavior. Claimed, paused, unknown and tombstoned profiles remain
  closed; deleting/re-registering a channel does not restore personal routing.
  This applies to
  administrators as chat participants too. Platform controls use live
  `admin_helpers::require_admin` plus `require_first_party_human`, including
  control reads; operator is insufficient.
- A closed guide interface must not issue a personal assistant credential or
  reuse a personal upstream session. Funding and execution identity must be
  explicit before any guide model admission is implemented.
- Legacy personal channel defaults remain unchanged. New installation fences
  apply only to setup-linked children. Organization activation is unavailable v1.
- Telegram child observations require exact child review. Setup-linked requests
  retain revision/digest/binding/profile generations inside the existing worker,
  connect helper and registration transaction; outer checks alone are insufficient.

## Durable stage checklist

- [ ] 0: Local containment of manager personal execution; operational runbook.
  Production containment/version readback is external and has not occurred.
- [ ] 1: Durable platform profile, closed authority, native receiver, admin controls.
- [ ] 2: Static templates, encrypted facts, owner-scoped setup, revision/expiry/cancel.
- [ ] 3: HMAC identity review, explicit human decision, binding/revocation.
- [ ] 4: Website/Telegram owner journey, schemas/hooks/forms/admin UI.
- [ ] 5: Digest approval, leased provisioning, exact child review, worker fences/outbox.
- [ ] 6: Atomic admission, explicit funding, readiness/test/activation and all-path gates.
- [ ] 7: Versioned restaurant publication, safe request URL, zero connector grants.
- [ ] 8: Local regression proof, migration/runbooks; external staging evidence.

### Current checkpoint: native provisioning, inference and activation

Primary runtime source snapshot: `cf3ba2eba823d6e36fbccb1fa7ed06da04437adadcc6d33e5c07d40cddc31b71`
(1,043 Rust/Cargo files), `/tmp/nyxid-botfather-native-checkpoint.json`.
Primary is compiling/running platform module tests from this frozen snapshot;
`/tmp/nyxid-botfather-native-checkpoint-tests.log`. No results recorded yet.
Worker Rust/Cargo files remain frozen until primary releases the window.

Implemented source now includes native fixed-model closed guide/customer services,
explicit sponsor/client payer lanes, atomic reservations/quotas, claim-fenced
approved provisioning, exact child review, existing native worker/registration
fences, receipt recovery, status outbox, customer test proof and atomic activation.
Frontend now exposes exact snapshot approval/child review/launch/activation and
admin model configuration/discovery/readiness. Anonymous draft adoption UI is
being validated during the backend freeze. These additions are not yet complete
acceptance evidence.

Still required: runtime feedback/fixes; distinct-payer/provider deadline tests;
all-path activation and late-effect races; effective lifecycle status and account
retirement cascades; model-discovery operation pricing tests; native outbox/recovery
claim tests; staged Telegram A13 evidence and operational runbooks. No live guide
has been enabled or production changed.

### Historical foundation checkpoint

Implemented locally, verification in progress:

- Explicit durable manager profile and bot-row transaction fence; default off.
- Admin-only first-party convert/read/pause/verify/repair/retire controls.
- Conversion transaction disables exact/default/sender routes, revokes their
  standalone callback keys plus linked gateway/assistant keys, removes encrypted
  assistant credentials, resets upstream session/reply bindings and retains a
  stopping turn for normal terminal settlement.
- Durable leased cleanup uses existing Stop, asynchronous-operation cancellation,
  signed machine cancellation and forced-settlement helpers. Scheduling is
  independent of connector-sweep configuration. New home turns are gated until
  cleanup commits. Gateway cleanup requires observed versioned deletion; no
  shared manager webhook deletion. Active/unresolved transport creation must
  finish/cancel before conversion.
- Native/gateway ingress, personal home selection, cached callback dispatch,
  asynchronous reply, proactive send, route setters, private/group/follow/relink
  and ordinary bot lifecycle/transfer refuse a converted manager. Link publication
  writes the bot fence transactionally and cleans newly minted keys if it loses.
- Funding selects an organization the configuring person can administer, with
  membership fenced in the conversion transaction. Arbitrary active organizations
  and personal wallets are rejected. Persisted funding consent still requires
  live admission revalidation in stage 6.
- Retirement deactivates the channel while retaining its profile/marker, allowing
  explicit admin manager clear/replacement. Token rotation/clear invalidate guide
  generation. Verification/repair remain dedicated admin operations.

Still open in this stage: regression results, pending-link race/recovery evidence,
full lifecycle mock coverage and primary dispositions. This is not completion of
stage 1: conversational model integration, closed guide tools and spend accounting
remain required. No activation endpoint or live guide has been enabled.

## Acceptance evidence

All A1–A16 remain open until their actual tests and required external evidence
are recorded. Local tests cannot prove native Telegram creation or successful
customer receipt. A7/A12/A13/A14 require real staging clients; A1/A2 also require
admin/two-account staging. No provider 202 response counts as delivery tested.

## Test environment

Primary provided an isolated local replica set on `127.0.0.1:27119`, set name
`nyxidBotfatherTest`. Database suites use
`NYXID_TEST_DATABASE_URL=mongodb://127.0.0.1:27119/?replicaSet=nyxidBotfatherTest`.
Transaction-dependent cases use `connect_transaction_test_database`, which
checks topology and a write/abort probe. Primary owns its lifecycle. Existing
standalone MongoDB is untouched; standalone runs are not atomicity evidence.

## Actual verification

- Initial `git status --short`: only the two plan documents were untracked.
- Read approved plan and review. Source trace confirms personal home selection
  in `start_chat_turn`, gateway person binding, route default/exact fallback,
  group owner-seen admission, and generic creation-worker independence.
- Early snapshot command: `NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27119/?replicaSet=nyxidBotfatherTest' cargo test -p nyxid --bin nyxid-server platform_onboarding -- --nocapture`.
  Completed successfully: 3 passed, 0 failed, 0 ignored (8079 filtered out).
  This compiled the initial containment snapshot; it does **not** validate the
  revised durable opt-in/admin/cleanup code.
- Queued duplicate worker rerun was cancelled (exit 130); no result. Primary
  nextest compilation was interrupted for disk capacity; no tests ran. Subsequent
  builds use `CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0`.
- Primary lean `cargo check --tests` failed on one Zeroizing Display error in
  the binding response. The source fix uses explicit `as_str()` only for the
  intended Telegram reply. Primary is rerunning the check; result pending.
- `cargo fmt --all` completed successfully; `git diff --check` passed at the
  current checkpoint. Re-run after subsequent edits.
- Primary reported early `cargo check -p nyxid --bin nyxid-server` passed
  in 2m46s, and 55 baseline channel-bots/detail frontend tests passed. The backend check overlapped edits and is not a pinned unchanged baseline;
  neither result verifies the completed implementation.

## External validation and rollout

Before any pilot: pin deployed NyxID/NyxAgent/gateway versions; drain incompatible
writers; separately authorize and disconnect the old personal manager link;
verify old keys and late replies fail; preserve the one creation webhook;
configure numerical caps/funding and allowlist; run two-account/admin Telegram
creation, cancellation/restart, customer delivery, pause/revoke and rollback.
Never migrate old home context or roll back to the personal public route.


## Frozen backend checkpoint: identity/control/setup foundation

Backend source is frozen for primary runtime verification before frontend work.
Aggregate SHA-256 of sorted `backend/src/**/*.rs` path/hash manifest:
`f5961f3702acff1cd096e153d8c7ee513ff7889f768ad3e96ecf64e87056a326`.
Manifest: `/tmp/nyxid-botfather-backend-checkpoint.sha256`.
This checkpoint does **not** complete the project or stages 1–3.

Changes since the earlier foundation check:

- Decision spacing commits before all code/state validation and uses Mongo
  `$$NOW` for both the admission predicate and timestamp, independent of replica
  application clocks. IP and actor windows remain separate durable admission.
- Link redelivery recovers the same public code/request from authenticated update
  metadata without consuming creation quota or cancelling its own request.
- Unlink/revoke cancels pending confirmations; confirmation additionally compares
  the previous binding generation, including absence. Account status effects
  transactionally admit the exact binding, profile generation and active user.
- Native expected command errors use fixed restart/throttle replies, bounded to
  ten replies per chat/profile/minute and passing the same final authority gate.
  Provider failures remain ACK-safe. No internal error prose reaches Telegram.
- Human settings can list/revoke their own active bindings. Platform profiles are
  discoverable through paginated first-party live-admin GET; ordinary bot detail
  remains denied after explicit conversion. Telegram setup statuses have human
  labels, and unrelated web setups are excluded until explicitly adopted.

New test cases (written, runtime results pending): server-clock spacing,
concurrent admission, rejected-code transaction rollback, replay deduplication,
private sender checks, expiry, profile disable/generation, unlink before binding,
revocation/late confirmation, prepared reply rejection, two-owner list/revoke
isolation, fixed error redaction, and native mock reply/provider-failure ACK.
Use the isolated replica-set URL and lean flags documented above. Primary owns
runtime execution against this frozen source; no database results are inferred
from typechecking.

Primary reported `/tmp/nyxid-botfather-lean-check-2.log` passed `cargo check --tests`
in 2m58s before these final identity/discovery/mock additions, with only the
expected unused approval_digest warning. Worker checkpoint check log is
`/tmp/nyxid-botfather-binding-checkpoint-check.log`; it began before the final
mock/test edits, so it is an intermediate check rather than pinned final proof.

Remaining implementation: frontend journey/admin settings; real closed guide
model and tools; explicit exact funding/quota admission; setup approval and leased
provisioning; existing Telegram worker/connect/registration fences; durable child
installation markers and all-path gates; published restaurant runtime; observed
customer test/atomic activation; outbox/reconciliation; full A1–A16 evidence and
external Telegram staging validation. No live activation is available.


## Progress after the frozen foundation checkpoint

Actual evidence reported by primary against the frozen source/binary:

- Onboarding nextest: **17/17 passed**, run ID
  `c8877d4e-017c-4fa8-a395-fcb8d8a26847`, compile 6m36s, run 4.483s;
  `/tmp/nyxid-botfather-frozen-runtime.log`.
- Pinned existing compatibility binary: routing **12/12**, channel bots **18/18**,
  Telegram **68/68** passed, no failures/ignored; logs
  `/tmp/nyxid-botfather-{routing,channelbot,telegram}-compat.log`.
- Primary frontend page/schema/idempotency tests **11/11** and TypeScript passed;
  `/tmp/nyxid-botfather-primary-frontend{,-typecheck}.log`.
- Primary telemetry suite **32/32**, including onboarding privacy;
  `/tmp/nyxid-botfather-primary-privacy.log`.
- Primary Playwright mocked-API smoke: five onboarding/admin pages at 1440/390px,
  no page errors or horizontal overflow. Browser actions also passed both widths.
  `/tmp/nyxid-botfather-browser-review.mjs`, log and screenshots under `/tmp`.
  This proves local UI/layout/actions only, not server/provider/Telegram integration.

Worker frontend evidence:

- `npx tsc -b`: first UI snapshot passed. A later run caught four test mock-call
  nullability assertions; corrected. Typecheck-3 passed (empty error log).
- Focused Vitest plus router/public-path tests: **36/36 passed**, six files,
  `/tmp/nyxid-botfather-frontend-tests-2.log`. Earlier focused run **9/9 passed**.
- Changed-file ESLint: **0 errors, 11 existing warnings** in shared shell files;
  `/tmp/nyxid-botfather-frontend-lint.log`.
- `npm run build` passed including credential-accept build and mock-footprint
  assertion; `/tmp/nyxid-botfather-frontend-build.log`.

Frontend now includes public starters and browser linking, authenticated setup
list/edit/review/archives, own-binding settings/revocation/adoption, and discoverable
admin profiles with named manager/org selection. Account/resource/code changes
remount local draft/consent state. Lost-create retries retain only an account/template
scoped pending UUID. Retirement/revocation submit the reviewed generation and require
rereview after refresh. Invalid setup IDs render unavailable. Link pages suppress
telemetry (including SPA/current URL/referrer), mark the input sensitive, and set
HTML no-referrer metadata. No provisioning/activation success is fabricated.

R10/R11 source changes (runtime evidence remains pending):

- Missing bot rows deny route authority; orphan route/cached callback regression added.
- Conversion captures exact turn, async operation IDs, machine job IDs and remote
  gateway ID/version. Effects and completion check the current server-clock lease;
  only the captured turn can settle. Remote machine cancellation has a separate
  exact-resource outbox so disconnected nodes cannot hold the old home thread forever.
- Node `CancelExact` stops the captured turn and exact job IDs, including separated
  runtimes. Its explicit advertised capability is required; no broad Cancel fallback.
  Conversion rejects affected old/offline nodes transactionally before mutation.
  Existing ordinary machine operation permissions are preserved.
- Captured gateway ID/version survive local row deletion; remote version drift
  refuses deletion and requires reviewed recovery. Missing conversations terminate
  their conversation cleanup without cancelling future owner work.
- New tests cover stalled claim/reclaim/new owner turn, async/machine survival,
  old-node rollback, missing conversation and versioned gateway cleanup.
- `cargo check -p nyxid -p nyxid-cli --tests` (lean flags): first run failed on a
  missing `ensure!` macro import; fixed with `anyhow::ensure!`. Check-2 **passed
  in 2m47s** with unused StreamExt warning (subsequently removed). Later exact
  outbox split is not yet typechecked by that result.
- `cargo test -p nyxid-machine exact_cancel` (lean flags): **1/1 passed**, 28
  filtered; `/tmp/nyxid-botfather-machine-exact-tests.log`.
- Primary is running the targeted CLI actual-job regression; CLI/machine source
  is held unchanged until its result. No duplicate full codegen is running.

Next coherent stage, now in implementation: explicit human digest approval and
child review; approved step authority bound inside native Telegram request creation,
worker, connect shortcuts/provider effects and registration/activation transactions;
durable child installation markers and all-path gates; native closed guide model
with platform-org funding and atomic quotas; reviewed runtime/customer-test activation.
All A1–A16 and remaining checklist stages remain open until complete evidence.

### Provisioning integration in progress (not a completed checkpoint)

- Primary's actual CLI exact-job regression passed 1/1 with `--bin nyxid`
  (23.38s compile, 0.01s runtime), log
  `/tmp/nyxid-botfather-primary-exact-jobs-bin.log`. The earlier `--lib`
  invocation selected no relevant tests and is not verification.
- Added durable installation, approved step authority, step-job, notice and delivery
  proof models and indexes. Human approval binds the encrypted draft's reviewed
  publication, exact resources and digest before queuing work.
- Native approved request publication is transactional. First `/start` requires
  the approved numeric sender before identity mutation; launch and first-start
  mutations fence the live setup, profile, binding and request generation.
- Connect gates precede saved/active shortcuts and provider calls. Registration
  and webhook-ready commits fence the supplied request snapshot transactionally.
  Live request admission matches sender, child, manager revision, observation,
  scope and connection generation, including the profile's observation.
- Durable installation markers deny ordinary personal route and registration
  paths even while inactive; missing setup remains a denial. This source work
  still requires typecheck, race tests, worker integration and native child ingress.
- Frontend daily-credit validation now uses exact integer picocredits and enforces
  greater than zero and at most 1000. Actual command in `frontend`:
  `npx vitest run src/schemas/platform-onboarding.test.ts`: **3/3 passed**.
- Guide funding contract: keep ordinary platform-key execution's acting-person
  policy unchanged. Implement a purpose-confined sponsor reservation/settlement
  path with live authorized org consent and org payer, fixed permitted inference
  binding/model, caps, and no personal connection fallback. Tests must prove the
  configuring admin's personal wallet is unchanged. This path is not yet implemented
  and A10/R3 remain open.

### Native integration work and diagnostics (still in progress)

- Primary intermediate provisioning check passed in 2m34s with 16 integration
  warnings; `/tmp/nyxid-botfather-provisioning-intermediate-check.log`. Source
  overlapped edits, so this is typecheck diagnostics, not frozen acceptance evidence.
- Primary runtime intermediate check exited 101: missing `ReturnDocument` import
  and a native guide `Send` lifetime error; two unused imports; **no tests ran**.
  `/tmp/nyxid-botfather-runtime-intermediate-check.log`. Import fixed, orchestration
  moved to services, and borrowed JSON response extraction separated from async
  effects. Worker integration check likewise failed on the unfinished customer
  dispatch name and guide lifetime; not counted as passing verification.
- Native guide now has actual fixed-model tool-less inference, typed draft/create/
  update-current/review actions, exact sender/profile/binding context, sponsor-org
  billing admission, exact bounded reservations and atomic server-clock quotas.
  Ordinary acting-person platform-key resolution remains unchanged. Activated
  child runtime uses the client's own personal execution payer and published facts.
- Bounded request lifetime is 90 seconds against 120-second inference leases;
  dispatch rechecks the live turn leases after funding. Sender quota rows retain
  seven days, beyond current spend windows and live leases. Before-forward budget
  holds are refundable; forwarded work conservatively consumes its reserved maximum
  from the daily cap, independently of the ledger's actual exact charge. Tests and
  recovery evidence for this contract remain pending.
- Native receipt metadata is durable before ACK. Message bodies remain transient.
  Interrupted processing becomes an explicit recoverable state with bounded fixed
  status notices; stale receipt claims must fence inference, mutations and sends.
  No automatic replay of inference or model-selected actions after a crash.
- Exact customer test proof requires an independent numeric sender replying to the
  observed sent message with its challenge. A successful send alone never marks
  delivery tested. Activation rechecks proof, owned specialist, bot readiness,
  consent and funding; no ordinary route fallback is permitted.
- Activated setup lifetime is client ownership, not draft retention. Activation
  clears draft expiry. Binding expiry ends onboarding authority; explicit unlink,
  replacement, account disable and platform changes still revoke public consent.
  A separate revocation epoch preserves a same-account binding renewal without
  transferring or extending onboarding authority silently. Runtime lifecycle tests
  and frontend status/copy follow-through remain pending.
- Fresh reapproval retains installation, native request, specialist and bot IDs
  at intermediate stages. Durable native creation provenance supports recovery
  after request retention/purge; management drift remains fail-closed. Stage-by-stage
  resume, expiry, shared-registration-fence and stalled-claim tests are still required.

### Owned reply / readiness follow-up (2026-10-14)

- Primary third native integration check exited 101 with seven E0308 signature
  mismatches; no Send diagnostic and no tests. Log:
  `/tmp/nyxid-botfather-native-integration-check.log`. Remaining receipt callers
  now pass owned database/profile/source/sender; rate-window calls borrow them.
- Model discovery resolves the GET `models` billing operation before admission
  and supplies the Anthropic protocol version header. Successful discovery and
  text calls await idempotent quota release; cancellation/error still uses Drop
  recovery. This removes reliance on asynchronously spawned release between
  sequential readiness phases. Runtime pricing/payer/lease sequencing tests
  remain pending, including distinct operator/client/customer fixtures.
- `cargo fmt --all` passed. Lean `cargo check -p nyxid --tests` running with
  debug disabled and incremental disabled; log
  `/tmp/nyxid-botfather-owned-reply-check.log`. No result claimed yet.
- Owned-reply check exited 101 with two remaining guide `Send` lifetime errors;
  no E0308 errors remained. `/tmp/nyxid-botfather-owned-reply-check.log`. The
  reply transaction now explicitly boxes its owned effect as a Send future;
  follow-up lean check runs in `/tmp/nyxid-botfather-guide-send-check.log`.
- Added a production-admission replica-set test for immediate same-scope admission
  after awaited release and idempotent stale release leaving the next lease intact.
  Not run yet; this is not model/provider or distinct-payer acceptance evidence.
- Setup UI now calls captured-digest approval, exact child review, creation launch
  and activation APIs. Refresh drift disables reviewed approval. Active lifetime
  copy distinguishes ownership consent from onboarding expiry. Frontend `npx tsc
  -b --pretty false` passed after those changes.
- Admin UI adds paused guide configuration, sponsor-funded model discovery and
  explicit readiness/enable. New admin POST discovery route enforces first-party
  human plus live admin and reviewed profile generation. This latest admin UI and
  endpoint are still undergoing typecheck; frontend/server interaction tests remain
  required. Native progress outbox, comprehensive worker/runtime/billing/isolation
  regressions, operational retirement/recovery and anonymous adoption UI remain
  open work, not external blockers.
- Follow-up Send check also exited 101 (same two guide boundary errors), log
  `/tmp/nyxid-botfather-guide-send-check.log`. Manager credential loading used a
  borrowed-string `and_run` provider snapshot callback. It now uses owned strings
  and `and_run2`, retaining snapshot read concern and both reads. New lean check
  `/tmp/nyxid-botfather-manager-send-check.log` pending; no success claimed.
- Native setup-linked waiting-consent entry now directs the owner to exact web
  child review under transactional request/reply authority instead of offering
  the legacy consent callback that bound requests deliberately refuse. Legacy
  native consent remains unchanged. This new branch still needs regression proof.
- Admin guide UI typecheck passed. Focused frontend tests initially failed because
  their hook mock lacked the newly added guide mutation; after updating the mock,
  all 9/9 passed across admin page, public onboarding and schema tests. These are
  bounded component checks, not provider/Telegram or backend integration evidence.
- Manager Send check exited 101 with missing new consent `fence_request` bool
  argument plus the same two Send diagnostics. Both source issues addressed:
  consent passes `connect=false`; full reply transaction now uses owned context
  with Mongo `and_run` and boxed callback. The speculative shared credential-loader
  refactor was reverted; its Send boundary passed and it was not the cause.
- Pending setup/detail and approval queries now refresh every five seconds for
  at most 120 successful updates and provide manual refresh. Approval renders its
  captured read-only facts, FAQ, limits, model, exact child and resource/payer IDs;
  unsaved editor facts are not used for approval. Launch links are tagged with
  actor, revision and state and hidden after any mismatch.
- Proof attempts now use metadata receipt dedup plus durable installation/sender
  rate budgets before lookup; invalid proof does not dispatch model inference.
  Tests for invalid-attempt consumption/replay and per-installation separation
  remain required.
- Added metadata-only status outbox with transactional queueing on observed child,
  native connection completion and activation, bounded retries, DB-clock claims,
  final exact claim/setup/binding/profile/manager fences. No message bodies or
  credentials stored. Runtime claim/retry/delivery tests not yet run.
- A frontend typecheck exposed incomplete new test fixtures; fixtures now contain
  full setup/review data. Subsequent `npx tsc -b --pretty false` passed. New
  provisioning component tests passed 3/3 (stale review, captured exact approval,
  activation denial before receipt). Latest polling/link-tag additions are pending
  revalidation. Full acceptance remains incomplete.

### Primary runtime freeze (2026-10-14)

- Lean owned-context check passed in 2m34s with one unused reply-wrapper warning,
  `/tmp/nyxid-botfather-reply-context-check.log`. Temporary probes and unused wrapper
  subsequently removed. Later notice/proof/fallback/distinct-client changes are
  not covered by that check. Primary will compile/test the coherent frozen source.
- Follow-up checkpoint check was cancelled at primary's freeze request (exit130),
  `/tmp/nyxid-botfather-native-checkpoint-check.log`; no result is claimed.
- Rust/Cargo files frozen and no worker Cargo process remains. Frontend/docs only
  until primary explicitly releases freeze. Include platform module tests and
  `explicit_turn_release_allows_immediate_same_scope_admission_and_is_idempotent`.
- Latest provisioning UI regression passed4/4, including missing-actor safety and
  disabled approval; primary independently verified the null guard (3/3 at its
  earlier fixture snapshot). The null-guard failure is recorded, not erased.
- During primary's Rust freeze, frontend anonymous draft adoption is implemented:
  exact own binding selection, read-only fact review, captured draft revision and
  explicit adoption (no automatic provisioning). `npx tsc -b --pretty false`
  passed; adoption tests2/2 passed (review required and stale revision refusal).
- Latest provisioning/public/admin focused tests12/12 passed, including saved
  snapshot visibility and revision/account launch-link hiding. Polling now counts
  both success and error updates toward its bound. Added subsystem/runbook
  `docs/PLATFORM_BOTFATHER.md`; outstanding remote-version recovery and retirement
  cascades are labeled implementation work, not falsely represented as complete.

### Acceptance audit at native checkpoint

All rows remain open; the matrix distinguishes implemented paths from missing proof.

| Case | Implemented path | Remaining local/external proof |
| --- | --- | --- |
| A1 | Opt-in authority, closed guide prompt/actions, no personal resolver/session | Tool-less funded model and distinct payer tests await this runtime batch; admin-as-client private inventory/personal-connection poison and staging admin chat remain |
| A2 | Public starter routes and existing live auth/signup journey | Real signup-policy/browser integration and two-account staging |
| A3 | Hashed link codes, human approval, same-chat confirmation, binding epochs | Foundation tests17/17 passed; expanded race/relay cases still required |
| A4 | First-party-only owner APIs, safe preview, secret-free DTOs and metadata receipts | Credential/storage/log contract tests and delegated denial coverage |
| A5 | Exact owner/binding/profile scoping; distinct operator/client fixture | Expanded client B/D substitution and late-reply tests; billing fixture must differ |
| A6 | Shared encrypted facts, guide captured current setup/revision, explicit adoption | Guide/web mutation and multiple-setup selection tests |
| A7 | Stable planned IDs, claim-fenced resource transactions, native reconciliation | Stage-by-stage crash/reapproval tests and staging native creation |
| A8 | Purpose-confined tool-less inference and permanent installation marker | Closed credential/permission/direct/wrapped effect regressions |
| A9 | Reviewed restaurant publication and HTTPS request links | Customer prompt/visibility tests; no confirmed-booking assertions |
| A10 | DB-clock atomic quotas/reservations, sponsor/client lanes, awaited release | New distinct payer exact-ledger/wallet/checkpoint, operation-price, pre-refusal and abandoned-hold tests await this runtime batch; slow preparation/chunks and broader concurrency races remain |
| A11 | Revision/digest/generation fences, runtime authority watch, active revocation epoch | Renewal/unlink/replacement passed in focused9; effective status/shared assessment, owner retirement/batch recovery, active binding expiry and asset destruction await this runtime batch; broader resume/ownership-change cases remain |
| A12 | Native manager credentials/shared webhook retained; admin repair/rotation | Foundation Telegram compatibility68/68 passed; new worker/rollback and staging evidence |
| A13 | Independent sender exact observed reply/challenge proof plus atomic activation | Local signed ingress/proof-attempt tests and actual independent Telegram delivery |
| A14 | Signed native creation provenance, serialized identity fence, exact child review | Barriered manual registration/observation, stale forms, competing setup tests |
| A15 | Installation-aware native/gateway/personal/group/follow/setter fences | Bounded existing routing12/12/channelbot18/18 passed; new child all-path tests |
| A16 | Native saved/active/new retry checks, request generation and transaction fences | Worker claim/registration race/revocation runtime tests |

Targeted frontend ESLint passed with no diagnostics after new controls/adoption.
Anonymous adoption tests2/2 and focused provisioning/public/admin tests12/12 passed.
These results do not close model/billing/Telegram acceptance cases.
- Primary frozen native runtime completed:27 tests,25 passed (one nextest LEAK),
  two cleanup failures. Log `/tmp/nyxid-botfather-native-checkpoint-tests.log`.
  Failed: conversion cleanup completion (pending count1) and stale cleanup retry
  (`Cleanup turn has not settled`). Read-only investigation identifies typed
  `AssistantConversation` settlement replacement dropping raw cleanup fields.
  Fix will preserve modeled markers, not weaken completion fencing; backend
  remains frozen while compatibility finishes. Legacy-manager test passes but
  leaked handle needs investigation/clean shutdown evidence.
- Primary same-binary compatibility so far: routing13/13, channelbot18/18 passed;
  Telegram still running. No complete compatibility claim yet.
- New mocked-browser provisioning flow passed at1440/390: exact captured child
  approval, creation launch, link removal on ready-state refresh and activation;
  zero page errors/overflow. `/tmp/nyxid-botfather-provisioning-browser.mjs`, `.log`,
  screenshots `/tmp/nyxid-botfather-provisioning-{1440,390}.png`. This is mocked
  UI action/layout evidence only. Initial attempt on unavailable4827 failed;
  worker started its own loopback Vite4828 (session49662), leaving primary services
  untouched. No live model, Telegram or production actions.

### Cleanup/proof/admin follow-up in progress

- Primary released native checkpoint freeze. Same-binary compatibility completed:
  routing13/13, channelbot18/18, Telegram68/68 passed; snapshot unchanged.
- Cleanup fence fields are now modeled in `AssistantConversation`, initialized by
  normal constructors and preserved by typed replacements. New regression asserts
  settled markers remain and a new owner turn is blocked until exact completion.
  Completion matching was not weakened. Legacy-manager test now explicitly drops
  its test DB/collection handles and shuts down its client; LEAK rerun pending.
- Proof attempt budgets precede capture authority writes. Proof dedup is inserted
  in the same Mongo transaction as proof lookup/confirmation; abort removes the
  row, so it cannot consume an uncommitted confirmation. Invalid attempts still
  consume durable early rate budgets and never dispatch inference.
- Added final active-person canonical RBAC write fence after guide readiness and
  in conversion/retirement/verification transactions. Legacy `is_admin` alone
  does not qualify. Long repair/credential effects and full delayed-readiness
  role-revocation tests remain to complete.
- Explicit binding unlink/replacement now invalidates all prior generations for
  the previous owner; same-owner renewal excludes active setups. New generation
  renewal/unlink/replacement and owned-runtime authority tests added, not yet run.
- Native dispatch now distinguishes intentional closed/invalid/quota ACKs from
  retryable infrastructure errors; manager lookup failures propagate. Handler
  fault/retry integration tests remain pending.
- Focused nextest replica-set rerun running with2threads and lean flags:
  `/tmp/nyxid-botfather-cleanup-proof-runtime.log`, source manifest
  `/tmp/nyxid-botfather-cleanup-proof-source.json` (1,047 Rust/Cargo files). No pass
  claimed yet. Rust/Cargo temporarily unchanged during this worker-owned run.
- Admin service/model picker uses purpose-confined sponsor-authorized catalog
  metadata and shared platform-key grants/provider snapshots. Model discovery
  accepts selected service without first inventing a model ID; results carry exact
  service and profile generation. Final readiness remains authoritative.
- Expanded mocked browser flow at1440/390 also verifies latest-revision reload
  restores Pause and submits correct revision; no page errors/overflow. Still
  mocked UI evidence, not server/proof delivery evidence.
- Next backend targets recorded: anonymous mutation must independently match
  verified sender scope; idempotent draft reuse must match current owner/binding/
  profile; anonymous review requires linking/adoption; effective lifecycle status,
  account retirement cascade, final service-readiness drift, distinct billing and
  model/receipt/worker race evidence remain required.
- First focused rerun finished exit101 with two tuple-pattern compile errors in
  `native_dispatch_status` and one unused-mut warning; no tests ran. Patterns
  corrected to `ChannelBotNotFound(_)`/`ChannelBotInactive(_)`; warning removed.
  Log retained at `/tmp/nyxid-botfather-cleanup-proof-runtime.log`.
- Anonymous update now independently computes verified sender scope; owned updates
  match live owner/profile/binding fields in their write predicate. Draft replay
  checks exact current owner/binding/profile and writes a live revision/expiry
  fence before returning a reference. Anonymous review directs linking plus
  explicit adoption rather than an owner-only detail URL. Added cross-anonymous
  captured-draft, foreign replay and review-handoff regressions; pending runtime.
- Focused rerun2 includes the new anonymous cases, same replica set/lean flags:
  `/tmp/nyxid-botfather-cleanup-proof-runtime-2.log`, source manifest
  `/tmp/nyxid-botfather-cleanup-proof-source-2.json`. No results claimed yet.
- Frontend initial empty-guide bootstrap test passed: named eligible service ->
  scoped discovery -> chosen model -> paused save -> explicit validation/enable.
  Admin page tests2/2 passed. Enable relies on mandatory server readiness instead
  of requiring a second UI discovery solely because the paused save bumped profile
  generation; stale dirty form state remains disabled.
- Primary independently validated latest frontend: six focused files20/20 passed,
  `/tmp/nyxid-botfather-primary-ui-lifecycle.log`; `npx tsc -b` exit0,
  `/tmp/nyxid-botfather-primary-ui-lifecycle-typecheck.log`.
- Mocked admin bootstrap browser flow passed1440/390: empty profile, named eligible
  service selection, scoped model discovery, save paused, explicit validate/enable;
  zero page errors/overflow. `/tmp/nyxid-botfather-admin-bootstrap-browser.mjs`,
  `.log`, screenshots `.../nyxid-botfather-admin-bootstrap-{1440,390}.png`.
  API bodies/generations are asserted in the mock; this is not funded server API
  or provider readiness proof. Those integration tests remain required.

- Focused rerun2 completed: 9/9 passed, zero failures/LEAK, nextest
  `4be36e99-6416-41ca-b72b-84dbaace7ab1`; build9m22s, run10.418s.
  `/tmp/nyxid-botfather-cleanup-proof-runtime-2.log`. This verifies both cleanup
  regressions, legacy opt-in/tombstone, proof transaction abort/child scope,
  final admin helper, renewal then unlink/replacement and anonymous write/replay.
- Primary same-binary personal conversation suite40/40 passed; full onboarding
  31/32 passed. Invalid-command notice provider failure incorrectly returned503;
  fixed notice send failure now ACKs after bounded admission while earlier DB
  failures remain retryable. Regression rerun pending.
- Added bounded abandoned native-budget sweep to existing independent30s runner.
  Mongo-clock turn deadline fences recovery against forwarding; durable dispatch
  admission always retains conservative spend, even if stale Drop says unforwarded.
  Pre-forward refunds and lease release commit together, once, matching reservation
  day so old holds cannot subtract a new UTC day's counter. Production admission/
  reservation process-loss, duplicate sweep, stale forward and day mismatch test
  added; not yet run. No new runtime verification claimed for these edits.

- Lifecycle batch source adds owner disable/delete retirement with transactional
  binding/setup/installation invalidation, live inactive-owner fence and independent
  recovery through setup expiry sweep. Delete clears ciphertext/publication; stable
  child provenance stays as a non-personal tombstone. Reactivation requires fresh
  review. Added regression, pending runtime.
- Provisioning retries now stop after five attempts (including crashed leases),
  mark matching setup failed-retryable and clear approval; fresh review resets
  attempts while retaining resource identities. Added effective active-runtime
  notice for profile/installation/specialist/binding unavailability, surfaced in UI.
- Guide readiness final mutation now matches the exact reviewed service projection
  transactionally; changed service cannot enable under an earlier readiness result.
- Budget recovery has its own DB-only30s ticker independent of Telegram notices,
  plus state/lease-deadline index. New handler regression injects a real Mongo
  collection validator failure before /link publication, expects503, restores DB
  and checks two redeliveries create one request; not yet executed.

- Lifecycle intermediate checks passed with lean flags: `/tmp/nyxid-botfather-lifecycle-recovery-check.log` (2m50s),
  `.../nyxid-botfather-lifecycle-funded-check.log` (2m40s),
  `.../nyxid-botfather-lifecycle-batch-check.log` (2m44s). They overlapped source
  additions, so they are intermediate typechecks, not frozen runtime evidence.
- Added production-billing tests with fake Lago and mocked catalog provider: distinct
  sponsor-guide payer, independent customer test/native worker/activation/personal
  payer, models operation price/protocol header, definite pre-generation zero
  settlement, admin demotion during funded readiness, and stalled Telegram notice
  concurrent budget recovery. These tests are unexecuted pending checkpoint.
- Ordinary account retirement now performs indexed no-platform-state lookup before
  any transaction. Existing session/key revocations precede platform cleanup.
  Recovery selects unfinished durable retirement across setups/bindings/installations,
  marks completed resources and orders batches; >100 owners/reactivation regression
  added. Active owner mutations/reapproval clear prior disabled-retirement progress.
- Primary standalone baseline (before this retirement batch): disable1/1 and delete1/1
  passed with no skips on task-owned Mongo27120. Post-change rerun required; replica
  set27119 remains the only atomicity test environment.
- Opted-in manager credential repair rechecks canonical live admin before credential
  and webhook effects, with final readiness publication fenced transactionally;
  absent opt-in preserves legacy standalone behavior. Runtime compatibility pending.
- Native receipt production recovery claim extracted for regression: reclaimed old
  processor cannot create a draft; duplicate delivery cannot replay model work;
  interrupted update sends fixed notice. Runtime pending.
- Frontend notice first typecheck failed due JSX placement; second invocation also
  failed (fix script used the wrong relative workdir). Corrected placement;
  `/tmp/nyxid-botfather-lifecycle-notice-tsc-3.log` exited0. No runtime claims from
  failed commands. Focused six-file UI rerun and final Rust batch typecheck pending.

- Follow-up readiness fence now re-reads the typed catalog service in its transaction,
  compares a focused projection (no encrypted credential in query), then writes by
  `_id` for a concurrent-write conflict fence. Legacy raw missing-default and changed
  destination during paid readiness regressions added, pending runtime.
- Terminal cancelled/expired rows receive retirement progress without changing
  terminal state. Recovery test now places100 terminal owners ahead of one live
  unfinished owner. Approved-but-unconfirmed link requests are included in indexed
  platform-state detection and recovery; they cannot resurrect after reactivation.
- Effective status uses shared read-only installation assessment inside a read-only
  snapshot, also called by active customer admission before existing write fences.
  UI displays effective Paused with the notice. Child deactivation/revision drift,
  manager delivery readiness and specialist destruction assertions added to the
  actual provisioning/activation test. Unexecuted until the broad checkpoint.
- Guide/customer payer assertions now require exact nonzero settled funding, durable
  wallet-credit ledger posting and signed account checkpoint, and exact available
  wallet delta; wrong operator/client/sponsor ledger charges are rejected.
- Gateway transport/cleanup helpers moved unchanged out of legacy handler into
  `nyxbot_gateway_service`. Admin recovery control draft is outside active source at
  `/tmp/nyxid-botfather-gateway-recovery-draft.rs`; controls are NOT mounted in this
  checkpoint. Finish them in next batch, including reviewed preserved changed binding
  or captured-version retry and no blind remote version adoption/deletion.
- Latest focused frontend rerun20/20 passed, `/tmp/nyxid-botfather-lifecycle-notice-ui.log`;
  effective-status TypeScript check exit0, `/tmp/nyxid-botfather-effective-status-tsc.log`.
  Rust readiness/status intermediate check passed2m56s, overlapping edits; final
  freeze check `/tmp/nyxid-botfather-runtime-freeze-check.log` still pending.

## Lifecycle/funding broad runtime freeze

- Rust/Cargo frozen at1,048 files, manifest
  `/tmp/nyxid-botfather-lifecycle-funded-source.json`, canonical sorted JSON SHA-256
  `f936746014cd4eeef0b0cb4f1eb45f0abb912d8aedd7ebfeafaa60af5b234c6b`.
- Frozen `CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0
  cargo check -p nyxid --tests` passed2m47s, no diagnostics,
  `/tmp/nyxid-botfather-runtime-freeze-check.log`.
- Broad run launched (session94906), replica set27119,2threads, lean flags:
  `cargo nextest run -p nyxid --bin nyxid-server --test-threads 2 -E
  'test(platform_onboarding) | test(platform_inference_service::) |
  test(platform_runtime_service::) | test(platform_native_receipt_service::) |
  test(platform_setup_notice_service::) | test(platform_bot_config_service::) |
  test(platform_bot_authority::) | test(platform_binding_service::) |
  test(platform_setup_service::) | test(platform_provisioning_service::) |
  test(platform_guide_service::) | test(platform_customer_service::)'`.
  `/tmp/nyxid-botfather-lifecycle-funded-runtime.log`. Compile/run pending; no test
  result claimed. Binary path will be confirmed after linking. Rust remains frozen.
- Effective-status component regression passed7/7, including stored Active with
  effective Paused: `/tmp/nyxid-botfather-effective-badge-ui.log`.
  Primary unchanged-source six-file20/20 passed, `/tmp/nyxid-botfather-primary-effective-ui.log`;
  primary TypeScript exit0, `/tmp/nyxid-botfather-primary-effective-ui-typecheck.log`.
- Remaining local work after feedback: mounted admin gateway inspection/review/retry;
  client-D cross-installation/late-effect cases; actual signed manager/customer ingress
  admission/failure/recovery and no paid replay; slow provider/chunk and delayed billing
  preparation; full stage-by-stage native reconnect/registration barrier tests;
  bounded notice/claim recovery; final UI/API docs and A1–A16 audit. All remain required.
  Real external staging Telegram delivery remains explicitly unperformed.

- Broad runtime build completed6m22s; initial run stopped after24/46:22 passed,
 2 failed,22 not run (default fail-fast). No broad acceptance pass claimed.
  Budget recovery compares BSON Decimal128 zero encodings rather than semantic
  Credits (actual Mongo zero exponent differs); value-preserving assertion fix
  required, not weakening the refund invariant. Readiness test waited for a POST
  that never occurred; independent continuation found the environment cause below.
- Primary pinned-binary continuation found funded guide/customer/models fail because
  task replica Mongo reports wire_version21 (7.x), while production billing bulk
  writes require8.0+. Do NOT bypass billing or count these as settlement proof.
  Primary is provisioning Mongo8+; same binary will rerun without rebuilding.
- Separate source regressions to fix after freeze: semantic Decimal128 comparison
  in budget/stalled-notice tests; legacy malformed personal-manager channel label
  now yields500 instead of its former isolated200 ACK. Transient platform DB
  pre-admission failures must still retry. Source/binary remain unchanged.
- Primary pinned-binary compatibility: standalone disable1/1 and delete1/1 on27120,
  route_context8/8, reservation5/5, conversations40/40, routing13/13, channelbot18/18
  passed. Telegram67/68 with the malformed-channel regression above. Manifest1048
  and binary checksum unchanged. Standalone successes are compatibility evidence
  only, not platform atomicity/billing evidence.

- Primary provisioned isolated Mongo8.0.16/wire25 replica set on27121,
  `mongodb://127.0.0.1:27121/?replicaSet=nyxidBotfather8`; transaction abort and
  production admin bulkWrite probes succeeded. ALL subsequent runtime tests use
  this URI. Primary owns lifecycle; old27119/standalone27120 are untouched.
- Unchanged binary on Mongo8 ran38/44 pass,6 failures,28.15s:
  `/tmp/nyxid-botfather-primary-lifecycle-mongo8.log`. Readiness demotion/service
  drift now pass. Remaining failures: two zero exponent comparisons; three tests
  asserted funding before deferred settlement drained; post-activation assessment.
- Worker reproduced activation failure personally with the pinned binary on Mongo8:
  `/tmp/nyxid-botfather-worker-activation-reproduce.log` (1 failed). Concurrent
  metadata-only probe `/tmp/nyxid-botfather-worker-activation-metadata.log` captured
  active installation/channel, exact approved revision1 and managed retired=true.
  Registration consumes creation identity by setting retired=true; assessment
  incorrectly required unconsumed. Fix requires consumed identity AND retains all
  live channel/owner/revision/observation fences. No blind removal of checks.
- Money assertions now require Decimal128 storage and decode to exact Credits;
  tests drain existing production settlement recovery before asserting nonzero
  wallet/ledger/checkpoint. Production remains deferred. Customer counts exclude
  unforwarded activation funding probe. Funded fixture explicitly requiresMongo8.
- Legacy ordinary malformed manager channel only isolates BSON deserialization
  failure when no profile/marker claims it; all transient DB errors and claimed
  target failures still propagate/retry. Native control errors remain isolated.
- Freeze released by primary after all pinned suites. Fix batch typecheck pending
  `/tmp/nyxid-botfather-mongo8-fixes-check.log`; fixes not yet runtime-verified.


### Gateway recovery and Mongo8 correction checkpoint

- Admin-only gateway list/inspect/review/resolve controls are now mounted locally.
  Three database tests cover reviewed version drift, demotion, no-delete resolutions,
  same-version retry and stale cleanup claims. Runtime results pending.
- Fixed semantic Decimal128 assertions and drained existing production deferred
  settlements in funded tests. No synchronous production settlement change.
- Active assessment requires consumed native creation identity (`retired=true`),
  while preserving live channel/owner/observation/revision fences.
- Active customer receipt recovery no longer applies guide binding-expiry rules;
  shared runtime fences still apply. The distinct customer test now exercises actual
  native send payload, fixed staff-confirmation copy and expired-binding recovery.
- Actual token-priced guide test reserves valid128-token output plus bounded input,
  asserts15-token0.015 settlement and cap denial before a second provider call.
- Lean check passed2m50s `/tmp/nyxid-botfather-gateway-token-check.log`; later
  customer send/recovery edits are included in the frozen runtime compile below.
- Frozen1046 Rust/Cargo files (backend/cli/machine plus root Cargo files), manifest
  `/tmp/nyxid-botfather-gateway-token-source.json`, canonical sorted JSON digest
  `5b8892f9e4d081bbed14b623c42ced887c3c6f8aed0488ab96b812a7132477cb`.
  Mongo8 URI27121, lean flags, nextest `--no-fail-fast --test-threads 2`, all platform
  modules plus gateway recovery. Session49274; log
  `/tmp/nyxid-botfather-gateway-token-runtime.log`. Compile/run pending; no pass claimed.
- Worker frontend focused3files12/12 passed, `/tmp/nyxid-botfather-gateway-copy-ui.log`.
  Initial tsc failed because the newly directly rendered gateway test's mock profile
  omitted required channel/execution fields; fixture corrected, rerun pending.
- Primary full frontend build exit0, `/tmp/nyxid-botfather-primary-frontend-build.log`.
  Primary mocked gateway browser actions both widths passed (exact review POST,
  disabled before confirmation, task disappearance); this is not real gateway API evidence.
- Admin UI explains profile edits pause existing client replies until renewed review.
  Quota copy uses singular credit for1. Restaurant customer send includes deterministic
  staff-confirmation wording with final message bound including test suffix.

- Corrected frontend test fixture TypeScript rerun exit0:
  `/tmp/nyxid-botfather-gateway-copy-tsc-2.log`.
  Seven focused onboarding/schema/idempotency/privacy files54/54 passed:
  `/tmp/nyxid-botfather-gateway-frontend-final.log`. Includes gateway exact-review
  checkbox/body/generation drift and singular-credit provisioning snapshot.
- Canonical `TELEGRAM_NEW.md` and `CHANNEL_BOT_RELAY.md` now describe explicit
  opt-in compatibility, native closed receiver, ordinary setter exclusion,
  funding/activation and safe gateway recovery. Production remains untouched.

- Read-only acceptance audit during freeze found a remaining UX contract gap:
  guide dispatch safely drops nonprivate messages, but does not yet send the plan's
  bounded generic private-chat handoff for groups. Add only fixed public help under
  live manager/profile authority and group-scoped admission; never binding/status data.
  This is outstanding implementation, not a completed acceptance claim.
- Changed frontend scope ESLint exit0 `/tmp/nyxid-botfather-gateway-copy-lint.log`.

- Frozen Mongo8 runtime completed:50 run,48 passed,2 failed,0 leaked; nextest
  `0655019a-64f6-43bf-8ab5-05a1a940d346`, build6m02s/run57.801s.
  `/tmp/nyxid-botfather-gateway-token-runtime.log`.
  All three gateway recovery tests and real token-priced admission passed.
  Definite refusal/deferred settlement, model discovery operation/payer/headers,
  both abandoned-budget tests, demotion/service readiness races and legacy native
  command ACK/retry regression passed. No broad all-pass claim.
- Remaining failures: exact checkpoint expectation omitted the fixture's opening
 10000-credit posting (actual9999.999, expected-0.001); expanded customer test
  SIGABRT stack overflow. Investigate/box large futures rather than raise stack limits.
  Active customer send/recovery proof therefore remains unverified in this run.
- Binary `/target/debug/deps/nyxid_server-ffcf344f44c98ff7` SHA256
  `97d6f670cbd526c70ac717f99a4f8576787cd23a612bb731a08b97ac2c576992`.
  Rust/binary remain frozen for primary same-binary legacy check; no worker Cargo
  remains. Formatting-only customer assertion wrapping also awaits release.


### Signed-ingress / delayed-preparation acceptance batch

- Primary pinned Telegram compatibility68/68 passed onMongo8,42.19s:
  `/tmp/nyxid-botfather-primary-gateway-telegram.log`. Former malformed ordinary
  manager ACK regression is fixed. Primary released Rust freeze.
- Exact funded-owner helper now asserts the actual `wallet_funded`10000-credit
  debit posting, exact usage postings and checkpoint/wallet10000-minus-usage.
  It no longer treats a provisioned wallet as an initially zero checkpoint.
- Large inference and customer polling futures are boxed in production boundaries;
  funded fixtures/calls are boxed in tests. No RUST_MIN_STACK override is used.
  Stack-overflow fix awaits runtime proof.
- Text and model discovery now revalidate captured service execution inputs after
  billing preparation: live catalog ACL/readiness projection/effective provider auth,
  in-memory credential/default-header equality. Credentials are never serialized
  into filters, logs or diagnostics. A FakeLago wallet-preparation barrier proves
  eligible URL drift and credential replacement cannot dispatch captured targets.
- Written cases (NOT yet runtime passes):
  - signed child webhook rejects wrong signature; injected pre-receipt DB failure
    returns503; restored redelivery twice admits one model call and actual staff-
    confirmation send, with no paid replay; expired onboarding binding recovery
    sends one fixed native notice with no extra model call;
  - child private/group/supergroup/channel/topic ordinary routes/defaults resolve
    closed; ordinary lifecycle target denied even to the owning client;
  - actual token-priced empty sponsor wallet refuses before provider;
  - two independent concurrent admission transactions race on exact budget and
    global concurrency, only one wins, pre-forward refund returns exact zero;
  - delayed headers plus profile pause cancel in-flight model work; uncertain
    dispatch retains conservative budget and no reply is emitted;
  - group manager handoff is fixed public copy, deduped and carries no setup data;
  - manual registration vs observation races behind held production settings fence,
    starting with waiting_bot/no-child/no-connected-row asserted; only authenticated
    event provenance is seeded (no pre-observing webhook);
  - production setup-notice claim/reclaim generations deny old claimant and unlink
    denies the replacement claimant without sending.
- Two intermediate lean checks passed2m45s and3m35s, overlapping later edits:
  `/tmp/nyxid-botfather-signed-ingress-check.log` and
  `/tmp/nyxid-botfather-preparation-batch-check.log`. They are diagnostic snapshots,
  not frozen acceptance. Formatting applied after prior runtime freeze release.
- Frozen all git-tracked/untracked Rust/Cargo source1082files, manifest
  `/tmp/nyxid-botfather-signed-preparation-source.json`, digest
  `8575c41c2acb2f7f7c8d57a79ad2576c404bc358bf3c82d8590fa3d20e163b54`.
  Session80535, lean nextest all platform+gateway filters,2threads,no-fail-fast,
  Mongo8 replica27121. `/tmp/nyxid-botfather-signed-preparation-runtime.log`.
  Compile/runtime pending. Rust remains frozen; no broad pass claimed.

Current concrete local acceptance remainder after this batch's results:

| Area | Still required local evidence/work |
| --- | --- |
| A1/A4/A8 | Admin/client private-service poison fixture plus explicit closed-action/credential/permission rejection through signed manager ingress; delegated/control denial response coverage |
| A2/A6 | Live signup/invite-policy integration and selected multi-setup guide/web revision continuity beyond mocked browser journey |
| A5/A9 | Independent client D installation/publication and customer/late-reply substitution across both children; unpublished/private publication absence through actual customer ingress |
| A7/A14/A16 | Reapproval at registered/connected/active stages, saved/active native connect retry and independently claimed worker revocation/registration boundary; stale competing creation forms |
| A10/A11 | Chunked-response deadline/lease recovery and delayed client-customer effects; this batch verifies preparation/slow headers and existing atomic refund paths |
| A12/A15 | Child group/owner_seen/follow/relink/alternate setters and stale route-key dispatch across lifecycle stages; manager rotation/retirement/clear with managed children |
| Operations | Full same-binary nearby compatibility, required changed-scope Clippy/fmt checks, final runbook/API/evidence reconciliation |
| External only | No real Telegram staging/deployment/containment performed. Real two-client/admin/customer creation, signed upstream events, observed A13 delivery and staging token rotation remain explicit operator validation. |

This is a remaining-work checklist, not a completion claim. Existing passed cases
remain recorded above; newly written cases only become evidence after actual execution.

- Standalone route-update compatibility decision (read-only audit during freeze):
  baselineHEAD `1819c7219b7fa0a2deb142ebce9564a086b88cb3` already calls
  `db::require_transactions` before indexes/migrations at server startup.
  `docs/SERVICE_HISTORY.md:64`: standalone Mongo fails startup, no lossy capture mode.
  Thus ordinary server route setters already have a replica/mongos deployment
  prerequisite. Keep transactional update/delete fence; no topology/error fallback.
  Direct library route updates on standalone do now require transactions; this is
  recorded explicitly, not represented as preserved standalone utility behavior.
  Existing no-platform account disable/delete direct-service compatibility remains
  separately preserved and verified on standalone27120.

- Signed/preparation frozen run finished57tests:56 passed,1 failed,0 leaked;
  nextest `4da413e7-ba2c-49b3-9ca7-36fac5dd7907`, build13m49s/run56.107s.
  `/tmp/nyxid-botfather-signed-preparation-runtime.log`.
  The stack overflow is resolved without a stack-limit override. Exact sponsor
  ledger/wallet/checkpoint assertions pass. All seven new standalone tests pass:
  group help, preparation drift, empty token wallet, concurrent budget/capacity,
  slow headers pause, manual-registration race and notice claim/unlink.
- Expanded customer test fails at expected503/got200 before its new receipt fault
  admission: fixture omitted `from.is_bot=false`, so production's strict verified
  sender parser intentionally returnsNone and ACKs. Correct realistic update and
  assert sender precondition after primary releases freeze. Do not weaken parser
  or error assertion. Earlier direct activation/customer nonzero exact ledger
  assertions execute before this failure; new signed send/recovery remain pending.
- Pinned binary SHA256
  `05856552ecf233f23d78c0abb970873f1abf5dd7f11dbbb069df4135f0f2b30d`.
  Rust remains frozen for primary delegated/invite/Telegram/routing compatibility.
  No worker Cargo remains. Manifest unchanged; fmt and diff checks passed.

- Primary new pinned compatibility: delegated/auth/invitation27/27 pass1.00s,
  `/tmp/nyxid-botfather-primary-signed-auth.log`; Telegram68/68 pass37.10s,
  `/tmp/nyxid-botfather-primary-signed-telegram.log`. Manifest1082 unchanged.
  Primary released freeze; no primary jobs remain.
- Race-diagnosis correction: top-level `managed_bot` returns before observation;
  `message.managed_bot_created` invokes observation. The original race fixture
  lacked complete created_by/observation/created_at provenance, rather than having
  pre-completed observation. Explicit provenance seed + waiting_bot/nochild
  preconditions is the correct isolated race test, and that version passed.
  No production change is needed for this correction.
- Primary independently confirmed baseline startup transaction prerequisite and
  closed standalone route-compatibility candidate. Primary stopped only obsolete
  task-owned Mongo7/standalone processes; all future tests still use Mongo8 replica27121.

### Combined acceptance integration (runtime pending)

- Primary authorized exactly two bounded acceptance workers. Provisioning owns its service and dedicated test file; lifecycle owns its dedicated test file. Main worker retains shared/native/inference/setup files and Cargo coordination.
- Registered both dedicated acceptance modules and exposed the existing real funded fixture through test-only support forwarding. No billing implementation was duplicated.
- Native finish_request now advances installation and queues ready notice only after the exact provisioning setup transition; installation transition must also match provisioning or the transaction aborts. Active/ready retries preserve state. Provisioning worker independently fixed the analogous Connected-step replay and added an active live-claim regression.
- Added real registration-policy/login/owned-resume case, corrected signed customer sender fixture, and expanded same-database/profile second-client runtime isolation plus closed admin-chat cases. These additions are written, not runtime evidence yet.
- Combined lean cargo check launched: /tmp/nyxid-botfather-combined-acceptance-check.log. No result recorded yet. No global formatting during concurrent worker edits; per-file rustfmt and git diff --check passed for main integration files.

- Build coordination correction: the first combined check queued behind our older pre-integration check. Canceled the queued process (exit130), then terminated only our older check/rustc (no result) and relaunched combined check. Other-worktree builds were identified by cwd and untouched. Combined active session75486; no check success claimed.
- Frozen combined source manifest: /tmp/nyxid-botfather-combined-acceptance-source.json, 1084 Rust/Cargo files, aggregate65c6f6ba03c87de6070bb5fc374feb1cee162e436151e5d5d064298db26cf38a. Both acceptance workers report source ready and no Cargo runs. Native and step Connected retries now preserve active state; primary reviewed both source paths. Runtime remains pending.
- Remaining local cases after this checkpoint still include signed selected web/native facts continuity with concurrent web edit, whole-response streaming/deadline proof, and any findings from combined acceptance runtime. Written new signup/admin-chat/same-DB-client/reapproval/lifecycle cases must not be counted as passing before the run.

- Combined check reported two missing Document references in new signup test and E0283 untyped lifecycle insertion collection. Main added import; assigned lifecycle worker explicitly typed all three model insertions and passed per-file formatting. Check was interrupted exit130 after collecting these diagnostics; NOT a successful typecheck.
- Corrected frozen manifest /tmp/nyxid-botfather-combined-acceptance-source-2.json:1084 files, aggregate7f33e0d6d220862658cabf642411f93abe763ee997fa67b159fcfe3ebb6bac14.
- Combined runtime launched session95213, log /tmp/nyxid-botfather-combined-acceptance-runtime.log. Exact command: `NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27121/?replicaSet=nyxidBotfather8' CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 cargo nextest run -p nyxid --bin nyxid-server --test-threads 2 --no-fail-fast -E 'test(platform_onboarding) | test(platform_inference_service) | test(platform_runtime_service) | test(platform_native_receipt_service) | test(platform_setup_notice_service) | test(platform_provisioning_acceptance_tests) | test(platform_lifecycle_acceptance_tests)'`. Compile/runtime results pending. Rust/Cargo remains frozen; no binary overwrite beyond this coordinated build.

- Session95213 resumed under main worker ownership after handoff. Frozen1084-file manifest verified unchanged. Runtime compilation reports three E0308 errors only so far in signup_journey_tests: AppState.config is AppConfig, not Arc<AppConfig>; replace Arc::make_mut with direct test-config assignments after diagnostic completion. No runtime tests have started and no success is claimed. Worker acceptance files have no reported diagnostics in this snapshot.

- First combined runtime compilation reported three E0308 errors in signup test config mutation (plain AppConfig mistaken for Arc). No additional diagnostics appeared; interrupted exit130 after ~14min active compilation, zero tests run. Direct assignments fixed all three; per-file rustfmt fixed import order, git diff --check exited0. Read-only cargo fmt check before fix exited1 solely for import order; not counted as passing.
- Corrected combined runtime session51486 launched with the exact prior Mongo8/lean/two-thread/no-fail-fast command. Log /tmp/nyxid-botfather-combined-acceptance-runtime-2.log. Frozen source-3 manifest1084 files aggregate1c4f6bba2e7e054285abd327dc100daf582faca9bb424a750ebaeee0f36299ba. Result pending. Main continues ownership; no completion claim.

- Corrected frozen source formatting: `cargo fmt --all -- --check` exited0, /tmp/nyxid-botfather-combined-acceptance-format-2.log. Source-3 manifest reverified1084 files unchanged. Combined runtime session51486 still compiling with no diagnostics at this check; no runtime result yet.

- Combined runtime-2 compilation was interrupted after 1:24:42 elapsed (no tests started). Compiler was in code generation/uninterruptible I/O; disk fell from 15GiB to 5.3GiB amid unrelated concurrent worktree builds. Terminated only cargo PID71040/rustc PID71172 for this worktree; no other process/artifacts touched. Log /tmp/nyxid-botfather-combined-acceptance-runtime-2.log ends at compile output with no diagnostics. This is an environmental interruption, not a pass/fail result. Source-3 manifest and fmt check remain valid; Rust freeze released.
- Remaining written continuity/deadline drafts are outside the repository and explicitly not evidence. Need a later lean coordinated runtime after unrelated target pressure is relieved.
