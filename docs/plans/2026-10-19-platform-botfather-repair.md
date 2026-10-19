# Platform BotFather repair

## Required behavior

A shared channel bot answers public FAQ/help for an unlinked visitor. The visitor can sign in to NyxID, review consent, prove control of that private channel conversation, and then use their existing personal NyxBot through the shared bot. The personal NyxBot retains the person's authorization, memory, history, and billing. Each channel identity is verified independently, including when one person uses several platforms or several bots. Signing in alone must not silently claim an external account.

The admin can connect a bot, enable the service, understand its status, pause it, and resume it without navigating unrelated child-bot provisioning or impossible public AI prerequisites. The end user can understand the current mode, link, recover an expired link, and unlink.

## Confirmed starting defects

- The public NyxAgent producer has no production readiness path: profile discovery is empty and policy verification always fails. No admin configuration can make that path ready.
- The page's old `enabled` badge does not describe the newer runtime. Pause emergency-stops both modes, while clearing that stop leaves both modes off.
- Personal linking, proof fields, display, and documentation assume Telegram.
- Conversion, optional child-bot setup, sponsor configuration, and runtime controls obscure the basic task.

## Implementation plan and acceptance criteria

### 1. Working public FAQ and coherent lifecycle

- Provide a useful public surface from approved FAQ content. This implementation uses deterministic FAQ answers; do not label them AI-generated or enable the unverified public NyxAgent execution path.
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

## Implementation handoff

The original partial scaffolding has been replaced or completed in this shared worktree. No commit, deployment, production configuration write or real-user message was made. The primary remains the personal reviewer.

- Public work uses only the approved deterministic FAQ corpus. Metadata-only admission queues its canonical article IDs and mode guidance atomically with receipt settlement. A sweep recovers a crash before initial delivery. Delivery renders the persisted reference and refuses authority changes after admission. Personal authority and tools never enter FAQ execution.
- Telegram, Slack, Discord, Lark/Feishu, WhatsApp and X share exact bot/platform/sender/private-conversation linking. Aurinko and OpenClaw are excluded for their current proof/transport limitations. Native Telegram manager compatibility, identity locks, old public sponsor policies and channel transport billing remain in place.
- Live controls fence admission, consent, proof, execution and delivery. Emergency Pause gates legacy setup too; both control revisions change when stop state changes. Resume is one conditional update, preserves explicit remembered modes and requires a visible selection when both are off. Discord command readiness is checked transactionally, including API writes.
- Shared provider identity reservations survive deletion and lost markers. Generic proof handles never appear as fake Telegram IDs in API/UI. Exact peer records remain durable; event replay identity is deterministic. Indexed identity lookups include tombstones. Work/input/rate bounds replace permanent visitor-count exhaustion.
- Provider-authenticated human provenance is distinct from proof eligibility and supported content. Slack uses exact ordinary-DM aliases, including malformed proof attempts; file uploads receive help. Discord uses application-command identity, so `/chat` arguments cannot become controls or FAQ. In-app command setup upserts only known commands and emits metadata-only audit.
- Final send claims capture exact encrypted bot/platform/OAuth credential material. Provider effects use that snapshot. Pre-effect preparation failures settle refused or retryable; partial/uncertain effects retain actual evidence and cannot replay. Telegram sends plain protected text; Slack and Discord suppress unintended formatting, previews and mentions. Expired Discord interaction context falls back to the same bound private DM.
- Admin setup lists eligible targets without a funding-org request; existing Resume appears first, newly prepared profiles remain visible across pagination and use fresh generations. Runtime status keeps recovery available for expected OAuth/provider outages. Recovery is a separate form. Repair invalidates old link authority and inventory visibly, and bounded cleanup cannot stop a new epoch's work.
- Consent, terminal success, Linked channels and thread labels show the real provider/bot/account/private-conversation identity. A friendly name is accompanied by the real external account ID. Personal work uses the person's existing NyxBot/memory/permissions/billing in a separate saved channel thread; the old home transcript is not automatically replayed. Browser bearer/proof/secret references and mutation variables are cleared at the required lifecycle boundaries.

The operational guide is `docs/PLATFORM_BOTFATHER.md`: provider matrix, setup, linking, pause/resume, repair, delivery/storage limits and rollout precautions. Historical public/personal design documents remain background; they do not expand current deterministic FAQ into an available public AI contract.

## Verification evidence

Final frontend evidence: 28 tests in six targeted files passed, as did the app TypeScript check and ESLint over all changed/untracked frontend TypeScript files. After the final stable ConnectionControls fix, the affected component/page suites passed all 10 tests and TypeScript/changed-file ESLint passed again. The final production build, including the last Discord proof guidance, passed (tsc-b, both Vite builds, legal prerender and mock-footprint assertion). The primary independently confirmed these results; final production-build log: `/tmp/nyxid-botfather-frontend-build-reviewed.log`.

The primary's real-frontend/mock-HTTP browser suite passed Slack and Discord consent/proof/success and token cleanup, paused FAQ-only atomic Resume, connection repair/Enter isolation and conflict retries, combined paused+repair refresh, prepared-profile pagination with consecutive repair generations, and visible repair success feedback surviving both generation changes. It also passed successful repair followed by runtime GET 503 and Retry, with Resume usable afterward, and explicitly asserted Discord's `/confirm` application-command `code` option guidance. Tests ran at 320px without overflow or page errors; screenshots were visually reviewed. Scripts/screenshots are in `/Users/chronoai/Library/Caches/nyxid-botfather-review/`. These are local fixture checks, not live provider acceptance.

The selected backend suite passed **334 tests, 0 failed, 0 ignored** in **197.68 seconds** against the isolated MongoDB **8.0.34** replica set. It covers platform config/authority/runtime, FAQ admission/delivery/crash recovery, generic linking and credential races, personal execution/revocation, the changed Telegram/Slack/Discord payloads, X polling/webhooks/billing and native Telegram compatibility. The only subsequent Rust changes removed the unused `admit_in_session` helper and added `deferred_pause_cleanup_preserves_a_turn_admitted_after_resume`.

The final source test build then passed in **8 minutes 6 seconds with no warnings**. Its selected inventory contains **335 tests**, including that cleanup regression. The regression ran from the final binary against the same MongoDB 8 replica set and passed **1/1**, with **0 ignored**, in **1.08 seconds**. It uses real admission, claim, Pause and Resume operations, then proves deferred cleanup clears the old turn while preserving the current-revision turn. The already passing broad suite was not repeated for an unused-helper removal and test-only addition. Final `cargo fmt --all --check` and `git diff --check` passed.

Task-owned target: `/tmp/nyxid-botfather-target`. All workspace/path packages used by the backend were package-scoped cleaned and rebuilt before these results; registry dependencies were retained. No unrelated target or Mongo process was touched. Final test URI: `mongodb://127.0.0.1:29744/?replicaSet=botfather8`, data `/tmp/nyxid-botfather-mongo8`, log `/tmp/nyxid-botfather-mongo8.log`.

Final source build and cleanup-test commands:

```sh
NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:29744/?replicaSet=botfather8' \
  CARGO_TARGET_DIR=/tmp/nyxid-botfather-target CARGO_BUILD_JOBS=2 \
  cargo test -p nyxid --bin nyxid-server --offline --no-run \
  --config profile.test.package.nyxid.debug=0 \
  --config profile.test.package.nyxid.incremental=false

NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:29744/?replicaSet=botfather8' \
  /tmp/nyxid-botfather-target/debug/deps/nyxid_server-1a6eebbd46f0a7a8 \
  --exact services::platform_bot_runtime_control_service::tests::deferred_pause_cleanup_preserves_a_turn_admitted_after_resume \
  --nocapture
```

The selected suite ran with `python3 /tmp/nyxid-botfather-run-tests.py`, using two test threads. Persistent evidence is under `/Users/chronoai/Library/Caches/nyxid-botfather-implementation/2026-10-19-final/`: `backend-build-source-final.log`, `backend-selected-tests.log`, the selected and final test inventories, `backend-deferred-cleanup-source-final.log`, `frontend-build-reviewed.log`, the test runner and the exact changed-file inventory. The runner retains the complete filter list and the explicit MongoDB 8 URI; it can rerun the selected suite on the final binary.

The initial run used the task-owned `mongod` symlink, which resolved to MongoDB 7.0.37. That run passed 324 tests and failed 10 X billing tests because the current billing ledger uses MongoDB 8 client bulk writes. Direct ledger settlement and X allowance/duplicate-billing reproductions passed with the same binary after switching to the explicit MongoDB 8 executable. The original 10-second settlement assertions remain unchanged. `/tmp/nyxid-botfather-backend-tests-initial-mongo7.log` retains that diagnostic run; it is not final acceptance evidence.

The signed Slack journey is a service integration test: real signature validation, adapter normalization, consent/proof, personal execution/origin and the production send/claim pipeline with only a local provider-adapter injection. It is not an HTTP ingress end-to-end test. Actual provider payload construction has separate adapter tests.

## Changed-file inventory

This includes the task scaffolding that was completed or replaced and the new implementation/test files.

```text
backend/content/platform-public-faq.json
backend/src/db.rs
backend/src/handlers/assistant_nyxagent.rs
backend/src/handlers/channel_webhooks.rs
backend/src/handlers/platform_bots.rs
backend/src/handlers/platform_personal_bindings.rs
backend/src/main.rs
backend/src/models/mod.rs
backend/src/models/platform_bot_config.rs
backend/src/models/platform_bot_runtime_controls.rs
backend/src/models/platform_channel_identity.rs
backend/src/models/platform_manager_delivery.rs
backend/src/routes.rs
backend/src/services/channel_adapters/discord.rs
backend/src/services/channel_adapters/slack.rs
backend/src/services/channel_adapters/telegram.rs
backend/src/services/channel_bot_service.rs
backend/src/services/channel_credentials.rs
backend/src/services/channel_inbound_service.rs
backend/src/services/channel_platform.rs
backend/src/services/channel_poll_service.rs
backend/src/services/channel_verification_service.rs
backend/src/services/mod.rs
backend/src/services/platform_binding_service.rs
backend/src/services/platform_bot_authority.rs
backend/src/services/platform_bot_config_service.rs
backend/src/services/platform_bot_connection_service.rs
backend/src/services/platform_bot_runtime_control_service.rs
backend/src/services/platform_bot_status_service.rs
backend/src/services/platform_channel_service.rs
backend/src/services/platform_faq_service.rs
backend/src/services/platform_link_intent_service.rs
backend/src/services/platform_manager_command_service.rs
backend/src/services/platform_manager_contract.rs
backend/src/services/platform_manager_delivery.rs
backend/src/services/platform_manager_dispatch_service.rs
backend/src/services/platform_manager_journey_tests.rs
backend/src/services/platform_manager_transport.rs
backend/src/services/platform_personal_binding_service.rs
backend/src/services/platform_personal_turn_service.rs
backend/src/services/platform_public_budget_service.rs
backend/src/services/telegram_new_tests.rs
docs/PLATFORM_BOTFATHER.md
docs/plans/2026-10-19-platform-botfather-repair.md
frontend/src/components/assistant/assistant-sidebar.tsx
frontend/src/components/assistant/nyxbot-settings-content.tsx
frontend/src/components/assistant/platform-personal-bindings.tsx
frontend/src/components/platform-manager-runtime.test.tsx
frontend/src/components/platform-manager-runtime.tsx
frontend/src/hooks/use-platform-onboarding.test.tsx
frontend/src/hooks/use-platform-onboarding.ts
frontend/src/hooks/use-platform-personal.ts
frontend/src/lib/assistant/nyxagent-transport.ts
frontend/src/lib/platform-personal.ts
frontend/src/pages/admin-platform-bots.test.tsx
frontend/src/pages/admin-platform-bots.tsx
frontend/src/pages/platform-personal-link.tsx
frontend/src/schemas/platform-onboarding.ts
frontend/src/schemas/platform-personal.ts
frontend/src/schemas/platform-runtime.test.ts
frontend/src/schemas/platform-runtime.ts
```
