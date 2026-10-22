# Assistant launch and public legal policies — 23 October 2026

## Authority and workspace

The user authorized implementation, review, pull request, merge, production activation and acceptance in this worktree. Historical plan restrictions describe earlier sessions and do not narrow this authorization. Preserve security/readiness guards, required checks and branch protection. Do not expose credentials or customer content.

- Workspace: `assistant-launch-and-legal`, Heca worktree `01a1654b-3af3-70b0-a01e-a45d8c2c09f1`.
- Branch: `feat/assistant-launch-and-legal`, initialized from fetched `origin/main` at `da8412fa1`.
- PM: `01a1654c-4cd4-7063-9063-c437983393b0`. Owns this plan, integration, independent review, serialized staging/commits/builds, release and production evidence.
- Implementer: Codex GPT 6.1 Sol, `01a1654d-47c5-73f0-b8ae-03ce8fd03c09`, session `01a1654d-47c7-75a1-8095-2376fb1ef8d8`. Owns application changes for scope 1–4 and targeted tests; excludes public legal pages, policy docs, this plan and canonical acceptance checklist. Evidence: `2026-10-23-assistant-implementation-evidence.md`.
- Legal editor/reviewer: Codex GPT 6 Astra, `01a1654d-4b1c-7263-920a-3f0832aa371f`, session `01a1654d-4b1d-7a30-adc6-3193c519e814`. Owns public legal pages and policy documentation, source-grounded assessment of #1930/#1957 and legal tests. Evidence: `2026-10-23-assistant-legal-evidence.md`.
- Both staff were explicitly placed in this existing worktree; no additional agents, concurrent staging or commits.

## Authorized scope and completion criteria

- [ ] Enable Assistant entry and public BotFather in production through supported configuration paths.
- [ ] Complete and evidence actual A1–A14 definitions from [canonical acceptance](2026-10-15-public-personal-nyxbot-acceptance.md). Preserve A15–A17 protections. A13 requires the real pinned runtime, two real identities and Telegram; mocks cannot complete it.
- [ ] Read current onboarding/repair records, especially deterministic FAQ activation in [19 October repair](2026-10-19-platform-botfather-repair.md); retain optional explicit AI policy and all readiness gates.
- [ ] Make Assistant the post-login default. Audit auth and `/dashboard` redirects; preserve intentional destinations, deep links, OAuth and consent flows.
- [ ] Gate Activity and Artifacts through existing feature flags using authenticated identity, allowing only `calvin.tan@aelf.io`. Verify navigation and direct URLs for Calvin and ordinary users. Preserve stored data.
- [ ] Enable normal user NyxBot availability flags. Activity and Artifacts are the Calvin-only exception.
- [ ] Inspect and enable Assistant billing, real LLM usage pricing, Lago and runtime readiness. Verify real charging/usage path without weakening readiness guards or making unrelated pricing decisions.
- [ ] Correct Terms §4.1/§14.1 and source-inconsistent privacy statements, including supported hosted-service data handling and sub-processors. Astra edits actual public policies. No unsupported guarantees.
- [ ] Independently review disjoint implementation and legal changes, address findings and run required repository checks.
- [ ] Ship PR, satisfy checks/reviews, merge through protected workflow, verify deployment revision, public legal links and production activation.

## Evidence and status

2026-10-23: Both requested staff were created with explicit placement and confirmed by this PM gateway. Both completed their assigned edits and independent cross-reviews. PM committed the application, public policies and evidence, then merged fetched main at literal `756a20c336bb44bcf4fe8651f7eb6a2e2c1f3a08` without conflicts (integration commit `54d41e8ac`). Fresh integrated checks and reviews use the same original agents. No PR, protected main merge, deployment or production setting write is claimed at this checkpoint.

| Gate | Status | Evidence / remaining work |
| --- | --- | --- |
| Staff placement and work | Confirmed | Both original child IDs completed edits and cross-reviews in this worktree; reused for final reviews, no duplicates |
| Source and current acceptance review | Completed for changed scope | AGENTS.md, DESIGN.md, current onboarding/repair, billing, feature flag and environment docs; current A1–A17 names below |
| Implementation and tests | Implemented; frontend verified | Default Assistant redirects, preserved explicit Studio return, authenticated default-off workspace preview; 240 distinct targeted application/BotFather tests verified incrementally; backend audience test awaits CI |
| Policy corrections and source assessment | Edited; cross-review passed | Astra edited public Terms/Privacy and assessed #1930/#1957 against source; Sol independently checked policy hashes and 4 rendering tests. Both issues remain open |
| Independent review | Cross-reviews passed; final PM/adversarial review in progress | Astra found and verified the Studio return fix; Sol independently reviewed legal changes; evidence documents pin scope and limitations |
| PR / required checks / merge | Pending | No bypass or direct main commit |
| Production access and activation | Admin access confirmed | FAQ active and billing enabled; global Assistant activation waits for deployed navigation guards |
| A1–A14 live acceptance | Open; limited production evidence below | FAQ runtime active; one real personal turn/settlement; pinned runtime and two real Telegram identities still required |
| Deployment and public links | Pending | Merge alone and HTTP reachability do not prove new revision is live |

## Remaining issues and rollout

Record exact source gaps, test failures and unavailable access here as discovered. Finish all independent code, policy, review and PR work before reporting any unavoidable live access gate.

## Production observations (23 October, Singapore)

PM used the existing first-party CLI admin session, without printing credentials, against `https://nyx-api.chrono-ai.fun`. Supported admin HTTP APIs are available. Ordinary Python HTTPS received 403; the installed CLI and curl succeeded. No host trust or security setting was changed.

- Backend `/health`: version `0.73.1`, commit `9cc4a7ec722d`. Frontend `https://nyx.chrono-ai.fun/build-version.json` reports the same commit/build ID. This predates this branch; these observations are not deployment evidence for new work.
- Authenticated account is Calvin, `calvin.tan@aelf.io`, person UUID `add69059-bece-4f0e-9559-99cfd10b47eb`, platform admin.
- `experimental:ai-assistant`: default false, no global override, enabled for ChronoAI organization and two existing person overrides. Ordinary global availability is not enabled yet. `assistant:nyxagent-engine` defaults true. Activity/Artifacts flag is being implemented. Global Assistant activation waits until the new navigation guard is deployed.
- `experimental:billing` global override is true. `/admin/billing/config`: billing enabled, Lago configured, resale disabled, fail-closed incident switch false, reconcile 300 seconds, exact cutover acknowledged. Lago API and webhook secrets are configured (presence only observed); zero default overdraft. No new pricing decision or billing environment change is needed from these observations.
- `/assistant/nyxagent/models`: `nyxagent/chat`, `nyxagent/channel`, `nyxagent/research`, all available.
- `/admin/billing/overview` and `/admin/billing/rates`: `chrono-llm-public` platform lane is synced with fresh input/output/cache read/cache write/image prices. This supports configuration readiness; task-attributed live inference/settlement was verified below. The uncredentialed `llm-nyx` orchestrator hop is distinct from its chargeable provider hops.
- Existing Telegram profile `0b4949e8-be13-4abc-8b65-7b1d33d4aa11`, channel bot `bb3f92a9-6cf9-405c-9bca-bc422c8555d5`: authoritative `/runtime` reports `state=active`, public and personal enabled, emergency stop false, `public_policy=null`, `faq_ready`. Legacy profile `enabled=false` describes the older onboarding guide and must not be mistaken for runtime disablement. No activation write was needed for the existing FAQ runtime.
- GitHub access works. Repository deployment history is empty; workflows publish images but no production deployment workflow was identified. This host has no SSH config, Kubernetes config, cloud deployment credentials or secondary NyxID profile. PM requested the existing deployment path and two authorized real test identities asynchronously while code/docs work continues.
- Both original staff are confirmed `running` in this worktree after the user changed the execution permission profile. Child-to-parent Heca messages are rejected by gateway authorization, so staff evidence documents are the durable coordination channel. No replacement staff were created.

## Current acceptance map

These are the actual canonical gate names. Historical local passing results remain historical; this launch has not yet established full live acceptance.

| Gate | Launch evidence state |
| --- | --- |
| A1 Public tool boundary | FAQ runtime has no model/tools; optional AI pinned containment proof remains unverified |
| A2 Operator isolation | Source and regression review pending; no two-owner live test |
| A3 Proof and consent | Source/regression and live identity journey pending |
| A4 Personal parity | Source/regression and live permissions/credentials journey pending |
| A5 History | Separate personal thread contract documented; live web/Telegram check pending |
| A6 Revocation | Regression and exact live unlink/Stop check pending |
| A7 Replay and races | Regression evidence pending; no fabricated live replay result |
| A8 Billing | Real priced personal turn and exact allowance/grant settlement verified below; wallet/public sponsor/org BYOK/duplicate proof remain open |
| A9 Full-loop limits | Deterministic FAQ selected; optional AI budget/runtime proof pending |
| A10 Groups | Regression and provider proof pending |
| A11 Simple setup | Current FAQ runtime active/ready; UI journey and failed-readiness behavior pending |
| A12 Existing product | Full frontend checks and cross-review passed; required backend/integrated CI and matching deploy pending |
| A13 Live proof | Real pinned runtime and two authorized NyxID/Telegram identities not yet available |
| A14 Stable consent/capacity | Current no-policy FAQ behavior understood; regression/live lifecycle checks pending |
| A15 Public retention | Preserve existing protections; optional AI upstream retention proof pending |
| A16 Signup completion | Preserved; personal-link browser fixtures pass, including rejected MFA and fresh same-chat confirmation; real journey pending |
| A17 Personal lifecycle | Preserve web continuation and no-fallback behavior; targeted checks pending |

## Task-attributed live Assistant and billing proof

At 2026-10-22 20:42 UTC (23 October Singapore), PM submitted one synthetic,
non-tool browser turn through `POST /api/v1/assistant/nyxagent/turns` using the
existing authorised account. HTTP 200; stream reached `turn.completed`;
persisted assistant message completed with the exact requested nine-character
reply and no error. No customer transcript was read or printed.

- Conversation: `nyxa-36acd4ac0261456aa100355bf4881994`.
- Turn: `44bf4e0e-f527-423a-9f83-0be7786170fc`.
- Its newly provisioned conversation credential is
  `67ac25fd-d191-4d05-87ba-7df9d9f56284`; no credential material was exposed.
- `/billing/usage?period=24h` rows were selected by the exact new credential's
  name suffix and ID. Provider `chrono-llm-public` reports 12,980 input tokens
  including 8,832 cache-read tokens, and 7 output tokens. All five configured
  metric rows are billable; cache-write/image quantities are zero.
- Exact charge: 4,148 uncached input × 0.000005 = 0.02074 credits;
  7 output × 0.00002 = 0.00014; 8,832 cached input × 0.0000008 = 0.0070656.
  Total **0.0279456 credits**, funded by existing allowances (0.02088) and
  grant credits (0.0070656), with wallet share zero. This confirms real priced
  inference and benefit settlement through the existing charge path.
- `lago_acked=false` is expected for these fully benefit-funded quantities;
  only wallet-funded quantities are sent to Lago. This test does not prove a
  purchased-wallet debit, public sponsor funding, org BYOK or duplicate recovery.
- Calvin's wallet has existing reservations and negative wallet availability;
  this turn used existing benefits. No top-up, new grant, overdraft change or
  reservation modification was performed.

This proves one actual personal browser turn on the existing production build,
not the new branch's deployment or all of A8/A13. Two authorised real
NyxID/Telegram identities, pinned runtime deployment evidence and the actual
production deployment path remain requested. The passing browser turn must not
be presented as completion of Telegram onboarding acceptance.


## Integrated validation and release handoff

- Full frontend `npm test`: 606 files / 6,567 tests passed, 112.12 seconds. This precedes the two added Studio regression cases; their affected suite passed 75/75 afterwards.
- Full `npm run build`: passed TypeScript, Vite production build, legal prerender and public-build footprint checks. Existing bundle-size warning is non-fatal. Final route-only change will receive the declared build again before publication.
- Full `npm run lint`: zero errors, 33 pre-existing warnings. Changed route/test files also pass focused lint after the final correction.
- Final personal-link Playwright suite: all seven Chromium cases passed, 20.8 seconds, one worker and no retries. The stale MFA locator now matches the actual accessible label; rejected-code, authentication and same-chat consent assertions are unchanged. These browser cases use synthetic APIs and are not live Telegram proof.
- Rust formatting and `git diff --check` passed. Backend compilation, transactional audience test, coverage and integrated required lanes must pass in GitHub CI before merge. No existing test or readiness/security gate was disabled.
- Independent review found one real P2: bare `/dashboard` was excluded from login return preservation. Sol fixed it, reproduced the old failure, and added bare/query Studio regressions; Astra's independent 75-test delta run passed. No remaining cross-review application/legal finding in the reviewed snapshot.
- The generic local gstack pregate did **not** pass: its text scanner classifies Rust's `key: ASSISTANT_WORKSPACE_PREVIEW_FLAG_KEY` constant reference as an env credential, and its lane mapper does not discover embedded Rust tests/runtime legal Markdown/monorepo bootstrap coverage. PM verified the flagged value is a public constant reference, not credential material. No scanner suppression, invented pass, CI change or weakened test is included. Actual repo checks and review evidence remain authoritative.
- Release preparation uses the requested ship and land-and-deploy workflows. No configured VERSION source exists, so no invented version bump. Merge and GHCR image publication will be reported separately from a verified production deployment.

The production rollout order is: merge reviewed head after required CI; deploy backend and frontend through the supported operator path; verify matching revisions and public `/terms` and `/privacy`; set the sole `assistant:workspace-preview` per-person enable for Calvin and verify no wider grants; enable `experimental:ai-assistant` globally and verify ordinary-user capabilities/navigation. Existing normal chat engine and billing are already enabled. Migration gates, unsupported providers and optional beta features require their own readiness and are not blanket-enabled.

Remaining external inputs are the existing production deployment command/runbook and access to two authorised real NyxID/Telegram test identities with pinned runtime revision evidence. Admin API access is available. No SSH/kube/cloud host trust or security state has been changed to obtain deployment access.

## Main integration and publication checkpoint

Main integration includes Usage Viewer access to NyxBot usage/feedback (#2026), bounded NyxBot error telemetry (#2024), and activation-time feature readiness with diagnostic reads (#2014). Activation readiness guards are preserved. Astra corrected the public Privacy feedback disclosure to include authorised administrative usage readers, including Usage Viewers, matching the merged handler and role scope. Her focused legal suite passed 4/4. Terms are unchanged from their independently reviewed revision.

The first read-only documentation audit identified one current guide error: the sign-up guide still promised default dashboard arrival. PM corrected `docs/site/web/getting-started/sign-up.md` to explain Assistant arrival, explicit Studio `/dashboard` returns and trusted protected/OAuth destinations. The remaining audit attempt will inspect the final frozen integrated candidate. No third audit or silent reuse of stale review evidence is permitted.

Protected merge requires a separate approving GitHub review under the active `main` ruleset `13955986`, in addition to the repository's passing-check requirement. Staff source reviews do not substitute for an approval from a separate GitHub actor. PM will prepare a concrete PR and run its real CI before requesting the outstanding review; no self-approval or protection bypass is authorised.

Fresh checks after integrating main and the feedback disclosure:

- `frontend: npm test`: **608 files / 6,612 tests passed**, 113.81 seconds. This includes the current default/return routes and public policy rendering, superseding the earlier frontend count for this candidate.
- `frontend: npm run build`: **passed**, including TypeScript, production Vite, public legal prerender, credential-accept output and mock-footprint assertion.
- `frontend: npm run lint`: **zero errors / 33 existing warnings**, exit 0.
- `cargo fmt --all --check` and `git diff --check`: **passed**. The backend transactional test remains unexecuted locally and must pass in real CI.
- PM inspected all 16 eligible test/gate edit hunks in the integrated diff (22 listed, none unread); no security/readiness checks, retries, timeouts or thresholds were weakened. The MFA locator follows the current accessible label and retains rejected-code and same-chat consent checks.
- Sol's fresh source-grounded legal/guide delta review found no substantive issue and pinned the current policy/test/guide hashes. This is static review, not new backend or live acceptance evidence.

Private execution logs and review reports are retained under `/tmp/nyxid-launch-integrated-*` on the PM host. The PR will carry the final review and documentation-audit outcomes. Current production observations above remain evidence of the older deployed revision only.
