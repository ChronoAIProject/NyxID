# Assistant launch: legal source evidence

Date: 2026-10-23. Reviewer/editor: Astra, scope 5. Baseline: `origin/main`
`da8412fa1`; working branch `feat/assistant-launch-and-legal`.

Final handoff: legal content is accepted by PM and frozen; final hashes match
Sol's independently reviewed policy snapshot. Application cross-review is
complete, including Sol's Studio return correction: the sole P2 is resolved and
no additional findings remain in the reviewed diff. Only this evidence document
was updated during the final review. Final checks and release remain PM-owned.

## Ownership and initial findings for MAIN PM

- Edit ownership: `frontend/public/legal/terms.md`,
  `frontend/public/legal/privacy.md`, focused legal-content tests if needed, and
  this document. Public legal page components and prerendering are in scope if
  needed. Shared router, landing, assistant, backend, AGENTS.md, DESIGN.md and PM
  plans are read-only for this reviewer.
- Terms §3.1 and §4.1 deny built-in AI; current NyxAgent/NyxBot functionality
  contradicts that description. §3.2's blanket memory-only body handling and
  approval-before-every-request statements also need correction.
- Terms §14.1 says the App has no fees and §14.2 assumes App Store payment
  processing. Current usage billing, wallets and hosted checkout contradict this.
- Privacy already has a 2026-10-19 update: it avoids the historic fixed deletion
  and audit-retention promises. Its introduction and collection/use sections
  still describe primarily authentication and omit persisted assistant data.
- GitHub issues #1930 and #1957 were fetched read-only on 2026-10-23; both are
  open. Their historic allegations must be assessed against current source.
- MAIN PM messaging was attempted through `heca_agent_send_prompt` after finding
  the PM via `heca_agent_list`. The gateway denied child-to-parent authorization.
  This durable document is the handoff channel requested by the task.

## Legal edit result

Read the full repository AGENTS.md and DESIGN.md, plus
`docs/PLATFORM_BOTFATHER.md`, `docs/NYXBOT_ONBOARDING.md` and
`docs/plans/2026-10-19-platform-botfather-repair.md`. The repair document explicitly
supersedes its historical diagnosis with main's functioning public runtime and
NativeV2 integration. Its old no-deployment/one-worker scope is not a restriction
on this launch. No production behaviour is inferred from those old test records.

Both canonical policy documents now carry `2026-10-23`:

- Terms §4.1 acknowledges hosted NyxBot/specialist AI and BYOK/platform paths;
  §3 and §4.2–4.4 no longer describe NyxID as a pure router or promise approval
  before every action. They distinguish public FAQ, public AI and personal chat.
- Terms §14.1 acknowledges paid metered features, credential-class/operation
  rates, allowances/grants/wallets and separate provider fees. §14.2 describes
  offered external checkout without inventing a current tariff, free allowance,
  refund entitlement, subscription or App Store billing implementation.
- Related factual contradictions were corrected: blanket memory-only proxy
  retention, CLI-local credential guarantees, universal encrypted/audited-access
  claims, all-partner-apps ownership, and a claim that all hosting-provider DPAs
  and processing locations were established by the published policy.
- Privacy now covers hosted assistant/automation content, channels, voice,
  Oracle/replay content, billing and feedback; distinguishes encrypted payload
  fields from database text; describes configurable upload defaults separately
  from history; and distinguishes supported recipients from a verified live
  sub-processor register. It retains the prior conservative erasure disclosures.
- Privacy cookie text now conditions Secure/HTTPS on deployment configuration;
  Google Fonts requests are disclosed separately from optional analytics. The
  children's section follows the existing Terms' age-18 eligibility and no
  longer promises immediate removal contrary to §6.

No non-legal application files, AGENTS.md, DESIGN.md, PM plan or acceptance
document were edited. No staging, commits, pushes, production writes, extra
agents or backend/full frontend builds were performed by this reviewer.

## Material source evidence

References below are repository-relative at `da8412fa1` unless noted; symbols
remain the anchor if concurrent app edits shift a line number.

| Policy statement | Source and limit |
| --- | --- |
| Hosted assistant uses an upstream engine and persistent conversations | `backend/src/services/assistant_nyxagent.rs:34` (`SERVICE_SLUG = llm-nyx`); `backend/src/services/assistant_nyxagent_runtime.rs:485` (turn context); `backend/src/models/assistant_message.rs:49` (stored `text`, status, activity and attachments). `docs/chat/08-nyxagent-engine.md` describes the engine contract. This establishes code support, not a deployed model/provider list. |
| Saved memory and working context are database content | `backend/src/models/assistant_agent.rs:144` (`memory`), `backend/src/services/assistant_team_service.rs:2053` (`remember`), `backend/src/models/assistant_checkpoint.rs:12` (`goal` and reply `output`). None of those content fields is application-layer ciphertext. |
| Uploaded payloads/extracted text and assistant credentials use encryption | `backend/src/services/assistant_upload_service.rs:175` (extract and encrypt), `backend/src/services/assistant_record_service.rs:35` (`encrypt_credential`) and `:180` (`encrypt_image`); `backend/src/models/assistant_attachment.rs`. These do not establish database-volume, backup or end-to-end encryption. |
| Upload defaults differ from message retention | `backend/src/models/assistant_upload_retention.rs:18` (`Policy::default`: 24h pending, 30d image/document, tool images `None`); `backend/src/services/assistant_upload_retention.rs:147` (age evaluation), `:360` (expiry tombstones); `backend/src/services/assistant_upload_service.rs:1027` explicitly preserves message metadata. Defaults are labelled as software defaults, not verified live settings or deletion deadlines. |
| Conversation deletion does not erase the agent's separate memory | `backend/src/services/assistant_nyxagent.rs:5224` (`delete`) transactionally removes conversation-owned messages, attachments, checkpoints and feedback; its comment explicitly says the agent remains. Live voice/external sessions may block deletion until ended/settled. No external-copy deletion is implied. |
| Reply feedback is retained and reviewed | `backend/src/models/assistant_message_feedback.rs:41` stores the optional comment; `backend/src/handlers/assistant_feedback.rs` gates admin/operator aggregate/recent reads; `backend/src/services/assistant_feedback_service.rs` stores metadata-only context separately from comments. |
| Voice sends audio and saves transcripts | `backend/src/services/voice/openai.rs`, `backend/src/services/voice/grok_runtime.rs` and `backend/src/services/voice/transcript.rs:237`; `backend/src/models/assistant_voice_session.rs:1` explicitly excludes stored audio/SDP/credentials from the session record. The policy promises neither no external retention nor a particular audio deletion schedule. |
| Some routed feature content is durable | `backend/src/services/async_service_operation.rs:430` encrypts retained asynchronous results; `backend/src/services/oracle_task_service.rs:1` describes prompt/response/task storage with terminal retention; `backend/src/config.rs:1506` reads Oracle retention; trigger retention/encrypted replay is implemented in `backend/src/services/trigger_service.rs` and documented in AGENTS.md Rule 14. Ordinary relay metadata is not a blanket no-content-storage promise for assistant features. |
| Shared bots distinguish FAQ, public AI and linked personal authority | `docs/PLATFORM_BOTFATHER.md:26` (FAQ without model/private context), `:31` (explicit public AI), `:37` (personal thread, normal permissions/memory/billing), `:69` (browser review plus same-chat proof), `backend/src/services/platform_personal_binding_service.rs`, `backend/src/services/platform_personal_turn_service.rs`. `docs/NYXBOT_ONBOARDING.md:25` separately documents the existing Aevatar registration path; it is not substituted for current shared-entry authority. |
| Service pricing and checkout exist | `backend/src/services/billing/config.rs` resolves configured credential-class prices; `backend/src/services/billing/owner_resolver.rs:53` resolves execution payer; `backend/src/services/managed_execution_service.rs:49` requires a synced bounded tariff for managed execution; `backend/src/services/billing/lago_client.rs:413`/`:485` links customer identity to configured Stripe checkout; `frontend/src/pages/billing.tsx:146` opens returned checkout. `docs/BILLING_OPERATIONS.md:24` describes account rollout. No specific live fee or tax is asserted. |
| Additional supported recipients | `backend/src/telemetry/mod.rs:49`/`:51` defaults to US PostHog ingestion; `backend/src/services/billing/lago_client.rs:485` sends customer ID/name/email and configured payment linkage; `backend/src/services/voice/credentials.rs` resolves model/voice credentials; `backend/src/services/call_channels/sms_delivery.rs:188` includes Twilio SMS; `backend/src/config.rs` configures SMTP/FCM/APNs and infrastructure. Provider availability, contracts and regions are separate operational facts. |
| Cookies/font requests | `backend/src/handlers/auth.rs:204` builds cookies with a conditional `Secure` flag; `frontend/scripts/prerender-legal.mjs:60` includes Google Fonts links. The policy does not describe font requests as consented analytics. |

## Terms §4.3: retired selector versus effective confirmation mode

MAIN PM specifically asked about the apparently conflicting comment at
`backend/src/services/assistant_nyxagent.rs:654`. The `TurnRequest.access_mode`
there is the **retired Ask/Full conversation selector** (the conversation model's
`AccessMode`), accepted but ignored. Its “every chat runs with Full access”
comment does not disable the separate owner action-confirmation policy.

The live path is:

1. `backend/src/services/assistant_account_tools.rs:702` invokes
   `confirmation_needed` after account authority checks. Service execution uses
   the same gate at `backend/src/services/assistant_service_execution.rs:642`;
   native meta-tools at `backend/src/services/assistant_native_execution.rs:200`;
   machine tools at `backend/src/handlers/machine_tools.rs:309` with their floor.
2. `assistant_acknowledgement_service.rs:544` calls `effective_access_mode`.
   `:239`/`:246` resolves the server-bound agent's `access_mode_override`, else
   the acting person's `assistant_settings_service::get(...).access_mode`.
   Tool input does not choose that agent binding or bypass the lookup.
3. `assistant_settings_service.rs:20`/`:27` loads persisted owner settings;
   absent rows use defaults. A legacy explicit destructive opt-out maps to
   `Autopilot` only if no new mode is stored. `models/assistant_settings.rs:8`
   defaults to `Nyxmode`.
4. `assistant_acknowledgement_service.rs:216` maps current settings:
   `Nyxmode` = **Auto** (sensitive/destructive ask), `AskEveryTime` = **Ask every
   time** (non-read asks), `Autopilot` = **Full access** (no confirmation).
   `:557` returns auto-allowed/proceed before webhook/policy floors when Full
   access applies; `:581` asks on those floors otherwise. Resource authority is
   separate from that confirmation choice.
5. `frontend/src/components/assistant/composer-settings.tsx:65` displays the
   agent override or owner setting; `:210` writes owner settings. NyxBot Settings
   uses the same fields in `nyxbot-settings-content.tsx:88` and `:152`.

Thus the edited Terms describe the effective current mode, not the retired
TurnRequest selector. No backend change or test execution was needed for this
finding. Existing `assistant_access_mode_tests.rs` exercises the effective-mode
and confirmation gates, but those backend tests were not run in this task.

## Issue #1930 disposition

Fetched read-only with `gh issue view 1930 --repo ChronoAIProject/NyxID --json
number,title,body,state,url,updatedAt` on 2026-10-23. Issue is **OPEN**, updated
2026-10-18; it describes v0.71.0 and its recheck at `f96cf87`, not this baseline.

| Historic allegation | Current source-grounded disposition |
| --- | --- |
| 1. Automatic 90-day audit purge is promised but absent | The audit index block `backend/src/db.rs:709` has no TTL; the 90-day TTL at `:841` is a different collection. Current privacy §6 already discloses retained audit/billing records and no fixed software period. Preserve that wording; no purge claim is made. |
| 2. Permanent deletion within 30 days; missing credentials/services/endpoints/nodes | Fixed deadline removed by the prior privacy update. Current erasure inventory explicitly includes `user_api_key` (`personal_erasure_service/local.rs:270`), `user_service` (`:310`), `user_endpoint` (`:311`) and owned node drain/removal (`:680`/`:708`). The historical four-collection omission is not accurate as a blanket current claim. Full coverage/race proof remains open under #1957; no complete-erasure claim is made. |
| 3. Server analytics bypasses consent | Repaired in current source: `telemetry/context.rs:57` derives explicit device/account narrowing; `telemetry/mod.rs:174` checks matching context, `:184` obtains live consent revision, `:254` rechecks before dispatch. The policy already describes account-plus-device consent and withdrawal limits. No claim that sent requests can be recalled. |
| 4. Export right is presented as self-service | `routes.rs` still supplies no personal export download. Current privacy §7 explicitly directs access/export to manual contact, states no self-service download, and does not invent a delivery deadline. The manual fulfilment operation itself needs operator ownership; source cannot prove it is performed. |
| 5. Mobile toggle absent | Repaired: `mobile/src/features/account/AccountSettingsScreen.tsx:713`–`:724` contains account and device switches and calls `deviceConsent.setConsent`; account loading/retry/mutation is at `:153`. Historic no-toggle allegation does not describe current source. |
| 6. Session/refresh tokens described as encrypted | Already corrected: `models/session.rs:50` stores `token_hash`; `models/refresh_token.rs:11` stores `jti`. Privacy distinguishes these from encrypted one-time deliveries and assistant credentials. |
| 7. Regular JWT rotation promised with only one key | Prior policy removed the unsupported schedule. Current `crypto/jwt.rs:280`–`:365` supports retained verification keys, and `handlers/oidc_discovery.rs:140` publishes the live key set. Capability is repaired; an actual operational rotation schedule remains unverified. |
| 8. Recipient list omits billing/AI/voice | The 2026-10-19 policy already added these categories. This edit adds fuller content types, messaging/gateway and hosted flow detail, and separates supported integrations from live sub-processor facts. No provider/DPA/region/training guarantee is inferred from code. |

Recommendation: keep #1930 open for its explicit outstanding work, or let PM
re-scope it with #1957 as the completeness follow-up. This policy edit alone
does not establish complete personal erasure and does not close either issue.

## Issue #1957 disposition

Fetched read-only with the same `gh issue view` fields. Issue is **OPEN**, updated
2026-10-20. Its remaining requirements are real on this baseline:

- `backend/src/services/personal_erasure_registry.rs:7`/`:22` registers only
  revocation receipts for final cleanup. It is not the requested exhaustive
  Delete/Retain/NotPersonal ownership registry with source-completeness checks.
- `backend/src/handlers/users.rs:281` returns status, domains and provider
  effects, but no registry-derived `retained: [{collection, reason}]` list.
- `personal_erasure_service/local.rs:337` implements bounded pages and
  `worker.rs` implements leased progress; existing acceptance/domain/provider
  tests do not prove all-collection interruption/recovery or all writer races.
  `docs/security/PERSONAL_ERASURE_INVENTORY.md:162` explicitly preserves those
  coverage limits, and `docs/SECURITY_REMEDIATION_W1_W4.md:31` names #1957.
- `handlers/users.rs:567` accepts durable erasure and explicitly distinguishes
  immediate authority fencing from ongoing work; `local.rs:742` retains audit,
  financial and lifecycle evidence. Neither source-account absence nor a
  completed local phase proves external deletion. Privacy §6 keeps this clear.

No full registry, all-collection fixture test, chain verification after erasure,
or production erasure was run or claimed by this legal review.

## External facts for MAIN PM / operator

Code cannot verify the current hosted provider/sub-processor legal entities,
processing regions, DPAs or transfer mechanisms; model-provider retention or
training terms; live retention overrides; backup expiry/restoration procedures;
actual key-rotation cadence; manually fulfilled export requests; or delivered
provider erasure. Policy wording directs those enquiries to the operator and
does not substitute software defaults for deployment facts.

Existing corporate/contact/address, contractual governing-law/arbitration,
indemnity/liability and content-licence provisions are not established by source.
In particular, Terms §9.4's existing perpetual User Content licence, §11.1's
incident-procedure statement and §8.6's app-store distribution/compliance claims
remain matters for the business/legal owner. This source review is not proof of
those facts. No new contractual guarantee on these subjects was added.

PM reported a successful synthetic live browser turn and attributable billable
input/output/cache rows funded by existing benefits. That is **PM-reported live
evidence**, not independently observed here; it supports removing the no-fees
claim but does not establish a universal tariff or all-user availability.

## Focused verification and freeze

- `cd frontend && npm test -- --run src/pages/legal-pages.test.tsx`: **4 passed**.
  Tests read the actual published markdown and render the real Terms/Privacy
  pages; verify hosted AI/paid use, confirmation modes, encryption/retention/
  erasure limitations, cross-policy/contact links and raw-document fallback.
  An initial test-only theme mock omitted `useResolvedTheme`; corrected it and
  reran successfully. No product failure was hidden.
- `cd frontend && npx eslint src/pages/legal-pages.test.tsx`: passed, including
  the final rerun after the test-only mock correction.
- `git diff --check -- frontend/public/legal/terms.md
  frontend/public/legal/privacy.md frontend/src/pages/legal-pages.test.tsx
  docs/plans/2026-10-23-assistant-legal-evidence.md`: passed.
- Executed the actual `scripts/prerender-legal.mjs` in an isolated temporary
  frontend tree with canonical markdown/CSS and the installed node_modules.
  Both HTML outputs passed current-date, canonical URL, cross-policy link,
  content and SHA-256 ETag checks. No shared `frontend/dist` or build was touched.
- Read/checked root `/privacy` and `/terms` routes (`frontend/src/router.tsx:278`),
  exact nginx static mappings (`frontend/nginx.conf.template:160`/`:172`) and
  canonical `/legal/*.md` assets. Both routes are outside auth guards; public
  links are `/`, `/privacy`, `/terms`, and the existing contact email.

Legal content is ready for PM review; application cross-review follows below.
No live endpoint or deployment verification is claimed by this reviewer.

PM accepted legal content pending final hash matching. Frozen SHA-256 values
match Sol's independently reviewed snapshot exactly:

| File | SHA-256 |
| --- | --- |
| `frontend/public/legal/terms.md` | `906c5a071a7d7695b28b16e29ca17eda10a903b8739f2e4d4b8fffabe443b6ea` |
| `frontend/public/legal/privacy.md` | `30eb46cb452e86af5e646c3cef758452a1f327a66c492c616fad2555b806f4f1` |
| `frontend/src/pages/legal-pages.test.tsx` | `0fea0d7b56c1cb28d9b23308a201622b9d6bd9b58e23bc0c5de983f9052a6f08` |

## Independent application cross-review

### Finding resolved by Sol and independently rechecked

**P2 — A signed-out visit to bare `/dashboard` can lose its intended Studio
destination.** `frontend/src/router.tsx:468` still explicitly excludes
`/dashboard` from return preservation and redirects to bare `/login`. Previously
successful login defaulted to `/dashboard`; the changed defaults now send that
visitor to `/assistant`. `main.tsx` has a separate guard that does preserve the
path, but it does not remove the router guard's conflicting outcome (client
navigation, logout/session expiry or load ordering can invoke the route guard).
The new auth tests use an already supplied `/dashboard?explicit=1` return and
therefore do not exercise this plain-URL omission.

Recommended correction: preserve `/dashboard` just like other explicit protected
destinations in `dashboardLayout.beforeLoad`; keep bare `/login` as the normal
Assistant default. Add a router regression for signed-out `/dashboard` and its
return target. Application files remain read-only for this reviewer; Sol/PM owns
the correction or documented scope decision.

PM confirmed this P2 and assigned the correction/regression to Sol. The completed
delta review below resolves it; the preceding description records the original
finding. No application file was edited by this reviewer.

### Remaining application review

No additional findings in the reviewed 14-file application diff (production
code plus tests). SHA-256 of `git diff --` over changed `backend/src/` and
`frontend/src/` tracked files before the pending dashboard fix:
`b074e12b7676db0f408646069ee4874a230db87e5c8dd03fbf680d87944f2310`.

- Password, MFA and browser device login resolve a trusted explicit return
  first, then use `/assistant`. `resolveTrustedAuthReturnTo` still restricts
  HTTP(S) to frontend/configured backend origins. The auth-layout redirect
  retains the same ordering. Social auth's existing callback goes to the
  trusted target or frontend root (`backend/src/handlers/social_auth.rs:1316`),
  which now defaults authenticated users to Assistant. OAuth popup completion,
  invitation/consent and proof flows are not redirected indiscriminately.
- Root route and post-checkAuth boot redirection agree on `/assistant`.
  Remaining `/dashboard` destinations are intentional Studio navigation,
  admin-denial recovery, CLI/OAuth completion choices or development mock paths,
  except the explicit-destination omission identified above.
- `assistant:workspace-preview` is registered with `default_enabled: false`
  in both mutually exclusive production/test registries
  (`feature_flag_service.rs:158`, `:312`, `:351`). No new override resolver or
  hot-path database lookup is introduced. `resolve_personal_features` at `:834`
  still derives the capability from live memberships/standard overrides;
  `handlers/users.rs:366` returns it through authenticated `/users/me`.
- `useFeature` and `userHasFeature` fail closed for missing/loading capabilities.
  `assistant-sidebar.tsx:610` reads that capability to conditionally show the
  two existing `ComingSoonItem`s. There is no production email allowlist,
  local-storage switch or mock authority. The new frontend tests deliberately
  reject a matching email plus a local hint without server capability.
- Activity/Artifacts have no actual route or data endpoint in this surface.
  `/assistant/activity` and `/assistant/artifacts` therefore remain 404 rather
  than gaining hidden-but-callable implementation. The selected cohort sees
  disabled placeholders. `useAssistantWorkspaceCounts` still uses authorised
  pending approvals and a constant zero for artifacts; it does not retrieve
  concealed artifact data.
- The standard resolver intentionally permits global/org/role/user overrides.
  The code itself does not constrain the audience to Calvin. PM must use the
  authorized per-user override and audit other scopes to achieve the selected
  rollout; the implementation evidence correctly retains this deployment gate.
- Backend audience-test source covers default-off, selected-user enable,
  ordinary-user denial, org-context resolution and disable. It was not compiled
  or run here. PM owns that verification and the release build/CI.

Independent focused application command, from `frontend`:

```sh
npm test -- --run \
  src/components/auth/auth-flow.test.tsx \
  src/components/auth/mfa-verify-form.test.tsx \
  src/components/auth/web-device-login.test.tsx \
  src/components/assistant/assistant-sidebar.test.tsx \
  src/pages/assistant-workspace.router.test.tsx \
  src/hooks/use-feature-flag.test.tsx
```

Result: **6 files, 166 tests passed**, exit 0, 7.77 seconds. This is independent
local verification, not production acceptance. These passing tests precede the
dashboard regression fix and do not settle the P2 by themselves.

### Dashboard fix delta review

**Resolved; no remaining application findings in the reviewed snapshot.**
`frontend/src/router.tsx:467` now excludes only `/` from explicit return
preservation. A signed-out `/dashboard` therefore receives the same absolute,
same-origin `return_to` as other protected destinations. The login/auth layout
still validates that return through `resolveTrustedAuthReturnTo` before choosing
the default (`router.tsx:179`; `frontend/src/lib/return-url.ts:22`). Bare login and
authenticated root still default to `/assistant`; the correction adds no new
authority or external-origin exception.

The new regression at
`frontend/src/pages/assistant-workspace.router.test.tsx:230` exercises the real
route tree for both bare and query-bearing Studio URLs. It starts signed out,
asserts the login redirect contains the original destination, then enters that
login URL authenticated and asserts navigation back to Studio, without rendering
the Assistant default. This verifies both sides of the route return contract;
it does not simulate a real password exchange. Sol separately records that the
bare-URL case failed before the correction and passed afterwards; that pre-fix
run is Sol's evidence, not an independent rerun by this reviewer.

Independent delta command from `frontend`:

```sh
npm test -- --run src/pages/assistant-workspace.router.test.tsx
```

Result: **1 file, 75 tests passed**, exit 0, 7.05 seconds. This includes the two
new Studio cases, the Assistant default routes and unavailable Activity/Artifacts
direct URLs. `git diff --check -- backend/src frontend/src frontend/public/legal`
also passed. The previous six-file/166-test application result remains the wider
independent check; the 75-test delta run overlaps that suite and is not an
additional 75 distinct cases. No full build or backend test was run here.

Re-read the final production diff: auth defaults, the existing standard feature
resolver, default-off preview registration and conditional disabled placeholders
remain as assessed above. The only follow-up application correction is the Studio
return handling and its regression. SHA-256 of `git diff --` over the 14 changed
tracked `backend/src/` and `frontend/src/` files at final review:
`7aca1ff104552976a2f951b68580a21fd34e65c34a039b4864056cb6a50b615f`.
HEAD remained `da8412fa1d93f0252ac0ae9de5b1124adef7ed5e`.

Final direct SHA-256 comparison of Terms, Privacy and `legal-pages.test.tsx`
matches all three accepted values in the freeze table above. No legal content
changed during this delta review. All files except this evidence document remain
frozen for this reviewer. Backend/CI validation, live provider facts, the selected
per-user override and absence of broader overrides, deployment and final public
checks remain with PM; this review performs no live access or production writes.

## 2026-10-23 integration note: feedback readers after #2026

PM integrated literal incoming main
`756a20c336bb44bcf4fe8651f7eb6a2e2c1f3a08`; the resulting merge HEAD for this
focused policy correction is `54d41e8ac6ad59e9fecd83914d7ab4b11dac9091`.
The earlier source reviews and native report remain scoped to their recorded
snapshots. This note supersedes their admin/operator-only feedback description
and the previous Privacy/test hashes; it does not claim a full review of the
integrated application or complete a native review token.

Read the actual merged handler and documentation. At
`backend/src/handlers/assistant_feedback.rs:363`, feedback reads now call
`require_admin_usage_read`; `AdminFeedbackRow.comment` is returned at `:354`.
`backend/src/handlers/admin_helpers.rs:108`/`:122` checks the current user's
resolved permission through `role_service::can_read_admin_usage`.
`backend/src/services/role_service.rs:261` permits admins/operators and people
whose direct or group roles hold `nyxid:admin:usage:read`; the seeded Usage Viewer
role is an example, not the only way to receive that permission.
`docs/chat/08-nyxagent-engine.md:1768` explicitly documents that these readers can
see feedback comments and context. It does not expose conversation message text.

Updated only the feedback-reader bullet in `frontend/public/legal/privacy.md:48`
to disclose review by authorised administrators, operators and users with
permission to read administrative usage reports, including Usage Viewers. Added
one assertion to the existing public Privacy rendering test at
`frontend/src/pages/legal-pages.test.tsx:69` to require that disclosure. The
effective date remains 2026-10-23; Terms and all other policy content are unchanged.

Focused verification from `frontend`:

```sh
npm test -- --run src/pages/legal-pages.test.tsx
```

Result: **1 file, 4 tests passed**, exit 0, 948 ms. Only the legal suite was run;
no application/backend suite or build was started. Scoped whitespace validation
also passed. These checks cover local policy rendering, not production access.

Updated freeze hashes:

| File | SHA-256 |
| --- | --- |
| `frontend/public/legal/terms.md` (unchanged) | `906c5a071a7d7695b28b16e29ca17eda10a903b8739f2e4d4b8fffabe443b6ea` |
| `frontend/public/legal/privacy.md` | `c5169c1ae5b5d0cd76b10ad82b65352e9a920dcc3cd122afaf6745f6bc467c33` |
| `frontend/src/pages/legal-pages.test.tsx` | `4d722a97a659a0bb5dc941c4fcfe1afeadcaf95649f74f8a08e12aad8219b510` |

This correction and this evidence document are frozen for PM's fresh integrated
reviews. Only the three authorised files (Privacy, its existing test and this
document) were edited. No staging, commits, Git integration, other repository
edits or production writes were performed by this reviewer.
