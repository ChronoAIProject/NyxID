# Public BotFather and personal NyxBot completion

Owner/reviewer: Heca agent `01a1455d-978c-7570-a0d6-ad29fa235713`.
Sole implementation worker: `01a1456f-23be-7620-ae2c-da4c4d780472`,
Codex `gpt-6.1-sol`, `x_high`, `full-access`. No additional agents.

## Product acceptance

One platform bot answers visitors' NyxID FAQs and helps them sign up or sign in.
After verified account linking, the same private chat becomes that person's
NyxBot. Login status is explicit and derived from live authority; logout returns
the chat to public behavior. Telegram additionally remains the person's main
bot manager. Existing other supported platforms provide chat surfaces, without
Telegram-native management flows.

Signup and a browser login do not prove ownership of an external chat identity.
Linking binds the verified platform sender/private conversation to the reviewed
NyxID account. The chat login is durable until revoked or otherwise invalidated;
closing a browser tab is not logout from the chat. Logout disconnects the exact
chat authority and does not log the user out of unrelated devices or erase their
saved NyxID history.

## Implementation sequence

1. Map the current paths to the acceptance table and reuse the existing account,
   channel, billing, and public-runtime implementation.
2. Complete the missing upstream public runtime, then verify the real NyxID and
   NyxAgent wire contract together.
3. Complete explicit login, status, logout, guidance, and account transitions.
4. Verify personal chat parity and Telegram bot management; implement the missing
   non-Telegram chat behavior through authentic platform identity and transport.
5. Exercise successful journeys, isolation, rejection, concurrency, recovery,
   cancellation, funding, and platform compatibility.
6. Primary reviews frozen checkpoints, sends findings to the same worker, and
   independently verifies fixes. Final implementation is accepted only after
   every recorded finding has a resolution and its required evidence.

## Acceptance table

All rows begin pending. Source inspection, compilation, simulated transport,
cross-service execution, and live platform delivery are distinct evidence.

| ID | Required behavior | Evidence needed | State |
| --- | --- | --- | --- |
| A1 | Visitor receives useful public FAQ answers and signup/login guidance | Public execution reaches approved FAQ and signup tools through the implemented upstream | Pending |
| A2 | Signup/sign-in preserves the intended chat link through verification | New and existing account journeys; invitation/email/MFA policy respected | Pending |
| A3 | Personal access begins only for the verified sender and reviewed account | Same-chat proof; wrong-account, forwarded, replayed, group, expired, and substituted inputs rejected | Pending |
| A4 | Chat status reports live login state clearly | Logged-out, pending, linked, disabled-account and revoked cases; no other user's information | Pending |
| A5 | Logout restores public behavior immediately | Binding and pending-proof revocation; late personal replies and effects fenced; unrelated work preserved | Pending |
| A6 | Linked chat is the user's own NyxBot | Correct user/grants/permission mode/memory/billing; web conversation continuity; two-user isolation | Pending |
| A7 | Telegram retains bot management after login | Native creation/registration and owned bot list/manage workflow; shared manager never becomes customer-owned | Pending |
| A8 | Other supported platforms are account-aware chat surfaces | Verified native identity and direct/gateway delivery; Telegram management routing absent; existing channels retain behavior | Pending |
| A9 | Public execution cannot acquire personal capabilities | Real implementation enforces public tools, dedicated credentials, callback routing, profile/build binding and model limits | Pending |
| A10 | Funding and limits match the selected audience | Sponsor-paid public calls; personal payer after login; exact budgets, concurrency and failure settlement | Pending |
| A11 | Retries, cancellation and cleanup preserve authority and delivery | Duplicate ingress, uncertain sends, late replies, restart recovery, absolute expiry, cleanup-before-execution and revoked-key cleanup | Pending |
| A12 | Admin setup works and reports actionable failures | Discovery/readiness/enable journey, stale revisions, changed deployment, preserved explicit settings and no partial activation | Pending |
| A13 | Complete source passes appropriate checks and independent review | Final revisions, commands, outputs, compatibility coverage and zero open findings recorded below | Pending |
| A14 | Intended deployment and live chat journey work | Reviewed deployed revisions; real visitor/login/logout and Telegram management observations | Pending |

## Review rules

- Worker owns production code and tests; primary owns this review record and
  performs the independent review personally.
- Every discovered issue is tracked, including lower-severity behavior, tests,
  wording, maintainability and operational problems. A claimed fix is rechecked.
- Do not weaken authority checks, substitute a personal-only rollout, invent
  runtime proof, skip required tests, or equate a mocked send with user receipt.
- Serialize builds and preserve unrelated processes/artifacts. Source must be
  identified for each result; tests on changing files are diagnostic only.
- No unreviewed production enablement, rollout, or real outbound chat messages.
  Prepare a concrete deployment and live-test procedure after implementation.
- Any unresolved dependency remains open and prevents a completion claim.

## Baselines and confirmed blocker

NyxID baseline: `5aed82c57`. NyxAgent inspected baseline:
`57ab8be89618a61b98e2ebc76331d121f96d731b`.
The [live diagnosis](2026-10-16-platform-botfather-live-diagnosis.md) records the
actual `public_contract_unsupported` result and uninitialized manager runtime.
The [public runtime contract](2026-10-16-platform-public-runtime-contract.md) and
[wire fixture](../fixtures/platform-public-v1.json) are the existing integration
boundary; any necessary changes must be implemented and tested at both ends.

## Findings and evidence

Controls source checkpoint `87fccc0cd` is available for independent review.
The worker reports formatting/diff checks passed; no Rust compile or test pass
is attached. No implementation checkpoint has been accepted yet.

### Cross-platform decision

Generalize the existing dedicated platform-manager authority through explicitly
opted-in per-platform entry profiles. Keep a single public/personal linking,
revocation, execution and billing pipeline. Platform, bot/channel, sender and
private-conversation identity must remain authentic and namespaced; opaque IDs
must never be coerced into Telegram integers. Preserve legacy Telegram records
and wire compatibility. Native Telegram onboarding remains a platform capability.
Direct and supported gateway ingress must intercept opted-in entries before
ordinary owner/guest routing and fail closed after retirement. Existing ordinary
channels retain their behavior.

### Controls review, in progress

Source inspected: current changes to command parsing, login-state projection,
delivery validation and exact unlink reuse. These observations are preliminary
until a frozen checkpoint and executed tests are available.

| ID | Finding | Required resolution | State |
| --- | --- | --- | --- |
| R1 | Final-send revalidation covers `Status` only. Delayed `Confirmed`, `Unlinked`, and login-link replies can describe a state that changed before the send claim. | Bind all stateful command replies to their committed effect/proof; reject or regenerate stale acknowledgements. Test confirm→logout, logout→new-link/confirm, and login-link→logout before send. | Closed for Telegram at frozen build21; independent committed-effect and final-status regressions passed; native integration recheck pending |
| R2 | Login status is persisted as rendered prose in `command_status`, then used as authority identity. Text edits can invalidate pending metadata and introduce an unnecessary second database write after enqueue. | Persist typed status/effect metadata atomically with the delivery record and render canonical text at the delivery boundary. | Closed at frozen build21; typed committed effects source-reviewed and independently executed |
| R3 | All command replies use the reserved personal-send lane. Public `/status` and `/help` traffic can consume the manager's personal reserve. | Separate public control-response pacing from retained personal replies while keeping logout effects immediate; test bounded public-control load and personal progress. | Open |
| R4 | Upstream `api::respond` acquires capacity after `Responses::prepare` has persisted an `in_progress` response. Capacity rejection or draining strands an unexecuted personal/public session and makes retry return `outcome_unknown`. | Admit capacity/shutdown before prepare writes and preserve personal behavior. Test rejection, capacity restoration, and successful retry with the same idempotency key, plus shutdown. | Closed at upstream `f4da4471`; independently verified |
| R5 | NyxID generates public credentials with `AGENT_KEY_PREFIX = "nyxid_ag_"`, while upstream public execution accepts only `nyx_`. Every actual public credential is rejected. | Align the supported public credential format and add a cross-service fixture using NyxID's actual generated format; retain all signed public-authority checks. | Open |
| R6 | Upstream verifies a manifest from one resolved runtime, then `PublicRun::new` reloads policy and resolves a runtime again. A policy/runtime change between these steps can select an executable that was never checked against the signed manifest. | Carry the verified resolved runtime through execution and verify its identity at launch; add a deterministic runtime/policy-switch regression. | Closed at upstream `f4da4471`; independently verified |
| R7 | Public model output validation accepts a reasoning item, but the next request's input normalizer rejects every reasoning item. A legitimate reasoning/tool response can fail on continuation or cold resume. | Define bounded reasoning-history semantics on both services and exercise reasoning plus FAQ search/read/answer and cold resume with the real Codex smoke. | Upstream native2 real reasoning/tool/cold-resume replay passed; aligned actual NyxID pairing pending |
| R8 | Independent Codex 0.160.1 capture with the draft's public configuration emits five native tools plus the public three, extra `tool_choice`/`client_metadata`, `include:["reasoning.encrypted_content"]`, and a 10,653-byte initial body for a short question. The strict normalizer and 8,192-byte callback limit reject it. | Disable native tools through effective app-server configuration, explicitly normalize supported orchestration fields, retain bounded reasoning semantics, and prove the full real prompt/tool sequence fits affordable declared limits. | Upstream native2 pinned Codex smokes passed with confined tools and measured byte bounds; aligned NyxID execution pending |
| R9 | `Responses::execute` calls `session.touch()` before the final public fence and restores absolute expiry only after that fence succeeds. A fence failure persists the touched session through generic error handling, extending stored lifetime to now plus 24 hours. | Enforce the original absolute expiry before every success/failure persistence path; test failure at the final public fence. | Closed at upstream `f4da4471`; independently verified |
| R10 | `/setups` still reads the separate legacy onboarding binding after personal login. Without that binding it tells the user to `/link`, which now starts personal linking again; with an older binding it can resolve a different account from the current personal chat. | Make Telegram management explicitly follow the verified personal sender/account, preserve deliberate legacy compatibility, and test login→setups plus old onboarding account B→personal account A→management. Cover the retained setup-selection and owned-bot creation paths. | Closed for Telegram at frozen build21; independent current-account management, exact selection/context and hosted creation journeys passed; final native integration recheck pending |
| R11 | The public SSE validator checks only the final `response.completed.response`, then forwards earlier events unchanged. A native `view_image` call in an earlier event can execute inside Codex even when the final response contains only an allowed message. | Validate every streamed event/item and its consistency before forwarding; reject native/provider items and conflicting completions. Test a real native-call attempt and prove no local effect. | Closed at upstream `f4da4471`; independently verified |
| R12 | The control-capacity regression holds every permit of process-global `CONTROL_SENDS` across database operations. Other journey tests in parallel libtest can fail with `CapacityLimited` instead of their intended send or authority outcome. | Scope saturation to the tested transport/state and verify normal parallel test execution without a suite-wide serial requirement. | Open |
| R13 | Default registration tells users to wait for a verification email, but no SMTP sending path or frontend verification route exists. Current password login permits active unverified people, while existing signup coverage forces auto-verification and bypasses the misleading default handoff. | Make signup guidance match an executable flow under existing auth policy; prove default/configured signup→login→review. Do not silently enable auto-verification or introduce a new global login gate. | Closed at frontend f2818e928 plus build21; default/configured signup, login and MFA journeys independently passed |
| R14 | Public MCP initialization and tool listing run before the Responses turn timer. Their RPC uses the shared 150-second HTTP timeout and only a pre-request expiry check, exceeding the signed 90-second turn bound under a stalled callback. | Bound the complete RPC and body read by remaining turn/session lifetime, recheck after response, and test stalled initialization/list/tool callbacks with no late effect or orphan preparation. | Closed at upstream `f4da4471`; independently verified |
| R15 | NyxID's real discovery producer advertises `max_output_tokens:256`, while the new upstream public policy asks for 512 and rejects a smaller catalog bound. Default discovery therefore fails before readiness. | Align actual discovery producer bounds with the intended public policy and verify using a fixture produced by NyxID, retaining full-loop affordability checks. | Open |
| R16 | `/logout` on an already revoked binding leaves its prior revocation reason unchanged. Sender resolution continues returning `Unavailable` after owner retirement or other revocation, despite the logout acknowledgement promising public chat. | Record the sender's logout independently of the historical revocation cause, so status and routing return to logged-out/public while old personal authority remains denied. Exercise actual owner/control retirement before logout, not only an inactive user with an active binding. | Closed for Telegram at frozen build21; real owner/control retirement then logout journey independently passed |
| R17 | `CommittedCommand` exposes independent effect/reply fields. For `Link`, final send checks the intent metadata but does not bind the ephemeral bearer to that exact intent; an internally substituted effect/token pair can cross the intended proof association. | Constrain command construction or validate the exact token-to-intent association before sending. Add a substituted bearer/effect regression. This is an internal invariant gap; no external injection path is claimed. | Closed at frozen build21; exact substituted bearer/effect regression independently passed |
| R18 | Public automatic compaction is configured at 6,000 tokens while every model body is capped at 8,192 bytes. A real Codex capture of two short search/read/signup turns grows past the byte ceiling before compaction triggers. | Implement byte-aware bounded continuation/compaction within the signed call, token and cost limits; preserve substantive reasoning. Prove real multi-turn and cold-resume behavior through the bridge, including compaction. | Upstream native2 byte-bound compaction and cold full-loop replay independently passed; paired NyxID execution pending |
| R19 | With the configured custom provider, actual Codex 0.160.1 local compaction calls `/responses` with `tools:[]`; it does not use the draft's `/responses/compact` path. The normalizer rejects the empty list, and continuation accounting only recognizes the unused compact path. | Support the real bounded local compaction grammar and count it against continuation and model budgets. Preserve no-tool compaction semantics and exercise the actual Codex sequence through the bridge. | Upstream native2 actual no-tool compact callback and continuation accounting passed; paired NyxID execution pending |
| R20 | Public execution fetches JWKS and resolves/hashes runtime before capacity admission; the new public control endpoints have no bounded admission. A shaped unsigned header can trigger outbound verification work while execution is saturated, using the shared 150-second client timeout. | Bound public pre-auth/control concurrency and complete JWKS request/body duration, keeping cleanup available under execution saturation. Verify refusal has no network/storage/model effects and respects draining. | Closed at upstream `f4da4471`; independently verified |
| R21 | The draft signup/MFA backend journey submits its wrong OTP to the private router, where the public MFA verification route is absent, and only asserts non-200. A 404 satisfies this check without testing OTP rejection. | Submit the wrong code through the real MFA handler or correctly mounted public route, assert the specific rejection and no account/chat authority, then prove the valid retry reaches review and same-chat confirmation. | Closed at frozen build21; real wrong-OTP rejection and valid MFA/review/confirmation journey independently passed |
| R22 | Both new signup browser cases race the registration-to-login animation. Trace proves the email fill hits registration, the non-exact `Enter your password` selector hits `Re-enter your password`, and the click selects the registration panel's navigation button. Neither case sends `/auth/login`. | Await the visible login panel, use exact/scoped login fields and submit control, and assert a login request before account review. Rerun both signup cases and the complete focused browser suite. | Closed at NyxID frontend `f2818e928`; independently verified 7/7 |
| R23 | Upstream output validation accepts any nonempty function `call_id` up to 256 bytes, and input normalization preserves it; NyxID callback history permits only ASCII alphanumeric, `_` and `-`. A `call:1` tool call can be accepted and executed before its next model continuation is refused. | Align the bounded call/output ID grammar across upstream output validation, normalization and NyxID callback validation. Preserve supported opaque IDs unchanged, or reject unsupported IDs before Codex sees them. Add paired valid/invalid call and output cases, including missing, empty, oversized and control-containing IDs. | Closed at build21 plus upstream next3; both compiled consumers passed the shared call/output grammar fixture and upstream rejects before tool dispatch |
| R24 | The new management draft promises Telegram creation for the account linked to chat but sends a generic `/channel-bots/connect/telegram-new` URL. That page uses the browser account, which can differ from the linked account. The reused `nyxid__channel_bot_setup_link` also emits a generic URL; creation under B does not complete A's owner-scoped watch. | Bind both deterministic and NyxBot creation handoffs to the current personal account using the existing hosted connect-request authority or an equivalent explicit account check. Test chat account A with browser account B and successful creation under A. | Closed at frozen build21; deterministic and natural creation reject browser account B and complete under linked account A |
| R25 | The new management draft reads setup rows directly and renders their stored state without applying live `expires_at` semantics. An overdue collecting or approval row can appear usable until the cleanup worker runs. | Share the live setup-expiry projection within management transactions and final-send validation. Test overdue rows before cleanup and preserve active setup semantics. | Closed at frozen build21; live expiry, delayed selection and prior-context preservation journeys independently passed |
| R26 | The management integration draft removes the non-personal `/confirm` handoff. Once runtime controls exist, an already-issued, browser-approved legacy onboarding challenge is consumed by reserved-command refusal before the legacy confirmation handler can run. | Retain an explicit bounded handler for original private legacy confirmations without using the old binding for personal chat or falling back to owner routing. Exercise an outstanding challenge across runtime opt-in. | Closed for Telegram at frozen build21; actual pending legacy confirmation survives runtime opt-in without selecting personal authority |
| R27 | Switching the NyxBot creation tool to hosted links requests a 120-minute lifetime, but hosted creation clamps it to 60 minutes while the tool still advertises 120. The replacement also removes the existing checked return-to-conversation target, and its 60-character labels may exceed the hosted 128-byte limit. | Derive reported lifetime from the actual created link, preserve the checked conversation return through hosted completion, and bound Unicode labels compatibly. Test expiry metadata and correct-account navigation alongside wrong-account refusal. | Closed at frozen build21; actual hosted expiry, Unicode label bound, checked return path and wrong-account refusal independently passed |
| R28 | The creation tool now mints an exact hosted channel-connect request, but its completion watch still selects the newest same-platform bot owned by the person within a time window. An unrelated bot created in another tab can satisfy the watch and be linked to the requested specialist. | Bind new watches to their hosted request and resolve only its completed `bot_id` under live owner/platform checks. Preserve deliberate compatibility for old unbound watches. Test a competing same-owner bot and the intended request's exact completion. | Closed at frozen build21; exact hosted watch, competing/replaced requests, expiry grace, newer routing and concurrent insertion regressions independently passed |
| R29 | Removing the registration verification-token debug log leaves `RegisterResult.email_verification_token` unused in production, with a stale logging comment. Check19 emits a new `dead_code` warning; remaining readers are verification tests. | Remove unused production exposure without restoring secret logging or broadly suppressing the lint. Preserve existing verification tests and registration policy; rerun the affected compilation and tests. | Closed at frozen build21; production token exposure removed, registration/verification suites passed, warning absent |
| R30 | Upstream README still claims no NyxID source changes are needed and describes NyxID's chat UI as Aevatar-only, without documenting the new paired public mode. The public deployment instructions say to merge only the `public` policy section, then describe a runtime pin that lives outside that section. | Document the paired public prerequisites and actual NyxBot integration; make the runtime-selection/pinning step explicit alongside discovery and readiness. Preserve the distinction between a local smoke wrapper and the production runtime artifact. | Closed at upstream next3 frozen source; paired prerequisites, runtime pin and local-versus-deployment distinction independently reviewed |
| R31 | The new `create_personal_telegram_in_session` requires stored `user_type: person`, while `User` defaults missing legacy fields to Person and the personal binding fence deliberately accepts those accounts. A valid linked legacy account can chat but `/onboarding` refuses its hosted creation request. | Reuse the active-person fence semantics, including missing legacy `user_type`, while continuing to reject organizations and disabled people. Exercise the real management creation path for a linked legacy person. | Closed at frozen build21; linked legacy person without stored user_type completes the real account-bound creation journey |
| R32 | The A8 plan reuses text controls, but Slack's current adapter accepts JSON Events API messages rather than native form-encoded slash-command requests. Discord's adapter does not install application commands. Fabricating normalized `/login` or `/confirm` text cannot prove users can actually invoke those controls. | Implement a usable native control surface for each selected transport and align help and browser confirmation instructions with it. Exercise actual event formats, installation prerequisites where applicable, and normal FAQ/personal messages after linking. | Open; cross-platform implementation prerequisite |
| R33 | The queued funded-account test expects wallet `balance_credits` to decrease immediately, and the paired public test measures settlement by its before/after difference. Actual settlement retains the provider-confirmed balance and increments `pending_lago_debits` until provider refresh. Both assertions contradict the real wallet protocol. | Assert the exact pending-debit delta and effective availability reduction, released reservations and grant remainder. Include readiness in the before snapshot and prove the operator wallet is unaffected. Preserve production accounting. | Open; queued test expectation finding |
| R34 | The registered retirement/logout regression queries ingress receipts using `platform_config_id`, but `PlatformManagerIngress` and its producer persist `profile_id`. Its terminal unwrap cannot find the correctly written receipt. | Correct the exact persisted-field lookup after build20 and execute the owner-retirement/logout/public-routing journey. Preserve the frozen compiler attribution. | Closed at frozen build21; corrected persisted ingress lookup and retirement/logout journey independently passed |
| R35 | The A8 gateway sketch retains native identity and control classification, but does not yet specify a commitment to the exact authenticated command arguments or ordinary text. Matching event metadata alone would let altered gateway content inherit the original receipt. | Bind a domain-separated keyed commitment to canonical native content, exact identity and selected transport generation; compare it before any gateway command or turn effect. Reject same-receipt command, code and ordinary-text substitution without retaining raw proof material. | Selected3 commitment identity/content/transport regression independently passed; final registered replay pending |
| R36 | The staged native migration unconditionally resets a ready marker to preparing, and concurrent starters may both observe an old index then race to drop it. A restart can transiently refuse native writes or fail startup despite equivalent completed work. | Keep ready migration idempotent, coordinate or verify concurrent exact-index retirement, and prove simultaneous startup plus interrupted recovery on Mongo. Prevent legacy index registration from recreating retired global constraints. | Selected3 actual Mongo concurrent/interrupted/completed migration tests independently passed; final registered startup replay pending |
| R37 | The staged Discord installer verifies bot/application IDs but does not bind the configured webhook public key to that authenticated application. The existing adapter trusts `ChannelBot.public_key` as supplied. Correct token identity alone cannot establish that signatures come from the selected Discord application. | Compare the configured verification key to the authenticated provider application's `verify_key` before native activation and fence later changes. Test mismatched-key refusal and a matched-key signed control journey. | Selected3 provider-key mismatch refusal and real signed Discord controls independently passed; final registered setup replay pending |
| R38 | The staged native credential snapshot only loads, fingerprints and supplies shared signing credentials for `credential_source: platform`. X actually uses `connection` plus shared platform webhook credentials; the existing live secret producer has a separate branch for it. Injected test secrets hide this mismatch. | Capture and fence the actual connection webhook signing source, and retain live connection owner/provider/revocation/credential-epoch authority without treating routine OAuth refresh as replacement. Exercise stored credential production and signature/connection changes before native effects. | Selected3 real stored connection signing, OAuth refresh/scope and epoch-replacement tests independently passed; final registered replay pending |
| R39 | The staged authenticated batch retains native facts and credentials but not the configuration used to authenticate/classify them. Receipt creation separately accepts a configuration, fences its bot, and fences the batch's original credentials without establishing that both refer to the same entry. | Bind the exact configuration inside the non-deserializable authenticated batch and compare it before receipt/fence writes. Reject cross-bot/platform reuse and changed transport/generation on the same bot. | Selected3 authenticated-batch substitution regression independently passed; final registered replay pending |
| R40 | The staged Discord acknowledgement branch maps every authentication-helper error to 401, including database/internal failures while loading the bot. An unavailable verifier is reported as an invalid platform signature. | Keep signature/identity refusal as 401 and classify unavailable authentication storage/state as 503. Exercise the handler with unavailable or malformed persisted state in addition to valid, invalid and stale signatures. | Selected3 actual HTTP ACK authentication and unavailable-state cases independently passed; final registered replay pending |
| R41 | Build21 reports an unreachable wildcard in the paired test's `ExecutionEvent` match after both enum variants are already covered. | Remove the redundant arm in the next justified source batch and retain warning-free final compilation. Do not interrupt the current build or delay its test execution for this cleanup. | Open; observed compiler warning, not a compilation failure |
| R42 | The staged entry-only `PlatformThread` decoder checks the platform label but accepts an unsupported native identity version or a conflicting `manager_bot_id`. Runtime controls accept matching unsupported contract/identity versions. The structural mixed-version check alone does not validate these entry-only records. | Enforce the supported version and exact entry/manager agreement before stored origins or controls can participate in authority queries. Cover unknown versions, conflicting identifiers and actual persisted origin/delivery compatibility, while preserving valid legacy proofs. | Selected3 whole-record identity tests plus literal legacy proof and retained-origin/delivery journeys independently passed; final registered replay pending |
| R43 | The staged Discord installation record retains the initial `owned_command_id`, actor and credential fingerprint forever. Initial success with `None` rejects a subsequent request carrying the now-owned command ID; credential/admin changes also refuse with no reconfiguration path. `Dispatched` with no remote candidate cannot recover, including a failure during the installer's pre-mutation GET. | Implement explicit, reviewed installation recovery and reconfiguration using exact stored ownership and current provider observations. Cover successful activation followed by revalidation, credential rotation, and an uncertain attempt with no observed command. Never recover by deleting ownership state or blindly replaying a provider upsert. | Jobs1 all 11 Discord installation cases independently passed, including credential reconfiguration and uncertain-absence recovery; final registered replay pending |
| R44 | The staged Discord installer claims ownership only by local channel ID. A pending attempt and a replacement local channel for the same authenticated application can hold separate installation rows and race Discord's name-based upsert. Ordinary registration already serializes active remote identities; the missing helper invariant concerns in-flight/replacement ownership. The current test covers only sequential pre-existing-command refusal. | Establish a unique ownership claim for the authenticated application/command namespace before remote mutation. Exercise a delayed old attempt and replacement-channel attempt, proving one owner/mutation and explicit recovery without weakening ordinary registration. | Selected3 actual application ownership, delayed/replaced dispatch and stale PATCH fencing independently passed; final registered replay pending |
| R45 | New `/bots` and `/bot` responses derive status solely from `ChannelBot.is_active`. Production credential and webhook failure paths set `status: failed` without clearing that flag, so an unavailable bot is advertised as active. Pending verification is also described as active. | Render bounded status from both operational status and the enabled flag, with safe actionable guidance. Exercise active, pending, failed and paused bots through management and preserve final-send status revalidation. | Open; source-confirmed management status defect; queued after build21 |
| R46 | The new funded account-parity journey inserts `billing_migrations/_id: exact-v2` a second time. `Journey::new` already obtains its database through `telegram_new_tests::fixture` and `connect_transaction_test_database`, whose shared helper inserts that same marker. The duplicate insert prevents the A6 assertions from running. | Reuse and assert the shared fixture's readiness marker. Preserve production billing and the common database helper; execute the actual two-account funded callback and web-continuation journey after correction. | Open; source-confirmed test setup failure; build21 source remains frozen |
| R47 | The new NyxID ManagerBinding decoder rejects explicit-null alternative projections, whitespace-padded native bot IDs and noncanonical Telegram manager strings, while the verified upstream decoder still accepts them through optional-field collapse and looser validation. Canonical producer output remains compatible. | Align the accepted identity wire grammar on both services and add shared valid/invalid JSON cases while preserving canonical v1 serialization and digest bytes. | Narrow compiled grammar parity independently passed: upstream next3 plus exact staged NyxID model, shared 32 JSON/BSON cases and legacy bytes; registered backend and opaque runtime pairing pending |
| R48 | The staged Discord command projection ignores `nsfw`, so an otherwise exact owned command can be accepted while hidden from ordinary DM users. Its schema omits that field on repair, while nested options are compared as raw provider JSON and may reject equivalent response metadata/defaults. | Explicitly author and verify public availability; compare a bounded semantic option projection that retains real constraints and handles documented provider metadata. Cover restricted owned-command repair and realistic create/list responses without adopting unrelated commands. | Selected3 actual command creation/repair and semantic projection tests independently passed; final registered setup replay pending |
| R49 | Native reply capture fences the connection epoch and credential fingerprint, but non-Discord send subsequently calls the live token resolver. A same-ID credential replacement between claim and materialization can provide a successor token while the receipt and signing secrets still describe the captured credentials. | Carry snapshot-bound send material across the final claim, allowing same-epoch OAuth refresh without admitting a replacement epoch/account. Exercise replacement between claim and materialization plus compatible refresh through the actual producer. | Selected3 captured-token replacement/refreshed-epoch and real producer tests independently passed; final registered replay pending |
| R50 | Native reply code establishes a deadline but awaits Discord handle decryption and non-Discord token resolution/signing-secret decryption outside the timeout. A stalled OAuth refresh or key provider can exceed the bounded delivery lifetime before the timed provider request begins. | Bound complete materialization, dispatch and response processing by the retained deadline, with accurate refusal versus uncertain-effect handling. Test a stalled resolver/decrypt path and zero late provider sends. | Jobs1 native materialization/claim/cleanup/persistence deadlines and actual paid-X header/body uncertainty accounting independently passed; final registered replay pending |
| R51 | Native Discord no-fallback tests observe the local follow-up server but instantiate the ordinary adapter with its production API base. An accidental generic channel send could bypass that observation, and a remote refusal could still satisfy the expected `Refused` result. | Bind every potential test egress to local fixtures or an explicit counted refusing adapter; assert zero generic sends on successful interaction replies and all failure/deadline paths. Preserve actual signature/receipt production. | Selected3 loopback-only actual adapter success/refusal/decrypt cases independently passed; final registered replay pending |
| R52 | The external native draft has two verified-update constructors: one uses the receipt ID as `observation_id`, while `NativeAdmission::verified_update` uses the credential fingerprint. Native manager validation interprets this field as the retained receipt ID, so the latter constructor cannot pass its fence. The native reply fixture also hand-builds the fingerprint form. | Keep one canonical retained-receipt constructor and exercise real native receipt-to-manager admission, controls and reply capture using it. Remove the inconsistent projection and handcrafted substitute. | Selected3 canonical retained-receipt admission, command and actual final delivery claim independently passed; complete native personal journey pending |
| R53 | The actual paired discovery test serves `keys: [{}]`: `Handshake::new` uses a shared test AppState whose `jwk_json` is empty, then mounts the real JWKS handler without initializing the matching public key. Its signed assertions therefore fail upstream verification before readiness or model execution. | Give the fixture matching real signing keys and a public JWK through the actual key producer; preserve strict upstream verification and rerun the actual pair. | Open; reproduced by frozen direct execution and local observation proxy, upstream returns 403 `public_authority_rejected` |
| R54 | Build22 adds `LocalTransport.personal_gate` for the deterministic R12 regression, but the account-parity fixture's initializer omits that required field. | Initialize that ordinary fixture with `personal_gate: None`, retain source attribution for the interrupted attempt and rebuild the coherent batch. | Missing initializer corrected; complete copied metadata and selected3 codegen passed, final registered build still pending |
| R55 | The staged identity migration renews its 120-second ownership lease only between MongoDB operations. A longer index build or collection update expires the lease before the next renewal/final-ready write, and an expired worker can still finish an in-flight retirement while a successor starts. | Keep the owner-fenced lease alive across long migration operations, stop and join renewal on completion/cancellation, and refuse subsequent retirement/readiness after ownership loss. Cover a short-lease stalled operation and actual ownership loss without changing the registered build. | Selected3 integrated migration module independently passed six real-Mongo cases including heartbeat/publication overlap and ownership loss; final startup replay pending |
| R56 | The staged shared gateway authenticates through `validate_api_key` and retains the returned pre-touch `state_version`. That validator increments the stored version while touching `last_used_at`, so the subsequent exact `fence_key` rejects every valid primary bearer. Its child credential result is also discarded. | Capture and revalidate the actual post-validation credential authority, preserving concurrent mutation/revocation fences; explicitly reject unsupported child bearers or fence their live authority. Exercise actual authentication through shared handler success and mutation paths. | Jobs1 actual gateway primary authentication, benign touches, mutation refusal and complete local setup suite independently passed; final registered replay pending |
| R57 | Staged frontend opaque identity validation accepts C1 controls and measures its 256 limit in UTF-16 code units; both Rust identity consumers reject C1 controls and bound UTF-8 bytes. This is a client schema consistency issue, not an established authority bypass. | Match the server control-character and byte rules and cover C1 plus multibyte length boundaries without reintroducing the lint violation. | Closed for the external UI schema: primary isolated exact-source tests 10/0 and targeted ESLint pass; registration pending |
| R58 | The staged receipt configuration fence joins the selected gateway, but the shared manager read/fence helper used by commands and personal effects checks only native receipt, profile, credentials and bot. Retiring the gateway leaves those rows active, so work verified before retirement can still commit later authority or effects before final delivery refuses. | Reuse a selected-transport read/fence evaluator throughout shared manager authority checks; cover retirement and route/provider key revocation between gateway verification and actual control/turn/effect claims, preserving explicit cleanup semantics. | Jobs1 gateway verification-to-command retirement/revocation fences and reviewed selected-transport recovery independently passed; final registered replay pending |
| R59 | New native administrative conversion passes `billing: None` to X account verification. Production XAdapter refuses that with BillingNotConfigured, while its local test adapter has a cfg(test) bypass that can hide the failure. | Supply the existing channel verification billing context and retain live setup authorization; execute local HTTP verification with real funding/metering and insufficient-credit refusal. | Selected3 actual paid X conversion and unfunded zero-effect case independently passed; final registered replay pending |
| R60 | The staged native gateway producer does not match the actual CMAEG consumer: `groups: disabled` is invalid; normalized Discord/Slack/WhatsApp sources omit required trust; Lark/Feishu forward no required raw event and pass app ID where bot open ID is required; X is absent from the gateway platform enum. | Validate generated requests and forwarded callbacks against the actual gateway. Expose only supported selected transports before marking pending, preserve authenticated raw evidence where required, and use the documented reserved-record recovery protocol. Never silently fall back or alias an unsupported platform. | Actual-CMA grammar replays and jobs1 local gateway setup/recovery/handler suites independently passed; final registered and live transport acceptance pending |
| R61 | Native pending conversion writes the sticky ordinary-route refusal before evaluating existing pending transport/shared-route-key blockers. Final conversion can then reject with instructions to cancel or change an ordinary route, but the new marker makes those target-route operations refuse. | Evaluate deterministic conversion blockers in the initial fenced claim and retain final checks; provide exact pending-attempt recovery for later drift without deleting separation markers. Cover a real pending setup with an active route and a shared-key preflight. | Jobs1 pending/shared-key preflight, retained setup recovery and successor admin/funding cases independently passed; final registered replay pending |
| R62 | Native gateway setup validates the sealed remote channel ID with the 256-byte native sender helper. An actual CMA pointer for `https://cma.example` seals to 264 bytes, so both observation and configuration validation refuse legitimate setup. | Use a dedicated gateway-ID contract matching the consumer’s bounded sealed identifier, preserve native sender limits, and verify actual sealed IDs through recovery/configuration/UI. | Actual-CMA sealed-ID replay and jobs1 local create/binding/observation/activation suite independently passed; final registered replay pending |
| R63 | Shared gateway Responses rejects a missing partition with generic 403. CMA creates the first conversation only after HTTP404 with a conversation code, so a new real sender cannot reach the implemented PUT-conversation endpoint. | Return a distinct authenticated missing-partition outcome mapped to protocol404 conversation_not_found; exercise first callback without precreating a partition and retain forged/retired authority refusal. | Jobs1 actual gateway first-event404, conversation PUT, successful retry and local setup independently passed; final registered replay pending |
| R64 | Shared gateway success emits only response.completed. CMA resolves acceptance only on response.created or response.in_progress, so a native effect can finish while gateway acceptance fails. | Emit a valid acceptance followed by empty completion and verify actual stream-consumer acceptance with one native delivery and no gateway duplicate. | Jobs1 actual gateway acceptance/empty completion and single native delivery independently passed with local setup; final registered replay pending |
| R65 | Existing repair HTTP accepts a native profile ID/generation and calls Telegram configure_manager before native verification fails. Native UI hides the button but the API can mutate the unrelated Telegram manager. | Refuse incompatible identity before Telegram credential/provider work and guard the service caller; test native repair has zero Telegram effects with legacy positive compatibility. | Selected3 real native repair HTTP zero-effect refusal and legacy positive independently passed; final registered replay pending |
| R66 | Native pending and final conversion check funding membership, but the intervening paid verification admission and Discord installer dispatch fence omit it. Revoking funding-org authority after pending can still permit provider costs/mutations before final conversion refuses. | Reuse live funding authorization at actual verification/installer effect claims; test revocation after pending and during discovery, zero provider/billing effects, and exact retained recovery. | Selected3 real funding revocation after pending and installer live admin/funding fences independently passed; final registered replay pending |
| R67 | An activated native profile has no reachable revalidation/update path: native conversion and inspection require no platform_profile_id, while legacy verification/repair correctly rejects native. Real signing credential rotation, connection replacement or selected gateway retirement leaves the entry unavailable with no supported recovery. | Implement the planned reviewed native profile update API/UI using live admin/funding/provider/ownership fences. Preserve permanent routing separation, invalidate prior receipts/proofs/turns through existing epoch/generation revocation, and return paused for explicit enablement. Exercise active direct credential replacement and selected gateway recovery without fallback. | Jobs1 reviewed active direct rotation/reenable and selected-gateway replacement/recovery cases independently passed; final registered replay pending |
| R68 | Native admin cards derive an unqualified Enabled/Paused badge from the legacy guide's profile.enabled. Native conversion sets it false and public/personal runtime changes do not update it, so a serving native entry still appears Paused. Native cards also promise Telegram client-setup and creation-webhook behavior. | Render native lifecycle/runtime state truthfully, or restrict the guide-only badge to Telegram. Use native-specific setup/repair guidance and cover enabled, paused, retired and unavailable/loading native states in UI tests. | Exact external correction independently verified: status3 UI34/0, full TypeScript and targeted lint pass; coherent final registration still pending |
| R69 | The new native lifecycle review hashes complete raw bot and gateway documents. Ordinary command/admission traffic changes effect counters and activity metadata in those rows, invalidating an otherwise unchanged administrator review before reservation, including while runtime audiences are paused. | Compare an explicit security/lifecycle projection that excludes benign counters and telemetry, then fence the exact live row inside the reservation transaction. Test benign command/key touches preserve a review while credential, routing, profile and gateway ownership changes still refuse. | Jobs1 real command/key-touch gateway review and same-epoch OAuth scope/refresh cases independently passed; final registered replay pending |
| R70 | Initial native public-probe consent allows absent runtime controls. If selected gateway authority is revoked after review, shared manager fencing returns None, but the no-controls fallback checks only native credentials and bot state and accepts that None with consent.controls_id=None. | Reuse the full native selected-transport, pending-marker and credential evaluator in initial funded-probe fencing. Exercise a freshly converted native gateway with no controls and revoke its gateway/provider/route key between review and funding; require zero provider calls, reservations and meters. | Jobs1 initial-consent funding regression independently passed matched positive plus four drift refusals; actual full readiness pairing still fails before Ready and remains open |
| R71 | Newly issued shared gateway keys retain sub-millisecond created_at in memory. After BSON persistence, fence_key compares the complete security projection against the millisecond-rounded stored key; first dispatch therefore refuses before any gateway POST. | Compare the same persisted representation or retain normalized/persisted key snapshots without relaxing credential, expiry or live mutation checks. Add a nonzero-submillisecond issuance/roundtrip positive and preserve mutation refusals. | Jobs1 real gateway creation/activation with nonzero-submillisecond issuance and mutation refusals independently passed; final registered replay pending |
| R72 | A missing or expired native reply handle raises Forbidden during the final capture transaction. The send result branch only classifies Conflict/NotFound as terminal refusal, so the expected authority failure escapes and leaves the pending delivery for recovery. | Classify expected native final-capture authority failures and persist refusal under the same absolute deadline, preserving unexpected storage errors and zero provider sends. | Jobs1 real command missing/expired-handle refusal and bounded cleanup regressions independently passed; final registered replay pending |
| R73 | Selected native admin, gateway, reply and personal suites abort on the default test-thread stack. Their later passing runs explicitly use a 32 MiB stack, while the repository CI nextest profile does not configure that override. | Make the affected test futures fit the ordinary runner or provide deliberate scoped stack allocation, and execute the affected suites under the exact CI environment. Do not infer CI compatibility from the enlarged-stack diagnostic runs. | Open; prior default-stack aborts retained; final CI-compatible execution required |
| R74 | The recovered operator guide describes Telegram conversion and slash controls but omits native setup, the supported transport matrix, Discord's actual command surface and reviewed recovery/update behavior. | Document the reachable native UI/API, exact supported transports and displayed controls, pending setup recovery, paused reconfiguration and compatible-replica migration requirements. Keep Telegram management explicitly scoped. | Closed by source review of the integrated operator guide against actual UI/hooks/routes; replica upgrade explicitly precedes the first v2 write, including paused/pending setup |
| R75 | The native paired fixture's no-tools probe response omits root id/status/total_tokens and message id/role required by the actual upstream response validator. Its scripted streamed completion also omits total_tokens. | Correct synthetic provider responses to the actual Responses contract and execute the corrected full pairing with the retained stage diagnostics. Preserve strict production validation and existing accounting limits. | Source incompatibility independently confirmed; exact runtime failure stage and corrected pairing remain pending |

Independent frozen-source review of `87fccc0cd` found the intended R1–R3/R12
structures in place: effects commit with ingress, delivery references carry the
effect on insertion, final send compares immutable metadata and fences the live
intent or binding generation, status is re-evaluated, and static text is rendered
from typed metadata. Controls have separate local and distributed capacity. The
saturation fixture uses a task-local zero-capacity semaphore and reserves no
global permits. `git diff --check 87fccc0cd^ 87fccc0cd` passed independently.
These findings remain open for execution evidence. Adjacent lifecycle review
identified R16 and R17 and sent both to the sole implementation worker.

Follow-up draft source separates explicit `logged_out_at` from the original
binding revocation cause and checks the login bearer HMAC against the exact
committed intent before delivery. Its new lifecycle regression invokes actual
owner retirement, checks the old revocation reason remains, and verifies later
ingress reaches `PublicDisabled` in the deliberately public-disabled fixture.
The substituted-bearer regression exchanges tokens between two senders. These
are promising source resolutions for R16/R17; neither is accepted until frozen
review and execution. The upstream compilation report is corroborated by
`/tmp/nyxagent-botfather-check3.log` (dev check finished in 10.78 seconds), though
it predates subsequent edits and has no matching frozen revision.

The controls build was interrupted with SIGINT (exit 130) before tests ran when
free disk fell below 1 GiB. The worker removed only its disposable
`/tmp/nyxid-botfather-target` after the compiler exited; logs and sources remain.
No pass result has been accepted. Future backend tests use isolated databases on
the verified writable `nyxid1916` replica set at port 27196 and serialized lean
builds when sufficient space is available. Unrelated builds remain untouched.

### Independent baseline frontend checks

On unchanged frontend source from NyxID `5aed82c57`, installed dependencies with
`npm ci --no-audit --no-fund` and ran the five focused Vitest files for the
personal-link handoff/API, auth flow, MFA form and registration page with one
worker. Result: **5 files, 38 tests passed**. Log:
`/tmp/nyxid-botfather-review-frontend.log`. This establishes baseline component
coverage; it does not establish the complete signup/verification/chat journey.
The existing Chromium suite also passed **4/4** with
`npx playwright test --config playwright.platform-personal.config.ts`; log:
`/tmp/nyxid-botfather-review-browser.log`. It uses the actual frontend and stubbed
HTTP to verify clean-link navigation, one-time approval/reload, saved history
after disconnect, and expired-review renewal. It does not contact Telegram or
exercise a real backend signup.

Protocol decision: preserve substantive Codex reasoning through a strict optional
Responses `reasoning` object, with an enumerated effort and disabled summary.
Pin the configured effort to profile identity and retain existing cumulative
token/cost bounds. The installed executable is the full Codex CLI 0.160.1, not a
standalone app-server. Use a task-owned wrapper that checks its SHA and invokes
`app-server` for synthetic-model wire verification; execute the smoke explicitly.

Independent reviewer wire capture used full Codex CLI 0.160.1 with `app-server`,
an isolated temporary home and a local synthetic Responses server. Executable
SHA-256: `09fa44fdc37a5fc70dc1ace31235f90468a2e193d0e85f7552eab068ea2582be`.
The synthetic reasoning → FAQ tool → answer turn completed, and its second
request retained a reasoning item. This confirms R7 and R8; it bypassed the
draft upstream bridge and therefore does **not** establish public-runtime success.
Report: `/tmp/nyxid-botfather-review-wire-results.json`; captured synthetic
payloads: `/tmp/nyxid-botfather-review-wire-payloads.json`; reproducible harness:
`/tmp/nyxid-botfather-review-wire.py`. No external model or chat calls were made.
The 10,653-byte request is the raw Codex body. A diagnostic projection retaining
only public tools and removing orchestration-only fields is 4,431 bytes initially
and 4,851 bytes on continuation, so the declared 8 KiB bound may remain viable
after a correct normalizer and runtime configuration; it still needs a complete
real-flow test.

The independent SSE mismatch reproduction emitted a native `view_image` item
before a benign final response. Codex internally read a task-owned synthetic
one-pixel PNG and included its base64 bytes in the next request, without any
dynamic-tool RPC. This is R11 evidence, not an integrated upstream pass.
Harness: `/tmp/nyxid-botfather-review-event.py`; synthetic request capture:
`/tmp/nyxid-botfather-review-event-payloads.json`. No user file was read and no
external model or chat call was made.

An independent source comparison confirms that the two repositories currently
have identical public tool definitions and public instructions. The public-policy
JSON and wrapper-script Python syntax parse successfully. These are narrow
source checks, not compilation or cross-service execution evidence.

The worker subsequently reported upstream `cargo check --all-targets` completed
after a duplicate import fix. This is draft compilation evidence only, without
a frozen source hash or executed tests; it does not close acceptance rows.

Independent full-loop wire capture with the same Codex 0.160.1 executable
completed FAQ search → article read → signup-status tool → final answer in four
model calls. Synthetic tools used corpus-derived FAQ text and the actual NyxID
MCP result envelope; signup status was synthetic and sent no real link. All four
requests declared exactly the three public tools and retained reasoning history.
Raw body sizes were 5,732 / 6,621 / 7,386 / 7,963 bytes. A diagnostic projection
of the draft normalizer's removals measured approximately 4,348 / 5,171 / 5,870 /
6,381 bytes. Neither result executes the Rust normalizer or callback validation.
Harness: `/tmp/nyxid-botfather-review-full-loop.py`; report and payloads use the
same prefix with `-results.json` and `-payloads.json`. The worker received them
for integrated regression coverage. No paid model or external chat calls ran.

A second capture repeated the three-tool flow in a second short turn, with
synthetic model input usage estimated as raw body bytes divided by four. Codex
completed both turns against the local synthetic endpoint without compaction.
The second turn's diagnostic normalized sizes were 6,609 / 7,432 / 8,131 / 8,642
bytes, demonstrating R18's byte/token threshold mismatch. The capture bypasses
the strict Rust bridge, which would enforce the 8,192-byte ceiling. Harness,
report and payloads: `/tmp/nyxid-botfather-review-multiturn.py`,
`/tmp/nyxid-botfather-review-multiturn-results.json`, and
`/tmp/nyxid-botfather-review-multiturn-payloads.json`. No callback, Rust-normalizer,
compaction or cold-resume success is claimed from this diagnostic.

An explicit `thread/compact/start` diagnostic after the first full tool loop
confirmed R19. Codex used `/responses`, retained `reasoning:{effort:"low"}`, and
sent `tools:[]` with a compact prompt in history. It emitted context-compaction
notifications and completed against the synthetic endpoint. The fifth raw body
was 7,931 bytes. Artifacts: `/tmp/nyxid-botfather-review-compact.py`,
`/tmp/nyxid-botfather-review-compact-results.json`, and
`/tmp/nyxid-botfather-review-compact-payloads.json`. The unused synthetic
`/responses/compact` branch received no request. This is real Codex wire
evidence; it bypasses and does not validate the Rust bridge.

Disk later fell to approximately 235 MiB while unrelated compilers were active.
The primary removed only its previously installed `frontend/node_modules` in
this worktree, recovering about 476 MiB. Frontend source, lockfile and prior test
logs remain intact. Reinstall dependencies before the next frontend verification;
do not remove unrelated build artifacts. No primary compiler or harness remained
running after these checks.

### Continued paired protocol review

The draft upstream test log now records 37 library tests, one channel test,
six model-profile tests, four self-invocation tests and 32 service tests passing.
The default run explicitly ignored the real public Codex smoke and existing
external-dependency smokes. The first explicit public Codex execution failed at `thread/start` with
`runtime_protocol_error`; the second failed with `turn_failed`.
`/tmp/nyxagent-botfather-codex-public2.log` records HTTP 502
instead of 200 after 26.53 seconds. These changing-source diagnostic runs do not
close any acceptance row. The sole worker is investigating the bridge failure.

The draft public stream validator now buffers and checks all events, rejects
unapproved items, binds output indices and item identity, checks text/argument
deltas against completed items, and compares each done item with final output.
The real smoke currently performs four calls in its first turn and one final
call on cold resume; that second turn does not establish R18's full repeated
three-tool flow or R19 compaction. The worker has been asked to cover both.

Paired source review found that `platform_public_policy::operation_matches`
omitted the newly added `reasoning_effort`. Both live-context and readiness
comparisons use this helper. This remains part of R7: exact effort equality and
an effort-only drift regression are required before accepting the reasoning
contract. Draft signup text removes the nonexistent email-verification promise
without changing login policy; A2/R13 still need default registration and
sign-in execution evidence.

The R20 focused draft run in `/tmp/nyxagent-botfather-public3.log` passed six
regressions (including admission before JWKS/storage and complete JWKS timeout),
failed captured-wire normalization, and ignored the explicitly separate Codex
smoke. Source review confirms execution admission moved before verification,
with independent bounded discovery/readiness and cleanup lanes; JWKS headers and
body share a five-second timeout and shutdown cancellation. A frozen checkpoint
and final rerun are still required.

Inspection of the failing captured fixture found `parallel_tool_calls:true` in
all four real Codex requests and in their diagnostic callback projections. The
upstream normalizer and NyxID callback both require false, so neither fixture
currently validates. The primary sent this concrete R8 diagnosis to the worker:
preserve the raw capture, enforce the intended sequential semantics, and produce
new callback fixtures from actual Rust normalization before claiming parity.

A further R18 diagnostic applied only the documented orchestration projection,
forced the intended `parallel_tool_calls:false`, and removed one developer text
part exactly equal to the retained top-level public instructions. It saved 524
bytes in each captured request. The eight request sizes become
3,825 / 4,648 / 5,347 / 5,858 / 6,086 / 6,909 / 7,608 / 8,119 bytes. The last has
only 73 bytes remaining, so this can unblock that capture but cannot establish
safe general multi-turn behavior. Artifact:
`/tmp/nyxid-botfather-review-static-dedup-analysis.json`. This is a diagnostic
projection, not Rust normalization or compaction execution evidence.

The worker's real Codex smoke now passes both complete FAQ-search → article-read
→ signup-status → answer turns, with a new isolated Codex process on resume.
The primary read `/tmp/nyxagent-botfather-codex-public8.log`: one explicitly
executed smoke passed in 3.08 seconds; normalized request bytes were
2,685 / 3,474 / 4,155 / 4,648 / 5,549 / 6,338 / 7,019 / 7,512. The callbacks are
still local synthetic HTTP; NyxID callback execution and real platform sends are
not established. The source continues changing and is not a frozen checkpoint.

Public-only startup now disables the pinned CLI's four built-in skills before
initialization and skips host skill discovery. The attempted recorded-cwd change
was reverted; the actual fresh process cwd and home remain isolated. This removes
the repeatedly injected skill catalogue while preserving all substantive history.
The primary inspected these settings and confirmed the shared raw/callback fixture
is identical in both repositories (SHA-256
`ddf80012bfe2710484789fd6a9977bf6d108b9a3416b7da72541bdc793a1dd8a`).
Its real-normalizer callbacks use false parallel execution and full bounded tool
schemas; callback sizes are 3,884 / 4,707 / 5,406 / 5,917 bytes. The previous raw
capture remains intact. R18/R19 still require real compaction, stored-rollout
bounds and explicit continuation/renewal coverage.

### Compaction and durable continuation review

The primary read `/tmp/nyxagent-botfather-public9.log`: seven focused tests
passed, but the real compaction smoke failed after the next complete model/tool
loop with `session_too_large`. Compaction reduced model input while persisted
rollout history still exceeded the 65,536-byte session bound. This is an observed
R18 failure, not a successful compaction/persistence result.

The subsequent changing-source run in
`/tmp/nyxagent-botfather-public10.log` passed all eight explicitly selected tests,
including two complete four-call loops, one real local compaction, and another
complete loop in a cold process. Its thirteen normalized model bodies were
2,685 / 3,474 / 4,155 / 4,648 / 5,549 / 6,338 / 7,019 / 7,512 / 7,042 /
3,453 / 4,242 / 4,926 / 5,422 bytes. After compaction the serialized session was
17,172 bytes with 9,770 rollout bytes; after the next full loop it was 39,314
bytes with 27,765 rollout bytes. The primary read this log directly. The local
synthetic model and tools do not establish NyxID callback execution or live
platform delivery, and these sources are not yet frozen.

The bounded maintenance-turn policy is accepted within existing scope: before
adding new user input, an explicit no-tool compaction spends one model call and
one continuation from the existing four-call budget, persists its replacement
history, and asks the user to send the question again. The next actual question
gets its own normal four-call turn. The proposed wording is: “I’ve summarized
our earlier messages to keep this chat going. Please send your question again.”
No increased budget or silent history truncation is authorized.

Draft source review confirms transactional continuation counters in NyxID and
exact no-tool compaction grammar. Runtime rollout pruning uses only the pinned
Codex `compacted` record's nonempty `replacement_history` boundary. Responses
defers the maintenance notice until persistence and clears it on failure. Final
evidence must also cover streaming failure, exact retry classification, immutable
response records exhausting headroom, worst permitted serialized growth, and
oversized-state failure persistence. These requests were sent to the same worker;
R18/R19 remain open pending frozen review and execution.

The all-target draft run `/tmp/nyxagent-botfather-all11.log` completed with
85 passed, zero failed and eight ignored. The two real public Codex smokes are
among the ignored tests and require their separate explicit execution. This run
predates the following R18 follow-ups and is not final-revision evidence.

Further R18 source review identified two concrete edge cases. A just-compacted
state estimated at 2,048 request bytes plus roughly 230 output bytes would trigger
another maintenance turn for the same 3,500-byte question, indefinitely, unless
the runtime records compaction and refuses repeated maintenance without progress.
Also, `Responses::prepare` appends and persists its in-progress response record
before execute-time size checks. Even with bounded success/failure persistence,
that preparation write can exceed the signed session cap when prior immutable
records are near the limit. Both cases were sent to the sole worker for bounded
refusal/recovery and no-model-effect tests. The primary also requested R11
coverage for conflicting nonempty early SSE arguments/content.

### Frozen upstream checkpoint: independent review and replay

Reviewed commit: `f4da4471f014f1d5772b37c57d074f4856cc6562`, based on
`57ab8be89618a61b98e2ebc76331d121f96d731b`, in the task-owned NxyAgent checkout.
The primary personally inspected the new authority/manifest, execution/cleanup,
Responses bridge and stream validation, isolated runtime, compaction boundaries,
shared preparation/persistence changes and regression sources. The working tree
was clean and stayed at the same commit throughout independent execution.

Independent commands used the worker's SHA-pinned full-CLI wrapper at
`/tmp/nyxagent-botfather-codex-0.160.1`, the existing
`/tmp/nyxagent-botfather-target`, debug symbols disabled for dev/test,
`CARGO_INCREMENTAL=0` and `CARGO_BUILD_JOBS=1`:

- `cargo test --all-targets --locked`: **91 passed, zero failed, nine ignored**.
  Log: `/tmp/nyxid-botfather-review-upstream-all.log`.
- `cargo test --locked --lib public -- --include-ignored --nocapture`:
  **19 passed, zero failed, zero ignored**, including all three real Codex public
  smokes. Log: `/tmp/nyxid-botfather-review-upstream-public.log`.
- `cargo test --locked --test codex_smoke standalone_tool_roundtrip_and_cold_resume -- --exact --ignored --nocapture`:
  **one passed**, exercising ordinary chat/research/channel tool calls, images
  and cold resume with the same actual CLI. This specifically checks the shared
  launcher changes. Log: `/tmp/nyxid-botfather-review-upstream-personal.log`.
- `cargo fmt --check` and frozen baseline-to-checkpoint `git diff --check`: passed.

The public replay reproduced all thirteen normalized body lengths exactly:
2,685 / 3,474 / 4,155 / 4,648 / 5,549 / 6,338 / 7,019 / 7,512 / 7,042 /
3,561 / 4,350 / 5,034 / 5,530 bytes. Final serialized session size was 39,548
bytes with 27,980 rollout bytes. The synthetic attack harness had no panic.
Preparation refusal preserves the prior durable session and idempotency key;
failed candidate persistence retains bounded prior state; recent compaction or
immutable replay exhaustion refuses another ineffective maintenance operation.
Only a complete Codex replacement history supersedes old rollout records.

This closes the upstream-scoped findings R4, R6, R9, R11, R14 and R20. R5, R7,
R8, R15, R18 and R19 retain their paired NyxID producer, callback, accounting or
renewal execution requirements. A1–A14 are not marked complete by these upstream
results. In particular NyxID backend compilation/tests, signup/MFA continuity,
personal-account management, cross-platform entry and live deployment remain
unfinished. No paid model or external platform call was made. The primary
released the serialized build slot after these runs; no primary process remains.

### Continued NyxID source review and frontend replay

The primary restored frontend dependencies and replayed the current A2 drafts
while the worker held the Rust build slot. The five focused auth/link component
files passed **38/38** (`/tmp/nyxid-botfather-review-frontend2.log`). Chromium
passed **five of seven**, with both new signup cases failing before a login
request. Trace review established R22's registration-panel selector race; the
MFA navigation/rejected-code case and the four existing cases passed. Full log:
`/tmp/nyxid-botfather-review-browser2.log`. The four changed frontend source
hashes were identical before/after both runs and are retained at
`/tmp/nyxid-botfather-review-frontend2-source.json`. These are draft results,
not final acceptance. Targeted ESLint on the four changed frontend files passed
(`/tmp/nyxid-botfather-review-frontend-lint2.log`). Frontend source was released
to the worker for correction.

R22 correction was then personally inspected: the helper waits for the login
heading, selects the unique form containing the exact login password field,
fills exact fields, and waits for the actual POST response. Signup and MFA cases
assert one login request; the review fixture refuses unauthenticated or
incomplete-MFA access. Independent `npx playwright test --config
playwright.platform-personal.config.ts` passed **7/7** in 27.4 seconds; log
`/tmp/nyxid-botfather-review-browser3.log`. Source hashes were unchanged across
the run (`/tmp/nyxid-botfather-review-frontend3-source.json`), with the corrected
spec SHA-256 `551145e7ecd6bcf8d09b0e6c35a88be5859a77a35102b6c6aff82ae8621fa361`.
This closes the browser harness finding. Real backend auth/MFA execution and
the final combined source checkpoint remain separate requirements.

The worker froze those exact four frontend files at `f2818e928`. The primary
compared every committed file to the independently replayed hash and verified
the commit whitespace check. The worker also reports a successful TypeScript
build (`/tmp/nyxid-botfather-frontend-type27.log`). This frontend checkpoint does
not include the backend drafts or the later cross-platform UI work.

A6 source tracing confirms that personal entry resolves the admitted person's
real NyxBot, calls the shared owner-turn claim, provisions the ordinary encrypted
conversation credential, and runs the shared owner runtime. Existing journey
tests assert durable replies/web history, but the fake egress checks only the
owner context variant. Two-account memory/settings, credential/permission and
billing-route isolation plus web continuation evidence remains required.

A8 must extend the paired public protocol too: upstream `ManagerBinding` and
NyxID probe fencing still require numeric Telegram bot identity. Updating ingress
alone cannot make discovery/readiness work for other platforms. The primary has
released the reviewed upstream source for this required paired follow-up, with
a new checkpoint and independent replay required afterward. Public tool/FAQ and
account-review wording must describe the actual platform; existing adapter
normalization and default-human fallbacks are not new linking provenance.

The subsequent upstream A8 draft introduces mutually exclusive legacy Telegram
and explicit `platform_entry` v2 projections. The new projection carries a typed
platform, channel-bot UUID and authentic opaque platform-bot ID. Source tests
compare legacy JSON unchanged and distinguish platform/channel digest inputs.
This draft is unbuilt. Final pairing must also exercise actual NyxID-produced
discovery/readiness and execution authority with opaque identities; a local
`ManagerBinding::validate` test alone does not establish that integration.

A8 transport review also identified the existing boundaries the implementation
must preserve. `nyxbot::provider_channel` rejects platform markers through
`org_access_holds` before `responses` can inspect an event; ordinary connect/link
operations reject those markers too. An enabled platform gateway therefore
needs explicit transport authority without relaxing ordinary owner-channel
checks. Its relay callback must not receive the platform operator's access token:
the current suppression in `channel_inbound_service` covers Telegram-manager
credentials and X public posts, not a new cross-platform entry profile.
Retained native receipts currently have IDs but no positive human/original
proof, so legacy records cannot authorize new linking. The current
`native_sender_kind` default outside Telegram/Lark and generic private-chat
normalization are not sufficient proof. X private destinations must bind the
authenticated sender and receiving bot, not merely resemble two numeric IDs.
These are recorded A8 implementation requirements, not claims of a new live
exploit in the existing Telegram-only entry.

Additional A6 review traced the production `assistant_nyxagent_egress` path. It
validates the raw conversation credential against its stored key and admitted
person, fences the exact platform origin, constructs the billing route with the
person as actor and payer, and fences again after pricing/reservation awaits.
The runtime prepares memory/instructions from that person's agent and retained
thread. The existing journey's `SavedReplyEgress` bypasses these production
operations; it therefore cannot establish payer, key, approval or account
isolation. Required execution should observe distinct account facts and real
credential/billing provenance, then continue the same thread through the web
owner path. No additional source finding is inferred from the coverage gap.

Further A8 adapter review found that Discord's real interaction parser renders
options as `/confirm code=P-…`, whereas the current personal command parser
expects `/confirm P-…`. Entry integration must exercise the actual signed
interaction and normalize supported controls explicitly, without changing ordinary
Discord formatting. Slack's stored bot identity comes from `auth.test.bot_id`
(`B…`), which is distinct from a bot's `U…` user identity in authorization data.
WhatsApp filters authenticated app-wide events by the actual phone before parsing;
its normalized raw data is only the inner message. Tests must retain those actual
adapter boundaries rather than substitute Telegram-shaped text for every platform.

The resumed worker's finish notification was a provider overload/disconnection,
not completion. The same worker was resumed; no additional agent was created.
Its owned diagnostic backend build17 was interrupted to batch coherent backend
corrections. The primary confirmed cargo 26687 and rustc 58076 had exited; neither
compilation nor tests passed. A read-only process sample showed active compiler
type/borrow checking under paging before interruption, not a deadlock. The owned
target and `/tmp/nyxid-botfather-build17.log` were preserved. A later metadata
check18 was also interrupted before a result, and the worker explicitly released
the Rust slot for the upstream replay below. Unrelated processes were preserved.
A6 production-egress and A7 management are now being integrated as source drafts.
R24/R25 record concrete management account-handoff and expiry-projection findings.

Continued A7 review found that the draft `platform_personal_setup_selections`
collection currently has a writer but no reader. Inserting a row does not make
the reported selection affect the next conversation turn; R10 remains open for
a live owner/binding-generation consumer and an executed follow-up. R26 records
the adjacent legacy-confirmation regression, and R27 records the new hosted-link
compatibility issues. All were sent to the sole worker before its next compile.

The check19 batch adds a live selection consumer and exact hosted-request
watching. Review of that batch found additional closure conditions within the
same findings. R10 must not announce an expired/cancelled B as selected while
leaving prior A selected for the next turn. R26 must route its explicit legacy
outcome through the HTTP adapter or shared service logic, preserving the required
handler-to-service layering. R28 needs a bounded completion grace for a link
completed just before expiry, a claim tied to the captured hosted request when a
pending watch is replaced, and protection for a newer manual bot-to-specialist
choice. The new exact lookup must not invoke `connect()` to overwrite that newer
choice. These were sent for correction after check19, without interrupting it.
The same R28 terminal-state review requires cancelled/expired hosted requests to
settle their exact watches with truthful outcomes; returning early for every
non-completed link leaves a declined request displayed as pending until retention
expiry. This case was also sent to the worker for that correction batch.

Further R10 review found a second selection acknowledgement race: select live A,
then live B before A's send claim. The current renderer checks A's ownership and
live state but never compares the current personal selection, so it can still
announce “Selected setup A” while the next turn uses B. The worker received the
requirement to fence the selection snapshot or render truthful superseded status,
with a delayed-A-send regression alongside live-A/expired-B continuity.

The selected-setup context also needs truthful capability guidance. Ordinary
account tools manage channel bots, but do not edit platform onboarding drafts;
the corresponding HTTP setup editor requires a first-party human. Include the
checked review destination and direct draft changes through that review flow.
Do not imply an unavailable setup-edit tool. Final A7 proof must exercise an
actual permitted owned-bot management change through the linked account's normal
tool/confirmation path, alongside foreign-bot and shared-manager rejection;
listing, creation and a selected-context string alone do not establish that.

R26 also needs to preserve the existing legacy code normalization: the binding
service accepts bounded lowercase and hyphen/ASCII-whitespace-grouped codes,
whereas the new dispatch predicate only accepts a single uppercase eight-byte
token. The worker received the exact existing `code_hash` behavior and the
requirement to reuse its nonsecret format rules while excluding personal `P-`
confirmations and retaining the authenticated original-private input checks.

The A8 wording pass must also update the approved FAQ corpus. Its current entries
still describe only Telegram and teach only `/link` and `/unlink`; they do not
explain `/status`, `/login`, or `/logout`. Public answers are confined to this
corpus, so deterministic `/help` alone does not cover ordinary questions about
checking login or logging out. The worker received the requirement to describe
live chat status, pending-proof cancellation, browser-versus-chat logout, and
Telegram-specific management in the approved help itself. Public tool-definition
wording changes must retain paired upstream/NyxID schema and digest compatibility.

### Disk pressure during check19

The primary independently observed free space fall to 153 MiB and immediately
notified the worker. No primary compiler was running. The idle task-owned
`/tmp/nyxagent-botfather-target` was verified as a non-symlink Cargo cache at
`/private/tmp/nyxagent-botfather-target`; process inspection and `lsof` found no
users. The primary removed only that disposable approximately 1.2 GiB cache.
Upstream source, pinned wrapper, fixtures and all execution logs were preserved;
the active NyxID target and all unrelated processes/caches were untouched.
Free space subsequently read 3.3 GiB, including external fluctuations. Any future
upstream replay must rebuild its target. Prior verified results remain attached
to their recorded exact commits; cache cleanup is not new execution evidence.

### Independent upstream identity and call-ID replay

The primary personally reviewed and replayed the clean upstream checkpoint
`6abecf8516da556e71ba4772af2e7beb5d695850`. The legacy Telegram binding retains
its exact serialized bytes and digest. The explicit v2 extension requires one
typed platform, a channel-bot UUID and an authentic opaque bot identity; mixed
or missing projections are refused. Tool-call IDs now share the bounded ASCII
alphanumeric/underscore/hyphen grammar at output validation, input normalization
and tool dispatch. The paired NyxID positive fixture originally supplied invalid
empty FAQ-search arguments; the primary reported it and verified the draft now
uses a valid query. Its backend execution remains pending.

Independent commands used the retained target, lean environment and SHA-pinned
Codex 0.160.1 wrapper recorded above:

- `cargo test --all-targets --locked`: **93 passed, zero failed, nine ignored**.
  Log: `/tmp/nyxid-botfather-review-upstream-v2-all.log`.
- `cargo test --lib --locked public -- --include-ignored --nocapture`:
  **21 passed, zero failed, zero ignored**, including the three real local Codex
  smokes. Log: `/tmp/nyxid-botfather-review-upstream-v2-public.log`.
- `cargo fmt --all -- --check` and checkpoint `git diff --check`: passed.

The thirteen normalized body lengths again matched 2,685 / 3,474 / 4,155 /
4,648 / 5,549 / 6,338 / 7,019 / 7,512 / 7,042 / 3,561 / 4,350 / 5,034 / 5,530
bytes. Compaction persisted a 17,402-byte session and 9,986-byte rollout; the
following complete cold-resumed turn persisted 39,551 and 27,983 bytes. The
negative native-tool smoke refused the attempted calls as expected; no attack
harness panic occurred. The upstream worktree remained clean. No paid provider
or external platform call was made.

The upstream portion of R23 is verified. R23 remains open until the paired
NyxID callback tests execute; A8 also requires actual NyxID discovery/readiness
and execution using opaque identities. This checkpoint does not establish those
producer or adapter paths. The primary released the serialized Rust slot back
to the worker after the runs; no primary compiler remains.

### Replacement local test database

Before the next backend run, a bounded read-only probe found the previously
recorded MongoDB port 27196 unavailable (`ECONNREFUSED`). The primary left all
existing instances untouched and started a separate task-owned MongoDB 8.0.34
replica at `mongodb://127.0.0.1:27197/?replicaSet=nyxid_botfather_review`.
Writable-primary discovery and an actual transaction commit/read passed; the
temporary health database was then dropped. Data/log/PID and the ownership
marker are in `/tmp/nyxid-botfather-review-mongo.ic1WN1`. The worker received
the new test URL. The instance is bound to loopback with a 0.25 GiB WiredTiger
cache and 32 MiB oplog; preserve it until implementation and independent review
tests are finished.

### Rollout review preparation

The intended Telegram entry is `@nyxid_manager_bot`, channel bot
`bb3f92a9-6cf9-405c-9bca-bc422c8555d5`, platform profile
`0b4949e8-be13-4abc-8b65-7b1d33d4aa11`. Re-read its live generation and runtime
revisions immediately before any authorized update; the earlier generation-1
snapshot is evidence, not reusable write authority.

Before requesting rollout approval, freeze the two reviewed commit IDs, attach
the check results and close every finding. Prepare the exact upstream public
policy, immutable application/runtime identities, trusted NyxID issuer/origin,
and storage permissions for `public-sessions/*` and `public-revocations/*`.
Existing production documentation grants only `sessions/*` and `jobs/*`; that
policy alone cannot persist the new public records.

The reviewable activation sequence must retain the existing configured budgets
and sponsor, discover through the NyxID `/public/profiles` admin route, perform
the bounded `/public/readiness` probe, and apply the exact returned deployment
evidence with current runtime revisions. Discovery alone is not readiness.
Record all compatible replicas and frontend revisions before enabling entry
profiles; ordinary owner-channel fallback is never an activation mechanism.

Prepare a live observation script for one new person and one existing person:
public FAQ; signup/login; website account review; same-private-chat confirmation;
status; personal permissions and billing; bot listing and Telegram creation;
logout; cancelled-proof rejection; public FAQ again. Add one direct and one
supported gateway non-Telegram profile, with separate senders. Each real send and
production activation requires explicit authorization at the final reviewable
step. Local tests are not evidence of external receipt. Rollback must pause entry
traffic with compatible authority-enforcing binaries and retain durable markers.

### Cross-platform migration review gates

The existing `platform_bot_configs` unique index covers only `manager_bot_id`;
ingress deduplication covers numeric Telegram fields. A8 must update the actual
indexes and lookup predicates together with the serde contracts. Adding a v2
wire field alone does not namespace stored identities. Test equal opaque IDs
across different platforms/channels, retrying the same native event through
direct and gateway paths, and startup against an existing v1 index/data set.
Avoid delimiter-based identity concatenation without unambiguous encoding.

| Boundary | Required compatibility evidence |
| --- | --- |
| Existing Telegram rows | Read old bindings, pending proofs, assistant thread origins, public sessions, delivery records and cleanup scopes; retain the intended old identity/digest semantics. |
| New entry identity | Carry platform, exact channel bot, native bot, opaque sender and private conversation through consent, execution, billing, delivery and revocation; reject missing or conflicting projections. |
| Native admission | Authenticate and select the receiving bot before producing human/original/private evidence; retain transport billing and durable retry identity before ordinary routing. |
| Gateway admission | Bind the provider channel and original native receipt; default/legacy/device metadata cannot authorize controls. Preserve ordinary provider and owner-channel ACLs. |
| Retirement and recovery | Deleted/retired/missing-control entry markers continue to consume traffic; stale receipts, public cleanup and delayed replies cannot revive owner routing or prior account authority. |
| Admin activation | Show the actual platform/transport and capability; validate discovery/readiness with NyxID-produced v2 identity. Require compatible replicas before authoring v2 records. |

These are A8 completion requirements derived from source inspection. They do not
claim that the unfinished migration has passed, or add a separate approval step
for local implementation.

### Continued backend source review

Check19 has reported two test compile errors in
`platform_manager_journey_tests/management.rs`: the assertions read
`ChannelBot.platform_profile_id`, which is deliberately a raw persisted marker
and is not a typed `ChannelBot` field. Inspect that durable marker in the tests;
the compiler's similarly named `platform_bot_id` suggestion is not equivalent.
The production check also reported the R29 unused verification-token field.
At this observation the test compiler had not exited, so there is no overall
backend check or test pass. The worker retains the sole Rust slot and is holding
registered source fixed for attribution.

Personal source review of the R19 draft confirms that continuation counting
joins the same transaction as model-call and token admission. An existing call
cannot change its continuation classification on retry, and legacy rows without
a continuation allowance fail closed. The parallel admission regression uses
two transactions against one allowance. The R21 draft now calls the actual MFA
verification handler, asserts `AuthenticationFailed("Invalid MFA code")`, and
checks unchanged authenticated-session count, retained pending MFA state, and
absence of a personal binding before a valid retry. Both remain open until
executed on the frozen correction batch.

The A6 draft exercises actual encrypted ordinary credentials, account tools,
distinct memory and access modes, and same-thread web continuation through the
production outer egress. Its current meter assertions cover only the NoAuth
outer hop; funded inner-model credential-class and personal-payer evidence is
still required. Paired public evidence must also use NyxID-produced discovery
and an actual normalized no-tool compaction callback, in addition to the four
FAQ/signup model bodies.

The A8 UI change must include `platform-personal-link.tsx`, the saved binding
list, and `schemas/platform-personal.ts`. The current schema hardcodes the
Telegram thread and transport, while the consent and disconnect pages hardcode
Telegram wording. Retain truthful platform-specific privacy and confirmation
guidance when generalizing those views. At native ingress, positive command
provenance must originate after authenticated `prepare_webhook` and receiving-bot
filtering, survive metadata-only receipt storage, and be checked before ordinary
agent routing. Merely deserializing `InboundMessage` or gateway activity does
not establish that provenance.

R32 follows from the actual Slack `parse_inbound`/event parser and Discord
`register_webhook`/interaction parser. A bounded private-message control form
or a supported native command ingress can satisfy it; silently assuming typed
slash commands arrive as ordinary messages cannot. Discord's current signature
verifier authenticates the timestamp and body but does not test timestamp age.
When admitting new entry controls, establish the intended freshness separately
and exercise stale native evidence without changing ordinary-channel authority.

### Queued correction preview review

The primary personally reviewed the worker's thirteen-file external preview at
`/var/folders/jk/8w5kqy_507ncyl6hxl7dgph00000gn/T/nyxid-botfather-batch-preview-rsn082vo/backend/src`.
It is separate from the registered check19 source and has no compilation or
execution result. Source review found the R10 receipt-bound selection projection,
R26 explicit handler handoff and shared legacy-code normalizer, R27 actual
lifetime/checked return/UTF-8 label bound, R29 test-only zeroizing verification
token, and R31 legacy-person predicate coherent. Those findings remain open
until execution on the frozen batch.

For R28, new connection creation takes the existing bot write fence and checks
for any pending/active connection in that same transaction. Automatic completion
refuses an existing connection instead of relinking it. The retained activation
update requires `status: pending`, preventing a disconnected row from reviving.
The new regression covers a manual connection established before the watch runs;
a deterministic simultaneous-creation regression was requested for the new
transaction check. The exact request snapshot also fences pending-watch
replacement, and cancelled/expired requests produce terminal notices.

The A6 preview adds an actual inner-model proxy request with the ordinary
conversation credential. Its planned assertions distinguish account A's grant
funding from account B's wallet funding, verify same-account web continuation,
and leave the manager operator's wallet unchanged. A7 now performs a real owned
bot label update through normal account-tool confirmation. Its shared-manager
refusal currently acts as another account, so that case can pass through ordinary
ownership denial alone. A same-owner manager mutation refusal was requested to
establish the platform confinement boundary independently.

The unregistered paired-runtime test now configures base64 child secrets and a
same-sender next-turn refusal at the unchanged 0.50 cap. Before execution it must
mount actual JWKS/MCP routes, check successful health status, assert the exact
settled-meter set rather than a potentially empty loop, and use a captured
normalized compaction request instead of a hand-constructed approximation. The
public event producer currently emits only two events, so the draft's channel
capacity of eight is sufficient; no event-channel deadlock finding was made.

R33 was found by tracing the queued payer assertions into
`billing/reservation.rs`: successful wallet settlement increments
`pending_lago_debits` and releases `reserved_credits`; it does not immediately
rewrite the provider-confirmed `balance_credits`. The existing signed HTTP
readiness test already checks that protocol. The queued A6 and paired-public
assertions were sent for correction before execution. This is an incorrect test
expectation, not a discovered production charging defect.

The fourteen-file replacement preview at
`/var/folders/jk/8w5kqy_507ncyl6hxl7dgph00000gn/T/nyxid-botfather-batch2-fvac_kx6/backend/src`
adds the requested simultaneous-creation regression. A task-local, exact-bot
barrier brings automatic and manual calls past their empty preflight, commits
the manual connection, and then releases automatic insertion to test its
transactional refusal. It checks the sole active/pending connection, retained
manual specialist and failed exact watch. The shared-manager account-tool case
now uses a same-owner raw-marked bot and, if requested, its ordinary confirmation
card before asserting platform-specific refusal. Both source corrections were
reviewed personally; neither has been executed.

The original primary synthetic compaction capture remains available at
`/tmp/nyxid-botfather-review-compact-payloads.json`. Its 7,931-byte raw no-tool
request has the expected checkpoint prompt and fixed instructions. The worker
prepared an upstream fixture update preserving the raw request and comparing
its callback against the actual Rust normalizer, with optional export for the
NyxID paired test. The normalizer comparison is pending; Python-built expected
JSON alone is not proof of cross-service compatibility.

R33's external correction now checks unchanged provider balances, exact pending
debits, `available_credits()` reductions, released reservations and account A's
remaining grant. The public before snapshot includes readiness settlement.
Production accounting is unchanged; the tests remain unexecuted.

After check19 had reported two E0609 test errors and continued heavily paging
without additional diagnostics, the primary directed the worker to interrupt
only the verified task-owned cargo/test compiler, preserve its logs and source
hashes, and register the reviewed batch after both exit. This is not an overall
check result or pass. The next compiler should produce the actual test binary
with the retained lean target when resource headroom permits, followed by
focused execution; no second task compiler or unrelated-process interruption
was authorized. Record the worker's actual exit before attributing source edits.

The worker confirmed check19 cargo PID 97211 exited 130 and test compiler PID
6885 fully exited before source changes. The primary independently observed both
PIDs absent. `/tmp/nyxid-botfather-check19-outcome.json` records the interrupted
result; the original log and attribution manifest remain. Known diagnostics were
the two raw-marker E0609 assertions and registration-token warning. No backend
execution pass is inferred. The worker is registering the reviewed correction
batch and corrected `/healthz` pairing endpoint before the actual test build.

### Registered build20 review

The worker registered the reviewed batch and froze forty backend/fixture hashes
in `/tmp/nyxid-botfather-build20-source.json` at HEAD `f2818e928` plus the
explicit pending diff. The primary independently verified every hash. Comparing
the registered files with the fourteen-file batch2 preview found only the
expected raw-BSON manager-marker assertions, `/healthz` correction and R33 wallet
accounting corrections. Formatting and diff checks were reported by the worker;
actual `cargo test --bin nyxid-server --no-run --locked` is running with the
retained lean target and Mongo 27197. Log: `/tmp/nyxid-botfather-build20.log`.
There is no compile or execution pass yet.

The primary also checked the registered A6 assertions: both personal turns and
same-conversation web continuation reach the production inner proxy, produce
three provider calls and three settled inner meters, retain the expected
account/key identity and distinct memory, and charge A's grant versus B's wallet.
The operator wallet stays unchanged. This describes test coverage in source,
not an executed result. The paired public test captures the wallet after
readiness and asserts all twenty settled component meters from four callbacks.

For A8/R32, Discord's current interaction parser turns options into
`/name key=value`, and its generic gateway-message parser always labels the
conversation as a group. A message-text control by itself therefore does not
establish a usable Discord private-chat path. The design must identify an actual
Discord ingress/installation path and its authentic producer evidence; a signed
MESSAGE_CREATE body fabricated for an interactions URL cannot substitute for
that integration. This is part of the existing R32 acceptance requirement.

The build20 source review found R34 in the retirement/logout journey: its
receipt lookup uses a different field name from the actual ingress model and
producer. The worker was asked to queue the one-field correction after the
current frozen build, without interruption. This is an unexecuted test defect,
not evidence that production logout fails.

### A8 design review

The primary read `/tmp/nyxid-botfather-a8-design-sketch.md` and accepted the
explicit per-platform-profile direction for implementation. The sketch retains
Telegram v1 numeric serialization and proofs, adds authentic native v2 identity
through the same authority services, describes named index migration and durable
retirement, and selects real Slack DM text controls and installed Discord
`/nyx` private interactions with an `ask` subcommand. No A8 implementation or
execution pass is inferred.

R35 requires an exact content commitment on retained native receipts, in addition
to identity and message classification. Gateway transport authentication does
not establish that a received command/code/body is the one authenticated by the
native adapter. The worker was asked for same-receipt command, confirmation-code
and ordinary-text substitution regressions before any authority or model effect.
The commitment must be keyed/domain-separated and retain no raw proof material.

Additional design requirements sent to the worker: install replacement legacy
and native indexes before removing exact known old definitions, gate new native
writes on migration readiness, test interrupted migration and legacy proof
redemption, and reject ambiguous v1/v2 documents. Discord command setup must
preserve unrelated application commands, refuse a conflicting existing `/nyx`,
validate exact command/application identity and option shape, and make the actual
`/nyx ask` interface visible in guidance. The gateway path must retain an explicit
shared transport after ordinary-channel conversion cleanup, suppress the operator
relay token and refuse unavailable/stale transport or receipt evidence with no
direct or ordinary-owner fallback.

Build20 has reported E0061 at `handlers/nyxbot_tests.rs:5344`: the local
conversation-settlement test helper shadows the parent watch-settlement
function. The worker queued `super::settle` after compiler exit. The source
remains frozen and there is no completed build result.

A1's paired evidence must also cover real FAQ search/read/signup tool execution.
The registered paired test's final real Codex request deliberately produces only
a final answer; it proves generated-key/JWKS/MCP initialization/model-callback
plumbing but cannot alone close A1. The worker was asked to run the real four-call
loop through actual NyxID MCP with locally redirected native delivery after the
generic transport slice, verifying separate ephemeral link delivery and no bearer
in model history. Public tool failure/retry instructions still use `/link` and
must follow the selected native control surface. Exact-schema changes require
current local captures while preserving old raw evidence.

The primary verified the new compact fixture preserves the exact final request
from the five-request local raw capture, and the upstream/NyxID fixture files
are byte-identical (SHA-256
`1990572c7a859efbe4b0521e952fe716b08cdd3b344e44ce8ddde9c1228e5e64`).
The expected compact callback serializes to 5,397 compact-JSON UTF-8 bytes.
Actual Rust normalization/equality execution remains pending; this check proves
capture preservation and fixture pairing only.

The primary independently ran `cargo fmt --all --check` successfully on the
build20 source and rechecked its forty hashes without drift. Build20 later
reported two additional test type mismatches: account parity assigns
`Option<String>` to the native bot ID's `String` field, and the paired public
meter assertion compares `Option<String>` service ID to `String`. The worker
queued exact corrections. Review also caught an overly broad R34 replacement
in the external fix script; it now targets only the ingress filter and preflights
all four replacements before writing. No registered source changed.

R30's upstream README and deployment correction was reviewed personally. It now
describes actual NyxBot integration and paired public prerequisites, explicitly
pins `runtime.channel` when merging policy, and distinguishes the local full-CLI
wrapper from the production standalone artifact. No further finding on that
diff remains; record its frozen upstream revision before closing R30.

The R32 native-control requirement includes personal action cards. Their current
renderer offers plain `yes <code>`/`no <code>` replies, which would not reach an
interaction-only Discord receiver. The worker was asked to render a supported
native decision input or exact web-review target, and exercise an approved and
refused account action through that real ingress while retaining existing
turn/key/code/routing fences.

The first external A8 native/migration draft was reviewed before registration.
R36 captures migration restart/concurrency behavior in `ensure_until`; the
worker was asked for actual concurrent-start and ready-idempotence coverage,
plus receipt-expiry retention indexing. The authenticated adapter wrapper and
keyed content-commitment helpers are scaffolding only until production ingress,
lookup, reply and effect paths use them and their tests execute.

Continued A8 review traced the native classifiers into the existing adapters.
The current X production DM path is the signed Activity webhook handled by
`channel_webhooks` and `channel_adapters/x_webhooks`, with target and subscription
tag checks. The staged classifier's polling-only comment does not cover that
path. The worker was asked to construct positive native evidence there while
preserving the existing billing-dependent legacy polling policy. This remains
an A8 integration requirement, not an implemented or executed capability.

R36 also covers legacy writer compatibility: a partial uniqueness constraint
using only `identity_version: 1` excludes profiles inserted without that new field
after the backfill. The worker was asked to stamp every new producer and either
retain uniqueness for absent-version legacy records or enforce a concrete
incompatible-writer cutover. A completed migration marker alone does not prove
that older writers cannot bypass the replacement constraint.

The staged Discord command installer introduced R37: an authenticated application
lookup should also establish the webhook verification key used by the native
adapter. Separately, R32's usable interaction endpoint must authenticate the
Discord PING and command request before returning its acknowledgement; the
existing generic `Challenge`/`Immediate` path responds before verification.
Discord's URL verification includes invalid-signature probes that require 401.
The worker was asked for actual handler-level valid/invalid PING and deferred
control tests, with model work outside the acknowledgement deadline.

R36's revised external source now short-circuits a ready marker, uses an
owner-fenced renewable preparation lease, checks index absence on an idempotent
drop, installs receipt-expiry TTL and routes the legacy index helpers through the
same migration. Its legacy profile partial filter is now `native_entry: null`,
which includes missing native descriptors. The primary independently executed
five exact index-expression checks on local MongoDB 8.0.34: absent-version and
explicit-null legacy duplicates are rejected; native IDs stay outside that
legacy constraint; both BSON numeric types enter the legacy binding constraint;
native rows without numeric fields are excluded. Script and output are
`/tmp/nyxid-botfather-review-index-semantics.js` and `.log`; the isolated database
was removed. This is Mongo index-semantics evidence only. Actual Rust migration,
lease/concurrent-start and stored legacy proof tests remain unexecuted.

R37's external installer correction now compares the authenticated application
verification key bytes with the configured bot key before command listing or
mutation, and returns that verified key. Production enablement/ingress fences
and execution remain pending. Following its new `CredentialSnapshot` dependency
into the actual X secret producer exposed R38: connection-backed X signatures
come from shared platform credentials, a branch missing from the reused snapshot.
The worker was asked for stored-source rotation tests rather than only passing
synthetic `PlatformVerifySecrets` directly to the adapter.

### Corrected backend build21

The primary coordinated stopping build20 after three definite compilation errors
and about eighteen minutes without additional diagnostics. Only owned cargo
24321 was interrupted; it exited 130 and rustc 24510 fully exited. The preserved
`/tmp/nyxid-botfather-build20-outcome.json` records interrupted/no-pass, the log
hash and original forty-file source manifest. No backend tests ran.

The worker applied the four preflighted corrections and started the actual lean
test-binary no-run build21, log `/tmp/nyxid-botfather-build21.log`, source manifest
`/tmp/nyxid-botfather-build21-source.json`. The primary independently reversed
each exact correction in memory and verified it reproduces that file's build20
hash: qualified watch settlement, native bot ID `String`, optional meter service
comparison and the exact R34 ingress query. The other thirty-six files and HEAD
are identical; all forty live source hashes match build21. A8 stays external and
unregistered. The worker reports formatting/diff checks passed; build21 has no
completed compile or test result yet.

The continued external A8 review found R39 at the authenticated-batch/receipt
boundary. Its correction now retains the immutable configuration inside the
batch, compares it before any fence write, and derives the receipt configuration
from that batch. The new cross-bot, cross-platform and changed-transport cases
commit the refused helper transaction to detect accidental fence writes. This
is source-review evidence; registration and execution remain pending.

R38's next draft authenticates with the stored signing source before calling
the ordinary OAuth refresh producer, then takes a fresh admission snapshot.
Its fingerprint sorts and deduplicates scopes while the transaction guards the
exact raw scope value. Tests now use a local token endpoint with an expired
access token, real refresh, reordered/duplicated scopes and an unchanged epoch;
an unsigned request must make no token call. These tests remain external and
unexecuted. Gateway redemption and actual ingress integration still need review.

The A8 namespace gate includes the remaining bare `manager_bot_id` fallbacks in
`platform_bot_authority` and the Telegram admin/delivery/invalidation helpers in
`platform_bot_config_service`. Once native IDs may overlap Telegram IDs, those
legacy queries must be scoped explicitly. The worker was asked for an ordinary
Telegram bot beside a same-ID native profile, plus a Telegram manager rotation
that leaves the same-ID native entry untouched. The Discord admin integration
also needs a concrete recovery path when remote command installation succeeds
before the local activation commit; a retry must not silently adopt an unowned
command or blindly repeat an uncertain provider write.

The primary prepared `/tmp/nyxid-botfather-review-build21.py` for independent
execution after the worker releases the completed test binary. It invokes the
binary directly, verifies the frozen HEAD and forty source hashes before/after
each selected suite, refuses empty test selections, runs serially against the
isolated-test MongoDB URL, and retains commands, full logs and hashes. Its
coverage includes manager journeys, controls, proofs, bindings, public funding
and contract tests, and the six hosted-watch regressions. Only Python syntax
was checked at preparation time; it has not executed a backend test. The ignored
actual-upstream pairing remains a separate required run after upstream rebuild.

The staged Discord HTTP handler now checks stored-key signatures and bounded
timestamp age before its existing PING/deferred acknowledgement branch, with a
two-second verifier deadline. R40's correction preserves 401 for signature or
identity refusal and returns 503 for other verifier failures. The added handler
cases cover missing signatures, stale PING, valid PING, signed deferred command,
and missing/malformed persisted bot state. The primary read these changes; no
handler test has executed yet and no full native control reply is claimed.

Build21 subsequently emitted R41's unreachable-pattern warning in
`platform_public_contract_service/tests/handshake.rs:2005`. It remained running
with the source unchanged. The worker was asked to keep compiling and queue the
one-arm cleanup with the next necessary implementation batch.

After the provider handoff, the primary rechecked all forty build21 hashes; none
changed. Cargo 32214 and rustc 32382 remained active, with the same warning and
no completed compilation result. The independent binary replay is still pending.

The external identity projection review found R42 in entry-only conversation
threads and runtime controls. The worker added `entry_matches`, exact supported
contract versions and complete-record predicates. The primary read the revised
code: native records require version 2, a UUID v4 channel and exact manager ID;
structural legacy sender validation still permits negative group chat IDs. Private
chat eligibility remains a separate service decision. Draft stored-thread/origin
and group-handoff/delivery cases were added. This is source evidence only; the
actual old intent/challenge proof, persisted-origin and migration executions
remain required before acceptance.

R43/R44's next external revision retains UUID v4 installation IDs and adds a
unique authenticated-application index before helper use. It archives reviewed
replacement attempts and persists a twenty-second dispatch deadline through the
HTTP request and bounded response read. The paused-old-worker regression covers
an expired attempt with no observed remote command. These changes have been read
but remain unregistered and unexecuted.

R44 also applies to an existing command update. The current recovery delay is
conditional on `observed.is_none()`, so an explicitly reviewed replacement can
overtake a still-authorized PATCH when a command is visible. The worker was asked
to fence recovery of every active dispatched attempt using its persisted
deadline and to cover paused/in-flight existing-command reconfiguration. The
production admin integration must use the durable installer; the older raw
installer must not remain an alternative production mutation path. No finding
is closed from these source checks.

The worker then changed R44 recovery to require the persisted dispatch deadline
plus the uncertainty interval for both visible and absent commands. The new
visible-owned-command case pauses the original PATCH, rejects replacement while
it remains authorized and during the uncertainty interval, then checks fresh
provider reads before the replacement PATCH and refuses the resumed old worker.
The raw installer is now private and test-only; production mutation requires a
fixed deadline. The primary read these changes without a further finding on
that bounded recovery path. Execution and the actual administrative
authorization/configuration integration remain pending.

The primary prepared a separate direct-binary parallel replay at
`/tmp/nyxid-botfather-review-parallel-build21.py`. It requires the capacity test
and other manager journeys to be present, runs that suite with four libtest
threads and verifies the same forty frozen hashes before and after. The serial
replay also has an optional `--keep-going` mode to collect every selected suite's
result despite R46's known test setup defect; any source mismatch still stops
immediately, and any failed suite makes the final process fail. Both scripts
were syntax-checked only. No test executable or backend execution result exists
at preparation time.

At 10:31 SGT, a one-second read-only sample of the owned build21 rustc process
showed active LLVM code generation (`compile_codegen_unit`, `module_codegen`
and `codegen_mir`) with a 17.9 GiB physical footprint. The sample is retained at
`/tmp/nyxid-botfather-build21-review-sample.txt`. This explains the slow memory-bound
build and confirms progress at sampling time; it is not a completed compilation
or test result. No compiler was interrupted or restarted.

The worker proposed an encrypted receipt-bound reply handle for native Discord
delivery, because direct shared-entry interception occurs before ordinary relay
message persistence. The primary reviewed that direction: retain positive
receipts without tokens/content, construct the handle only from authenticated
native preparation, fence the exact receipt/configuration before decryption,
and preserve platform operation billing and uncertain-send handling. Existing
Discord `apply_interaction_context` intentionally falls back to a regular channel
send after fourteen minutes. Shared-entry delivery must refuse expired, missing
or malformed handles before that ordinary fallback, with zero fallback sends in
its regressions. This integration is still external and unexecuted.

The external R45/R46 corrections were also read. Status now uses a bounded
projection of both `is_active` and `status`, and account parity asserts the
existing readiness marker. Review caught a fixture mismatch in the added status
journey: `mark_bot_failed` deliberately does not change an already-active Telegram
bot. The worker was asked to use a valid production failure transition. The
current delayed case changes status before reply rendering; it must not be
reported as proof of rejecting a stale already-prepared body at the final claim.

R45's revised external test now uses an owned non-Telegram bot for the real
active-to-failed producer transition. A separate task-local pause occurs after
body preparation and before the final send transaction; it changes status,
resumes and expects refusal with no provider send. The primary read that test
and the isolated seam. R45/R46 source corrections have no remaining finding in
this review, but are not registered or executed; bounded test-side waits were
requested so an earlier admission failure cannot hang the suite.

The post-handoff source review traced the staged identity helpers through proof
HMAC construction, sender binding and native discovery/readiness fences. The
v1 proof parts retain their original domain, prefix, decimal sender/chat and
suffix order; native parts use a separate v2 domain and complete platform/channel
namespace. This is source evidence only: persisted legacy proof redemption and
opaque-identity producer pairing remain required. R47 records the remaining
accepted-wire grammar difference between the new NyxID decoder and the verified
upstream. The existing backend build and its forty source files were unchanged.

R48 was checked against Discord's [official application-command documentation](https://docs.discord.com/developers/interactions/application-commands),
retrieved read-only into `/tmp/nyxid-botfather-review-discord-commands.html` and
`.txt`. Age-restricted commands require an adult user and a separate DM setting;
`nsfw: true` is therefore material to public control availability. The provider
also documents nullable localization fields and localized option decorations,
which are not equivalent to a changed command bound. No authenticated Discord
request or command mutation was performed. These are source/documentation
findings; the staged installer has not executed.

R47's next source revision was reviewed personally. Both implementations now
separate missing/null fields and require the same supported native identity and
canonical legacy bot ID. The identical shared fixture currently has 32 cases
(7 accepted, 25 rejected), SHA-256
`bea4466186ce590bbf1cbef1cc367b9bf04c910fe1d045e7f5d0c8d8ce71342b`.
The upstream and staged NyxID tests consume that fixture, and NyxID includes v1
JSON/BSON byte comparisons. No further source finding remains on this change;
compilation and execution are pending. R45's external regression now bounds
both barriers at twenty seconds and the spawned send join at thirty seconds.

The R48 correction now authors `nsfw: false`, unrestricted default member
permissions and direct-message availability, and compares a recursive option
projection that preserves required/choice/autocomplete/bounds semantics. Its
provider fixtures add nullable/localized decorations; an owned otherwise-exact
age-restricted command requires the exact corrective PATCH. The primary read
the correction without a further source finding. It remains external and
unexecuted. Admin integration must still fence live actor, target and credential
snapshot in the installation dispatch transaction after network preflight; a
caller check before the helper's awaits is insufficient. Native output rendering
also needs the actual platform bound (Discord 2,000 characters), retaining one
provider operation per durable delivery claim and a usable saved-chat fallback.


Build21 completed its actual test-binary compilation in 71m45s and produced
`/tmp/nyxid-botfather-target/debug/deps/nyxid_server-a25f220fcfa166bf`.
The log contains the single R41 warning and no compilation errors. Primary
independently rechecked the frozen HEAD and all forty source hashes with no
mismatch. The worker is running focused suites before independent replay;
compilation alone does not establish a backend test pass. No source was changed
and no second compiler was launched.

The primary reviewed the staged receipt-scoped native reply service and recorded
R49/R50 for snapshot-bound OAuth materialization and complete deadline coverage.
The Discord branch avoids the generic adapter channel fallback and uses the
retained interaction capability; actual signed ACK/reply, no-fallback, billing
and uncertain-send tests remain required. These observations concern external
A8 drafts and do not alter build21 attribution.


Worker build21 execution has reproduced R46's duplicate readiness-marker setup
failure and an R12 regression-test race. The manager journey suite completed
27 passed and 2 failed in 171.27s. The R12 test starts the automatic reply
consumer before saturating its task-local control lane, then tries to deliver
the same saved reply again. The correction must gate the actual first send and
prove it succeeds while the control lane is saturated; accepting a duplicate
result would not establish that property. The task-local override also does
not automatically propagate into a spawned consumer. Primary independent
execution remains pending the worker's release of the frozen test run.

A compatibility runner at
`/tmp/nyxid-botfather-review-compatibility-build21.py` selects existing callers
of the changed watch, registration, connection and manager services, using the
same HEAD/hash verification as the focused replay. It has only been syntax
checked. The staged native controls still require the agreed platform-aware
chat/browser guidance: Slack plain DM controls and Discord's installed `/nyx`
subcommands, including action-card replies, must match the actual parser and
final-send rendering. Telegram's canonical guidance remains covered separately.
Source-review hashes are retained in
`/tmp/nyxid-botfather-review-a8-r49-r50-source.json`; this is provenance rather
than a frozen checkpoint or execution result.


The completed worker build21 focused run is **100 passed, 2 failed, 1 ignored**
across fifteen suite selections. Its authoritative result file is
`/tmp/nyxid-botfather-worker-build21/results.json`: journeys 27/2, commands 3/0,
link intents 12/0, binding 2/0 plus stop 1/0, budgets 10/0, contracts 26/0/1,
proxy 9/0, public tools 4/0, and six exact watch cases 6/0. The paired upstream
test is the one ignored case, not executed proof. Final HEAD, forty hashes and
test-executable SHA-256 remained fixed. The binary SHA-256 is
`1895d4074c2e04ba4d47ee871f19c81092977ddf40ca018dfc44827b83daa809`.
Primary independent serial replay has started at
`/tmp/nyxid-botfather-review-build21-serial`, with compatibility and parallel
replay to follow before source registration.

R12's external correction now gates the actual automatic consumer before its
first send and installs the shared zero-capacity task scope inside that consumer.
It checks the exact original reply, a `Sent` outcome, and one provider request
while controls remain saturated; all new waits are bounded. Primary source
review found no further issue in that correction. Execution remains pending.


Primary independent four-thread manager replay completed: **27 passed, 2 failed,
0 ignored**, in 191.61s. Both failures exactly match R46 and R12; no additional
concurrency failure was observed. Results and logs are retained at
`/tmp/nyxid-botfather-review-build21-parallel/results.json` and `parallel.log`.
The runner verified the forty source hashes, HEAD and exact binary SHA-256.
The capacity regression remains failed; its external correction has not run.
Focused serial and existing-caller compatibility tests are still executing
against the same frozen build.


Primary reviewed the revised R49/R50 native reply sources: encrypted credential
material is retained by the claim, the send lifetime encloses claim lookup,
decryption, provider request and body, and a new real refresh-producer fixture
covers expiry between ingress and final claim. Provider-attempt tracking uses
the actual X billing boundary. These are still external drafts. Integration must
call pre-claim refresh under the same absolute deadline and execute paid/no-paid
attempt, response-timeout and duplicate-send cases. R51 records an additional
local-test observability gap in the Discord no-fallback fixture.

R51 source correction now reuses the existing test-only Discord adapter base
constructor for every native reply fixture, claim and send. Both interaction
webhooks and any accidental ordinary channel path would reach the same local
server. Primary verified that no production-default Discord adapter remains in
that test module; the tests are still unregistered and unexecuted. Native browser
review also requires the saved-conversation schema/sidebar path and neutral
loading guidance, beyond the new platform-specific confirmation command.


Independent build21 existing-caller replay completed its first suite:
`handlers::nyxbot::tests::` **128 passed, 0 failed, 0 ignored** in 1129.45s.
This includes ordinary owner/guest, image/late-reply, action-confirmation,
hosted-watch and Telegram/X channel paths. Full log:
`/tmp/nyxid-botfather-review-build21-compatibility/01.log`. Remaining
compatibility selections and the focused serial public-contract suite are
still in progress; this does not claim a complete compatibility run.


Independent focused serial build21 replay completed with **100 passed, 2 failed,
1 ignored**, matching the worker result exactly. Both failures remain R12/R46.
The runner exited 1 and verified frozen HEAD, all forty source hashes and the
binary; results are `/tmp/nyxid-botfather-review-build21-serial/results.json`.
Compatibility first ten suites have completed **224 passed, 0 failed, 0 ignored**;
its final Telegram suite remains active. The two changed setup/signup journeys
are now selected separately at `/tmp/nyxid-botfather-review-build21-signup`,
because their module is outside the earlier `platform_setup_service::tests::`
filter. They have not completed yet.

Independent build21 compatibility replay is complete: **292 passed, 0 failed,
0 ignored** across eleven selections, including all 68 Telegram creation tests.
The two separately selected default/configured signup journeys also completed
**2 passed, 0 failed, 0 ignored**. Both runners exited zero and verified the
frozen HEAD, all forty source hashes and binary identity. Authoritative results:
`/tmp/nyxid-botfather-review-build21-compatibility/results.json` and
`/tmp/nyxid-botfather-review-build21-signup/results.json`. These do not clear
the focused R12/R46 failures or the still-ignored actual upstream pairing.

Primary has frozen the pending upstream R47/raw-compaction/docs draft at
`/tmp/nyxagent-botfather-review-upstream-next-source.json` and started serialized
fmt, Clippy with warnings denied, all-target tests, explicit public smokes and
the actual server build. Results/logs use the same prefix. Registered NyxID
source remains frozen through the subsequent actual pairing test. No new worker
compiler or additional agent was started.

Upstream fmt and diff checks passed. Clippy with warnings denied stopped on
three style diagnostics: `bridge.rs` let-and-return and collapsible-if, plus
`wire.rs` derivable-default. The primary released only those corrections to the
worker before a new manifest/rerun. No upstream test or build pass is claimed
for this draft. The failed log and source manifest remain preserved.

Primary independently ran six selected copied-overlay frontend suites:
**71 passed, 0 failed**. Full application typechecking passed with the overlay's
`tsconfig.app.json`; an initial `tsc -b` invocation was invalid because that copy
does not contain the root project-reference config. Targeted ESLint caught
`no-control-regex` in the native opaque identity schema. The worker replaced it
with equivalent explicit character-code validation; schema/lint replay is
running separately. Logs and hashed source provenance use
`/tmp/nyxid-botfather-review-native-ui` and `-ui2`. Registered frontend source
is unchanged, and these component fixtures are not native backend evidence.

R52's external constructor now uses the retained receipt ID; the duplicate
dispatch constructor is gone. Its new signed Discord receipt regression reaches
real login/status/admission producers. The final delivery test is still being
integrated; manually inserted Dispatching rows remain narrow materialization
fixtures. R50 caller review found that the newly staged final database claim
transaction still awaits outside the absolute timeout. Passing a deadline into
capture prevents a late send but does not bound a stalled transaction. The
worker must include session/claim awaits in the same deadline and prove zero
late sends before R50 can close.

Build21 now supplies the previously missing execution evidence for R1/R2,
R13/R16/R17/R21/R29/R34. Those findings are closed at that frozen checkpoint,
with native integration explicitly still pending where relevant. The manager
journey log contains successful committed-effect, substituted-bearer,
status-before-send, real signup/MFA and owner/control-retirement cases;
registration/verification compatibility and separate signup selections also
passed. R12 and R46 remain actual failed tests. This finding-level closure
does not accept A8 or the final combined implementation.

The corrected native schema replay passed and targeted ESLint is clean:
`/tmp/nyxid-botfather-review-native-ui2-results.json`. Both staged and copied
files matched the recorded hashes. Upstream's second Clippy run reached test
code and found four additional mechanical style diagnostics (three nested
conditions and an unnecessary vector); these are being corrected before the
next frozen rerun, retaining all earlier logs.


Upstream next3 independently passed fmt, diff, Clippy with warnings denied,
all-target tests (**94 passed, 0 failed, 9 ignored**), explicit public tests
(**22 passed, 0 failed, 0 ignored**, including three real pinned Codex smokes),
and the actual server build. Full hashes, commands and logs are in
`/tmp/nyxagent-botfather-review-upstream-next3-results.json`; source remained
identical to `-source.json`. Server SHA-256:
`3666f7525977acb0d70ae65aaca3a4a28e7d64d15ec387d2f1c598a5f4039571`.
The actual Rust normalizer export semantically equals the shared NyxID fixture,
including the preserved raw compaction projection. Its fixture compact callback
is 5,397 UTF-8 bytes; the separate live smoke compact callback is 7,042 bytes.
Live smoke saved Session/rollout sizes were 17,402/9,986 after compaction and
39,549/27,981 after the resumed full turn.

Frozen build21's required ignored pairing is now explicitly executing against
that server; `/tmp/nyxid-botfather-review-paired-build21.py` verifies both sources
and binaries before and after execution. No paired outcome is claimed yet.
R50's staged claim timeout now includes session/transaction waits; terminal
native delivery persistence must also release its permit within a bounded
lifetime and preserve uncertainty if the write cannot be confirmed.


The frozen build21 actual pairing failed before readiness or model execution:
**0 passed, 1 failed, 0 ignored**, exit 101. A task-owned loopback observation
proxy reproduced the failure without changing either source set and captured
HTTP 403 `public_authority_rejected` from `/v1/public/profiles`. The actual
fixture serves an empty JWK despite signing with its generated key: R53.
Direct and observed results, commands, source and executable verifications are
`/tmp/nyxid-botfather-review-paired-build21-results.json` and
`/tmp/nyxid-botfather-review-paired-build21-observed-results.json`.
The observer records synthetic metadata only; its server children exited.

Both frozen verification sets are complete. The primary released registered
NyxID source for the coherent correction/native integration batch and assigned
the sole Rust build slot to the existing worker. Upstream next3 remains frozen
pending a justified paired change. The failed pairing is a fixture defect, not
a readiness/model pass, and requires actual re-execution after correction.

Primary read the new external native command delivery and deadline regressions.
The signed Discord command now reaches actual retained receipt, command ingress,
final transaction and provider follow-up, asserting durable Sent, one request,
exact rendered body and zero generic channel fallback. The deadline regression
uses entered flags inside the actual claim callback and terminal write stage;
a confirmed provider response followed by lost persistence remains Dispatching,
returns Uncertain and sweeps without resend. These are source observations only;
registration, compilation and execution remain pending.

R50 follow-up: native preparation/claim/success persistence now have deadline
coverage, but caller failure branches still await refusal/retry writes without
the carried timeout after acquiring a send permit. Public signup also has an
oversized-body post-claim finish outside that deadline. Primary requested bounded
failure cleanup with an entered-flag stall regression, preserving Pending or
Dispatching recovery when persistence cannot be confirmed. A provider-free
refusal and a possibly delivered response must retain distinct outcomes.

Build22 froze forty-two files, adding matching generated test signing/JWK
material and the previously reviewed R12/R41/R45/R46 corrections. Primary
verified all hashes and the restricted changed-file set. Before compiler
diagnostics, review found R54: the account-parity LocalTransport initializer
misses the new required personal_gate field. The primary requested interruption
of only the owned attempt, correction and a new frozen build to avoid waiting
for the inevitable missing-field error. No build22 pass is claimed.

Finding closure audit against preserved independent build21 results now closes
R10/R24/R25/R26/R27/R28/R31 for the executed Telegram/hosted workflows. The
management cases in serial/01.log and parallel replay passed; six exact-watch
and concurrent-route cases in serial/results.json passed as well. Their source
asserts actual missing legacy user_type, Unicode label bytes, returned TTL and
checked hosted callback. R23 closes against compiled shared grammar tests in
serial/08.log and upstream next3 public tests. R30 closes against reviewed,
frozen next3 documentation. These closures do not claim account-parity A6,
full paired FAQ/signup, native A8 or deployment acceptance.

Primary handoff review verified build23 HEAD and all forty-two frozen hashes,
with the original owned compiler still running and no new compiler started.
Build22 remains interrupted/no-pass; build23 includes the explicit account-parity
initializer correction. The external R50 failure-cleanup revision was read:
the original native deadline is derived before preparation, retained on errors,
and bounds refusal/retry writes plus the public-signup oversized-body finish.
Its new entered-flag regression deletes retained evidence after command commit,
then proves cleanup is reached and a timeout leaves Pending for safe refusal
recovery. No further cleanup source finding in this pass; compilation and
execution, including paid X attempt/body/funding coverage, remain pending.

R55 follow-up review: staged renewal now shares one scoped select future with
the migration operation, with no detached task. Short-lease entered-stall and
actual ownership-replacement tests are meaningful but unexecuted. A remaining
completion race can report failure after the operation publishes ready: the
concurrent heartbeat sees the removed preparing lease before the ready-write
response resolves. Primary requested bounded convergence on authoritative ready
and a publication/heartbeat overlap regression. Owner-conditional error release
also needs a timeout so failed storage renewal cannot lead to unbounded cleanup.

Build23 ended by the task-owned disk guard after 497.35 seconds when free disk
was 2,065,362,944 bytes. The primary verified cargo, wrapper, compiler and guard
PIDs absent. The recorded cargo exit is -2 (SIGINT), result no-pass, with all
forty-two source hashes still verified. No compiler diagnostic or new executable
was produced. The outcome and guard logs remain at
`/tmp/nyxid-botfather-build23-outcome.json` and
`/tmp/nyxid-botfather-build23-disk-guard.jsonl`. The primary requested continued
external integration work instead of immediately restarting at recovered
6–7 GiB. Original source/upstream freezes and build21 retained binary remain.

R55 follow-through source review found bounded authoritative-ready convergence
and owner-conditional timed release in place. The publication-overlap test
asserts its post-ready stall was entered and cancelled, then verifies the ready
marker remains unchanged; the ownership-loss test also reaches and cancels the
release stall without changing the replacement owner. No additional source
finding remains in this revision. R55 stays open for registered compilation and
execution, alongside the other native migration compatibility gates.

Primary reviewed the external Discord InstallationAuthority slice. Production
installation materializes captured credentials and joins live administrator,
exact bot marker/route, profile generation/configuration/funding and credential
fences with Prepared-to-Dispatched after authenticated provider lookups. Its
barrier regressions mutate real stored admin role, token, profile or route and
assert claim rollback plus zero provider writes. Source-only acceptance for
that transaction; handler conversion/activation remains unfinished. Activation
must bind the exact installation attempt/state, including a result returned by
the Observed reuse path, which currently fences authority without snapshotting
the installation row in that transaction.

A resource-bounded primary model check compiled exact copied staged production
identity/projection/ingress/binding/link models using retained dependency rlibs,
without Cargo or registered source changes. It passed 126 assertions comparing
registered v1 JSON/BSON bytes for intents, approved challenges, bindings,
personal turn proofs and negative-chat group ingress; native roundtrips and
mixed/null/version/manager/channel rejections also passed. Evidence and exact
copied-source hashes are under /tmp/nyxid-botfather-review-models1. An initial
harness-only Int32-versus-canonical-Int64 positive fixture mismatch is preserved
as attempt1; the corrected run changed no production source. Compiler warnings
identified an outer doc comment on a macro invocation and unused event-identity
imports in two model files; worker has the exact locations for cleanup. This
check does not redeem stored proofs, execute services or establish a passing
backend/A8 build. All 42 registered freeze hashes remain unchanged.

The follow-up isolated model check at /tmp/nyxid-botfather-review-models2
compiled the exact staged ManagerBinding and production Credits module. All
32 shared identity cases passed in JSON and BSON; canonical v1 bytes match
the actual registered legacy model in both serializers. The shared fixture
is byte-identical to frozen upstream next3's independently executed fixture.
The original 126 model assertions also pass. This supplies narrow compiled
consumer evidence for R47 on both sides; registered backend compilation and
actual opaque-v2 discovery/readiness/ingress pairing remain open.

Installer follow-through now returns a non-deserializable InstalledControl
bound to the exact Observed attempt snapshot on all three successful paths.
The provider-success path updates its local observed state before constructing
the wrapper. A new retained-result test supersedes attempt/fingerprint and
asserts activation-fence refusal without a second provider mutation. Primary
source review found no further wrapper issue; the actual administrative
activation caller still has to join this fence with its live authority checks.
Gateway review also covers shared provider lifecycle/context endpoints and
late ordinary delivery routes, not only the responses entrypoint.

Primary gateway review found R56 before registration: the authentication helper
returns its pre-touch API-key model, while its normal metadata write increments
the persisted state version. The shared gateway then fences that stale version.
The existing key-service regression already establishes the version increment;
new real shared-handler positive and revocation tests are required for the fix.
Registered and upstream source freezes remain unchanged.

R56 follow-through source now reloads the live exact primary credential after
authentication and compares its non-timestamp authority inside the transaction,
then fences the transaction-local version. Child credentials are refused. The
primary found the original stale-version failure addressed in source and asked
that ordinary routes avoid the new double validation/timestamp mutation. The
retained build21 validator regression independently passed 1/0 at
`/tmp/nyxid-botfather-review-r56-key-validator`; this establishes the existing
validator behavior only. Real shared-handler positive, revocation and concurrency
regressions remain uncompiled. The 42 registered and 76 upstream manifest hashes
and both HEADs were independently verified unchanged again.

The third isolated model replay compiled only the exact warning-cleanup copies
with `rustc -D warnings`, passed all 126 serialization assertions and the 32
shared identity cases, and confirmed those source hashes remained unchanged.
Evidence: `/tmp/nyxid-botfather-review-models3`. The narrow compiler slot was
released immediately; no full service execution is inferred.

Gateway effect review found R58 in the native branch of the shared manager
authority evaluator. Receipt creation/redemption and final send check the
selected transport, while later command and personal-effect transactions use
the manager evaluator without that check. The worker has the exact retirement
and key-revocation interleavings to cover. Deferred webhook tests must also
explicitly carry their local transport through spawned tasks; task-local test
overrides do not propagate automatically.


Primary reviewed the actual local CMAEG protocol and consumer for native gateway
setup/forwarding, finding R60 before registration. Empty reply sinks with
`auto_final: false` are accepted; differing model strings have not been shown to
be a parser failure. The current normalized callback cannot satisfy Lark/Feishu
raw-event verification, and X has no gateway platform variant. Durable reserved
record recovery is documented by the consumer and must replace a terminal
"administrative recovery" string without an implemented recovery path.

R56 now avoids ordinary key validation side effects when selecting the shared
namespace. R57 now uses UTF-8 bytes and rejects C1 controls, with positive and
negative multibyte boundary cases. R58 now joins the selected-transport evaluator
from the common native manager read/fence path. R59 now supplies the existing
ChannelBilling context. These source corrections are not backend execution
results; their real handler, retirement and funded operation tests remain open.


Primary isolated R57 replay passed 10 schema tests and targeted ESLint on exact
staged source, including C1 rejection and 256-byte multibyte boundaries. Evidence
is `/tmp/nyxid-botfather-review-r57/{source,result}.json`. The first lint attempt
lacked the repository config in the copied overlay; the corrected attempt used
the exact repository config and passed. Both external source hashes remained
unchanged. R57 is closed for that schema correction only.

The supported native transport matrix is explicit: direct for every implemented
native platform, with the gateway option for Discord, Slack and WhatsApp. The
current gateway has no X support and requires raw events for Lark/Feishu; those
platforms are direct-only for this implementation. Unsupported choices must
refuse before any pending marker or provider effect, with no automatic switch.
R61 tracks deterministic conversion blockers that otherwise strand a pending bot.


R60 narrow compiled producer/consumer proof: the primary copied the complete
CMAEG core and four actual request/definition validator modules, then compiled
the exact native request_policy, capability helper, platform spelling and model
constants in an isolated wrapper. All 15 assertions pass: Discord/Slack/WhatsApp
requests deserialize and validate through actual create and definition gates;
Lark/Feishu/X gateway requests refuse locally; each supported fixture rejects
the old disabled-group, missing-trust and unsupported-X variants.
`/tmp/nyxid-botfather-review-cma-contract1/result.json` records all hashes.
All 49 copied consumer file hashes and five producer extracts remain unchanged.
Initial harness attempts exposed retained dependency mismatches (NyxID utoipa5.4
versus CMA meteroid-utoipa5.6, then a host indexmap without serde). The corrected
harness compiled the cached exact 5.6 macro/library with retained compatible
dependencies; no production annotations or source were altered. No Cargo or
full backend/gateway build ran. The compiler slot is released. This verifies
pure request production/validation only, leaving real callback authentication,
retained receipt redemption, gateway recovery and HTTP journeys open.

The actual generic gateway observer emits actor.kind=unknown and relay callback
verification always requires a signed reply token. R60 also covers those runtime
projection mismatches. Sender authority must continue to come from the retained
authenticated original-private-human receipt, and any dedicated reply token must
remain unable to acquire ordinary owner routing or duplicate native delivery.


Primary R60/R62 response-contract replay compiled the exact extracted fingerprint
formula and observe_record, the actual CMA definition builder, public definition
projection and response constructor. All three supported request fingerprints
match the consumer. Actual sealed ChannelPointer IDs are 264 bytes for the
synthetic https://cma.example origin; every valid ready response is refused by
opaque's 256-byte native limit. Replacing only the ID and matching callback with
a short synthetic value makes all three observations pass. This is R62, also
present in validate_configuration. Evidence remains at
`/tmp/nyxid-botfather-review-cma-contract2`; no HTTP/database or full backend
claim. Initial compile errors were isolated harness module/return-type/hash
dependency setup, corrected without altering extracted production functions.
The compiler finished and its slot was released.


Actual CMA runner review found R63/R64 beyond create/recovery grammar. The
first real event calls Responses before ensuring a partition and only invokes
PUT conversation after a 404 conversation code. The staged missing-partition
branch returned generic Forbidden. Separately, the shared success stream had
only message_frames(None), which emits completed; CMA's forward_turn resolves
its accepted oneshot only for created/in_progress. Both were sent with exact
consumer paths and required first-event/stream tests; no runtime pass inferred.


R64 now has independent local HTTP execution evidence. The exact staged
message_frames(None) output passed through copied actual CMA SSE parsing and
forward_turn reaches terminal but closes acceptance unresolved. A control stream
with created before the same empty completion resolves acceptance and carries
no output text. `/tmp/nyxid-botfather-review-cma-stream1` records copied/extracted
sources, compiler/run output and binary hash. This is a consumer protocol proof,
not execution of the native handler or delivery. The compiler slot is released.
R62 contract3 compiled but still failed because the live observe_record retained
its opaque call; the worker was notified of that exact missed replacement.


R62 corrected call-site replay now passes45/0 with all six exact source extracts
unchanged. Actual sealed264-byte IDs from CMA are accepted for all three supported
platforms; the fingerprint formulas still match, reserved/ready observations
project correctly, and malformed IDs, mismatched key/binding/policy/fingerprint
refuse. Native sender limits remain256. Evidence:
`/tmp/nyxid-botfather-review-cma-contract4/{source,result}.json`. The compiler
finished and its slot is released. Complete native configuration/HTTP activation
execution remains pending.


Primary replay of the exact corrected native_completion_frames plus unchanged
message_frames passes through the actual CMA SSE parser/forward_turn over local
HTTP. Acceptance resolves, response IDs match, completion arrives, and output is
empty. Evidence: `/tmp/nyxid-botfather-review-cma-stream2/{source,result}.json`.
Both extracted source hashes remained unchanged. This closes the narrow R64
wire mismatch; actual shared handler, first-event partition and exactly-one
native-send execution remain required. The compiler slot is released.

R62 UI replay uses exact external files and real sealed IDs from contract4.
Full TypeScript and targeted ESLint pass; existing native setup/admin page
coverage passes5/0. Adding backend-equivalent malformed-ID cases yields9/2:
the schema accepts key id256 and a one-character ciphertext that the backend
rejects. The worker received the concrete contract test. Evidence remains at
`/tmp/nyxid-botfather-review-native-admin-ui1/{source,result}.json` and tests2.log.
The component test currently exists only in the worker UI overlay and must join
the coherent source batch before registration.

R63/R65/R66 follow-through source was personally reviewed. Missing-partition is
a typed outcome after gateway/key and retained receipt/config/content/credential
fences, mapped to404 conversation_not_found before native dispatch. Native
profiles now fail repair and verify_manager before Telegram credential/provider
work. The shared funding helper fences active org and current admin membership
inside pending, verification, installer effect and final-conversion transactions.
These source findings await their real handler and revocation-barrier regressions;
no full A8 compile or backend execution is inferred.


R62 corrected UI independently passes13/0 across native setup/admin and actual
CMA sealed-ID cases. Full TypeScript and targeted ESLint pass. All19 external
file hashes remained unchanged; the component test now exists externally.
Evidence: `/tmp/nyxid-botfather-review-native-admin-ui2/{source,result}.json`.
The first copied-overlay run lacked root docs/site and emitted Vite warnings;
the exact docs tree was copied and the clean rerun is tests2.log. No registered
frontend changes or backend authority-execution claim.


R60 narrow callback execution now passes for Discord, Slack and WhatsApp. The
primary extracted the native post-claim emission block and exact production
reply-token, callback-token, JWK and HTTP-forward functions, then sent local HTTP
through copied actual CMA callback verification, JWKS loading, bounded payload
parsing and generic observation. Valid callbacks authenticate with no owner token;
reply lifetime remains bounded by the receipt. Altered body bytes, another route
and another route key are rejected with the expected consumer reasons.
`/tmp/nyxid-botfather-review-cma-callback2/{executed-source,result}.json` records
all executed sources and the passing warnings-denied compilation. Database
claims are explicit fixture inputs and telemetry counters are no-ops; this is
not a full native admission or receipt-redemption test. The first isolated compile
selected a UUID library without v4 and a mismatched URL library; a later wrapper
visibility warning also failed warnings-denied. Retained attempts show the
corrections affected only harness linkage and wrapper declarations.

The same run executes the actual CMA Activity-construction block, validates that
Activity and runs the complete input/media renderer. Token-free outputs are in
`fixtures/{discord,slack,whatsapp}-verified-activity-input.json` and were sent to
the worker for persisted-receipt handler tests. All producer extracts still
match current source. The compiler slot is released. Registered42-file NyxID
and76-file upstream manifests and both HEADs independently remain unchanged.

R65's staged HTTP regression calls the production repair handler with a counted
local Telegram transport, compares retained legacy profile/credential BSON,
checks direct verify refusal and executes a positive legacy repair. R59/R66's
staged X conversion fixture supplies the real stored connection and billing
context, checks original transport payer/pending debit, insufficient-credit
zero-provider-effect behavior, and entered pending-to-verification barriers
for membership revocation and organization deactivation. Primary source review
found the tests targeted; they still require the coherent backend build and
execution. Native runtime enablement now shares the selected-transport and
credential evaluator used by native manager effects.


The callback3 variant changes only the synthetic input to canonical `/status`,
then repeats actual signed callback verification, Activity construction/validation
and input rendering for all three platforms. It passes with warnings denied and
exports positive-control fixtures for the worker. Callback2 retains its Unicode
substantive-text case; its `/status` plus argument was never a successful manager
control claim. Evidence: `/tmp/nyxid-botfather-review-cma-callback3/result.json`.
All outbound requests in these harnesses use loopback fixtures only.


Primary R55 execution compiled the complete exact staged migration module with
its six existing scenarios, the exact gateway/Discord index installers, and the
installation model. All six migration scenarios plus the exported startup wrapper
and three BSON date tests pass (10/0); isolated database cleanup was explicitly
run afterward. It covers replacement-before-retirement, interrupted and expired
lease recovery, concurrent startup, renewal during a longer operation, actual
ownership loss, and ready-publication/heartbeat overlap. All copied/extracted
production sources remain unchanged. Evidence is
`/tmp/nyxid-botfather-review-migration1/{source,result}.json`, `compile2.log`
and `run2.log`. The first harness run used a65-byte database name and failed
before migration; only the wrapper prefix was shortened for the corrected run.
Warnings-denied compilation passed. The error adapter and fresh real-replica-set
DB factory are isolated harness code; no full backend or integrated startup
claim. The compiler finished and its slot is released.

Primary reviewed the new first-event gateway HTTP regression and caught its
fixture mounting the compatibility responses handler, which supplies no runtime
producers. The worker changed it to the actual production responses_with_runtime
handler and confirmed the staged production route matches. The test now targets
a real signed native receipt, first404, actual PUT conversation, one native
follow-up and empty accepted gateway completion; execution remains pending.
The registered42-file manifest and HEAD independently remain unchanged.


Primary extended R61 with concrete pending failures: a mismatched Discord
verification key or revoked connection after the sticky marker cannot be repaired
through ordinary owner paths; changed configuring admin/funding also has no
reachable successor review. R67 separately records the same missing lifecycle
after successful native activation. The approved A8 sketch includes profile
updates, but the current exposed native routes only convert and inspect a pending
setup. Both fixes must retain separation markers and invalidate old authority;
no production mutation or exploit claim is implied by this source finding.

Primary continuation review traced both per-bot and shared managed webhook
dispatch into the native interceptor, selected gateway dispatch, retained receipt
redemption, canonical manager admission and snapshot-bound native delivery.
The current source keeps native callback content committed and does not select
ordinary owner routing on a selected-gateway failure. These are source findings,
not new backend execution evidence. The supported direct WhatsApp/Lark/Feishu
branches still need actual authenticated adapter-to-receipt/control/reply coverage;
the current native producer regression file covers Slack, Discord and X.

R67 acceptance also includes binding the retained receipt's manager authority
epoch inside the common command/admission transaction. Its present native lookup
checks profile/configuration/expiry, while the epoch comparison in
dispatch_native_admission happens earlier. The worker received this requirement
for the new lifecycle producer and an entered-before-update race. This is an
integration requirement for the pending lifecycle work, not a demonstrated
deployed exploit.

The worker's complete copied-backend metadata preflight1 exited1 after205.26s
with source_verified_after=true. The retained159 diagnostics include typed
legacy fixture/caller conversions and copied environment includes; this was
metadata only, without code generation or behavioral execution. A corrected
metadata2 is in progress under the worker's sole Rust slot. Primary independently
reverified all42 registered build23 hashes and all76 upstream next3 hashes plus
both HEADs; no source attribution drift was found.


Primary prepared `/tmp/nyxid-botfather-review-native-checkpoint.py` for the next
successfully compiled registered checkpoint. It requires nonempty native suite
selections, verifies frozen source/HEAD/executable and attribution before and
after execution, and refuses ignored/missing cases as a passing suite. Python
syntax validation passed; interrupted build23 was correctly refused before any
test execution or output-directory creation. This is replay preparation only.
The rollout worksheet now records the exact direct/gateway capability matrix
and the pending/active recovery acceptance required by R61/R67.

R68 is independently reproduced in the exact copied admin page at
`/tmp/nyxid-botfather-review-native-status1/{source,result}.json` and `tests.log`.
The two existing parent-page cases pass; two added native cases fail on the
unqualified Paused badge and Telegram client-setup guidance. Hooks and the runtime
child retain the original test mocks, so this proves parent rendering only.
The current external production page and copied page hashes still match.
Initial invocations used an unsupported Vitest minWorkers option and failed
before test execution; those logs are retained separately from the executed npm
test command. The worker has the concrete regression source.

The complete copied-backend metadata2 preflight exited1 after497.35s with
source_verified_after=true. Its earlier159 diagnostics are cleared in that
snapshot; two Send lifetime errors remain at the legacy/native conversion
handler routes. This is metadata-only evidence, without code generation or a
new backend executable. The worker retains the sole Rust slot for corrections.
Primary independently reverified the42 registered and76 upstream manifest
hashes and both HEADs without drift.

Continued lifecycle source review found the receipt manager-authority epoch now
compared inside the common admission transaction. R69 now uses explicit bot,
profile, connection, signing-source and gateway projections, with exact live-row
reservation fencing; its raw connection scope string still needs semantic
canonicalization to tolerate equivalent OAuth refresh results. R61/R67 also
need reviewed-update execution with an expired-but-refreshable connection:
reservation currently requires an unexpired snapshot before convert reaches
the existing refresh producer. Inspection must remain free of refresh effects.

The per-profile native update component clears its fixed channel ID after
successful conversion, although that select is disabled and the parent keeps
the same child instance. The worker received the concrete second-review case
under R67. These are mutable external draft findings, not deployed failures.

Independent native UI snapshot2 executes15pass/2fail across four focused suites.
The corrected parent card passes its native status/copy and retired cases.
The actual runtime child still reports Telegram transport failure for Discord;
the actual setup child loses its disabled channel selection after successful
update and cannot start another review. Hook responses are mocked and runtime
responses pass the production schema (HTTP contract1, independent of persisted
native identity2). Evidence and exact copied hashes:
`/tmp/nyxid-botfather-review-native-status2/{source,result}.json`, `tests2.log`.
The earlier diagnostic run used identity2 in that HTTP fixture; the final run
corrected it and added schema validation. Both failures were sent to the worker.

The corrected stable UI snapshot3 independently passes34/0 across six focused
suites, including the actual native runtime child, update-repeat regression and
actual-CMA sealed-ID cases. Full application TypeScript and repository-config
targeted ESLint pass. All21 external and copied hashes remain unchanged; the
worker's nine-file stable lifecycle manifest matches. Evidence:
`/tmp/nyxid-botfather-review-native-status3/{source,result}.json` and
`tests.log`, `types.log`, `lint.log`. This satisfies R68's presentation checks
and R67's repeated-review UI defect, without claiming backend lifecycle success.
The UI source hold is released; the registered repositories remain frozen.

Metadata3 exited1 after200.97s with its copied sources verified. It reports eight
mechanical BSON tuple-reference/array conversion errors after the transaction
closure rewrite. Corrections are external and metadata4 is running without code
generation. No new backend executable or behavioral pass is inferred.

R70's corrected external fallback now calls the common native transport,
credential and pending-marker evaluator and explicitly excludes pending setup
in its bot fence. Primary reviewed that structure; the initial no-controls
revocation/funding test remains required. R61/R67 now admit reviewed OAuth
refresh under live admin/funding and exact review/dependency fences before the
bounded refresh, then reload/fence material for the paused reservation. R69's
connection scope projection now uses a sorted set. The new regression uses the
actual local token endpoint and refresh producer; backend execution is pending.

Metadata4 completed exit0 after227.56s, with source_verified_after=true. This
successfully type-checks the complete copied backend test source without code
generation. Three unused-code warnings remain: InstalledCommand.verification_key,
platform_entry_discord_installation::install, and the superseded
platform_entry_identity::EventIdentity. They were sent for cleanup in the next
coherent batch, without a separate expensive build. The result does not validate
later external edits or establish any backend behavioral pass. Evidence:
`/tmp/nyxid-botfather-a8-metadata4/{source,command,result}.json` and `compile.log`.

The next external snapshot removes those three obsolete declarations while
retaining the production Discord observation's constant-time comparison of the
authenticated application's 32-byte verification key against captured bot
authority before command listing or mutation. Primary reviewed that path.

R70's new regression creates a fresh gateway through the real conversion
producer, obtains real reviewed_policy consent with no runtime controls, and
calls actual billing gate_and_reserve. The matched case requires a0.01 sponsor
reservation, funded probe and one meter; gateway retirement, either dedicated
key revocation and a pending-conversion marker each require a prepared probe,
zero reserve and no meter. The counted model provider intentionally receives no
request even in the positive case: this is funding-boundary coverage, with
model callback/dispatch still covered by separate integration gates. Source:
`platform_public_contract_service/tests/native_initial_probe.rs` in the external
overlay. Execution remains pending; the independent native replay runner now
requires this nonempty suite explicitly.

The gateway setup source uses the actual put_binding handler and covers fresh
activation, recovery of a retained ready record after a lost create response,
an incomplete reserved record requiring exact review and expiry of its
uncertainty window, two concurrent retry callers, and failed final activation
retaining ordinary-route separation. Retained ciphertext/key equality is
asserted; direct received HTTP body/idempotency equality was requested before
final freeze so recovery proves the actual repeated request as well as storage.
These tests are unexecuted. Metadata5 owns the sole Rust slot with copied source
frozen; primary independently reverified registered42/upstream76 hashes and
both HEADs unchanged and launched no compiler.

Metadata5 completes exit0 after207.97s with an empty compiler log. Primary
independently verified all1,122 copied manifest files without drift and saved
`/tmp/nyxid-botfather-review-metadata5.json`. The source-attributed result includes
the warning cleanup, bounded reviewed refresh, canonical scope projection, R70
funding regression and gateway setup tests. This remains a metadata-only check;
there is no newer runnable backend test binary or behavior pass.

Current lifecycle tests cover active credential rotation, harmless bot effect
counters, failed setup with a replacement owner-bound connection, and real
expired-token refresh with equivalent reordered scopes. They still require
execution. The successor case currently changes the connection, not the
configuring administrator/funding authority. Those successor-authority and
active gateway replacement/recovery cases remain required, together with old
receipt/proof refusal after completed update and explicit reenable, paired with
fresh native authority success; refusal during the pending pause alone cannot
establish continued revocation.

The added external direct-platform tests delegate to actual WhatsApp and
Lark/Feishu adapter authentication/decryption and stored credential production,
then retain a native receipt and execute the real command and durable delivery
producers against counted local endpoints. Their setup seeds native profile and
control state; they do not establish native administrative conversion. Exact
outbound text equality was requested in addition to destination, sent-ID and
no-duplicate assertions. These later external tests still need final snapshot
attribution and execution.

The subsequent source review confirms exact committed/rendered text assertions
for WhatsApp and decoded Lark/Feishu replies. Gateway retry tests now compare
received HTTP body bytes, the record-ID idempotency header, both dedicated key
hashes and the short-lived delegated creator identity. These edits are later
than metadata5 and remain unexecuted.

Reviewed native update now applies the setup uncertainty delay only to
dispatched gateways; observed active gateways retain historical deadlines and
must remain immediately reviewable. Primary flagged the initial optional
deadline parse, which allowed a dispatched row without a valid deadline to
proceed. The corrected branch requires that deadline and refuses missing or
malformed values before retirement or key mutation. Required paired execution
cases are observed/recent allowed, dispatched/recent refused, and
dispatched/missing refused with unchanged gateway and keys. Source hashes and
review scope are retained in
`/tmp/nyxid-botfather-review-lifecycle-followup.json`. Disk remains approximately
4.7GiB; no additional compiler was started by primary.

New lifecycle source regressions now exercise a real command and concurrent
validate_api_key touches between review and reservation, active gateway
replacement, actual DELETE-binding retirement and reviewed recovery, and a
successor administrator with new funding after the prior administrator loses
its role and organization membership. The successor case retains the original
bot/connection owner and checks the real settled verification meter's payer and
actor. The lifecycle snapshot now also includes the exact retired transport
still selected by the profile so its dedicated keys are revoked during recovery.
All remain draft execution requirements.

Primary found two test issues before the next snapshot: the dispatched-deadline
case expected recovery on the first incomplete create, although that first call
returns an error and only a later observation offers recovery; and the rotation
test's fresh positive stopped at browser approval without same-chat confirmation.
The worker received both corrections, including the requirement to assert the
new binding and logged-in status after successful fresh confirmation. The old
receipt and browser-approved challenge are already carried through update and
reenable in that draft. Existing public manager_filter and persisted revocation
markers both use manager_authority_epoch <= current epoch, so they already
include older epochs; no speculative cleanup change is required.

Both test corrections are now present in the external source: incomplete initial
creation asserts an error; fresh signed confirmation completes an active binding
for the exact account/current epoch and a further signed status reports LoggedIn.
Primary reviewed the retired-selected-gateway query and transactional key
revocation without a further source blocker. It remains profile-scoped and
excludes unrelated retired rows. The three affected external files differ from
metadata6 while its copied files remain unchanged, so these revisions belong to
the next metadata snapshot. Review hashes are in
`/tmp/nyxid-botfather-review-lifecycle-followup2.json`; execution remains pending.

Metadata6 completed exit1 after192.86s with source_verified_after=true. Primary
independently verified all1,123 copied source hashes. Its sole E0308 is the new
direct-platform helper returning Zeroizing<String> where its signature requires
String. The external correction uses expected.to_string(); it does not change
the failed snapshot. Evidence is retained in
`/tmp/nyxid-botfather-review-metadata6.json` and the original metadata6 directory.
Metadata7 now includes that correction, the later lifecycle tests and the new
native/legacy journeys; it is still a metadata-only check, not an executable.

The new Discord personal journey exercises real signature authentication,
native prepare/receipt production, authenticated HTTP browser review/approval,
same-chat confirmation, automatic durable delivery, HTTP history/list and a
browser turn through the real HTTP proxy against a counted local server, then
logout. Primary verified AuthUser extraction and automatic-consumer transport
wiring by source. The setup seeds a converted profile and controls; native model
output comes from the in-process SavedReplyEgress fixture. It neither exercises
the webhook HTTP handler nor proves native administrative conversion or a real
model response. Source: the external
`platform_manager_journey_tests/native_personal.rs`.

That first draft authenticates the browser as the bot owner, so it cannot detect
accidental owner fallback. Primary requested a distinct personal account,
operator history/turn refusal, and explicit credential/session reuse checks
against the actual browser HTTP request. The comment claiming encrypted-key
preservation currently exceeds its assertions. An unauthenticated review
refusal was requested beside the authenticated positive. These are test-evidence
gaps; no production defect is inferred. Metadata7's copied source remains fixed.

The new raw-v1 compatibility test independently constructs the legacy HMAC
framing and persists old-layout intent/challenge documents, then redeems them
through the revised service and sends a saved Telegram personal reply. Primary
compared its proof parts with the frozen v1 implementation. That tests persisted
proof compatibility followed by new origin/delivery production; it does not
itself load a pre-existing saved personal origin/delivery. The separate model
fixtures retain legacy thread and negative-group delivery BSON checks.

The independent native checkpoint runner now requires16 nonempty suites,
including the new native personal and legacy proof modules, and continues to
reject an interrupted or metadata-only build. A proposed copy with only
unrelated tests disabled may establish bounded behavioral evidence if its exact
production, selected test/helper inputs and exclusions are attributed. It does
not replace successful full compilation and compatibility replay of the final
registered checkpoint. No primary compiler was started.

Metadata7 completed exit0 after197.64s with an empty compiler log. Primary
independently verified all1,125 copied inputs and both owned compiler/wrapper
PIDs absent; evidence is `/tmp/nyxid-botfather-review-metadata7.json`. The
registered42 and upstream76 hashes and HEADs also remain unchanged. The new
journeys and lifecycle cases in this snapshot type-check without code generation;
their behavior remains unexecuted.

The subsequent native journey now uses a distinct personal account, requires
unauthenticated review401 and operator history/turn404, checks conversation/key
ownership, and compares the actual browser bearer hash, retained encrypted key,
credential row count and session. Primary caught a doubled conv_ prefix in the
new outbound-session assertion: the stored nyxagent_session_id already contains
that prefix and upstream_body sends it unchanged. The worker received the exact
correction. This later slice is outside metadata7 attribution.

The external assertion is now corrected and independently reread: the captured
session must equal the synthetic full conv_ identifier, and the outbound browser
conversation compares directly to that saved value. No further source finding
in this journey pass; the stronger assertions remain outside metadata7 and
require their own executable attribution. The16-suite replay guard was also
checked against build23 and correctly refuses its interrupted/no-pass outcome.

Static public schema comparison found exactly one remaining definition mismatch:
public_signup_link.description. The external native wording is56 UTF-8 bytes
longer than the frozen upstream Telegram wording. All three instruction/compact
constants match. All four historical raw Codex requests still contain the old
description; after aligning definitions, strict normalization will reject those
unchanged requests before callback projection. Preserve raw captures and either
recapture with the local pinned runtime or explicitly label separately derived
schema-adapted requests. Actual Rust normalization/callback equality and full
cold-resume/compaction/storage-bound replay remain required. Static evidence:
`/tmp/nyxid-botfather-review-public-schema-preflight.json`. No upstream source was
changed and no new runtime execution is inferred.

Selected1's separate codegen candidate is independently audited at
`/tmp/nyxid-botfather-review-selected1-source.json`: all1,834 copied inputs match,
and the only transform removes9,166 test-registration attributes across704
files outside19 selected files. Function bodies and helpers remain unchanged;
nine reason-bearing ignore attributes remain on unregistered functions. The
82 retained dependency files have separately recorded hashes. Selected1 exits1
after203.01s with only E0277 in the paid-X test's Value versus Zeroizing<String>
assertion. All copied hashes still match and both owned PIDs are absent.
Evidence: `/tmp/nyxid-botfather-review-selected1-outcome.json`. No executable or
behavioral pass was produced.

The paid-X fixture uses actual native conversion, retained command, durable
delivery and ordinary channel billing with a counted loopback provider. Primary
identified two evidence gaps: an if-let could skip send-meter attribution, and a
sequential stalled connection stopped accepting later connections, hiding a
possible resend. The next copy requires exactly one attributed send meter on
attempted cases, none when unfunded, an unchanged verification meter, and an
active accept loop with per-connection workers stopped and joined on cleanup.
Its expected header-stall forwarded hold and201-body-stall settled charge match
the existing production settlement boundary by source.

Selected2 is separately frozen and compiling with the same owned2GiB guard.
Primary verified all1,834 inputs and independently reproduced the same limited
registration transform; only the paid-X fixture differs from selected1. Source
audit: `/tmp/nyxid-botfather-review-selected2-source.json`. The independent
selected-test runner is `/tmp/nyxid-botfather-review-selected-checkpoint.py`; it
requires a successful attributed executable, the independent transformation
audit and16 nonempty suites, and labels its results as narrow behavioral
evidence. Its guard correctly rejects failed selected1. It does not relax the
full registered-checkpoint runner or release gate.

A later external compatibility case now replaces a saved message/thread and
pending delivery with literal old BSON, retaining the original v1 dedupe key,
then decodes, renders and sends through the existing durable row and checks
duplicate refusal. Primary caught one expected-metadata mismatch: enqueue marks
platform_delivery_recovery_queued=true, whereas the test's final whole-document
comparison still expected false. The worker received the correction to assert
that transition and preserve the remaining old document exactly. This case is
outside selected2 and remains uncompiled/unexecuted.

The later legacy saved-row regression now explicitly requires the recovery flag
to become true and compares every remaining saved-message field to the literal
old BSON fixture. Primary reread the correction; no production enqueue change
was made. Source evidence is
`/tmp/nyxid-botfather-review-legacy-retained1.json`. This resolves the assertion
finding only; the new case is still outside selected2 and awaits execution.

The new external native_paired draft connects actual native gateway conversion,
v2 discovery/readiness, pinned Codex, real MCP and funded model callbacks to a
scripted local provider. Primary found two evidence gaps: the timeout bounds
run_public_turn spawning rather than the later event drain, and ReplyReady is
counted without normal final delivery. The draft currently asserts only the
protected signup send and decrypts the stored final answer. Feedback requests
a bounded completion and the production final delivery consumer with exact
signup/final sends and duplicate refusal. Evidence:
`/tmp/nyxid-botfather-review-native-pair1.json`. This test and the neutral FAQ
content are later external drafts; selected2 remains unchanged.

Selected2 completed exit1 after301.44s at the final linker. Primary verified
all1,834 copied hashes and both owned processes absent; the only error is missing
Swift compatibility symbols from the retained apple_metal archive. A separate
relink completed exit0 in1.01s using the retained17 crate objects and installed
Swift compatibility paths. Primary independently expanded the compressed linker
diagnostic, verified all471 object/archive inputs (1,024,861,120 bytes), and
matched the executable SHA256
`93a9bbb179d1227abe07f8a690c3ba688df00d50f6be937e1dfeadcc5c77d30b`.
The unavailable temporary symbols.o is explicitly omitted; original crate-object
order was absent from the diagnostic and retained objects were sorted. This is
a separately attributed narrow executable, not a successful Cargo checkpoint.

Its list contains78 tests; all16 required native suites are nonempty and select
74 tests. The independent runner now accepts a relink only with its separate
source/input audit and preserves the failed-build refusal without that audit.
Evidence: `/tmp/nyxid-botfather-review-selected2-outcome.json` and
`/tmp/nyxid-botfather-review-selected2-relink1.json`. Worker serial execution has
started; primary has not yet replayed behavior. Registered42 and upstream sources
remain held, and the original selected2 failed result remains intact.

Worker selected2 run1 executed all16 suites with the original stack setting.
Completed suites reported39 passes and4 failures; four suites aborted on stack
overflow, so their partial output is not added to those counts. Failures include
Discord uncertain-absence retry, a relabel fixture using an invalid sealed ID,
native initial readiness blocked by gateway setup, and a render-text assertion.
Gateway partial logs also show setup failing before any POST. Run2 retries only
the four aborted selections with RUST_MIN_STACK=33554432 on the same executable.
Neither run is checkpoint acceptance.

R71 traces the gateway failure to fresh ApiKey.created_at precision: transport_key
uses Utc::now(), MongoDB stores milliseconds, and fence_key compares the live
decoded key to the original object while retaining created_at. Primary sent the
exact source trace and requested a persisted-representation fix with a deliberate
sub-millisecond regression. The new independent runner records the effective
stack environment; primary behavioral replay awaits worker run2 release.

Primary independent selected2 relink replay completed all16 nonempty selections:
59 passed,15 failed,0 ignored, with no aborts using the recorded32MiB test stack.
All source, binary and471 link-input hashes were checked before and after each
suite. Evidence: `/tmp/nyxid-botfather-review-selected2/results.json`. These
results reproduce the worker run1/run2 failures. Migration, real signed ingress
positives, claimed-send deadline/cleanup cases and direct WhatsApp/Lark/Feishu
producer cases pass within this narrow executable; setup/lifecycle, funded-X
fixture, missing-handle outcome, native personal journey and rendering failures
remain. The literal old-proof redemption and native-repair HTTP cases also pass.

A bounded LLDB attempt could not launch the retained executable before its60s
timeout, so it supplies no further explanation of the Discord uncertain-absence
retry Conflict. Primary verified wiremock scoped-guard removal is synchronous
and requested nonsecret test-only stage evidence (attempt/state/history and
local method/path counts). No production recovery bypass was suggested.

Selected3 direct codegen completed successfully in318.1s. Primary independently
verified all1,835 copied inputs and executable SHA-256
`5e19c1bb6683f86a9d6e19bdcda7f4c81b6cf99bad76d142ebdda73da8cc16a1`.
The transformation audit reproduces11,051 removed test-registration attributes
across867 files, with20 selected files unchanged; dependency source exclusions
are recorded although their retained archives are not rebuilt. Evidence:
`/tmp/nyxid-botfather-review-selected3-source.json`. This remains selected-test
evidence rather than a complete registered Cargo checkpoint.

Worker selected3 replay executed all16 suites:65 passed,10 failed,0 ignored,
with32MiB test stack. Primary verified all result/log hashes in
`/tmp/nyxid-botfather-review-worker-selected3-run1.json`; independent behavioral
replay is running separately. Gateway setup now reaches its local consumer,
whose fixture compares an unprefixed stored API-key hash against a prefixed CMA
digest and also double-prefixes its generated agent-key hash. Those failures do
not yet prove the complete R71 conversion positive. Native personal browser
approval reaches the existing750ms proof-decision spacing and returns429.
The paid-X fixture observes finalized metering before deferred wallet release;
its wallet assertion must await the actual release. R72 describes the remaining
production final-capture refusal classification gap. Discord uncertain-absence
retry is still unexplained. All corrections remain outside frozen selected3.

Primary audited the external native-schema derived upstream fixture. It changes
exactly eight signup descriptions (four Codex requests and four expected callback
requests), adding56 bytes per request, plus explicit schema-adaptation metadata.
The original raw fixture SHA-256 remains
`1990572c7a859efbe4b0521e952fe716b08cdd3b344e44ce8ddde9c1228e5e64`;
compact input/output and original provenance are unchanged, and both repositories'
external derived copies are byte-identical. Evidence:
`/tmp/nyxid-botfather-review-derived-native-fixture1.json`. Static expected callback
JSON sizes are3940/4763/5462/5973 bytes; actual Rust normalization, cold-resume and
compaction replay remain required. These are derived inputs, not new captures.
The native paired fixture also needs its later personal-enabled correction and
must honor a legitimate Deferred final reply after protected signup consumes the
same one-send-per-second public lane. Its shared100s execution/drain/delivery
bound and eventual exact Sent/duplicate/no-extra-send assertions remain required.

Independent selected3 replay is complete:65 passed,10 failed,0 ignored across
all16 nonempty suites, reproducing the worker results without timeouts or aborts.
Every source/binary/result/log hash remains valid. Evidence:
`/tmp/nyxid-botfather-review-selected3/results.json`. Both literal old-HMAC proof
redemption and the imported pre-existing BSON thread/message/pending-delivery
journey pass. The selected copy remains frozen; later corrections are unexecuted.
The gateway deadline-refusal test also prints an HTTP-fixture task panic while
reporting a pass. Its expected initial error masks failed fixture validation;
primary requested explicit propagation and an intended-reserved-response witness.

The worker's separately attributed Discord diagnostic reproduces the recovery
failure after a new attempt is Dispatched, history has one row, and a second POST
has occurred. This narrows the error to provider response/verification or its
completion write. Primary found an additional concrete fixture cause in retained
wiremock0.6.5: `MountedMockSet::handle_request` sorts its mock vector in place by
priority, but scoped `MockId` retains its original vector index. The priority1
failure mock moves to the front, so dropping its guard deactivates a different
mock and can leave the503 response active. The worker is checking a deterministic
single POST responder instead; no installer authority or recovery weakening is
justified by this failure. The diagnostic is not a full-backend acceptance run.

The corrected single-responder fixture passed the worker's diagnostic3 run:
4/0, including the uncertain-create recovery case and three BSON date helpers.
Primary verified its retained extract/harness hashes and outcome/log; the complete
backend replay of that correction remains pending. The external gateway fixture
now records caught validation panics and checks them at conversion/close, and the
deadline case requires one completed reserved response. The external R72 cleanup
is limited to native Forbidden/ValidationError failures and retains the original
deadline. Both source corrections are reviewed but still await selected execution.

The worker registered exactly the three audited upstream native-schema files and
retained the next3 server binary. Primary now owns a bounded upstream replay at
`/tmp/nyxagent-botfather-review-upstream-native1-results.json`; formatting,
whitespace and Clippy with warnings denied have passed. All-target tests, explicit
pinned local public smokes and server build are still running. The runner records
all source files and stops only its owned process group if free disk falls below
2GiB or a stage exceeds its deadline. No upstream pass beyond completed stages is
claimed, and the unchanged raw capture remains preserved.

Native1 tests and actual Rust normalization passed, but primary rejected its
standalone server artifact: the SHA was still next3 and its strings retained the
old Telegram-only description. Registration had preserved an older source mtime,
so Cargo reused stale non-test output. The outcome and rejection are retained in
`/tmp/nyxagent-botfather-review-upstream-native1-artifact-audit.json`. Source
content hashes were preserved while own-crate source mtimes were refreshed.

The separately attributed native2 run passed formatting, whitespace, Clippy with
warnings denied, all targets94/0/9, public22/0/0 including three pinned Codex local
smokes, and the server build. The new server SHA-256 is
`d71f74b8521c3aabe0ab90f6830431cba07ae13ad5ffe5266c15281967a34953`;
its new neutral description is present and the old description absent. Actual
Rust normalization exactly equals the derived fixture JSON. Full-loop normalized
bytes are2741/3530/4211/4704/5605/6394/7075/7568, compact7042, then cold resumed
full-loop3617/4406/5090/5586. This run saved17,455 then39,601 session bytes and
10,039 then28,033 rollout bytes. Evidence:
`/tmp/nyxagent-botfather-review-upstream-native2-results.json` and
`/tmp/nyxagent-botfather-review-upstream-native2-artifact-audit.json`.
The Rust slot was released after all owned commands exited. Native2 is accepted
for local pairing; actual NyxID pairing and deployment remain separate gates.

Selected4 is compiling the eight reviewed deltas on a separately frozen copy.
Primary verified1,836 copied inputs, unchanged20 selected files and the same
11,051 registration-attribute removals, plus1,502 retained dependency/native
files covering every82 explicit extern inputs. The source, compiler wrapper and
dependency manifests are recorded in
`/tmp/nyxid-botfather-review-selected4-source.json` and
`/tmp/nyxid-botfather-review-selected4-dependencies.json`. No codegen or behavior
outcome is yet claimed. The replay runner now revalidates these dependencies
before and after each suite as well as source and executable identity.

FAQ parity review found that native2 still compiles the historical upstream FAQ
fixture. The staged neutral mirror is byte-identical to selected4's approved
eight-article corpus (SHA-256
`3e1f9284ff8e2fe0c57a1797aff3a470f5b9e1fa65dee7be06fccb8f9f1f64bd`,
4,599 bytes). Both upstream's scripted read and the actual selected4 pairing
select the first article, whose body is unchanged at 246 bytes. The new
login/status article is 1,020 bytes; the personal article is now 573 bytes.
Copying the fixture alone will not exercise those longer bodies. Native2's
measured continuation/compact/cold-resume sizes therefore remain evidence for
its selected historical scenario. The oversized Unicode refusal smoke is a
separate negative case, not successful approved-article coverage.

Keep native2 unchanged through current pairing. At the subsequent coordinated
checkpoint, the upstream fixture must honor the requested article ID and
exercise the longest approved article through continuation, compaction and cold
resume within the existing limits. The source-only comparison is retained at
`/tmp/nyxid-botfather-review-faq-parity1.json`; no new behavior is claimed.

Selected4 subsequently stopped under its owned disk guard after 329.36 seconds,
with 1,874,800,640 free bytes at the trigger. Its recorded exit is 1 with reason
`owned-selected-build-interrupted-disk-headroom`; no executable or test/pairing
pass was produced. Primary independently reverified all 1,836 source inputs,
1,502 dependency inputs and previously audited manifests, and confirmed the
recorded wrapper/rustc processes were absent. The retained compile log has no
Rust error headers. Seven codegen units (05–11) are absent and two (04/12) are
empty, so these outputs do not support relink-only recovery. No retry or cleanup
was performed by primary. Evidence:
`/tmp/nyxid-botfather-review-selected4-interrupted.json`.

The worker launched a separately attributed jobs1 attempt using the exact frozen
selected4 tree and dependencies. Primary verified every input again, the sole
compiler-argument change (output path), and the wrapper's inherited empty pipe,
matching MAKEFLAGS/CARGO_MAKEFLAGS, removal of inherited MFLAGS, descriptor
lifetime, 2GiB guard and 1,200-second deadline. A direct descriptor observation
confirmed that the owned rustc held both pipe ends. Installed rustc documentation
supports this jobserver mechanism; it constrains available codegen job slots,
not total threads or memory. This is a concrete concurrency change from the
interrupted attempt, with no source/exclusion change.

The review is retained at
`/tmp/nyxid-botfather-review-selected4-jobs1-source.json`. Separate focused and
paired replay runners preserve prior runner files and require the reviewed
retry attribution, completed input verification and absence of unusable-jobserver
warnings. No jobs1 compile or behavior outcome is yet claimed. Native2 and the
registered NyxID source remain frozen.

The external longest-article FAQ smoke now selects the real 1,020-byte article,
honors the requested read ID, rejects unknown IDs, and dynamically requires full
ordinary turns, a compact callback and a resumed ordinary turn under the existing
byte/session limits. Source review of tests SHA `1d95291728e61cf6f681d2959efa5b8c202e7901be3d94ac9bcf4f91f1eb4eb9`
requested one assertion correction: a global count of repeated article outputs
can hide a missing later-turn output. Each normal turn must tie its own read
call ID to the complete body in both following callbacks. Also compare the saved
runtime thread ID across the sequence. This remains external, uncompiled and
unexecuted; native2 is unchanged. Review:
`/tmp/nyxagent-botfather-review-faq-draft1/review.json`.

The revised external tests SHA
`da7165f64aef7b521fb1e04dcd2fc4b2f754f9d09f0b2daf60f9de7b2aed7760`
addresses both source findings: every normal turn requires a fresh read call ID
and exactly one complete matching result in each subsequent callback, and every
saved state retains the same nonempty runtime thread ID. The full article remains
in the synthetic confirmed summary and is checked after restoration. Primary
reverified the unchanged native2 source manifest and found no further issue in
this assertion slice. Compilation and actual pinned-runtime execution remain
pending. Snapshot/review:
`/tmp/nyxagent-botfather-review-faq-draft2/review.json`.

The separate jobs1 build completed successfully in 438.97 seconds. Primary
independently verified the 242,934,368-byte executable SHA-256
`70870dbffbc53d8a28bfa8956619da90c157432102d5d425e37f60e0727b2343`,
all 1,836 source and 1,502 dependency hashes, prior retry/wrapper/command/compiler
attribution, absence of unusable-jobserver warnings, and exit of both owned
compile processes. The original selected4 interrupted outcome remains intact.
Evidence: `/tmp/nyxid-botfather-review-selected4-jobs1-artifact.json`.
This establishes a runnable selected-test artifact, not behavior or complete
registered acceptance. The worker is executing its serial suites and actual
native pairing first; primary independent behavior replay is held until release.

The worker's jobs1 serial run finished 74 passed / 1 failed / 0 ignored across
16 selections. Primary checked every suite summary and retained log hash in
`/tmp/nyxid-botfather-review-worker-selected4-jobs1-run1.json`. The remaining
native-personal failure is at the counted browser `/v1/responses` request lookup,
after native delivery/history assertions. Its preceding check only searches for
the `turn.completed` event name; production also emits that event on failed or
cancelled settlement. Exact terminal status/error diagnostics are required before
classifying the missing request or changing the fixture.

The worker's actual native pairing failed 0/1/0. Frozen source locates the failure
at the first `probe_public_readiness(...).await.unwrap()` (line 378), after
discovery. It did not return Ready and did not reach runtime activation or the
four-tool turn. Earlier prose attributing the conflict to controls update is
corrected; the original log/result is unchanged. Stage audit:
`/tmp/nyxid-botfather-review-worker-native-paired-jobs1.json`. The worker released
behavior ownership, and primary is now replaying the same suites serially before
the exact pairing. Source, dependencies and native2 remain frozen.

Primary independent jobs1 replay is complete: the 16 suites reproduce exactly
74 passed / 1 failed / 0 ignored, and the exact native pair reproduces 0/1/0.
The native-personal failure remains line 578; the first readiness probe remains
line 378. Neither run timed out. Source, dependency, executable, upstream and
runner/log identities were verified throughout. Results:
`/tmp/nyxid-botfather-review-selected4-jobs1/results.json` and
`/tmp/nyxid-botfather-review-native-paired-jobs1/result.json`; cross-run comparison:
`/tmp/nyxid-botfather-review-selected4-jobs1-replay-comparison.json`.

The newly passing cases establish the selected local Discord uncertain-install
recovery, complete gateway setup/recovery, native refusal cleanup, paid-X terminal
accounting and initial funded-probe transport fences. Finding statuses above now
name that narrow evidence. The full native personal/browser/logout journey and
actual four-tool public pairing remain incomplete. Complete registered backend,
ordinary management/account parity/R12/R45/R46 replay and final FAQ bounds remain
separate gates. Primary released behavior ownership with both runner sessions
exited; frozen inputs and all prior outcomes are retained.


### PR completion request and current-base preparation

The user explicitly requested completion of the end-to-end behavior followed by
a PR, with continued monitoring and correction of CI and merge conflicts. This
authorizes the eventual branch push and PR creation. It does not supply
deployment or real outbound-provider authorization.

On 2026-10-17, primary fetched `origin/main` at
`64116aace2fd4ac71c171c40a9efa9d2e99b85ed`. The registered HEAD is two commits
ahead and four behind. Both the committed branch and a separately attributed
135-file combined registered/external preview merge cleanly with this base.
The preview used a separate Git index and unreferenced objects; the working
branch, real index and compiled sources were unchanged. Evidence:
`/tmp/nyxid-botfather-review-pr-preflight1/result.json`. The exact final source
must still be integrated and checked against the then-current base.

CI requires workspace formatting and all-target Clippy with warnings denied,
full backend nextest against a transaction-capable MongoDB, billing route smoke,
frontend lint/tests/build, and wizard bundle freshness for frontend changes.
Selected native evidence does not replace ordinary manager/account-parity
execution or these complete checks. R73 records the default test-stack gap.

A separate merged frontend closure was extracted with 1,908 hashed inputs.
Primary stopped its owned lint wrapper/process group when free disk fell below
1 GiB; no lint, test or build pass is claimed. The 29 MiB source copy and
interruption record remain at
`/tmp/nyxid-botfather-review-pr-frontend1`. Heavy checks will be serialized and
bounded after resource headroom recovers; unrelated jobs/caches are preserved.


### Durable recovery after temporary artifacts disappeared

On the 2026-10-17 continuation, the previously recorded `/tmp/nyxid-botfather*`
and `/tmp/nyxagent-botfather*` source, binary and execution directories were
absent. Neither reviewer nor worker removed them in this task. The registered
checkouts remained intact. Earlier outcomes above are historical recorded
observations; their former local artifact links are no longer available for
replay, and fresh executable verification is required.

The complete combined NyxID source survived in Git commit
`ac3c4e17d227f27c0c3fd20b76d89350fa0e6e1a`, including both diagnostic test
changes. Its main-integrated preview tree is
`a2b8939c4df866e7421b978fa5c9a085cddc2672`. Both objects are now protected by
local recovery refs. The worker recovered the complete tree to
`/Users/chronoai/Library/Application Support/heca/recovery/nyxid-a8-20261017/source`,
with file hashes in its sibling `recovery.json`. Primary independently preserved
the 135 changed files and original object IDs in the durable Heca artifacts root.

Unchanged upstream native2 source is anchored at recovery commit
`a583712f97baca78fa5713affb7882eaa715757f`, with its complete 77-file source and
manifest under the durable `heca/recovery/nyxagent-native2-20261017` directory.
The original raw capture still hashes to `1990572c7a859efbe4b0521e952fe716b08cdd3b344e44ce8ddde9c1228e5e64`.
The actual installed Codex 0.160.1 Mach-O executable still hashes to
`09fa44fdc37a5fc70dc1ace31235f90468a2e193d0e85f7552eab068ea2582be`; a newly
generated wrapper pins that executable directly. A wrapper of the npm launcher
was rejected before use because it would not pin the underlying executable.

The longest-article FAQ draft was reconstructed separately from the retained
requirements and neutral NyxID corpus. Its new source hash differs from the lost
draft and has no inherited compilation or execution claim. Registered native2
remains unchanged while its matching server is rebuilt for diagnostic pairing.

The unchanged recovered native2 rebuild is now complete. Locked Cargo checks
passed formatting, all-target Clippy with warnings denied, all targets
94 passed / 0 failed / 9 ignored, and the explicit `public::tests::` filter
18 passed / 0 failed / 0 ignored, including three pinned Codex smokes. Four other
public tests also passed in the all-target run; the explicit filter count is
18, not the historical broader filter count of 22. Actual Rust-normalized
fixture output exactly matches the labeled derived capture. The rebuilt
standalone server reproduces SHA-256
`d71f74b8521c3aabe0ab90f6830431cba07ae13ad5ffe5266c15281967a34953`.
Source hashes and owned-process exit are recorded in
`heca/recovery/nyxagent-native2-20261017/check1/final-audit.json`.

The worker now owns the sole Rust slot for the durable `selected-recovered1`
diagnostic harness. It includes the two preserved runtime diagnostics and R73's
localized future-construction threads. Its selected test registration and
retained dependency provenance limitations remain explicit; this is not a full
registered Cargo or CI pass. The next execution will remove `RUST_MIN_STACK`
and check all selected suites before pairing with the exact rebuilt native2.

Recovered1 was stopped after a harness-only error: the direct compiler command
omitted the empty `CARGO_PKG_DESCRIPTION` metadata normally supplied by Cargo.
Recovered2 retained identical source/dependency/compiler identities and added
that metadata, empty authors and expected `test`/`docsrs` configuration names.
Its source symlink, changed output and Cargo manifest path spellings are
separately attributed. Primary independently reconstructed all 11,051 attribute
removals across 867 files and verified all 4,021 inputs against the recovered
source plus six R73 files; no additional function-body change was present.

Recovered2 reached object emission but its unchanged owned-process disk guard
stopped it after 909.87 seconds (exit -15, minimum 1,308,581,888 free bytes).
All source and 3,202 retained dependency hashes verified afterward, with an empty
compile log and no unusable-jobserver warning. It produced no executable or
behavior evidence. The original diagnostic stage therefore remains unexecuted.
The next bounded candidate will select the two remaining end-to-end tests and
include the independently confirmed R75 fixture grammar correction. Repeating
the known malformed fixture is not a prerequisite to verifying the corrected
journey. Complete registered-source and default-environment gates remain open.


The first two-test candidate, diagnostic-recovered3, was stopped after 73.04
seconds because it was based on the pre-main recovered snapshot. Its sources
verified, compile log was empty, and it produced no executable. The replacement
`diagnostic-recovered4` uses the complete integration tree based on main
`64116aace2fd4ac71c171c40a9efa9d2e99b85ed`, with R75's corrected responses and
R73's erased-future helper. Primary independently verified all 4,062 source
inputs and reconstructed exactly 11,201 registration-attribute removals across
893 files. Both selected test files, all production code and helper bodies
match the captured integrated originals. Source manifest SHA-256 is
`d7d593803601062f46fcc414d3340f382b626fc8c17d5e4c3619073092d60156`.
The independent transform audit is retained under the durable review root as
`diagnostic-recovered4-transform-audit.json`. Compilation and both runtime
journeys remain pending at this entry; retained dependencies and Rust 1.94.1
still limit this to diagnostic evidence. Full Cargo/CI on Rust 1.98.1 remains
required. The original malformed diagnostic has not been re-executed after
recovery, and its exact runtime cause is not newly established by source review.


Diagnostic-recovered4 was also stopped by its owned disk guard after 294.70
seconds (exit -15, minimum 1,846,747,136 free bytes). Its empty compile log,
4,062 source files and 3,202 dependencies verified afterward. It produced no
executable or object set and supplies no new runtime evidence. Repeated local
monolith attempts are now discontinued. The reviewable integration will be
published as a draft PR to obtain a fresh, full Rust 1.98.1 Cargo/CI build;
completion still requires passing the pending journeys and all CI gates.
A paired job in the private NyxAgent repository can check out an exact public
NyxID commit and run the full native suites plus explicit legacy/native pairs
without sharing private source or introducing cross-repository credentials.
