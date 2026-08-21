# Platform Services V1 — Non-Breaking Rework

*2026-08-21. Branch `travel-allowlist`, head `efc519bd`, base `origin/main` (`45e88998`). Input to
adversarial review, then implementation. Every claim cites the file:line at head unless marked
`origin/main`.*

**Requirement:** the owner must be **sure** PR #1472 introduces no breaking change — by
construction, not by audit. This document (a) confirms and extends the list of breaking changes
in the current head, (b) gives the specific modification that neutralizes each, (c) proves the
result is non-breaking by enumerating every runtime-reachable change and showing it is dormant,
reverted, or write-path-new, and (d) sequences the deliberate enablement of each protection
afterwards.

**Verdict up front:** the PR currently contains **four** breaking changes (the two confirmed
ones, the rate-limiter default, and a fourth not previously identified: a shared server-chosen
rate bucket that throttles the live assistant). All four can be made non-breaking. Two
write-path-only contract changes remain and are stated plainly in §7 — they affect admin API
callers creating new rows, never existing rows or runtime traffic.

---

## Finding summary

| # | Change | Status at head | Who breaks | Fix class |
|---|--------|----------------|-----------|-----------|
| 1 | Overlay trim (twitter/firecrawl/elevenlabs) | **Breaking, worse than known** — hits *existing* BYOK users live, not just fresh installs | BYOK MCP/agent users, Aevatar typed workflows | Revert the three overlays + dependent edits |
| 2 | Actor-addressed absent-policy deny | **Breaking** — `chrono-llm-public` via `/proxy` and `/llm` | Any actor-addressed caller of a policy-less master row | Flag, default off |
| 3 | Per-user limiter default 2 rps / burst 10 | **Breaking if any actor-addressed master traffic exists** (it does, once #2 is dormant) | Same population as #2, plus future platform rows | Default 0 (off) |
| 4 | **NEW:** server-chosen shared bucket, 2 rps / burst 10 per service across *all users* | **Breaking** — throttles live assistant chat | Every assistant direct-chat user | Remove |
| 5 | Credential-shape validation | **Not breaking for existing rows** (create-only, verified §5) | New-create API callers only (see §7) | Keep; contract change stated |
| 6 | Endpoint-sync stops resurrecting deactivated rows | Not breaking (stops a write; §6.3) | Nobody with a plausible workflow | Keep |

---

## 1. Overlay trim — revert; the trim cannot be shipped in any form that spares BYOK

### What breaks, precisely (three channels, one worse than previously understood)

The trimmed spec files are the **shared** overlays. `SLUG_TO_SPEC_KEY`
(`backend/src/services/catalog_spec_registry.rs:125-153`) maps them to the existing BYOK
catalog rows: `api-elevenlabs → elevenlabs` (`:128`), `api-firecrawl → firecrawl` (`:132`),
`api-twitter → twitter` (`:146`). Removed: twitter `create_tweet`, `get_me` (88 lines);
firecrawl `agent`, `agent_status`, `map_site` (184 lines); elevenlabs `list_voices` + the six
`convai` operations (294 lines).

**Channel A — existing BYOK users, immediately on deploy.** When a user provisions a catalog
service, the UserEndpoint **inherits the catalog row's `openapi_spec_url`** — the hosted
overlay URL — by default (`resolve_openapi_spec_url`,
`backend/src/services/unified_key_service.rs:363-376`, applied at `:828-829` with
`OpenApiSpecUrlInput::Inherit`). At MCP catalog build time, an instance-mounted spec **takes
precedence over the template `ServiceEndpoint` rows**
(`backend/src/services/mcp_service.rs:1157-1160`), and the hosted URL serves the **embedded**
overlay content (`catalog_spec_registry.rs`, `include_str!`). So existing users' tool menus are
rebuilt live from the trimmed files at deploy. The previously-assumed mitigation — "existing
databases keep their `ServiceEndpoint` rows" — is real but irrelevant: those rows are shadowed
by the instance spec for every inherit-provisioned user. A user posting to X through
`api-twitter__create_tweet` with **their own credential** loses the tool the moment the deploy
lands. (Concretely: the heca-gateway MCP session used to author this review exposes
`api-twitter__create_tweet` through exactly this path today.)

**Channel B — fresh deployments.** The startup sync seeds template rows from the overlays
(`backend/src/services/catalog_spec_sync.rs:36-69`); trimmed overlays mean new installs never
get the rows at all.

**Channel C — Aevatar typed workflows.** The overlays carry the `x-aevatar-tool` annotations
that `/api/v1/mcp/config` publishes for workflow admission (issue #1290). Existing Aevatar
workflows bound to `agent`/`agent_status`/`map_site`/`convai` operations lose their operations.
The firecrawl agent operations were annotated for Aevatar **on purpose** — the seeded
`known_limitations` text at `origin/main` says so, and head had to rewrite that text
(`backend/src/services/provider_service.rs:2569`) plus two test suites
(`backend/src/handlers/docs.rs:402-427`, `backend/src/services/api_docs_service.rs:1086-1106`)
to make the trim pass CI. Test rewrites of that shape are the diff telling us it changed a
shipped contract.

The ElevenLabs `convai` removal additionally deletes the shipped two-way agent-phone-call
capability (Conversational AI) from every BYOK user — a marketed capability, not dead weight.

### Why "trim" is unfixable rather than tunable

The overlay is a shared artifact with two consumers whose requirements now diverge: BYOK rows
must keep the full operation set (user's own key, user's own blast radius — restricting it
protects nobody), and platform rows must expose a reduced set. No edit to a shared file can
satisfy both. The restriction has to live on the platform row.

### The fix

1. **Restore the three files byte-for-byte** from `origin/main`:
   `git checkout origin/main -- backend/specs/catalog/twitter.openapi.json
   backend/specs/catalog/firecrawl.openapi.json backend/specs/catalog/elevenlabs.openapi.json`.
2. **Revert the edits that exist only to make the trim pass:**
   - `backend/src/handlers/docs.rs:402-427` (restore the `/v2/agent` assertions),
   - `backend/src/services/api_docs_service.rs:1086-1106` (restore
     `firecrawl_overlay_declares_aevatar_agent_operations`),
   - `backend/src/services/provider_service.rs:2569` (restore the firecrawl
     `known_limitations` text),
   - delete `trimmed_overlays_exclude_retired_operations`
     (`backend/src/services/catalog_spec_sync.rs:522-566`).
3. **Keep** `additive_sync_preserves_operator_deactivation`
   (`catalog_spec_sync.rs:452-520`) and all of §6.3 — that change is independent of the trim
   and non-breaking.

### Assessment of the proposed `platform-twitter` spec-key direction

Valid, and the right long-term shape **when a platform row needs spec-driven typed tools** —
add reduced overlays under new keys (`platform-twitter`, …), new `SLUG_TO_SPEC_KEY` entries for
the new platform slugs, leave BYOK keys untouched. But v1 should not ship it, because **v1 has
no consumer for it**: the platform X row's menu is hand-curated — one `ServiceEndpoint` row
created by the activation runbook (`V1_SPEC.md`, runbook step 3), published directly by MCP
platform publication (`mcp_service.rs:1241-1260`), with no `openapi_spec_url` mounted (runbook
step 2 verifies exactly this). A platform overlay today would be an unreferenced file that the
weekly drift guard (`.github/workflows/catalog-spec-drift.yml`) checks forever for nothing.
Menu-equals-policy is already guaranteed by construction for the X row. **Recommendation:
revert only; introduce `platform-*` spec keys in the release that first mounts one.** Both
directions converge; this is strictly less to review now.

---

## 2. Actor-addressed absent-policy deny — ship dormant behind a default-off flag

### What breaks at head

`validate_actor_addressed_master_credential_policy`
(`backend/src/services/proxy_service.rs:260-275`) runs unconditionally inside
`authorize_master_credential` (`proxy_service.rs:154`), which gates every actor-addressed
master-credential resolution: legacy catalog (`proxy_service.rs:1051-1060`), lenient/node
(`:1191-1200`), auto-provisioned UserService — the `/proxy`, `/llm`, MCP, POC funnel
(`:2362-2371`). `chrono-llm-public` is a master-credential row with no policy: on deploy,
`/proxy/s/chrono-llm-public` and `/llm/*` traffic to it denies with a not-found-shaped error.
That row is live, token-metered on the proxy path (`platform_metric_for_target` +
`should_capture_llm_usage`, `backend/src/handlers/proxy.rs:4108-4134`, pinned by the test at
`:6826` passing `"chrono-llm-public"` with `BillingMetric::Tokens`), and the owner has said it
must not be touched until usage is ascertained. The server-chosen (assistant) path is already
safe — `efc519bd` restored passthrough there (`proxy_service.rs:232-239`), and with policy
`None` the rule matcher at `handlers/proxy.rs:2021` also skips, so AdminManaged traffic is
doubly unaffected.

### Flag vs. grandfathering by row — flag wins

- **Grandfathering** (stamp rows existing at deploy as exempt, or cut over by `created_at`)
  bakes a deploy-time migration whose result differs per environment, leaves the dangerous
  legacy rows permanently exempt until someone runs the same audit anyway — now with per-row
  bookkeeping — and gives the operator no single place to see whether enforcement is on.
- **A flag** is one visible switch, no migration, instantly reversible, identical semantics in
  every environment. Its one weakness — new policy-less rows are unprotected while the flag is
  off — is closed *structurally* by the create-time requirement below, which is stronger than
  grandfathering's temporal cutoff because it can never rot.

### The fix

1. **Config:** `PLATFORM_POLICY_FAIL_CLOSED` (bool, **default `false`**) on `AppConfig`
   (`backend/src/config.rs`, beside the broker flags). Document in `docs/ENV.md` and the
   CLAUDE.md env block with the same "default-off hardening, runtime-overridable later" framing
   as `BROKER_REQUIRE_SENDER_CONSTRAINT`.
2. **Wiring:** a module `static PLATFORM_POLICY_FAIL_CLOSED: AtomicBool` beside the limiter
   statics in `backend/src/mw/rate_limit.rs` (or a small `mw/platform_enforcement.rs`), set
   once from `main.rs` next to `init_platform_user_rate_limiter` (`backend/src/main.rs:652`).
   `authorize_master_credential` reads it and passes it to the validator.
3. **Validator becomes pure and parameterized:**
   `validate_actor_addressed_master_credential_policy(service, fail_closed: bool)` — returns
   `Ok(())` immediately when `fail_closed` is false. With the flag off, the function is a no-op
   and `authorize_master_credential` is behaviourally identical to `origin/main`.
4. **Create-time requirement (structural companion, always on):** in `create_service`
   (`backend/src/handlers/services.rs:738`), when the row stores a master credential
   (`!credential.is_empty() && auth_method != "oidc" && auth_method != "none"` and
   `service_category == "internal"`), require `proxy_operation_policy` to be **present** in the
   same request (empty `rules` is acceptable — it means "actor traffic denied at the operation
   layer, server-chosen passthrough", the correct configuration for an assistant-only row).
   This makes a live-but-unrestricted platform row **impossible to create** regardless of the
   flag, which is the actual safety goal; the runtime flag only covers rows that predate the
   rule. This is a write-path contract change — see §7.
5. **Test rework** (the shipped tests assert always-on denial and will fail once the default is
   off):
   - `master_credential_without_policy_is_server_chosen_only`
     (`proxy_service.rs:3643-3662`): split — the DB test asserts flag-off behaviour
     (both gates resolve, i.e. parity with `origin/main`); a new pure unit test
     `actor_policy_validator_denies_when_fail_closed` asserts
     `validate_actor_addressed_master_credential_policy(&service, true)` returns the
     not-found-shaped error and `(…, false)` returns `Ok`.
   - Tests must **never set the global** (process-wide, parallel tests would poison each
     other); all on-behaviour goes through the pure validator. Same rule as the limiter's
     OnceLock (`master_credential_authorization_is_independent_of_limiter`,
     `proxy_service.rs:3815-3831`, already follows it).
   - The policy additions to unrelated fixtures (`proxy_service.rs:3744`, `:3893`;
     `handlers/proxy.rs:8364-8370`) are harmless under flag-off and **stay** — they also keep
     those fixtures valid for the eventual flag-on world.

With the flag off, no existing row changes behaviour; flipping it on later is one config change
(Phase B makes it a runtime toggle, §9.1), reversible without rollback.

---

## 3. Per-user rate limiter — default must be 0 (off)

### What it applies to, and what breaks at head

The limiter runs at the three actor-addressed master-credential resolution sites
(`proxy_service.rs:1056`, `:1196`, `:2367`), keyed `{service_id}:{user_id}`
(`backend/src/mw/rate_limit.rs:295-301`), defaults **2 rps sustained / burst 10**
(`backend/src/config.rs:868-875`), enabled unconditionally at startup
(`backend/src/main.rs:652-655`; `init_platform_user_rate_limiter`, `rate_limit.rs:321-326`,
disabled only when the sustained rate is 0).

It therefore throttles **exactly the traffic §2 preserves**: with the deny flagged off,
actor-addressed `chrono-llm-public` traffic resumes — and any user exceeding 2 rps sustained or
10 in-flight-burst on `/llm` or `/proxy/s/chrono-llm-public` starts receiving 429s. LLM callers
are the worst case for this shape: agents fan out parallel completions (the codex CLI runs
against the chrono provider today), and a burst of >10 requests in one second per user is
routine for tool-calling loops. Whether any live user currently exceeds 2/10 **cannot be
established without the production audit the owner has ruled out as a prerequisite** — so under
the "sure, by construction" bar the default itself is the breaking change.

### The fix

- `platform_service_rate_limit_per_second` default **`2` → `0`** in
  `backend/src/config.rs:868-872` (burst default may stay 10; it is inert while the rate is 0,
  and `init_platform_user_rate_limiter` already returns early on 0 leaving the `OnceLock`
  unset, so every enforcement call is a no-op — `rate_limit.rs:321-326`, `:340-353`).
- Update the doc rows that currently state default 2: `docs/ENV.md:175-176`, `CLAUDE.md`
  env block (the two `PLATFORM_SERVICE_RATE_LIMIT_*` lines).
- Test-config literals (`test_utils.rs:440-441` and the five struct literals in
  `config.rs`/`crypto/*`/`social_auth_service.rs`/`channel_relay_service.rs`) are inert
  fixtures and may keep any value; the unit tests construct `PlatformUserRateLimiter`
  explicitly and are unaffected.
- Keep the limiter code, the seam (`enforce_platform_user_limit`), and its unit tests
  (`rate_limit.rs:1058-1098`) — the machinery is sound; only the default was wrong. Note the
  known OnceLock property: turning the limiter **off** again after an env flip-on requires a
  restart. Phase B (§9.1) replaces the one-shot init with runtime-mutable atomics so both
  directions are instant; that refactor is not needed to make this PR non-breaking.

---

## 4. NEW FINDING — the server-chosen shared bucket must be removed

Not in the owner's list, found in the diff sweep: `resolve_admin_proxy_target` now calls
`enforce_platform_server_chosen_limit` (`proxy_service.rs:925-930`), which shares **one**
token bucket per service across **all callers** — key `{service_id}:server-chosen`, same 2/10
limits (`rate_limit.rs:304-311`, `:357-370`).

`resolve_admin_proxy_target` backs `TargetMode::AdminManaged`
(`backend/src/handlers/proxy.rs:1940-1945`) — the **live assistant surface**
(`backend/src/handlers/assistant.rs:693` → `execute_admin_proxy`). At head, every assistant
direct-chat turn across the entire user base draws from a single 10-token bucket refilling at
2/s per service. Two users chatting concurrently — or one user whose turn issues a few upstream
calls — produce 429s platform-wide. This is a production brownout switch shipped enabled.

It is also the wrong shape even at a correct limit: a service-wide shared bucket lets one user
starve everyone — the precise inverse of item 5's purpose ("one session must not exhaust the
demo for everybody"). V1_SPEC deliberately scoped item 5 to actor paths and excluded
server-chosen ("no actor at that gate; the assistant surface has its own controls").

**Fix: delete it.** Remove the call at `proxy_service.rs:925-930`, the
`check_server_chosen` method (`rate_limit.rs:304-311`), `enforce_platform_server_chosen_limit`
(`rate_limit.rs:355-370`), and their two tests
(`platform_server_chosen_bucket_is_service_scoped`,
`enforce_platform_server_chosen_limit_maps_to_rate_limited`, `rate_limit.rs:1069-1077`,
`:1089-1098`). If a server-chosen backstop is wanted later, the correct design is per-actor at
the assistant layer, which knows the user identity the resolver deliberately does not — queued
as follow-up §9.6, not rescued by a config default here.

---

## 5. Credential-shape validation — verified create-only; existing rows stay editable

Checked as instructed; **not breaking for any existing row**:

- The validator `validate_master_credential_shape` (`backend/src/handlers/services.rs:521-551`)
  has exactly one call site: `create_service` (`services.rs:929-934`). `update_service`
  (`services.rs:1349`) is untouched by the diff, and `UpdateServiceRequest`
  (`services.rs:248-310`) carries **none** of the validator's inputs — no `credential`,
  `auth_method`, `service_category`, `requires_user_credential`, or `provider_config_id`
  fields — so no edit to `chrono-llm-public`, the aevatar row, an SSH row, or any seeded
  provider-linked row can ever reach the new rejection. Admins can keep saving every existing
  non-conforming row: is_active toggles, visibility, policy attachment, metadata — all flow
  through `update_service` unvalidated by the new rule.
- Seeding bypasses the handler entirely (`provider_service.rs` writes documents directly), so
  provider-linked seeds (`api-twitter` with `provider_config_id` + encrypted-empty credential)
  are unaffected on fresh installs and restarts.
- A useful side effect of the reorder: `service_category` is now derived **before** OIDC client
  creation (`services.rs:926-934`, previously after), so an invalid category no longer leaves
  an orphaned OAuth client behind. Behaviour on the success path is identical.

What *is* new: three create-request shapes that previously returned 201 now return 400
(`connection` category + non-empty credential; `auth_method "none"` + non-empty credential;
provider-linked + credential, which is unreachable from the handler today). No shipped client
can produce them — the frontend create-service schema has no credential field at all
(`frontend/src/schemas/services.ts:148-170`) and the CLI has no admin service-create command
(user-facing `service add` targets `/keys`) — so the population is direct API automation only.
This is the write-path contract change stated in §7; if the owner's bar covers hand-written
admin scripts too, the two previously-possible rejections can ride the §2 flag, but the
recommendation is to keep them unconditional: both shapes stored an inert, never-injected
secret, and accepting a pasted credential that will never be used is itself the hazard item 3
exists to close.

---

## 6. Full-diff sweep — every remaining change, classified

Base `45e88998` → head `efc519bd`, 32 files. Everything not covered by §§1–5:

**6.1 Shared shape validation refactor** (`proxy_service.rs:244-258`,
`validate_master_credential_service`; server-chosen reordered to visibility-then-shape,
`:223-231`): both orderings return the identical not-found-shaped error for every input
combination; `authorize_master_credential_server_chosen` accepts exactly the same set of rows
as at `origin/main`. Non-breaking. The expanded server-chosen test
(`proxy_service.rs:3599-3630`) pins it.

**6.2 Rate-limiter plumbing** (`config.rs:96-101`, `:868-875`; `main.rs:652-655`, `:826`;
`rate_limit.rs:277-353`; the seven test-fixture literals): dormant once §3 sets the default to
0 — the `OnceLock` never initializes and every enforcement call is `None → Ok`. Non-breaking.

**6.3 Endpoint sync respects operator deactivation**
(`service_endpoint_service.rs:46-52` `EndpointSyncActivation`; `:281-292` bulk = `ForceActive`;
`:328-341` additive = `PreserveExisting`; `:360-366` and `:432-438` the two activation sites):
the admin reconcile path is byte-identical in behaviour (`ForceActive` reproduces the old
`is_active: true` write, `bulk_upsert_reactivates_on_admin_reconcile` pins it). The only
behavioural delta: the **startup** sync no longer flips an operator's `is_active: false` back
to true on boot. That is the removal of an unwanted write — the resurrection was documented as
a defect in both plan documents — and the only party who could notice is an operator who used
"deactivate + reboot" as a *reactivation* workflow, for which a first-class admin endpoint
exists (`PUT /services/{id}/endpoints/{endpoint_id}`, `is_active: true`,
`backend/src/handlers/endpoints.rs:50`). Keep. Recorded in §7 for completeness.

**6.4 Model doc-comment** (`downstream_service.rs:351-356`): comment only. After §2 the wording
must gain "when platform policy enforcement is enabled" — include in the §8 change list.

**6.5 Test-only and docs-only changes:** proxy fixture policies
(`handlers/proxy.rs:8364-8370`, commits `aad8755b`/`4dc00fee` — stay, see §2.5);
`assert_service_not_found` helper and test refactors (`proxy_service.rs:3585+`); the five
`AppConfig` test-literal updates; `CLAUDE.md`/`docs/ENV.md` env rows (edit per §3); the seven
new/updated documents under `docs/assistant/` (docs only; `V1_SPEC.md` needs the §8 amendments
so the two documents do not contradict each other).

**6.6 Confirmed absent:** no migration, no index change (`db.rs` untouched), no route change,
no response-shape change on any existing endpoint, no frontend or CLI change, no change to
`handlers/proxy.rs:2021` rule-matching semantics (policy-present rows behave exactly as at
`origin/main` — the only such rows are test fixtures, per the PR's own premise).

---

## 7. What cannot be made non-breaking — stated plainly

1. **New-create 400s (write path, admin API only).** After this rework, `POST /api/v1/services`
   rejects: (a) `connection` category with a non-empty credential, (b) `auth_method "none"`
   with a non-empty credential, (c) master-credential rows without `proxy_operation_policy`
   present (§2.4), and (d) master + `provider_config_id` (unreachable today). (a) and (b)
   previously succeeded while storing an inert secret; (c) previously succeeded and produced
   the exact hazard class this PR exists to close. No shipped client can send any of them; only
   direct API automation is exposed. **Existing rows, runtime traffic, and every read/update
   path are untouched.** If even this is above the bar, (a)–(c) can be gated on the §2 flag —
   at the cost of the "unrestricted platform rows are impossible to create" guarantee, which is
   the strongest property in the PR. Recommendation: keep them on.
2. **Boot no longer resurrects deactivated endpoints** (§6.3). A behaviour change by
   definition, visible only to an operator who relied on the defect as a feature. There is no
   such workflow in the repo, docs, or CLI; the supported path is the admin endpoint update.
3. Nothing else. With §§1–4 applied, every runtime code path reachable by existing traffic is
   bit-for-bit behaviourally identical to `origin/main`: the deny is a no-op behind a false
   flag, the limiter is uninitialized, the server-chosen bucket is deleted, the overlays are
   byte-identical, and the sync change only removes a write. That enumeration — §6 — **is** the
   by-construction argument: the proof obligation is closed by the diff itself, not by a
   production audit.

---

## 8. Execution plan — make the PR non-breaking (deployable with no audit)

Order matters only where noted; all of it is one PR revision.

1. **Revert the overlay trim** (§1): restore the three spec files from `origin/main`; restore
   `handlers/docs.rs:402-427`, `api_docs_service.rs:1086-1106`, `provider_service.rs:2569`;
   delete `trimmed_overlays_exclude_retired_operations` (`catalog_spec_sync.rs:527-571`).
2. **Delete the server-chosen bucket** (§4): `proxy_service.rs:925-930`,
   `rate_limit.rs:304-311`, `:355-370`, and its two tests.
3. **Flag the deny** (§2): add `PLATFORM_POLICY_FAIL_CLOSED` (default false) to `AppConfig`;
   static + init beside `main.rs:652`; parameterize
   `validate_actor_addressed_master_credential_policy(service, fail_closed)`; rework the two
   DB tests to flag-off parity assertions + pure validator tests; never set the global in
   tests.
4. **Add the create-time policy requirement** (§2.4) in `create_service` next to
   `validate_master_credential_shape` (`services.rs:929-934`), with unit tests:
   `internal_credentialed_create_requires_policy` (400 without the field, 201-shape with empty
   rules) — and amend the V1_SPEC X-row runbook body (it already carries the policy inline, so
   the runbook passes unchanged; the amendment is stating the requirement).
5. **Zero the limiter default** (§3): `config.rs:868-872` default 2 → 0; `docs/ENV.md:175-176`
   and `CLAUDE.md` rows updated to "default 0 = disabled; see V1_NONBREAKING.md §9 before
   enabling".
6. **Doc alignment:** update `downstream_service.rs:351-356` comment (§6.4); amend `V1_SPEC.md`
   items 2 and 5 to reference the flag/default and drop the pre-deploy-audit prerequisite
   (moved to §9); note in the PR body that item 4's trim was withdrawn and why.
7. **Verify:** `cargo check` + `cargo clippy --all-targets` locally; full suite in CI.
8. **Deploy canary (confirmation, not prerequisite):** one assistant chat turn; one
   `/llm` call as a normal user; `tools/list` for a BYOK `api-twitter` user shows
   `create_tweet`; admin PUT on `chrono-llm-public` (no-op field) saves; create-service with
   `connection` + credential returns 400 (expected new contract).

After step 8 the deployed system serves every existing row, caller, and published tool exactly
as `origin/main` did, with all four protections present in the binary but dormant, and with
newly-created platform rows structurally incapable of being born unrestricted.

## 9. Follow-up — switching each protection on, deliberately

Each step is independent, reversible, and gated on its own evidence. None is a prerequisite
for deploying §8.

1. **Runtime toggles (recommended before any flip):** move the flag and limiter limits into
   `PlatformSettings` following the broker rollout-policy pattern — env default + nullable DB
   override + background refresh + admin surface
   (`backend/src/services/platform_settings_service.rs:9-56`, refresh loop
   `backend/src/main.rs:245-271`, snapshot `AppState.broker_policy`). Replace the limiter's
   one-shot `OnceLock` with runtime-mutable atomics so off→on→off needs no restart.
2. **The audit** (moved here from V1_SPEC's deploy prerequisite): enumerate policy-less
   master-credential rows (the V1_SPEC §item-2 mongosh query) and, per row, 30 days of
   actor-addressed traffic from audit logs. `chrono-llm-public` is the known case; measure its
   `/llm` + `/proxy` usage and per-user peak rates while there.
3. **Policy for `chrono-llm-public`** per audit outcome — with the known trap: once a policy is
   present, AdminManaged assistant traffic is rule-matched at `handlers/proxy.rs:2021` too, so
   the rules must cover the assistant's forwarded paths (chat-completions-shaped,
   `handlers/assistant.rs:684-700`) as well as observed actor paths. Verify in staging with the
   assistant before production.
4. **Flip `PLATFORM_POLICY_FAIL_CLOSED` on.** Instant, instantly reversible via §9.1. Watch
   `master_credential_missing_operation_policy` warns (`proxy_service.rs:265-271`) — they name
   any row the audit missed.
5. **Set the per-user limits** from §9.2's measured per-user peaks (limit ≥ observed p99 with
   headroom), before the X row is exposed beyond the building. Watch
   `Platform per-user rate limit exceeded` warns (`rate_limit.rs:348`).
6. **Optional, design-first:** per-actor limiting for the assistant surface at the layer that
   knows the actor (§4); `platform-*` overlay spec keys when a platform row first needs
   spec-driven typed tools (§1).
