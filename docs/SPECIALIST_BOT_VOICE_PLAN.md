# Specialist platform voice diagnosis and implementation plan

Implementation baseline: `616a9131` (then-current `origin/main`, NyxID 0.71.0). The earlier checkout
was stale; observations from it are not evidence for this implementation.

## Required behavior and scope

A person can use billed platform OpenAI voice in their private specialist thread
without creating an OpenAI connection or adding an OpenAI tool grant to that
specialist. Audio transports conversation; all work still runs through the
thread's ordinary scoped Agent Key. BYOK requires the selected connection's
grant. Authored operation restrictions, live platform ACLs, rollout flags,
duration pricing, wallet admission, and person/org/thread/key fences remain
authoritative. Work is performed by the requested Heca implementer, with the
parent assistant reviewing personally. Project edits stay in this worktree;
verification uses an isolated temporary database. The initial implementation
handoff excluded commits, pushes, deployments and live changes. The user later
authorized a pull request; its integration and verification are recorded in the
[personal review](SPECIALIST_BOT_VOICE_REVIEW.md#pull-request-preparation).
No additional agents/subagents were used.

## Proven baseline defects and retained contracts

- `docs/chat/11-voice.md` sections 1, 4 and 12 specify private specialist voice,
  the bounded assistant inference policy, unchanged tool authority, live checks,
  and platform billing to the acting person after credential classification.
- `docs/AGENT_OPERATION_SCOPES.md` requires voice to enforce authored operation
  restrictions and resolve BYOK by its selected connection identity.
- At baseline, `voice/credentials.rs::authorize_inference` compared the catalog
  `DownstreamService.id` with `AuthUser::api_key_service_scope()`. That helper
  returns restricted keys' effective service allowlist. The ordinary connection
  IDs in that allowlist are `UserService` IDs, so the catalog-ID comparison
  rejected a granted BYOK connection. It also rejected grantless specialists
  before platform credential resolution. Both failures were reproduced locally.
  `effective_allowed_service_ids_with_access` does merge platform catalog grants;
  the defects are the connection/catalog identity mismatch and missing bounded
  no-grant inference policy, not omission of platform grants from that helper.
- `credentials::resolve` calls this authorization for both platform and BYOK
  before resolving credentials. The OpenAI runtime calls `credentials::resolve`
  again during live revalidation, so a start-only workaround would be incomplete.
- At baseline, `assistant_voice::options` prechecked authorization only for xAI
  at catalog level. OpenAI advertised options that start refused; selected BYOK
  connections were not independently checked. Official origin checks were also
  missing for OpenAI discovery, and own connections lacked read-only credential
  eligibility checks. The fixes retain the response shape and tariff/flag reasons.
- Platform resolution already uses `resolve_catalog_platform_target`, the shared
  platform ACL, fixed provider origins, separate rollout flags, and
  `BillingOwnerResolver::resolve_for_execution`. These controls must be reused.

## Limits of the available evidence

The original report has no attached trace or reproducible failing request in this
session. Local regressions establish the code defects; they do not establish a
deployed incident's cause. No paid provider session, provider entitlement,
browser acoustic behavior or production wallet state was verified. Local HTTP
and WebSocket provider fixtures exercise the application transport and billing.

## Concrete steps and acceptance criteria

1. **Complete — trace current contracts and write failing regression tests.** Read current
   `AGENTS.md`/`CLAUDE.md`, chat index/voice contract, operation scopes, platform
   keys/inference, billing and agent isolation docs; trace the actual model
   exception, discovery, credential resolution, and runtime revalidation.
   Acceptance: transaction-backed tests reproduce grantless platform rejection,
   granted-BYOK rejection and discovery/start disagreement on unmodified code.
   Capture commands and exact failure results below. Tests use only an explicitly
   isolated local MongoDB replica set supplied by the parent.
2. **Complete — repair voice authorization at the shared resolver.** Apply the established
   bounded platform model-transport policy only to platform audio. Check BYOK
   grants against the selected `UserService.id`. Keep operation authorization,
   input restrictions and conversation-context checks for either source.
   Acceptance: billed platform resolution works with an empty specialist grant
   set and no OpenAI `UserService`; BYOK works only for an authorized selected
   connection; no new tool or delegated authority is stored or emitted.
3. **Complete — make discovery match admission.** Check authorization separately for the
   platform option and each selected BYOK connection, for both supported
   protocols, preserving existing flag/tariff explanations.
   Acceptance: advertised options share start's authorization, origin and
   credential-eligibility checks at discovery time; denied scopes and ungranted
   BYOK connections are not advertised as available; one sibling's scope does
   not constrain another's selection. Startup remains authoritative for funds,
   concurrency, later state changes and provider entitlement. Discovery performs
   no wallet provisioning or reservation.
4. **Complete — verify billing and live fences end to end.** Exercise normal credential
   resolution, paid duration reservation and provider fixtures with a real
   isolated transaction-capable database; exercise live revalidation after
   relevant revocations and check unchanged specialist key grants/tool access.
   Acceptance: acting-person billing remains active; no-funds/unsynced-price,
   disabled flags, denied platform ACL, authored deny scope, revoked key and
   lost org/thread access fail closed before further provider effects. Existing
   voice/provider/billing/operation-scope suites and formatting checks pass.
5. **Complete — handoff for personal review.** Record changed files, red/green commands,
   results, local environment constraints, durable learnings and open findings.
   Acceptance: a concrete local diff and reproducible checks; no skipped
   integration tests represented as success and no claim of live verification.

## Execution log and handoff

- Working tree was clean at the current baseline.
- Parent confirmed Heca `gpt-6.1-sol` / `x_high` / full access. No settings changed.
- Initial plan written before implementation edits. Parent supplied a writable
  MongoDB 8.0.16 primary at `mongodb://127.0.0.1:27019/?replicaSet=voice-test`;
  all real tests explicitly use this URI. Parent owns Mongo cleanup.
- Initial cold build completed in 10m12s. Its executable listed zero
  `credential_tests`; the required recompile took 3m20s.
- **RED, unchanged production implementation:**
  `NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27019/?replicaSet=voice-test' CARGO_TARGET_DIR=target cargo test -p nyxid services::voice::credential_tests -- --nocapture`
  returned exit 101: **0 passed, 6 failed, 0 ignored, 8411 filtered**, 7.35s test
  time. Full exact output: `target/specialist-voice-red.log`. Platform, draft,
  billing and live-revalidation tests hit `Forbidden("This agent cannot use the
  selected voice service")` at their initial positive resolution. Granted BYOK
  failed with the same error. The discovery regression failed because OpenAI
  advertised an option despite its authored operation denial.
- Production changes now share exact private-thread binding checks, selected
  BYOK grant identity, and read-only credential eligibility across options and
  admission. Source/connection checks run before model expansion. The existing
  catalog-options fixture now contains a real endpoint and encrypted bearer key.
- OpenAI scope decision: §12 explicitly says its HTTP create plus WS attach
  needs the B1 WebSocket restriction. Both adapters therefore reject a scoped
  channel, including an OpenAI HTTP-only operation grant; an unrestricted,
  separately granted sibling remains eligible. Operation/input/context checks
  are retained. This follows the written voice contract, rather than inferring
  the rule only from the public WebSocket proxy.
- First expanded green run: **9 passed, 0 failed, 0 ignored, 8411 filtered**,
  10.89s test time. Exact output: `target/specialist-voice-green-initial-nine.log`.
  This run preceded the request-local authority extraction.
- Final source loads the exact bound `AuthUser` once per options request in an
  opaque `InferenceAuthority`; each service/source reuses that authority and
  checks its independent operation identity before model expansion. Each own
  connection still uses the shared read-only proxy snapshot. Resolve/live
  revalidation load fresh authority every time. A tenth query-count regression
  asserts one Agent Key read and one durable binding read across two models and
  three credential-source choices.
- **Final GREEN:** same cargo command as RED, after the complete fix and ten
  regressions: **10 passed, 0 failed, 0 ignored, 8411 filtered**, 17.88s test
  time; compile 3m48s. Exact output: `target/specialist-voice-green.log`.
- Compiled executable: `target/debug/deps/nyxid_server-4364b07c7739ce97`.
  SHA-256: `777b6fd1797591a056336c7ad2d691dbe94ec32ad9af26f6eefa1f4a09379735`.
  Baseline and hashes of all compiled changed source files are recorded in
  `target/specialist-voice-snapshot.txt`; parent independently confirmed the
  source hashes. Source has stayed unchanged during final verification.
- `cargo fmt --all -- --check` and `git diff --check` both returned exit 0;
  exact status logs are `target/specialist-voice-format.log` and
  `target/specialist-voice-diff-check.log`.
- **Compatibility infrastructure interruption (not validation):** ran
  `NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27019/?replicaSet=voice-test' target/debug/deps/nyxid_server-4364b07c7739ce97 voice --nocapture --test-threads=4`,
  selecting 76 tests, concurrently with parent's operation/org filters. MongoDB
  stopped responding; unrelated fixtures/index creation failed with
  `Io(UnexpectedEof)` and then the explicit-URI connection guard. The implementer
  interrupted its process (exit 130) to avoid overlap with restart. Exact failed
  output is preserved in `target/specialist-voice-compatibility.log` and copied to
  `target/specialist-voice-compatibility-infrastructure-failure.log`. This is not a
  passing suite and has no completed count. No assertions/timeouts were changed.
- Parent diagnosed the interrupted run: WiredTiger logged errno 24 (too many
  open files), fassert 50853 and abort. The owned server was restarted with
  per-process `ulimit -n 65536` (previous soft limit 8192), PID 14966; parent
  verified the same replica URI is writable primary. This changes only the test
  server process, not application code or assertions/timeouts.
- Verification is sequenced: parent runs operation scopes, then org agents; the
  implementer keeps the voice compatibility suite paused until parent's signal.
  Parent also owns final adversarial review.

- **Parent operation-scope GREEN:** same binary and explicit replica URI,
  `operation_scope --nocapture --test-threads=4`: **26 passed, 0 failed,
  0 ignored, 8395 filtered**, 195.94s. Exact output:
  `target/specialist-voice-operation-scopes.log`.
- **Parent organization-agent GREEN:** same binary and explicit replica URI,
  `services::org_agent_tests:: --nocapture --test-threads=4`: **26 passed,
  0 failed, 0 ignored, 8395 filtered**, 30.46s. Exact output:
  `target/specialist-voice-org-agents.log`. Parent confirmed no remaining test
  processes and signaled the implementer to rerun voice compatibility alone.
- **Final voice/provider/billing-voice compatibility GREEN:** same binary and
  explicit replica URI, `voice --nocapture --test-threads=4`, run alone after
  the parent suites: **76 passed, 0 failed, 0 ignored, 8345 filtered**, 41.01s.
  Exit 0; exact output: `target/specialist-voice-compatibility.log`. This includes
  all ten new regressions, the OpenAI/Grok providers, catalog options, and the
  three transaction-backed billing-voice scenarios. All five compiled source
  hashes match `target/specialist-voice-snapshot.txt`; source stayed unchanged.
- Parent independently checked the final log and completed personal review with
  no unresolved defects or actionable nits. Steps 4 and 5 are complete. The
  implementer's final test process has exited; no further tests are needed.
- Parent's separate personal review and verification record is
  `docs/SPECIALIST_BOT_VOICE_REVIEW.md`; its updates are parent-owned.

## Final change map and regression coverage

| Area | Implementation and evidence |
| --- | --- |
| Shared authorization | `backend/src/services/voice/credentials.rs`: `inference_authority` loads and matches the exact private thread's live chat/key/user/agent binding. `authorize_with_authority` applies selected-connection grants and the existing operation/input/context rules. `authorize_inference` loads fresh authority for start and runtime revalidation. |
| Own-key eligibility | `credentials::own_snapshot` shares the existing read-only proxy authority snapshot across discovery/start, retains owner/catalog/active/direct/bearer checks, rejects master routing and unsupported effective origins, and requires usable server credential metadata. Expected denials omit choices; infrastructure/data-integrity errors propagate. |
| Discovery | `backend/src/services/assistant_voice.rs::options` authenticates once per request, checks both providers' catalog origin, evaluates each source/connection once before model expansion, and preserves the response shape, tariff/rollout reasons and payer labels. |
| Existing fixture | `backend/src/services/voice/tests.rs::voice_options_use_catalog_models_voices_and_supported_protocols_only` now inserts a real encrypted bearer key and endpoint, and changes the own endpoint when testing xAI. Its pricing/model/voice/origin assertions are retained. |
| Normative contract | `docs/chat/11-voice.md` describes the implemented platform/BYOK/binding/discovery rules and corrects the draft credential table entry. |
| New regression module | `backend/src/services/voice/credential_tests.rs`, registered from `voice/mod.rs`: ten real-replica regressions; no optional/skip database path. |

The ten regressions cover grantless billed platform voice with zero OpenAI
connections and unchanged Agent Key authority/tool refusal; connection-ID BYOK
with ungranted sibling refusal and live grant removal; one bound-authentication
read across multiple sources/models; selected unrestricted sibling isolation for
both OpenAI and Grok despite another sibling's deny scope; catalog and effective
connection origins; fresh drafts and different-thread/ordinary/missing/guest/
rebound-agent/revoked/expired/destroyed-agent fences; rollout/ACL/tariff/empty-wallet
refusals; organization member-private threads and live membership revocation;
unusable own credentials and propagated snapshot integrity errors; OpenAI's
HTTP-only scope refusing sideband authority; and live provider closure on key
revocation with 18 confirmed seconds reserved/settled to the acting person's
wallet. Billing runs the real application service and MongoDB transactions with
FakeLago; HTTP/WebSocket providers are loopback fixtures. No paid external call
is involved.

### Reproduction commands

Run from the worktree root with the same isolated replica set:

```sh
NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27019/?replicaSet=voice-test' \
CARGO_TARGET_DIR=target cargo test -p nyxid services::voice::credential_tests -- --nocapture

NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27019/?replicaSet=voice-test' \
target/debug/deps/nyxid_server-4364b07c7739ce97 voice --nocapture --test-threads=4

cargo fmt --all -- --check
git diff --check
```

The parent separately owns `operation_scope` and `services::org_agent_tests::`
filters on this same executable. Execute suite processes sequentially to avoid
repeating the diagnosed test-server resource exhaustion. Failed infrastructure
logs must remain separate from the final passing output.

## Completed handoff and ownership

- All five plan steps are complete. Ten regressions and the voice compatibility,
  operation-scope and organization-agent filters passed with zero ignored tests.
  Counts overlap; do not sum these filters into a unique-test total.
- Exact red/green/compatibility outputs and the compiled source/executable
  snapshot are recorded above. The initial infrastructure-failure output is
  separately retained and is not counted as passing evidence.
- Parent completed personal review with no unresolved findings and owns final
  updates to `docs/SPECIALIST_BOT_VOICE_REVIEW.md`.
- MongoDB cleanup is complete: parent confirmed the task-owned mongod
  PID 14966 shut down gracefully with exitCode 0, log timestamp
  `2026-10-15T20:05:33+08`. Logs and evidence are retained. At this initial handoff,
  work was local and uncommitted; no pushes, deployments or live changes had been made. No additional
  agents/subagents were used, and no global settings were altered.
- External evidence limits remain as stated above; there are no unresolved
  local root-cause hypotheses or implementation findings.

## Durable implementation findings

- Effective service grants merge platform catalog IDs with UserService IDs.
  Authorization must use the identity of the selected execution source.
- A durable credential row alone is insufficient for a voice exception:
  authentication can return no chat, or resolve a different conversation. Match
  the exact private thread's user, conversation, agent and key before applying
  model-transport policy; an active work turn is not required for audio.
- Explicitly granted sibling connections retain separate operation identities.
  The platform catalog alias keeps conservative catalog-linked restrictions.
- Discovery may omit expected authority/credential refusals, but new filtering
  propagates infrastructure/data-integrity errors. It never reserves funds.
