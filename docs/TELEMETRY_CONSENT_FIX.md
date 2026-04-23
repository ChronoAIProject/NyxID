# Telemetry Consent — Failings & Fix Checklist

Status: Pre-fix audit. Use this document as the checklist for closing the consent-withdrawal gaps before NyxID telemetry v1 is considered shippable to EU users.

Pair document: `docs/TELEMETRY.md` (the spec this fix brings the implementation into line with).

---

## 1. Context — what shipped vs what's actually wired

NyxID shipped the **banner** side of telemetry consent across web, mobile, and CLI. It did not ship the **withdrawal** side end-to-end:

| Surface | Banner / first-run prompt | Withdrawal UI | Withdrawal actually works |
|---------|---------------------------|---------------|---------------------------|
| Web     | Yes — `consent-banner.tsx`  | **Missing** — Settings page has no telemetry tab | Only by deleting `nyxid.telemetry_consent` in browser localStorage |
| Mobile  | **Never prompted** — `MobileConsentProvider` exists but no UI triggers it | **Missing** — `AccountSettingsScreen.tsx` has no toggle | Only by reinstalling the app |
| CLI     | Yes — first-run TTY prompt (`consent.rs:132-170`) | Yes — `nyxid telemetry {enable\|disable\|status}` | **Local PostHog client only** — backend still emits telemetry events because `X-NyxID-Client` header is gated on DSN presence, not consent (see §6) |

Worse than "missing UI": the stores on both web and mobile **have no persisted opt-out state at all**. `clearConsent()` exists in both (`frontend/src/stores/consent-store.ts:34`, `mobile/src/lib/consent.tsx:91`) but it resets the store to `asked=false, enabled=false`, which re-triggers the first-time banner on next page load. It does not represent a persisted "this user opted out" state, and it does not call PostHog's `opt_out_capturing()` / `reset()`. Building the Settings toggle therefore requires *implementing* a disable path first, not wiring up existing code.

## 2. Why this is a legal issue, not a UX polish issue

NyxID processes telemetry on the basis of **explicit user consent** (Article 6(1)(a) GDPR) — that's the contract the banner creates. Article 6(1)(a) is chained to **Article 7(3)**:

> *"It shall be as easy to withdraw consent as to give it."*

Current reality:
- **Web:** give = one click on the banner. Withdraw = open DevTools, delete the `nyxid.telemetry_consent` localStorage key. **Not "as easy."**
- **Mobile:** give = never prompted. Withdraw = uninstall the app. **Not "as easy."**

This isn't defensible under legitimate-interest either, because the banner language explicitly frames processing as consent-based.

Further, the existence of the banner + privacy policy language + internal spec claiming a toggle that *doesn't* exist compounds the issue — it's not only a missing feature but **user-facing false statements**.

## 3. Evidence trail — every layer promises a feature that doesn't exist

| Layer | Claim | File | Reality |
|-------|-------|------|---------|
| Banner user-facing copy | *"You can change this later in Settings."* | `frontend/src/components/consent-banner.tsx:47` | False |
| Banner source-code comment | *"Users can reverse their choice later from Settings."* | `frontend/src/components/consent-banner.tsx:7` | False |
| Privacy policy page | *"You can change your telemetry choice at any time from the Settings page"* | `frontend/src/pages/privacy.tsx:203` | False |
| Privacy policy page | *"Revoke consent for third-party service connections"* (telemetry implied elsewhere) | `frontend/src/pages/privacy.tsx:168` | Telemetry withdrawal: false |
| Internal spec | *"Settings screen — telemetry toggle flip-off: reset() + anon-ID clear + setConsent(false)"* | `docs/TELEMETRY.md:662` | Never built |
| Internal spec | *"Mobile in-app Settings toggle flipping off calls reset()..."* | `docs/TELEMETRY.md:71` | Never built |
| Mobile privacy policy | *"No analytics SDKs are used"* | `mobile/src/features/legal/PrivacyPolicyScreen.tsx:105-108` | False — `posthog-react-native` is a direct dependency (`mobile/package.json:51`) |
| Mobile iOS privacy manifest | `NSPrivacyCollectedDataTypes` = empty array | `mobile/ios/NyxIDMobile/PrivacyInfo.xcprivacy:43-46` | Inconsistent with the app's own privacy screen (`PrivacyPolicyScreen.tsx:35-39`) which documents collection of account identity, push tokens, usage data |
| Web privacy policy | Region: EU | `frontend/src/pages/privacy.tsx:183-188` | Code default is US host `https://us.i.posthog.com` (`frontend/src/lib/telemetry.ts:37-38, 137-140`) |

## 4. Store shape — there is no persisted opt-out, only an "un-answer" reset

Both stores were designed as one-way doors for the banner, not as consent lifecycles:

```
frontend/src/stores/consent-store.ts
  setConsent(enabled)    → { enabled, asked: true }     // persists a choice
  clearConsent()         → { enabled: false, asked: false }  // resets, re-triggers banner

mobile/src/lib/consent.tsx
  setConsent(enabled)    → { enabled, asked: true }
  clearConsent()         → { enabled: false, asked: false }  // same shape
```

Neither `setConsent(false)` nor `clearConsent()` calls PostHog's `opt_out_capturing()` or `reset()`, and `initTelemetry()` in `frontend/src/lib/telemetry.ts:122-124, 190-219` is guarded by a one-way `inited` flag — once initialized, there is no code path that disables the client at runtime. Mobile has the same shape (`mobile/src/lib/telemetry.ts`).

**Implication for the fix:** building the Settings toggle requires first adding a `disableTelemetry()` / `shutdownTelemetry()` function that calls `posthog.opt_out_capturing()` + `posthog.reset()` and is safe to call from a running app. Until that function exists, any Settings toggle wired directly to `setConsent(false)` would silently fail to stop event capture.

## 5. Proof the team knows how to ship consent withdrawal

`frontend/src/pages/consents.tsx` is a **fully functional OAuth-consent revocation UI**: list all consents, per-row revoke button, confirmation modal, `useRevokeConsent` mutation hook, backend endpoint `DELETE /api/v1/users/me/consents/{client_id}`.

The telemetry equivalent was simply never finished. This is not a capability gap — it's an incomplete feature.

## 6. The second structural issue — backend bypasses local consent

Even if the web/mobile Settings toggles existed tomorrow, **backend telemetry would continue firing** for CLI and mobile users:

- `cli/src/api.rs:9-41` — attaches `X-NyxID-Client: cli` + version header whenever `NYXID_TELEMETRY_DSN` or `NYXID_SHARE_ANALYTICS` is set in the environment. **Never checks the user's `nyxid telemetry disable` state.**
- `mobile/src/lib/api/http.ts:500-522` — same pattern; headers driven by build config, not consent.
- `backend/src/handlers/auth.rs:399-416` and `486-535` — backend emits telemetry events (signup, login, logout) tagged with `surface="cli"` / `"mobile"` based purely on those headers.

A user who runs `nyxid telemetry disable` silences the *local* PostHog client but the backend continues to receive surface-tagged attribution and emits its own events. That makes local consent partial theater.

## 7. The third structural issue — audit log survives account deletion

`backend/src/services/admin_user_service.rs:420-444` lists the collections cascaded on account deletion:

```
SESSIONS, REFRESH_TOKENS, API_KEYS, USER_SERVICE_CONNECTIONS,
USER_PROVIDER_TOKENS, MFA_FACTORS, AUTHORIZATION_CODES, OAUTH_STATES,
CONSENTS, MCP_SESSIONS, APPROVAL_REQUESTS, APPROVAL_GRANTS,
SERVICE_APPROVAL_CONFIGS, NOTIFICATION_CHANNELS
```

`AUDIT_LOG` is not in the list. `backend/src/db.rs:266-288` shows no TTL index on audit_log either. Deleted users' IP, user-agent, and event-data rows persist indefinitely. This is an Article 17 (right to erasure) issue, distinct from the Article 7(3) consent issue above.

---

## 8. Fix checklist

Grouped by whether the item is load-bearing for Article 7(3) / Article 17 compliance (P0) versus customary best-practice (P1) versus nice-to-have (P2). Done criterion is verifiable from code, not from docs.

### 8.0 Progress — what's landed per PR

| Item | Status | Landed in |
|---|---|---|
| §8.1 Web consent withdrawal | ✅ Done | PR #485 |
| §8.2 Mobile consent withdrawal | ⬜ Pending | Next: mobile PR |
| §8.3 CLI portion (header gating) | ✅ Done | PR #485 |
| §8.3 Mobile portion (header gating) | ⬜ Pending | Next: mobile PR |
| §8.3 Backend integration test | ⬜ Pending | Backend PR |
| §8.4 `AUDIT_LOG` cascade + TTL | ⬜ Pending | Backend PR |
| §8.5 Web + CLI copy fixes | ✅ Done | PR #485 |
| §8.5 EU/US host decision (kept US + TODO for SCCs) | ✅ Done | PR #485 |
| §8.6 Age gate | ⬜ Pending | Age gate PR (FE + backend) |
| §8.7 Operator guidance | ⬜ Pending | Backend PR |
| §8.8 Mobile P1 contradictions | ⬜ Pending | Mobile PR |
| §8.9 `DO_NOT_TRACK` (CLI) | ✅ Done | PR #485 |
| §8.9 GPC (web) | ⬜ Pending | P1 hygiene follow-up |
| §8.9 ToS checkbox (web) | ⬜ Pending | P1 hygiene follow-up |
| §8.9 DSAR export endpoint | ⬜ Pending | P1 hygiene follow-up |
| §8.9 Node `last_error` sanitization | ⬜ Pending | Backend PR |
| §8.9 Browser DNT honoring (bonus — §10) | ✅ Done | PR #485 |
| §9.3 follow-ups (surfaced during PR #485) | ⬜ Pending | Various |

Update this table as each PR lands.

### P0 — Blocking for EU launch

#### 8.1 Wire consent withdrawal on web
- [ ] **Prereq** — add a `disableTelemetry()` function to `frontend/src/lib/telemetry.ts` that calls `posthog.opt_out_capturing()` + `posthog.reset()` and resets the module-level `inited` flag so a later re-enable can call `init()` again. (Today `initTelemetry()` is one-way; there is no runtime disable path.)
- [ ] Add a matching `enableTelemetry()` entry point if current `initTelemetry()` isn't reusable, so a user turning the toggle back on gets a fresh PostHog session
- [ ] Add a **Privacy & Telemetry** section to `frontend/src/pages/settings.tsx` (could be a 5th tab, or inside Security — team call)
- [ ] Section reads current state from `useConsentStore` (`enabled` + `asked`)
- [ ] Toggle ON → `setConsent(true)` + `enableTelemetry()` → user sees events begin to fire on navigation
- [ ] Toggle OFF → `setConsent(false)` + `disableTelemetry()` → no further events captured even while the tab stays open; verify `isTelemetryActive()` returns false
- [ ] **Copy — per-device clarity.** Update banner copy in `consent-banner.tsx:47` and Settings toggle helper text to state explicitly that the choice applies to **this browser only**. Proposed banner replacement: *"We collect anonymous usage telemetry to help us improve NyxID. We never capture credentials, form content, or the contents of your requests. This choice applies to this browser only — other devices and the CLI manage their own telemetry settings. You can change this later in Settings."* Settings helper text should repeat the per-browser scope.
- [ ] Done criterion: a user who accepts the banner can fully withdraw without opening DevTools AND without a page reload, AND banner/settings copy makes the per-browser scope explicit

#### 8.2 Wire consent withdrawal on mobile
- [ ] **Prereq** — add `disableTelemetry()` / `enableTelemetry()` in `mobile/src/lib/telemetry.ts` that call PostHog RN's `optOut()` / `optIn()` + `reset()`. PostHog RN's `reset()` handles its own internal distinct-ID cleanup, so NyxID code does not need to manipulate AsyncStorage keys it does not own. Today `setConsent(false)` only mutates our own consent AsyncStorage record; no lifecycle hook in `AuthSessionContext.tsx:99-113` calls `reset()` on a running app.
- [ ] Add a **Privacy & Telemetry** row to `mobile/src/features/account/AccountSettingsScreen.tsx`
- [ ] Row reads from `useMobileConsent()` — the hook returns flat fields (`enabled`, `asked`, `isHydrated`, `setConsent`, `clearConsent`), not a nested `.state` object. See `mobile/src/lib/consent.tsx:35-40, 109-117`
- [ ] Toggle ON/OFF calls `useMobileConsent().setConsent(bool)` **and** the new `enableTelemetry()` / `disableTelemetry()`
- [ ] Decision: whether to also add a first-run prompt on mobile (recommended: yes — matches CLI pattern, avoids silent default), or keep default-off and require user to find the toggle
- [ ] **Copy — per-install clarity.** First-run prompt copy (if added) and Settings row helper text must state that the choice applies to **this install only** — signing in on another device or reinstalling the app does not inherit the choice. Mirrors web §8.1 language but uses "install" rather than "device" because mobile reinstall resets AsyncStorage. Example: *"This choice applies to this install only — reinstalling, or using NyxID on another device, starts fresh. The web dashboard and CLI manage their own telemetry settings."*
- [ ] Done criterion: a mobile user can read their current telemetry state and flip it from Settings; after flipping off, no PostHog network calls fire even while the app stays in the foreground; copy explicitly scopes the choice to "this install"

#### 8.3 Gate CLI/mobile `X-NyxID-Client` headers on consent
- [ ] `cli/src/api.rs:build_cli_http_client` — replace the `telemetry_configured` DSN-presence check with a check against `telemetry::consent::resolve_consent()` (or equivalent); headers attach only when the *user* has opted in, not just when the *operator* has set a DSN
- [ ] `mobile/src/lib/api/http.ts:500-522` — `http.ts` is not a React component and cannot call `useMobileConsent()`. Expose a non-hook read path: either a module-level `getConsentSnapshot()` that reads AsyncStorage on init + subscribes to changes, or a small Zustand/event-bus mirror. Header attaches only when that snapshot says `enabled === true`
- [ ] Backend: add a lightweight integration test that hits `/api/v1/auth/register` and `/api/v1/auth/login` **without** the header and asserts `TelemetryEvent::UserSignedUp` / `AuthLoggedIn` are **not** emitted (backend attribution today fires on `surface` tags from these headers at `backend/src/handlers/auth.rs:399-416`, `:486-535` — these are pre-auth flows, so the scope is broader than "authenticated requests")
- [ ] Done criterion: a user who has declined telemetry produces zero `X-NyxID-Client` / `X-NyxID-Client-Version` headers on any request (authenticated **or** pre-auth signup/login), verified by inspecting network traffic AND by the backend integration test above

#### 8.4 Add AUDIT_LOG to account-deletion cascade
- [ ] Add `AUDIT_LOG` to the `user_scoped_collections` array in `backend/src/services/admin_user_service.rs:423-438`
- [ ] Decide retention policy for non-deleted users: TTL index on `audit_log.created_at` (e.g. 180 or 365 days) in `backend/src/db.rs:ensure_indexes`
- [ ] Align retention language in `frontend/src/pages/privacy.tsx` with the actual TTL chosen
- [ ] Decide whether to wrap the cascade in a Mongo transaction. Current code is a sequential loop of `delete_many` calls with no transactional guarantee — a crash mid-cascade leaves partial data. If a transaction is out of scope, document the cascade-order invariant (e.g. delete `audit_log` first so the user row is the last thing removed).
- [ ] Done criterion: after `delete_current_user_cascade` returns successfully, zero `audit_log` rows remain for that `user_id`, verified by integration test

#### 8.5 Resolve P0 user-facing contradictions tied to consent validity
- [ ] Banner copy line 47 + doc comment line 7 (`consent-banner.tsx`): remove "change in Settings" phrasing OR (preferred) leave it and complete 8.1
- [ ] Privacy policy line 203 (`privacy.tsx`): same — leave the sentence if 8.1 completes; otherwise rewrite
- [ ] Privacy policy line 183-188 PostHog EU claim: either change `lib/telemetry.ts` default host to `eu.i.posthog.com` (also applies to `mobile/src/lib/telemetry.ts:69-70, 86-89`), or rewrite the policy sentence to reflect the US default. (If the policy is rewritten to US, the "International transfer mechanism" item in §9.2 becomes **in-scope / not deferrable** — a lawful basis for EU→US transfer must be documented.)
- [ ] **Privacy policy — per-device scope of consent.** Add a sentence to both `frontend/src/pages/privacy.tsx` §8 (Cookies, Local Storage, and Analytics) and `mobile/src/features/legal/PrivacyPolicyScreen.tsx` stating explicitly: *"Your telemetry choice is stored per device / per install and does not sync across the web dashboard, mobile app, and CLI. Each surface manages its own telemetry setting."* This avoids the UX footgun where a user opts out in one place and assumes it applies everywhere.
- [ ] **CLI first-run prompt — match language.** Update `cli/src/telemetry/consent.rs:139-170` disclosure to include the same per-device framing for consistency: *"This choice applies to this machine only — the web dashboard and mobile app manage their own telemetry settings."* The existing line *"You can change this later with `nyxid telemetry enable|disable`"* stays.
- [ ] Done criterion: no policy/banner copy makes a withdrawal claim or regional claim that the code contradicts, AND every surface's consent-setting UI (web banner, web Settings, mobile prompt/Settings, CLI first-run) clearly states the choice is per-device. Verified by reading the relevant screens, not just grep (grep catches literals, not paraphrases or future locales).

#### 8.6 Enforce age gate at registration

Policies already restrict signup to users 13+ (web) and 16+ (mobile). Code has no enforcement — a 10-year-old can register today, which makes the policy a false statement and invites COPPA exposure in the US plus GDPR-K exposure in the EU.

**Scope decision upfront** — *checkbox affirmation, not birthdate.* Birthdate is a data-minimization step backwards (collects DOB we don't otherwise need); checkbox is the lighter-weight default chosen by most auth/SSO peers (Clerk, Auth0, Supabase). If legal later requires jurisdiction-specific logic (e.g. parental consent <16 in EU), that's a follow-up, not v1.

**Age bar decision upfront** — *16+ single global bar.* This is the highest age claimed by any current policy (mobile under-16). Applying it to both surfaces is simpler than per-surface logic, conservatively covers GDPR-K (which varies between 13 and 16 by EU member state), and requires updating web policy wording (`privacy.tsx:208-214`) from "13+" to "16+" to stay internally consistent.

- [ ] Add `accepts_age_requirement: z.literal(true)` (or similar) to `frontend/src/schemas/auth.ts` registration schema (`frontend/src/schemas/auth.ts:10-38`)
- [ ] Add matching checkbox input in `frontend/src/components/auth/auth-flow.tsx:255-262` with copy: *"I confirm I am at least 16 years old"*
- [ ] Match on mobile: add the same checkbox to the signup screen (`mobile/src/features/auth/AuthHomeScreen.tsx:428-439` currently has only passive browsewrap text; the schema that feeds its submit call needs the same `accepts_age_requirement` field)
- [ ] Backend: add a `accepts_age_requirement: bool` field to the register handler input (`backend/src/handlers/auth.rs` register path), reject registration with 400 if missing or false. Do not trust client-only validation.
- [ ] Update web privacy policy (`frontend/src/pages/privacy.tsx:208-214`) to say "16+" to match mobile and the new checkbox
- [ ] Done criterion: attempting to register without `accepts_age_requirement=true` returns 400 from the backend, verified by integration test

#### 8.7 Ship self-hosted operator guidance

NyxID is self-hostable. Any operator flipping telemetry on via `backend/src/config.rs:81-89` or via the `/public/config` endpoint (handler at `backend/src/handlers/health.rs:67, 118`, route registered at `backend/src/routes.rs:774`) inherits every obligation below, on behalf of *their* users. Without a guidance doc, operators ship a telemetry pipeline into production without the compliance paperwork that pipeline assumes. The fix here is documentation + a runtime check, not a feature.

- [ ] Add a new section to `docs/TELEMETRY.md` (or a sibling `docs/TELEMETRY_OPERATORS.md`) titled "Operator obligations when enabling telemetry." Must cover: publishing their own privacy notice, disclosing PostHog as a processor, documenting EU→US transfer basis if using US host, setting a retention window in their PostHog project, honoring DSAR/delete requests routed through their deployment.
- [ ] Add a startup-time warning in `backend/src/main.rs` (or `telemetry/mod.rs`) whenever **either** `NYXID_TELEMETRY_DSN` is set **or** `NYXID_SHARE_ANALYTICS=true`: log a single WARN-level line pointing to the operator doc, e.g. *"Telemetry enabled. If you are operating NyxID on behalf of third-party users, review docs/TELEMETRY_OPERATORS.md for your disclosure and processor obligations."*
- [ ] Add operator-identity env vars + `AppConfig` fields (`backend/src/config.rs`): `NYXID_OPERATOR_NAME` (string), `NYXID_OPERATOR_PRIVACY_URL` (string, URL), `NYXID_OPERATOR_CONTACT` (string, email or URL for DSAR/privacy inquiries). All three optional at parse time; see next bullet for runtime gating.
- [ ] Extend `PublicConfigResponse` in `backend/src/handlers/health.rs:38-58` with `operator_name: Option<String>`, `operator_privacy_url: Option<String>`, `operator_contact: Option<String>`. `public_config` handler (`backend/src/handlers/health.rs:67-128`) copies them from `AppConfig`
- [ ] Extend frontend `PublicConfigResponse` type + `usePublicConfig` hook to consume those fields
- [ ] **Runtime policy** — pick one of two stances (team decision, document which):
  - *Strict (recommended):* when `telemetry_dsn` is set but operator-identity fields are not, **do not enable telemetry** — the banner does not render, `initTelemetry()` does not run, and the server logs a hard ERROR line. Forces operator compliance before any data is captured.
  - *Soft:* telemetry enables, but the banner shows a placeholder saying "This instance has telemetry enabled but its operator has not published a privacy policy." Allows development and preview deployments to keep working without operator setup.
- [ ] Done criterion: an operator who sets `NYXID_TELEMETRY_DSN=xxx` **without** configuring operator-identity fields sees the server log line AND the chosen runtime behavior (hard-block or placeholder) verified manually

### P1 — Customary / peer-standard, not legally blocking

#### 8.8 P1 user-facing contradictions (previously bundled into 8.5)

- [ ] Mobile privacy policy line 105-108 ("no analytics SDKs are used"): either delete the sentence OR remove `posthog-react-native` from `mobile/package.json` and `mobile/src/lib/telemetry.ts`. Less urgent than §8.5 because it doesn't touch consent validity, but it's a factual lie in a legal document — fix before any broader launch.
- [ ] iOS `PrivacyInfo.xcprivacy` lines 43-46: either declare the actual `NSPrivacyCollectedDataTypes` that match the app's privacy screen, or tighten the privacy screen to match the manifest. Apple App Review flags this class of inconsistency on submission.

#### 8.9 Customary developer-tool telemetry hygiene

- [ ] Honor `DO_NOT_TRACK=1` env var in `cli/src/telemetry/consent.rs:78-99` alongside `NYXID_TELEMETRY` (convention used by Homebrew, Netlify, GitHub CLI)
- [ ] Honor `navigator.globalPrivacyControl` in `frontend/src/lib/telemetry.ts:154-160` alongside the existing `respect_dnt: true` (CPRA requirement)
- [ ] Add ToS + Privacy acknowledgement checkbox at web registration (`frontend/src/components/auth/auth-flow.tsx:255-262`, schema in `frontend/src/schemas/auth.ts:10-38`). Today the registration form links Privacy only, no ToS page exists
- [ ] Ship a DSAR / data-export endpoint under `/api/v1/users/me/export` returning a ZIP or JSON of user_id-keyed rows from `audit_log`, `sessions`, `api_keys`, etc. OR remove the access-rights language from both web `privacy.tsx:162-174` and mobile `PrivacyPolicyScreen.tsx:94-102`
- [ ] Sanitize node `last_error` strings before storage in `backend/src/services/node_metrics_service.rs:43-68` — currently stores raw (truncated) upstream error text, which can contain downstream secrets if upstream includes them

### P2 — Nice to have

- [ ] User-visible audit log for the user's own rows (not admin-only at `/api/v1/admin/audit-log`)
- [ ] Node-registration-time opt-in for node metrics (today metrics collect silently; matches Homebrew's "show notice before first send" pattern)
- [ ] Granular telemetry tiers à la VS Code's `telemetry.telemetryLevel` — all / errors / off — instead of binary

---

## 9. Implementation docs — leftover / deferred

Two buckets here. Already-working systems that don't need changes, and known compliance gaps that are real but belong in a separate workstream (tracked for visibility so they don't fall off the floor).

### 9.1 Working systems, left as-is

- PostHog egress scrubber (`backend/src/telemetry/scrub.rs`) is good — keep
- Durable erasure queue (`backend/src/services/telemetry_erasure_service.rs`) works — keep
- Backend PostHog hard-off default (`backend/src/config.rs:81-89`) — keep
- OAuth consent revocation (`frontend/src/pages/consents.tsx`, `/api/v1/users/me/consents/{client_id}`) — separate system, works correctly

### 9.2 Deferred to a separate compliance/operations doc

These surfaced during the audit but don't belong in a "telemetry consent fix" checklist. Suggested home: a future `docs/PRIVACY_OPERATIONS.md` that sits alongside `docs/TELEMETRY.md` and `docs/TELEMETRY_CONSENT_FIX.md` (this doc). None are trivial. All are real. Each one has a reason it's parked, not skipped.

- **Consent receipts / Art. 7(1) accountability.** No server-side record of who consented, when, to which policy version, on which surface. Today consent is localStorage/AsyncStorage only — if a regulator asks "prove user A opted in on date B," we can't produce evidence. *Parked, not skipped:* not in the same PR as the toggle work above, but **actively tracked as a blocker before any enterprise contract or public EU launch** — not a "we'll get to it." Requires new backend schema + migration + policy-versioning infra, which is why it doesn't fit in this checklist. Someone should open a follow-up ticket before this doc is closed.
- **International transfer mechanism (SCCs / adequacy disclosure).** EU→US data transfer to US-hosted PostHog requires a documented legal basis (SCCs 2021/914 or adequacy). *Conditional on §8.5:* if §8.5 chooses "move to EU host," this item stays deferred. If §8.5 chooses "keep US host, rewrite policy," this item **becomes in-scope** and must be closed before the checklist is done — the pointer from §8.5 already states this. In that case, the work is legal paperwork (not engineering), but it's not skippable.
- **Sub-processor / processor list.** Both privacy policies list data *categories* but not the actual *processors* (PostHog, SMTP vendor, APNs/FCM, Telegram). GDPR Art. 28 and customer DPAs expect a named, versioned sub-processor list. *Why parked:* docs/legal work.
- **Retention TTLs for non-audit-log collections.** §8.4 sets retention for `audit_log`. Sessions already expire via token TTLs; refresh tokens decay. Node metrics live inside the `Node` document and soft-delete is handled at `backend/src/services/node_service.rs:262-287`. PostHog retention is an operator setting on the PostHog project, not a NyxID code change. *Why parked:* most of this is either already handled or operator-owned — no clean engineering unit-of-work.
- **Breach notification procedure (Art. 33/34).** 72-hour notification playbook, contact list, template. *Why parked:* pure ops/legal process, no code.

The two items that *did* move from "nice to have later" into the active checklist (age gate §8.6, operator guidance §8.7) are the ones that are either already-lied-about in policy (age gate) or unique to NyxID's self-hosted positioning (operator guidance) — neither fits in a generic compliance-readiness doc.

### 9.3 Follow-ups surfaced during implementation (PR #485 — web + CLI consent withdrawal)

These didn't exist as checklist items before PR #485 started; codex review of the actual diff identified them and we chose to defer rather than expand scope. All three are narrow enough to belong in a follow-up; all three are documented here so they don't fall off the floor.

- **`nyxid telemetry disable` anon_id scope mismatch.** v1 treats consent as user-global (read + edited against default profile), but `TelemetryClient::init(profile)` still pulls the anon distinct_id from `~/.nyxid/profiles/<name>/anon_id`. `nyxid telemetry disable` only deletes the default-profile anon_id file, so a user who disables telemetry, later re-enables, and then runs `--profile <name>` resumes the old anonymous identity instead of getting the documented "fresh trail." *Severity:* narrow (only matters for named-profile users who toggle telemetry), documentation/contract mismatch rather than consent violation. *Fix:* either globalize anon_id too, or have `nyxid telemetry disable` delete anon_id files across all profiles.
- **Privacy policy region hardcoded vs operator-overridable `telemetry_host`.** `frontend/src/pages/privacy.tsx:187` states "PostHog, US region," which is correct for NyxID Inc's deploy but factually wrong for any self-hosted or EU-hosted operator who sets `telemetry_host` to `eu.i.posthog.com` or their own instance. *Severity:* tied into §8.7 operator guidance; operators are expected to publish their own privacy notice once §8.7 ships, which makes NyxID Inc's `privacy.tsx` copy irrelevant for them. *Fix path:* land §8.7 (operator-identity fields in `/public/config` + degraded banner when operator policy missing), then either (a) soften the region claim in NyxID Inc's copy to reference operator setting, or (b) accept that operators own their own policy once §8.7 exists.
- **CI snapshot regression guard for false-claim strings.** §10 specifies a snapshot-on-commit check that fails if any of the known false-claim strings ("change in Settings" pointing to nowhere, "PostHog EU region" when code uses US, "no analytics SDKs" when SDK is bundled) reappear in `privacy.tsx` / `consent-banner.tsx` / `PrivacyPolicyScreen.tsx` / `TELEMETRY.md`. Unit tests in PR #485 cover code-level correctness but don't guard against future copy drift. *Severity:* prevention-only; there's no active regression today. *Fix:* a single vitest file reading the target paths and asserting literals are absent — ~15 min. Recommended to land alongside the mobile PR so both surfaces' copy are guarded at once.

## 10. Verification plan post-fix

Before closing this document:

- [ ] Manual test, web (§8.1): fresh browser → click banner Allow → navigate to Settings → flip toggle off → **do not reload** → assert `isTelemetryActive()` returns false AND no PostHog network traffic fires on subsequent navigation. Reload test is a separate pass for persistence.
- [ ] Manual test, mobile (§8.2): fresh install → first-run prompt (if included) → accept → Settings → flip off → assert `posthog-react-native` stops capturing while app stays in foreground
- [ ] Manual test, CLI (§8.3): `nyxid telemetry disable` → run any command, authenticated OR unauthenticated → inspect request headers → assert no `X-NyxID-Client` / `X-NyxID-Client-Version`
- [ ] Automated test (§8.3): backend integration test hits `/api/v1/auth/register` and `/api/v1/auth/login` without telemetry headers; asserts no `UserSignedUp` / `AuthLoggedIn` telemetry event is emitted
- [ ] Automated test (§8.4): integration test in `backend/src/services/admin_user_service.rs` asserts zero `audit_log` rows remain for a user_id after `delete_current_user_cascade`
- [ ] Copy audit (§8.5, §8.8): manual review of `frontend/src/pages/privacy.tsx`, `frontend/src/components/consent-banner.tsx`, `mobile/src/features/legal/PrivacyPolicyScreen.tsx`, `docs/TELEMETRY.md` — read each claim, verify against current code. Grep pass is a first filter, not proof. Snapshot-on-commit check added to CI for the specific literal strings as a regression guard.
- [ ] Per-device scope copy check (§8.1, §8.2, §8.5): verify every consent-setting surface explicitly scopes the choice to *this browser / this install / this machine*. Surfaces to check: web banner, web Settings toggle helper text, mobile first-run prompt (if added), mobile Settings row helper text, CLI first-run prompt, web privacy policy §8, mobile privacy policy. Any surface missing the per-device scoping fails this check.
- [ ] Automated test (§8.6): backend integration test asserts registration without age-affirmation returns 400
- [ ] Manual test (§8.7): test both activation paths — set `NYXID_TELEMETRY_DSN=xxx` (run once), then set `NYXID_SHARE_ANALYTICS=true` with DSN unset (run again). For each, omit operator-identity fields and assert the WARN log line fires on startup.
- [ ] Manual test (§8.7 outcome): verification depends on whether team chose the strict or soft runtime stance in §8.7:
  - *If strict:* assert the banner does **not** render, `initTelemetry()` does **not** run, no PostHog traffic fires, and startup emits a hard ERROR log line
  - *If soft:* assert the banner renders with the placeholder "operator has not published a privacy policy" copy, and telemetry otherwise functions
- [ ] Legal review sign-off on updated privacy policy language (whichever direction §8.5 chose — code change or copy change)

---

## 11. One-sentence summary

NyxID shipped a consent banner whose own copy, privacy policy, and internal spec all promise a Settings-page withdrawal mechanism that the code does not contain and was never designed to support — leaving EU users unable to withdraw consent as easily as they gave it, which is the test Article 7(3) imposes on Article 6(1)(a)-based processing.
