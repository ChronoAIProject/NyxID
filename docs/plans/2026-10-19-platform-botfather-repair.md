# Platform BotFather repair

## Main integration — current architecture

The historical repair below was reviewed at `0d9b3171e`. Integration now merges
`origin/main` at `3fc6386a2`, including the working bounded public runtime (#1914)
and NativeV2 entry (#1925). Historical claims that the public AI contract is
unavailable describe the old baseline only.

NativeV2 is the sole non-Telegram shared-entry path. It retains opaque provider
identity, authenticated direct/gateway ingress, `/nyx` Discord commands, encrypted
receipt-bound transport and main's personal account parity. Telegram retains its
TelegramV1 manager path. The earlier parallel generic mapper and `/chat` setup
were discarded during integration. Deterministic approved FAQ is the no-policy
default; saved public AI policies remain explicit and functional with their real
sponsor, limits, readiness and deployment contract.

Current acceptance and final local evidence are recorded in the integration
handoff below. Evidence from the original repair is historical and does not
verify the merged executable.


## Required behavior

A shared channel bot answers public FAQ/help for an unlinked visitor. The visitor can sign in to NyxID, review consent, prove control of that private channel conversation, and then use their existing personal NyxBot through the shared bot. The personal NyxBot retains the person's authorization, memory, history, and billing. Each channel identity is verified independently, including when one person uses several platforms or several bots. Signing in alone must not silently claim an external account.

The admin can connect a bot, enable the service, understand its status, pause it, and resume it without navigating unrelated child-bot provisioning or impossible public AI prerequisites. The end user can understand the current mode, link, recover an expired link, and unlink.

## Historical diagnosis before main integration

- At the original repair baseline, the public NyxAgent producer had no production readiness path: profile discovery is empty and policy verification always fails. Main #1914/#1925 subsequently supplied the working public runtime; its explicit AI path is preserved.
- The page's old `enabled` badge does not describe the newer runtime. Pause emergency-stops both modes, while clearing that stop leaves both modes off.
- Personal linking, proof fields, display, and documentation assume Telegram.
- Conversion, optional child-bot setup, sponsor configuration, and runtime controls obscure the basic task.

## Implementation plan and acceptance criteria

### 1. Working public FAQ and coherent lifecycle

- Provide a useful public surface from approved FAQ content. This implementation uses deterministic FAQ answers; do not label them AI-generated. Preserve the now-working main public AI runtime only as an explicitly selected, validated policy.
- FAQ activation does not require a model profile, inference sponsor, or public AI budget. Preserve any real channel transport billing requirements and explain them where relevant.
- Define one authoritative runtime state used by backend responses and UI. Distinguish paused, active, setup incomplete, and connection repair required with concrete recovery actions.
- Resume atomically clears an emergency stop and restores explicitly selected modes. Normal setup enables FAQ plus personal linking in one understandable operation.
- Preserve pause, revocation, replay protection, fail-closed authority checks, durable delivery, and compatibility with existing profiles. Existing rows must have a recovery path.
- Public FAQ cannot read personal data or execute private tools. Linked-user failures must not silently switch into another user's authority.

### 2. Cross-channel personal linking

- Reuse the authenticated channel adapters and channel bot ownership checks. Inventory each currently available adapter; implement the same flow wherever a verified private sender and reply transport exist. Document concrete provider limitations rather than claiming unsupported behavior works.
- Bind proof to the exact platform, bot authority, sender, private conversation, and current generation. Reject groups, bot/self events, edits, replayed or forwarded proof, mismatched sender/conversation, expired intent, and revoked authority.
- Sign-in, web consent, and a fresh confirmation in the originating private conversation are separate necessary steps. Do not infer account ownership from matching usernames, email text, phone display, or another platform's login.
- Resolve one personal NyxBot for the signed-in person through the existing personal turn path. Other identities remain unlinked until independently verified.
- Unlink, ownership change, credential change, removal, and pause revoke admission and pending delivery as appropriate. Personal permissions, spend, and history remain isolated.
- Preserve existing Telegram profiles and links. Generic identity mapping, if retained, must be server-owned, unambiguous, bounded, durable, and invisible as fake Telegram IDs in the UI/API.

### 3. Admin and end-user UI

- Admin flow: select/connect a supported bot → review public FAQ and personal linking behavior → enable → see status and channel-specific test instructions.
- Existing profile: show current state and the next useful action, with Resume prominent when paused. Show actionable backend failures without discarding their meaning.
- Keep optional child-bot provisioning and advanced AI controls away from the required basic flow; hide unavailable paths or explain their actual availability.
- End-user flow: ask FAQ or request link → sign in → review the exact bot/platform/account and disclosures → confirm in the same private conversation → personal mode. Show progress, expiry/retry, success, and unlink clearly.
- Linked channels settings show real platform and conversation identity, plus an understandable disconnect action. No raw internal generation handles or implementation diagnostics in the main flow.
- Preserve accessible labels, keyboard operation, responsive layout, and existing project UI conventions (`useAppForm`, TanStack Query, Zod DTOs).

### 4. Verification and personal review

- Meaningful Rust tests for runtime resume, FAQ dispatch/delivery/recovery, linking proof, isolation/revocation, and real adapter ingress normalization. Use an isolated MongoDB replica set for transactional tests.
- Frontend tests for setup, existing paused-profile recovery, runtime status, failure recovery, and cross-channel link/consent/unlink. Build and lint changed surfaces.
- Exercise the UI in a browser against local fixtures/server as available, including mobile width and failure states. Record exactly what was verified.
- Personally review the complete diff for correctness, security, compatibility, lifecycle cleanup, duplicate effects, billing, accessibility, dead code, copy, and maintainability. Return all findings to the same implementation worker until resolved, then rerun affected checks.
- Update operational documentation with the actual setup/unblock procedure and provider support matrix. Do not reuse historical test evidence as new verification or claim live provider/production acceptance without running it.

## Work arrangement

The user requested one Heca implementation worker with `gpt-6.1-sol`, `xhigh`, and `full-access`. The primary agent performs the review personally. Do not create Codex subagents or additional Heca workers/reviewers. No deployment, production configuration changes, or outbound messages to real users are part of local implementation verification.

## Integrated implementation

The integration retains main's authoritative NativeV2 entry for Slack, Discord,
WhatsApp, Lark/Feishu and X, and TelegramV1 for the native Telegram manager. It
removes the original repair's competing generic identity mapper and Discord
`/chat` setup. Native direct/gateway selection, opaque identities, exact
receipt-bound encrypted transport, provider deadlines, tombstones and ordinary
owner-route separation remain in force.

No public policy selects the approved deterministic FAQ. A saved public AI
policy retains the working main runtime, its real contract, deployment, limits,
readiness and authorized organization sponsor. Basic FAQ conversion and Resume
have no inference or sponsor prerequisite. Existing channel ownership and
transport billing do not change.

Public FAQ admission commits canonical article IDs, live mode guidance and the
settled ingress receipt together. Mongo server-clock windows admit at most 20
requests per sender and 200 per profile per minute before recording a receipt.
Recovery renders only persisted approved references through the production
public-send pipeline; no raw FAQ question is retained. Live authority changes
refuse old sends, and duplicate or ambiguous provider effects are not replayed.
Provider-specific navigation is composed separately from the verbatim articles.

Pause remembers explicit mode choices, advances authority even if both lanes
were already off, and disables public/personal admission. Resume clears the stop
and restores selected modes in one conditional update. Deferred cleanup matches
old revisions and leaves newly admitted turns alone. Unsupported authenticated
human media and control-shaped Discord question arguments receive fixed help
before any FAQ, personal or public AI work is admitted.

Recovery uses the authoritative profile rather than the ordinary owner's target
picker. A successor platform admin can repair basic setup without a sponsor org.
Expected credential outages produce actionable connection status while database
failures remain errors. Repair pauses and invalidates old proofs and personal
links; inventory shows them disconnected. A fresh proof establishes independent
current authority. Success guidance survives generation remounts and failed
status refreshes, while secret/proof state is erased.

See `docs/PLATFORM_BOTFATHER.md` for the actual provider/setup/recovery matrix,
Discord named application-command options, personal thread and billing behavior,
optional public AI and child workflows, X signed-webhook requirement, delivery
limits and rollout cautions.

## Integration acceptance matrix

| Required behavior | Concrete local evidence |
| --- | --- |
| FAQ without inference/sponsor, public authority even for linked people | FAQ admission/delivery journeys, renderer corpus/provider invariant, runtime FAQ activation |
| Atomic metadata-only queue/receipt, crash and duplicate recovery, bounded rates | FAQ crash/canonical-mode and window/abort/live-change regressions |
| Ordinary signup/login, email verification and MFA, explicit consent and same-chat proof | Main auth/MFA journey; NativeV2 Discord journey; existing personal Playwright suite |
| Same owner's bots/platforms independently verified | New native+Telegram same-owner journey; wrong-platform proof refusal; separate saved threads and local logout |
| Existing NyxBot permissions, memory, credentials, payer and web continuation | Main two-account account-parity journey and actual NativeV2 personal producers |
| Pause/Resume/repair/disconnect without authority revival | Runtime CAS/both-off/deferred cleanup; confirmed native replay after disable/logout/repair; fresh proof and inventory |
| Provider provenance, direct/gateway choice and exact receipt-bound replies | Main entry/native direct, gateway, credential, receipt and delivery suites |
| Human upload and Discord question arguments cannot execute controls/models | Actual signed Slack adapter file-share normalization and canonical unsupported admission regression |
| Eligible admin targets and successor recovery | Live-admin/pagination/tombstone target regression; no-sponsor successor repair; unavailable OAuth status/inventory |
| Clear simple UI and safe browser handoff | Six targeted frontend suites; personal Playwright; native admin and Slack/Discord browser harnesses |

## Fresh integration verification

The final integrated backend test target at
`bce616754cd1190fc93364a8acab915e55cb0f6f` compiled without warnings in
**7m 48s**. Formatting passed. The **13 priority tests passed**, with 0 failures
and 0 ignores, in **33.94s** against the isolated MongoDB 8 replica set. The
complete **554-test** selected suite passed on that same binary: **552 passed,
0 failed, 2 ignored**, with 8,892 unrelated tests filtered out, in **845.71s**.
The runner exited 0. Earlier failed fixture/readiness runs remain diagnostics
in the evidence cache. The original repair's 335-test result at `0d9b3171e` is
historical and does not verify this merged executable.

- Six targeted Vitest files: **44 passed**, 0 failed.
- App TypeScript and ESLint over all 14 frontend TypeScript files changed from
  main baseline `3fc6386a2`: passed.
- Production frontend build: passed, including TypeScript, both Vite builds,
  legal prerender and mock-footprint assertion.
- Existing main `platform-personal` Playwright suite: **7 passed**; the primary
  repeated it after final expiry cleanup, **7/7 in 19.6s**.
- Native admin and Slack/Discord link browser harnesses: passed on the real UI
  with local HTTP fixtures, at 320px with no overflow or page errors. The primary
  also reran these and visually reviewed screenshots. They cover atomic
  remembered FAQ-only Resume, empty targets/successor repair, generations 3→4,
  persistent repair notice, status 503 Retry, optional AI without obsolete mode
  controls, ordinary login gate, exact account ID, consent, one-time proof,
  reload behavior, terminal cleanup and Discord's named `code` option.

Frontend commands, run from `frontend`:

```sh
npx vitest run src/components/platform-manager-runtime.test.tsx \
  src/components/platform-manager-runtime.native.test.tsx \
  src/components/native-platform-entry-setup.test.tsx \
  src/pages/admin-platform-bots.test.tsx \
  src/schemas/platform-runtime.test.ts src/schemas/platform-personal.test.ts
npx tsc --noEmit -p tsconfig.app.json
# npx eslint over the exact changed .ts/.tsx inventory
npm run build
npx playwright test --config playwright.platform-personal.config.ts
```

Logs use `/tmp/nyxid-botfather-integration-*`. Browser scripts and reviewed
screenshots are in `/Users/chronoai/Library/Caches/nyxid-botfather-review/`.
Local test providers are loopback/fixtures, not live provider or production
activation. No deployment, production write or real-user message was made.

Backend verification uses the task-owned target
`/tmp/nyxid-botfather-target` and isolated MongoDB **8.0.34** replica set:
`mongodb://127.0.0.1:29744/?replicaSet=botfather8`. No unrelated target, Mongo
process or worktree is modified. The final source build, selected inventory,
priority run and complete transactional results are recorded in the durable
evidence cache.

The final build source is `bce616754cd1190fc93364a8acab915e55cb0f6f`.
The runtime regression includes disabled credential-owner status, expired
unrefreshable status/inventory, actual Pause and atomic Resume with an expired
refreshable connection, removal of refresh access after snapshot capture, and
continued refusal of expired credentials for message admission and delivery.
Status, inventory and mode changes make no provider request. The existing
authenticated same-epoch refresh and repair journeys passed in the complete run.

Backend commands, from the repository root:

```sh
cargo fmt --all --check
CARGO_TARGET_DIR=/tmp/nyxid-botfather-target CARGO_BUILD_JOBS=2 \
  cargo test -p nyxid --bin nyxid-server --offline --no-run \
  --config profile.test.package.nyxid.debug=0 \
  --config profile.test.package.nyxid.incremental=false
# Both runners set NYXID_TEST_DATABASE_URL to the isolated MongoDB URI above.
python3 /tmp/nyxid-botfather-integration-run-priority.py
BOTFATHER_REQUIRE_FINAL=1 \
  python3 /tmp/nyxid-botfather-integration-run-tests.py \
  /tmp/nyxid-botfather-integration-test-build-final.log
```

The two upstream paired optional public-AI tests were the only ignores:

- `services::platform_public_contract_service::tests::handshake::actual_upstream_discovery_readiness_and_four_call_default_budget_settlement`
- `services::platform_public_contract_service::tests::native_paired::real_codex_native_v2_discovery_readiness_faq_search_read_signup_answer_use_actual_mcp_funding_and_durable_native_delivery`

They require `NYXAGENT_PUBLIC_TEST_BINARY`, `NYXAGENT_PUBLIC_TEST_POLICY` and
`CODEX_APP_SERVER_BINARY`, including a verified Codex 0.160.1 wrapper. All three
variables are unset locally. The upstream clone's `config/policy.public.json`
exists at `/Users/chronoai/Desktop/aelf-frontend-work/NxyAgent-botfather/`, but
no `nyxagent` executable is on PATH or in that clone's debug/release target.
The former `/tmp/nyxagent-botfather-target` and pinned
`/tmp/nyxagent-botfather-codex-0.160.1` wrapper are absent. The ordinary installed
Codex executable does not establish the required pinned fixture. These were
read-only availability checks; no upstream repository work was started and no
paired-runtime pass is claimed. The selected local public-runtime contract,
funding, readiness, deadline and settlement tests use loopback providers.

Durable local evidence is under
`/Users/chronoai/Library/Caches/nyxid-botfather-implementation/2026-10-19-main-integration/`,
including exact runner scripts, selected test inventory, command logs, fixture
availability manifest and all 47 changed paths relative to main baseline
`3fc6386a271f77b53323a4cf3a37beae262d7cae` (31 backend, 14 frontend, 2 docs).
The tested binary's SHA-256 is
`c2bfb70c18f61508b7f653fd282c95b2618da51eb2050b6e00474b7ac758d59f`.
Only documentation changed after this source build. Primary owns the remaining
PR #1945 CI, final review, push and authorized squash merge; no remote operation
or deployment is implied by these local results.

## 2026-10-20 outbound delivery revision

The BotFather UX work replaces the former single-message/link fallback for saved
replies and proactive posts with complete ordered text, a shared 64-part planner,
per-part durable receipts, native manager photo/document delivery and conditional
web links. The current contract, recovery and rollout rules are in
`docs/TELEGRAM_NEW.md#full-saved-reply-delivery`. Earlier verification records
above remain historical; the new phase records its own commands and results.
