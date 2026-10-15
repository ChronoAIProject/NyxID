# Part 4: frontend contract review and implementation slices

Status: conditional acknowledgement of candidate `1b5aea084`, 2026-10-15.
Base: `7ff461c92`, branch `nyxbot-manager/frontend`. This proposal changes no
production code. PM and Part 1 must settle the corrections below before UI work.
Part 2's proposal at `338d92f5b` agrees that settled replies need origin metadata.

Reviewed the project instructions, `DESIGN.md`, the
[plan](2026-10-15-public-personal-nyxbot.md), the
[final Opus verdict](2026-10-15-public-personal-nyxbot-opus-review.md#convergence-verdict-2026-10-15-final-review-of-the-revised-plan),
the [workstreams](2026-10-15-public-personal-nyxbot-workstreams.md), and the
candidate's HTTP fixtures and `services/platform_manager_contract.rs`.
The candidate was read from sibling `NyxBot-manager-integration` at the stated commit.

## Acknowledged contract

Accept the reserved HTTP paths, body-carried POST preview/review/approve/deny/status,
human-session authority, owner-only paged bindings and generation-fenced revoke.
Accept decimal credit strings, string Telegram IDs, no-store responses, omitted
patch fields preserving state, and explicit null clearing only where declared.
Rust service proofs remain internal; they are not browser DTOs.

Accept separate personal/public controls, explicit emergency-stop clearing,
24-hour intents followed by fresh 10-minute challenges, and same-private-chat
confirmation. Public failures never select a personal payer. Personal failures
never select public execution. Consent reuses the existing NyxBot permission mode.
The new thread starts after linking and remains usable on the web after unlink.

The current frontend seams require these specific changes:

| Source | Consequence |
| --- | --- |
| `frontend/src/main.tsx`, `router.tsx`, `lib/telegram-claim-handoff.ts` | Router construction happens during module evaluation. Reuse the existing early handoff pattern; a page effect is too late to scrub a bearer fragment. |
| `lib/telemetry.ts::stripQueryString`, `SENSITIVE_PATH_PATTERNS` | Query stripping leaves a URL fragment intact. The new linking path needs exclusion and URL sanitation must remove both query and fragment. |
| `components/auth/auth-flow.tsx`, `lib/return-url.ts` | Login, MFA and social auth can retain a trusted `return_to`; email registration returns to login without authenticating. Store the intent separately from that clean return URL. |
| `pages/platform-onboarding.tsx`, `hooks/use-platform-onboarding.ts` | Existing onboarding uses query-carried codes, token-bearing query keys and onboarding-only disclosures. Personal linking needs separate hooks/page/storage. |
| `pages/admin-platform-bots.tsx`, `schemas/platform-onboarding.ts` | Current setup selects a catalog model and saves paused before enabling. Keep its sponsor prerequisite and existing amounts, then use the new runtime endpoints. |
| `schemas/assistant-nyxagent.ts`, `types/assistant.ts`, `hooks/use-assistant-nyxagent.ts`, `components/assistant/assistant-sidebar.tsx` | Zod currently strips the proposed unknown fields. Extend decoding and the conversation presentation mapping explicitly. Existing `channel` describes owned channel bots and cannot represent shared-manager authority. |

## Corrections required before freeze

### C1. Separate server support, missing controls and first Enable

The absent-controls fixture must return `supported: true, runtime: null` when
this server supports the feature. Absence means both lanes are off. It does not
mean unsupported. Flag-off/unmounted routes or an explicitly unsupported response
render unavailable; a failed GET never manufactures a runtime or revision.

Propose a second, strict creation arm for the existing runtime PUT. Use
`expected_runtime: "absent"` instead of invented revision zeros or nullable
revision defaults. It is mutually exclusive with the three existing expected
revision fields. Creation succeeds only while controls are absent, under live
manager/admin fences. A concurrent creator returns 409 and requires a fresh GET.
GET, profile discovery and readiness must not create enabled controls.

The single first **Enable bot** action explicitly requests both intended lanes:

```json
{"expected_runtime":"absent","personal_enabled":true,"public_enabled":true,"public_policy":{"source":"nyxagent","profile":"nyxagent/chat","total_daily_credits":"1","sender_daily_credits":"0.1"}}
```

If disabled controls already exist, first Enable uses their real expected epoch
and revisions with the same two explicit enable flags. Public validation and
both control changes commit atomically. Failed readiness leaves the prior state
intact. Neither form clears an emergency stop. Persisted creation defaults remain
false; the explicit human Enable request authorizes the changes to true.

Later public settings/Enable/pause requests omit `personal_enabled` entirely:

```json
{"expected_manager_authority_epoch":1,"expected_personal_control_revision":2,"expected_public_policy_revision":4,"public_enabled":true,"public_policy":{"source":"nyxagent","profile":"nyxagent/chat","total_daily_credits":"2","sender_daily_credits":"0.2"}}
```

Public pause sends the same expected fields plus `public_enabled: false`, with
no personal patch. These requests never change personal controls or consent.
Personal-only rollout has an explicit, clearly labelled action under Advanced
and no public proof dependency. Its creation payload is:

```json
{"expected_runtime":"absent","personal_enabled":true,"public_enabled":false}
```

For an existing runtime, personal-only updates send its real expected fields and
`personal_enabled: true`, omitting public changes. Do not silently turn a failed
combined Enable into personal-only success. Show public unavailable and offer the
personal action explicitly. Profiles/readiness also need the absent-runtime
precondition arm so discovery and validation work before initial creation.
Readiness uses the same proposed policy, without enable flags. Part 1/3 must
define the creation proof's revalidation against the newly allocated revisions.

### C2. Freeze the fragment and signup continuation behavior

Reserve the browser URL `/nyxbot/link#intent_token=<opaque-token>` separately from
`/onboarding/link`. Capture and remove the fragment synchronously before router,
auth or telemetry initialization. The module must have no router/telemetry
dependency. Sanitize malformed bearer fragments too. `telegram-claim-handoff.ts`
already demonstrates early capture; the new purpose needs separate storage.

Keep the token in memory and a dedicated tab-scoped `sessionStorage` record for
auth navigation/reload, with a local maximum of 24 hours shortened by server
`expires_at`. Use a clean same-origin `/nyxbot/link` return URL. Never place the
token in a URL query/path, `return_to`, OAuth parameters, query key, persisted
Zustand state, wire log, exception, toast or analytics property. Mark review/code
content sensitive and suppress telemetry on the linking path. Clear stored
material on completed confirmation, denial, cancellation or intent expiry.
Storage failure leaves the URL scrubbed and explains how to reopen `/link`.

Preview and review must not consume the intent. A 10-minute challenge expiry
permits fresh review using the still-valid intent. Bind review/digest to the
authenticated account and current `permission_mode`, using the actual values
`nyxmode`, `ask_every_time`, `autopilot`. Account or policy changes discard the old
challenge and require review again. No new Telegram permission editor is added.

Registration uses the existing invitation and enabled-auth-method policies.
Preview's `invitation_required` is informational; `/public/config` and the auth
backend remain authoritative. Pending/failed config does not assume open signup.
An absent/invalid invitation leaves the intent resumable and offers sign-in or
the existing invite-request instructions. Email registration, verification and
MFA do not approve consent. Email verification in another tab must allow return
to the original tab; a tab without the handoff explains how to reopen the original
link. No raw token is copied into an email verification return URL.

Add `replace_existing: true` to approve only after an explicit replacement
confirmation when review says `replaces_existing_binding: true`. Omitted/false
must refuse a replacement. The reviewed digest binds replacement semantics.
Do not disclose the previous owner's account or transcript. Status remains a
body-carried POST, scoped to the reviewing account, with bounded polling and
manual refresh. Only `confirmed` displays linked success.

### C3. Complete public DTOs without successful defaults

Freeze profile entries as `{id: string, label: string}`, with `default_profile`
an advertised ID or null. A requested draft profile of `nyxagent/chat` is the
product default; it is not proof that discovery advertised or approved it.
Unavailable discovery returns `profiles: []`, `default_profile: null` and typed
readiness. No fallback list or fabricated ready response is acceptable.

Use one readiness shape throughout: `status`, nullable `reason`, nullable
`checked_at`, nullable `expires_at`, and nullable `evidence_digest`. Ready requires
real timestamps and evidence; unavailable never carries usable evidence.
The readiness endpoint returns `{public_readiness: ...}`; profile discovery's
`readiness` uses that same value type. Freeze its closed reason enum with Part 3.
Missing runtime, missing sponsor, unavailable profile/engine, insufficient funds,
quota/capacity exhaustion, expired proof and infrastructure failure need truthful
rendering. Retry never implies Enable or an automatic repeated paid probe.

Map existing `guide.limits.daily_credits` to the total and
`guide.anonymous_daily_credits` to the per-person draft quota. Those are the
actual current accounting inputs in `platform_inference_service::reserve_budget`.
Use `"1"` and `"0.1"` only when no configured amounts exist. Keep
`funding_owner_user_id` from manager setup as the sponsor; selecting it again is
unnecessary. An absent sponsor requires an authorized-org choice, never the admin.
Compare exact amounts with `lib/credits.ts::parseCredits`, including quota <=
total, and accept at most 12 decimal places. Telegram IDs stay strings throughout.
New runtime schemas must not weaken legacy child/onboarding schemas.

Advanced can expose profile override and explicit sponsor change. Rate/concurrency
and execution-limit editors require Part 3 to publish their request/response
fields and bounds first; candidate v1 currently declares none. Until then show
server-provided effective limits read-only and preserve them on edits. Do not
copy direct-model timeout/token defaults into a multi-step agent runtime.

### C4. Project thread metadata on index, history and settled replies

Accept candidate `platform_thread` on conversation DTOs and `platform_origin` on
active turns as optional-nullable additive fields. Also project the same safe
origin summary on settled message DTOs, as Part 2 proposes. Ordinary web turns
on a platform thread have null origin. Do not expose the full internal proof.
Freeze that `platform_thread.manager_id` matches the personal binding's
`manager.id`, and distinguish it from the profile ID used in admin route paths.

Return metadata on both conversation index and history responses. Keep existing
owner-scoped live notifications so external Telegram starts refresh the thread
list/history through current live invalidation and polling. A thread badge must
survive settlement/reload. Use binding inventory metadata for manager labels;
no fabricated bot/profile names are needed. Freeze binding `state` as
`active | revoked`, or publish the actual additional values before fixture work.

Confirmed status may retain the candidate's minimal binding ID/generation.
Invalidate owner inventory on success. `conversation_id: null` means the dedicated
thread has not been created yet; invite the user to send their first private
message. Never attach the old home thread. After unlink retain history and app
approvals, with no Telegram destination. Consent must explicitly describe account
and service access, personal billing, existing memory/persona, sensitive card
confirmation power, platform-operated Telegram transport without end-to-end
encryption, and work continuing in NyxID after unlink.

## Small implementation slices after freeze

| Slice | Frontend scope | Focused validation |
| --- | --- | --- |
| 1. Contract and handoff | New personal-link schemas/hooks/storage; `main.tsx`, router, public-path and telemetry integration; `apiClient` actor guards and secret-free query keys | Schema fixtures, exact credits/IDs, module-order scrub and identity-change tests |
| 2. Admin setup | `admin-platform-bots.tsx`, platform schemas/hooks/lib and tests; `useAppForm`, three fields, one Enable, Advanced with independent personal action | First-create/both-enable versus public-only edit/pause, preserved amounts/sponsor, real unavailable rendering, CAS and emergency stop |
| 3. Signup and linking | Dedicated personal landing/review/status page; auth continuation integration; owner link list/revoke in existing NyxBot settings panel | Invitation and auth variants, 24h/10min clocks, replacement, same-chat confirmation, unlink and account-switch races |
| 4. Assistant presentation | NyxAgent schemas, transport/view mapping, sidebar/thread badges and ordinary app cards | Owner-visible post-link history, external live starts, settled origin, no public/home import, post-unlink web use and deletion/recreation |

Reuse Tailwind/shadcn components and semantic tokens from `DESIGN.md`. Reuse
useAppForm's dirty behavior, opting out for programmatic prefill. No dependency
change is currently needed. Selective improvements from #1896 remain optional;
the old branch is not an integration base.

## Required browser fixtures and failure cases

| Fixture family | Required observations |
| --- | --- |
| Bootstrap privacy | Start from a bearer fragment with analytics consent already granted. Before router construction/history/auth/telemetry, the URL is clean. Raw token appears only in permitted POST bodies and dedicated handoff storage. Check request URLs, return URLs, query keys, telemetry, diagnostics, malformed fragments, refresh and StrictMode. |
| Runtime support and setup | Unsupported/flag-off, supported-null, initialized-disabled, personal-only and both-enabled fixtures. First Enable explicitly sets both flags; unavailable public proof leaves state intact and permits explicit personal-only rollout. Public edits/pause preserve personal revision/consent. Test stale revisions, emergency stop, expired proof, empty profiles, missing sponsor and failed/paid discovery. Ready fixtures exist only in tests. |
| Exact data | Preserve configured budgets unlike defaults, `0.000000000001`, quota greater than total, invalid/excess-precision input, and IDs such as `9007199254740993` without numeric conversion. |
| Signup continuity | Navigation/reload and registration longer than 10 minutes but less than 24 hours; existing sign-in, email verification, social auth, MFA, required/invalid invitation, open registration, config outage and unavailable auth methods. Check original-tab return, missing handoff and blocked storage. |
| Consent state | Review, awaiting confirmation, confirmed, denied, cancelled, expired, replay, wrong account/chat, replaced binding and changed permission mode. Approval alone never renders linked. A fresh challenge must work while its intent remains valid. All decisions/status are POST bodies. |
| Threads and unlink | Two owners and multiple bindings, owner-only list/revoke, null initial conversation, active and settled Telegram-origin messages, ordinary web turns on that thread, pending cards/images, unlink with continued web work, deletion and fresh recreation. No public transcript, old home transcript or previous owner's history is imported. |

Browser fixtures must intercept the HTTP boundary with realistic DTOs, existing
auth/config responses and assistant live events. Remove interception for the
later integrated run. Fixture success proves UI behavior only; public readiness
and cross-service acceptance still require Parts 3/5's real evidence.

Review-stage validation is source inspection, JSON parsing of the three payload
examples, link/reference inspection, and `git diff --check`. No builds, installs,
live accounts, Telegram actions or production validation were run in this stage.
