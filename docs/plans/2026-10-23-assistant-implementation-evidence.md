# Assistant implementation evidence — 23 October 2026

Implementer: Sol. Same worktree/branch `feat/assistant-launch-and-legal`.
PM owns release, production settings, merge and activation. No staging, commits,
pushes or production mutations by this implementer. Backend builds require PM's
slot; none started.

## Final implementation status

Application implementation is frozen after the explicit Studio login-return fix
and PM-authorized MFA browser-fixture locator correction.
All application frontend changes have targeted passing checks: **240 distinct
tests across 15 files**, verified through the incremental runs recorded below.
The full personal-link browser suite also passed **7/7** with the real frontend
HTTP client and synthetic API responses; these are separate browser cases.
The backend registry and audience test are authored and formatted but **not
compiled or executed**; PM owns that build slot. The independent legal
cross-review below found no substantive source mismatch in the hash-pinned
snapshot and its **4 rendering tests passed**. No live acceptance is claimed.

- Assistant is the default destination after password/MFA/app login, authenticated
  auth-page visits and the authenticated root/boot redirect. Trusted explicit
  `return_to` destinations remain authoritative, including Studio, OAuth/consent,
  invitations and device/Agent Key approval links. `/dashboard` remains Studio.
- Activity and Artifacts use the standard default-off
  `assistant:workspace-preview` flag and authenticated server capabilities.
  They retain their existing disabled previews; unknown direct URLs stay 404.
- No BotFather change is required for deterministic FAQ without a model/sponsor.
  Saved optional public AI and A15–A17 protections retain their existing gates.
- The billing source audit found no necessary implementation gap. Existing
  effective-pricing, resync, readiness and settlement paths support PM's audit.
  Actual production pricing and a real billed call remain PM acceptance work.

Final owned source/test map: `backend/src/services/feature_flag_service.rs`;
`frontend/src/{main.tsx,router.tsx,lib/feature-flags.ts}`;
`frontend/src/components/assistant/assistant-sidebar{,.test}.tsx`;
`frontend/src/components/auth/{auth-flow,mfa-verify-form,web-device-login}{,.test}.tsx`;
`frontend/src/hooks/use-feature-flag.test.tsx`;
`frontend/src/pages/assistant-workspace.router.test.tsx`;
`frontend/e2e/platform-personal.spec.ts` (subsequently assigned by PM for the two
stale MFA labels). This evidence document
is the only owned documentation change. Legal files/tests and the PM plan are
owned by the other staff.

## Initial file map and findings

- Login/default route audit: `frontend/src/router.tsx`, `frontend/src/lib/return-url.ts`,
  auth forms, login/MFA/device/social callbacks and their tests. Preserve explicit
  return destinations, consent and deep links; `/dashboard` remains Studio.
- Assistant navigation: `frontend/src/components/assistant/assistant-sidebar.tsx`,
  `frontend/src/pages/assistant.tsx`, workspace routing helpers/tests. Activity and
  Artifacts currently lack their own feature gate. Add a backend-resolved flag
  using the existing feature flag registry and enforce navigation/direct URLs.
  No browser email/local-storage hint will confer authority.
- Feature authority: `backend/src/services/feature_flag_service.rs`, user
  capabilities projection and `frontend/src/lib/feature-flags.ts`.
- BotFather: `docs/PLATFORM_BOTFATHER.md`, `docs/NYXBOT_ONBOARDING.md`, canonical
  acceptance and 19 October repair including 20 October delivery revision.
  Deterministic FAQ without model/sponsor is already integrated; saved optional
  public AI keeps real sponsor, contract, budget and readiness requirements.
  Canonical A1–A17 are open; historical local tests do not constitute live proof.
- Billing: inspect `assistant_settings_service`, `assistant_readiness_service`,
  `admin_assistant`, billing config, Lago setup and runtime rate/settlement paths.
  User requests ordinary NyxBot flags enabled and real LLM charging; PM will
  audit/activate settings. Keep rollout/readiness guards and existing pricing.

## Coordination

`heca_agent_list` identified MAIN PM `01a1654c-4cd4-7063-9063-c437983393b0`.
`heca_agent_send_prompt` to PM returned `not authorized ... a caller may act only
on itself, agents it spawned, or ... its team`. This file is the authorized
durable handoff fallback. No additional agents were started.

## Commands and evidence

- `pwd`: correct assigned worktree.
- `git status --short --branch`: clean `feat/assistant-launch-and-legal` at start.
- Read-only file/redirect/feature searches performed with `rg` and `sed`.
- Final exact test commands and outcomes are below. No live provider/production
  requests or production mutations were performed by this implementer.


## PM review incorporated

PM requested ordinary rollout targeting rather than a hardcoded email cap. The
new `assistant:workspace-preview` flag defaults **off** and uses the existing
resolver without extra reads. PM owns the sole production per-user enable for
Calvin UUID `add69059-bece-4f0e-9559-99cfd10b47eb`, and auditing that no global,
org or other-user grant broadens the audience. Client navigation consumes only
server-resolved capabilities. Email/local-storage hints do not enable it.
Activity and Artifacts remain their existing disabled placeholders. Neither has
an implemented route or data endpoint; direct URLs remain 404 with router tests.
No data is removed or migrated.

PM live findings (reported by PM, not independently observed here): production
0.73.1 / `9cc4a7ec722d`; BotFather profile
`0b4949e8-be13-4abc-8b65-7b1d33d4aa11` has authoritative runtime public/personal
active, emergency false, `public_policy=null`, FAQ ready. Legacy `enabled=false`
is not the runtime authority; no activation write is needed for that profile.
Models chat/channel/research are available. Production billing global on,
BILLING_ENABLED on, Lago configured, FAIL_CLOSED false, reconcile 300, exact
cutover acknowledged. PM reports `chrono-llm-public` input/output/cache/image
prices synced and fresh. No environment change needed; no implementer write.

## Normal-use feature activation map for PM

- `experimental:ai-assistant`: navigation gate (`components/dashboard/sidebar`);
  turn on globally only after the hiding changes deploy, as PM intends.
- `assistant:nyxagent-engine`: canonical browser/personal engine, default on.
- `experimental:billing`: actual charging and billing UI rollout; production
  already globally on according to PM. Keep per-user/org disabling overrides
  in the audit because they affect personal billing resolution.
- `assistant:live-turn-gate`: enforcement rollout, not a new chat engine;
  recommended only after every auth/MCP/proxy/LLM/worker replica supports it, as
  documented in `docs/chat/08-nyxagent-engine.md`. PM should inspect before enable.
- Ordinary extended functions are separate opt-ins with prerequisites:
  `assistant:channel-media`, `assistant:attachment-writes`,
  `assistant:operation-scopes`, `assistant:skill-pin-disable`,
  `assistant:owner-invocations`, `assistant:org-agents`,
  `assistant:machine-capabilities`, `assistant:machine-contexts`,
  `assistant:agent-learning`, `nyxbot:thread-follow`, `nyxbot:service-adapters`.
  Preserve their registry/docs upgrade and provider prerequisites; none is
  required for ordinary personal text chat. PM decides which normal-use
  functions to activate after readiness checks.
- Direct/Aevatar wire log, advanced model controls, voice/provider invoice gates,
  hosted remote-machine admission and `nyxbot:gateway-*` are distinct rollout
  or routing choices, not prerequisites for browser NyxBot. Do not bulk-enable.
- `assistant:workspace-preview` remains default off and Calvin-only by the
  production per-user override above.

## LLM billing source audit and supported activation path

- The initial `llm-nyx` hop deliberately uses `CredentialClass::NoAuth` and marks
  itself as the assistant parent (`assistant_nyxagent_egress.rs`, context near
  line 323). It opens/settles the ordinary meter but is not the child model
  charge. `llm-nyx` must retain `auth_method=none`, no user credentials,
  `forward_access_token=true`, no delegation-token injection, server transport.
- Actual provider calls use `handlers/llm_gateway.rs`: resolve final credential
  class and resource owner, `resolve_for_execution`, create `BillingRouteContext`
  with service billing, record trusted turn/model provenance, open before
  forwarding, and settle token usage/stream results through durable settlement.
  MCP/proxy child inference also records trusted assistant provenance.
- `BillingOwnerResolver::resolve_for_execution` charges platform master-key
  traffic to the acting person. BYOK preserves the resolved personal/org
  resource owner and ACL. There is no new sponsor or alternate payer fallback.
- Title/utility inference (`assistant_oneshot_inference.rs`) also opens the
  LlmProvider meter with the actual service/credential lane, marks forwarding,
  and settles measured usage; it excludes `llm-nyx` as an inference candidate.
- Inspect every inference service actually selected by the deployed NyxAgent,
  including `chrono-llm-public`, and the utility-inference service. Use
  `GET /api/v1/admin/billing/services/{service_id}`, `/admin/billing/rates`,
  `/admin/billing/overview` for effective lanes, sync status and fresh rates.
  Platform credentials need the platform-key lane; BYOK uses its own lane.
  A missing lane is free when either lane is present; an unsynced primary
  uses the legacy fallback, and unpriced/unsynced extra components may be free.
- Existing supported price write is the service Billing editor / existing
  `PUT /api/v1/admin/billing/services/{service_id}` or `PUT /services/{id}`;
  preserve existing prices, both lanes and components. After authorized changes,
  `POST /admin/billing/services/{id}/resync`, read back synced state and fresh
  cache, and verify actual usage/ledger funding. No code change to billing is
  necessary from the source audit so far. No unrelated price decision made.
- A positive-priced call additionally requires the acting person's provisioned
  wallet/subscription, plan entitlement, fresh tax/rate state, and credit/benefit
  funding. `GET /users/me.capabilities.billing_available` describes environment
  + rollout, not successful wallet provisioning or a settled charge.
- Acceptance: use a real approved caller/turn and compare its provider child
  usage meter, billed owner, actual input/output/cache/image quantities, exact
  funding split, wallet debit and balanced ledger entry. Parent aggregate tokens
  are separate in admin NyxBot model totals. Verify Stop/failure/duplicate paths
  through existing durable settlement, rather than interpreting a synced price
  alone as real billed evidence.

## Final targeted verification

- `npm exec -- vitest ...` initially had no worktree dependencies and fetched
  an unintended Vitest 5 transient runner; startup failed on missing local Vite.
  No test ran; not counted as evidence.
- `npm ci --no-audit --no-fund` in frontend: exit 0, 641 locked packages installed.
- `rustfmt --edition 2024 backend/src/services/feature_flag_service.rs`: exit 0.

All Vitest/ESLint commands below ran from `frontend/` with locked local tools.

```sh
./node_modules/.bin/vitest run src/components/auth/auth-flow.test.tsx src/components/auth/mfa-verify-form.test.tsx src/components/auth/web-device-login.test.tsx src/components/assistant/assistant-sidebar.test.tsx src/pages/assistant-workspace.router.test.tsx src/lib/feature-flags.test.ts src/lib/return-url.test.ts
```

Exit 0, **7 files / 161 tests passed**, 8.03s. Additional cases were added and
verified in the later runs below.

```sh
./node_modules/.bin/vitest run src/pages/assistant-workspace.router.test.tsx src/components/platform-manager-runtime.test.tsx src/components/platform-manager-runtime.native.test.tsx src/components/native-platform-entry-setup.test.tsx src/pages/admin-platform-bots.test.tsx src/schemas/platform-runtime.test.ts src/schemas/platform-personal.test.ts
```

The **6 BotFather files / 51 tests passed**. The workspace suite had two harness
failures: a reconstructed test router omitted the production router's
`defaultNotFoundComponent`, so it rendered generic Not Found instead of the
expected custom 404. Corrected only the test harness to reuse the production
option; all workspace cases passed in the following runs.

```sh
./node_modules/.bin/vitest run src/pages/assistant-workspace.router.test.tsx src/hooks/use-feature-flag.test.tsx src/pages/login.test.tsx src/lib/return-url.test.ts
```

Exit 0, **4 files / 94 tests passed**, 7.49s.

```sh
./node_modules/.bin/vitest run src/components/auth/auth-flow.test.tsx src/components/auth/mfa-verify-form.test.tsx src/components/auth/web-device-login.test.tsx src/components/assistant/assistant-sidebar.test.tsx src/pages/assistant-workspace.router.test.tsx src/hooks/use-feature-flag.test.tsx src/lib/feature-flags.test.ts src/lib/return-url.test.ts src/pages/login.test.tsx
```

Exit 0, **9 files / 184 tests passed**, 8.34s. Includes 73 workspace-router
cases, authenticated/default-off navigation, untrusted local hints, protected
direct URLs, default Assistant entry and trusted explicit returns.

The subsequent frontend edit added three web-device return cases (explicit Studio,
OAuth and invitation). Its affected suite was rerun:

```sh
./node_modules/.bin/vitest run src/components/auth/web-device-login.test.tsx
./node_modules/.bin/eslint src/components/auth/web-device-login.test.tsx
```

Both exit 0; **9/9 tests passed**, 565ms. Before the Studio follow-up below,
distinct application cases were **187 across 9 files** (184 plus 3 newly verified
cases); with the 51 BotFather cases, **238 distinct passing targeted tests across
15 files**. Two further router regressions bring the final count to **240**.
These are incremental results, not a claim of one uninterrupted 240-test run.

```sh
./node_modules/.bin/eslint src/components/assistant/assistant-sidebar.tsx src/components/assistant/assistant-sidebar.test.tsx src/components/auth/auth-flow.tsx src/components/auth/auth-flow.test.tsx src/components/auth/mfa-verify-form.tsx src/components/auth/mfa-verify-form.test.tsx src/components/auth/web-device-login.tsx src/components/auth/web-device-login.test.tsx src/hooks/use-feature-flag.test.tsx src/lib/feature-flags.ts src/main.tsx src/pages/assistant-workspace.router.test.tsx src/router.tsx
```

Exit 0 over all **13 changed TypeScript files**, after correcting a conditional
hook in a test mock. The final web-device edit also passed the separate lint
command above.

PM's ship pregate classified the newly added Rust `key: CONSTANT` reference as
an env.kv secret. An initial rename requested by PM did not solve the scanner's
calibration issue. PM restored the descriptive
`ASSISTANT_WORKSPACE_PREVIEW_FLAG_KEY` name; Sol leaves it unchanged. The
persisted flag key remains `assistant:workspace-preview`; no runtime change or
scanner suppression. Rust formatting and `git diff --check` passed. Scanner
calibration and CI lane discovery remain PM-owned without a bypass.

## Studio login-return follow-up — corrected and frozen

Astra independently identified a valid P2: the existing
`dashboardLayout.beforeLoad` condition excluded bare `/dashboard` from
`return_to`. With Assistant now the default, a signed-out explicit Studio visit
could resume at Assistant. Sol missed this exception in the initial redirect
audit; PM confirmed and authorized its correction.

Removed only the `/dashboard` exception in `frontend/src/router.tsx`. Explicit
Studio requests now use the same trusted-origin return preservation as other
protected routes. Bare login still defaults to Assistant.

Added two integration cases to `assistant-workspace.router.test.tsx` for bare
`/dashboard` and `/dashboard?explicit=1`. Each opens the route signed out,
asserts its generated login return URL, simulates the authenticated login
handoff, and checks that the router sends the browser to the exact Studio
destination rather than Assistant. A minimal LoginPage mock isolates the route
guards; actual login form behavior is covered by the existing auth tests.

Exact affected-only verification from `frontend/`:

```sh
./node_modules/.bin/vitest run src/pages/assistant-workspace.router.test.tsx -t 'signed-out explicit Studio'
```

Before the fix: exit 1, **1 failed / 1 passed / 73 skipped**, 1.50s. Bare Studio
failed because no preserved-login navigation occurred; the query-bearing Studio
case passed. This establishes that the new regression catches the omission.

```sh
./node_modules/.bin/vitest run src/pages/assistant-workspace.router.test.tsx
./node_modules/.bin/eslint src/router.tsx src/pages/assistant-workspace.router.test.tsx
```

After the fix: Vitest exit 0, **1 file / 75 tests passed**, 7.16s (5.86s test
execution); ESLint exit 0. Final `git diff --check` exit 0. Final distinct local
application coverage is **189 tests across 9 files**, plus **51 BotFather tests
across 6 files**: **240 across 15 files**, and the separate four legal tests.
No unrelated suites or backend build were rerun for this fix. App files are
frozen again; no staging, commits, pushes or production writes.

PM independently reports the prior full frontend run passed **6,567 tests in
606 files**, the production build passed, and lint had **0 errors / 33 existing
warnings**. Those are PM-provided results before this two-case follow-up, not
new full-suite/build runs by Sol.

## Personal-link MFA fixture correction — verified and frozen

PM reported that `frontend/e2e/platform-personal.spec.ts:363` and `:368` used the
stale accessible label `Enter 6-digit verification code`. The actual
`MfaVerifyForm` field has label and aria-label `Authenticator code`
(`mfa-verify-form.tsx:181`, `:191`). Changed only the two locators to
`getByLabel("Authenticator code", { exact: true })`.

Retained the invalid-code submission, visible rejection, zero personal reviews,
signed-out assertion, valid-code verification, intent return and separate
same-chat confirmation assertions. No application behavior, security gate,
fixture response, timeout or retry setting changed for this correction.

Exact commands from `frontend/` and actual results:

```sh
./node_modules/.bin/vitest run src/components/auth/auth-flow.test.tsx src/components/auth/mfa-verify-form.test.tsx src/components/auth/web-device-login.test.tsx src/pages/login.test.tsx
```

Exit 0, **4 files / 52 tests passed**, 3.16s. These are affected login suites
already included in the 240 distinct local application/BotFather test count.

```sh
./node_modules/.bin/eslint src/router.tsx src/pages/assistant-workspace.router.test.tsx e2e/platform-personal.spec.ts
```

Exit 0, no output/errors/warnings.

```sh
./node_modules/.bin/playwright test --config=playwright.platform-personal.config.ts
```

Exit 0, **7 passed**, 20.8s, Chromium, 1 worker, configured retries **0**.
The full suite covers fragment-token hygiene and invitation separation; signup
and login for both email-verification states; MFA rejection then successful
verification before review; one-time approval and same-chat confirmation;
retained personal history after disconnect; and review expiry/refresh. The
dedicated config started Vite development mode on strict port 4611 and used
the existing synthetic API route fixture with the actual frontend HTTP client.
No backend build or live provider/Telegram/production request was required.

Final `git diff --check` exit 0. Implementation and fixture files are frozen
again for Astra's PM-requested delta review. No staging, commits, pushes,
production writes or additional agents. Backend build remains PM's serial lane.

## Independent policy cross-review for PM

Read-only review of Astra's complete working-tree diff for
`frontend/public/legal/{terms,privacy}.md`, including surrounding clauses and
the separate public-page test. **No substantive source-backed problem found
in the reviewed snapshot.** Sol made no policy/page/legal-test edits.

Reviewed SHA-256 values (pin this verdict to the exact contents):

| File | SHA-256 |
| --- | --- |
| `frontend/public/legal/terms.md` | `906c5a071a7d7695b28b16e29ca17eda10a903b8739f2e4d4b8fffabe443b6ea` |
| `frontend/public/legal/privacy.md` | `30eb46cb452e86af5e646c3cef758452a1f327a66c492c616fad2555b806f4f1` |
| `frontend/src/pages/legal-pages.test.tsx` | `0fea0d7b56c1cb28d9b23308a201622b9d6bd9b58e23bc0c5de983f9052a6f08` |

Substantive comparisons:

| Policy claim | Source checked and assessment |
| --- | --- |
| Hosted AI, platform/BYOK connections, saved transcripts/memory/context and feedback | `assistant_nyxagent.rs`, `models/assistant_agent.rs`, `models/assistant_checkpoint.rs`, `models/assistant_message_feedback.rs`, `handlers/assistant_feedback.rs:363`. Hosted AI and retained content are described; feedback is available to authenticated admin/operator readers. The old no-inherent-AI and no-persistent-content claims are removed. |
| Auto / Ask every time / Full access | `assistant_acknowledgement_service.rs:217` and `:544`. Auto confirms sensitive/destructive actions, Ask confirms non-read actions, Full access bypasses confirmation floors while separate resource authorization remains. The revised wording matches that distinction. |
| FAQ versus optional public AI; browser review plus original private-chat confirmation | `platform_bot_runtime_control_service.rs`, `platform_link_intent_service.rs:193`, `:715`, `:800`, and the BotFather/onboarding contracts. The policy preserves the difference between deterministic FAQ, configured public AI and linked personal authority; website sign-in alone does not establish the link. |
| Application encryption limits and upload retention | `assistant_upload_service.rs:206`, `:387`; `models/assistant_attachment.rs`; `models/assistant_upload_retention.rs:18`; `assistant_upload_retention.rs`; `docs/chat/10-uploads.md`. Encrypted file/extracted-text payloads are distinguished from database conversation/memory/context/feedback content. 24-hour pending, 30-day bound images/documents and conversation-lifetime tool defaults match source and are explicitly deployment-configurable. Expiry is not represented as immediate physical erasure. |
| Conversation deletion versus persistent agent memory and external copies | `assistant_nyxagent.rs:5227` deletes local conversation messages, attachments, feedback and checkpoint, and clears the home-conversation pointer; agent memory is a separate stored record. Policy avoids promising deletion of provider/channel copies or undoing completed actions. |
| Metered paid service, credential-class lanes, actual final charge and external checkout | `handlers/llm_gateway.rs:630`, `assistant_oneshot_inference.rs:990`, `billing/owner_resolver.rs:53`, `billing/config.rs:413`, `handlers/billing.rs:600`. Paid model calls, possible BYOK rates, allowance/grant/wallet funding and hosted checkout are supported. No free-only promise or universal app-store-payment claim remains. |
| Configured recipient categories and incidental public font traffic | Oracle/trigger models and record services support the disclosed retained content. `frontend/index.html:9` loads Google Fonts. The text distinguishes supported integrations from the actual deployed provider inventory and makes no unsupported universal region, contract, retention or model-training claim. |

Independent rendering check from `frontend/`:

```sh
./node_modules/.bin/vitest run src/pages/legal-pages.test.tsx
```

Exit 0, **1 file / 4 tests passed**, 1.28s (457ms test execution). Covers public
Terms/Privacy rendering, linked policy content and source fallback when fetching
fails. These four tests are separate from the 240 application/BotFather cases.

At review time Astra's owned evidence document still marked its source inventory
and issue disposition in progress. That completion and the hosted deployment's
actual provider/region/retention information remain Astra/PM-owned. PM should
compare the hashes at final freeze; subsequent policy changes require a delta
review. This source comparison does not assert live settings or close A1–A17.

## PM-owned remaining checks and live gates

Backend slot request, **commands proposed only, not executed**:

```sh
cargo test -p nyxid services::feature_flag_service::tests::shipped_registry_key_literals_are_pinned -- --exact
cargo test -p nyxid services::feature_flag_service::tests::workspace_preview_defaults_off_and_supports_targeted_user_rollout -- --exact
```

The new audience test uses the non-skipping transaction test database helper.
It covers ordinary/selected identities default-off, selected-user platform
override, personal/org resolution and explicit user disable. No added resolver
DB reads, email audience rule or browser-controlled authority were introduced.
Backend compilation and required CI lanes remain PM release checks. PM reports
the full frontend suite, production build and lint passed as detailed above;
Sol ran only the affected suite/lint after the Studio correction. No expensive
backend build was started by this implementer.

After deployment, PM must configure only Calvin's platform per-user override
(`add69059-bece-4f0e-9559-99cfd10b47eb`), audit the absence of global/org/other
user grants, and verify ordinary/Calvin authenticated navigation. Deploy the
hiding change before globally enabling Assistant navigation. Pricing, runtime
settings and production billing checks remain PM-owned, including an actual
metered model call and its exact settlement/ledger evidence.

Canonical A1–A14 real BotFather acceptance and A15–A17 protections are not closed
by these local tests. No fabricated Telegram identity, linking, model charge,
delivery or production acceptance proof is recorded. The canonical checklist,
PM launch plan, policy files, AGENTS.md and DESIGN.md were not edited by Sol.
