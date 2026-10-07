# Slug routing: Fable review

Status: design review for the "stable slug prefers a working user connection,
then platform, else explicit error" proposal. Read-only; no code changed.
Companion to the primary's combined proposal. Line refs are against `main`
at 9bb33bcf.

## 1. Recommendation in one paragraph

Ship this as three releases, not one. Release 0 adds a single route
evaluator that both execution and previews consume, replaces the resolver's
scattered 400/404 exits with one structured "no usable connection" error, and
makes every surface (`/keys`, detail page, CLI, MCP) render the evaluator's
verdict. It changes no routing decision except making the one silent
fallthrough that already exists visible. Release 1 turns on the only new
routing behaviour the user asked for: a present-but-unusable personal
connection falls to the platform candidate, gated by the catalog predicate
that already defines "platform credential", with a user opt-out and explicit
billing disclosure. Release 2 handles the cases the primary flagged as open
(unusable personal to org, multiple personal accounts, "make default"). Do not
ship any release that switches between two user-owned accounts automatically.

## 2. What the code does today (evidence)

### 2.1 The resolver short-circuits and errors instead of trying the next candidate

`resolve_proxy_target_from_user_service` (`backend/src/services/proxy_service.rs:1394-1562`)
does: personal exact slug row -> `finish_resolution`; else personal pool; else
legacy personal guard -> `Ok(None)`; else org walk. Any failure inside
`finish_resolution` propagates:

| Condition | Where | Result today |
|---|---|---|
| `UserApiKey.status != "active"` | `proxy_service.rs:2650-2655` | 400 `API key is expired` (or revoked/failed/...) |
| credential not materializable | `proxy_service.rs:2660, 2672` | 400 `OAuth connection is not complete...` / `No credential stored...` |
| auto-provisioned row no longer eligible | `proxy_service.rs:2359-2397, 2497` | 404 `Service is no longer available` |
| org role/scope blocked | `proxy_service.rs:1556-1560` | 403 `OrgRoleInsufficient` |
| pool has no active member | `service_pool_service.rs:395`, `errors/mod.rs:725` | 502 code 11403 |

None of these carries "which candidates exist and why each was skipped".

### 2.2 There already is a silent platform fallthrough, and it is inconsistent

When no `UserService` matches (including when the personal row is
**disabled or deleted**, since `find_by_slug` filters `is_active: true`,
`user_service_service.rs:580-589`), the handler falls to the legacy path
(`handlers/proxy.rs:1342-1348`) and `resolve_proxy_target`
(`proxy_service.rs:986-1119`) injects the catalog master credential whenever
`!requires_user_credential && auth_method != "none"` (`proxy_service.rs:1102-1112`).

So for a master-credential catalog slug today:

- personal key **expired** -> 400, no platform
- personal row **disabled or deleted** -> platform credential, silently, with
  platform billing class and platform rate limit, no header saying so

That asymmetry is the strongest argument for an explicit evaluator. It also
means "Disable" already has slug-level consequences nobody designed.

### 2.3 "Platform connection" is two different things

1. **Catalog master credential**: `DownstreamService.credential_encrypted`
   non-empty with `requires_user_credential=false`. Reached via the legacy
   path above, or via an auto-provisioned personal `UserService`
   (`source="auto_provision"`, `api_key_id=None`, `master_credential=true`,
   `proxy_service.rs:2492-2555`). The auto-provision predicate
   `is_public_internal_master_credential_service` (`proxy_service.rs:2269-2279`)
   requires `visibility=public`, `service_category=internal`,
   `service_type=http`, and **`provider_config_id.is_none()`**.
2. **Platform vendor templates**: internal `platform-*` slugs
   (`catalog_spec_sync.rs:216-218`), hidden from catalog reads and from
   `create_key` (`unified_key_service.rs:825-829`), used by server-chosen
   assistant operations (`platform_operation_service.rs`). Not addressable by
   user slug. Out of scope for slug routing.

Only (1) is a fallback candidate. Note the `provider_config_id.is_none()`
clause: every OAuth-provider-backed catalog row (GitHub, Google, Lark, X) is
already **structurally ineligible** as a platform candidate. That is the
right answer to "account-bound credentials cannot substitute platform
identity", and the proposal should cite it rather than add a new rule. The
legacy path (`resolve_proxy_target`) does not check this clause itself; it
relies on `authorize_master_credential -> validate_master_credential_service`
(`proxy_service.rs:150-152`). Confirm that helper excludes provider-backed
rows before relying on it.

### 2.4 Same slug, both a user key and a platform key: possible but data-dependent

One catalog slug is one `DownstreamService` row. `create_key` on a catalog
slug does not reject a user-supplied credential when the row also carries a
master credential (`unified_key_service.rs:812-945`; the only rejection is
the `platform-*` vendor guard). So the user's scenario exists only for rows
that are `internal` + `public` + master credential + BYO allowed. Whether any
production catalog row is shaped like that must be checked against data, not
seeds. If none is, the feature is really "canonical slug maps to a
*different* platform slug", which needs an explicit catalog-identity mapping
(see 3.2), not slug-text matching.

### 2.5 Identity is `catalog_service_id`, not slug text

- `resolve_unique_slug` auto-suffixes (`llm-openai-2`), and `PreserveExact`
  lets a user pick any slug for a catalog service
  (`unified_key_service.rs:100-153`). `UserServiceResolution.catalog_service_slug`
  is loaded separately (`proxy_service.rs:2844-2870`) and the tests assert it
  can differ from `UserService.slug` (`proxy_service.rs:4378-4382`).
- Consequence today: a user with a working key under custom slug `oai` who
  calls `/proxy/s/llm-openai` (the catalog slug) gets the legacy platform
  path, not their own key. Canonical selection must key on
  `catalog_service_id`.
- `find_by_catalog_service_id` is an unordered `find_one`
  (`user_service_service.rs:644-656`). With two active personal rows for the
  same catalog (multi-connection is explicitly supported for OAuth,
  `unified_key_service.rs:860-869`), catalog-id resolution is nondeterministic.
  The primary's evidence is correct.
- `auto_provision_no_auth_services` skips creating the platform row when
  **any** row (active or inactive) exists for the catalog id
  (`unified_key_service.rs:1728-1745`). So a user who ever added their own key
  never gets a platform row, and after deleting it they get the legacy path
  instead. Also it runs lazily from `list_keys`; agent-only users who never
  open `/keys` never get platform rows at all.

### 2.6 Four partial "is it usable" evaluators, none of them the resolver

| Surface | Evaluator | What it checks |
|---|---|---|
| `/keys` | `build_key_view` (`unified_key_service.rs:3993-4002`, `oauth_connection_status` 4059) | `status`, `connection_status`, `credential_missing`, `node_status`, `is_active` |
| frontend card/table | `keys.tsx:152-156` precedence in the component | `connection_status` > node status > credential status |
| CLI | `commands/service.rs:25-47 display_status` | same rules, re-implemented |
| MCP tools | `mcp_service.rs classify_credential` (~1762) | active-key map + node online |
| `/proxy/services` | `proxy_discovery_service.rs` | connection presence only |
| assistant readiness | `assistant_readiness_service.rs:219-330` | fixed capability registry, mirrors personal > legacy > org |

None of these knows about pools, approvals, API-key scope, org role, or the
auto-provision eligibility recheck. Each can disagree with what the proxy
will actually do. The user's "clear way to visualize" is unachievable until
they all read one decision.

### 2.7 Things a fallback would silently change

- **Billing class**: `final_credential_class` (`handlers/proxy.rs:4131-4160`)
  maps `master_credential` to `NyxidManagedMaster`; resale pricing applies
  only in that class (`billing/route_context.rs:55-60`); platform per-user
  rate limiting applies only on master-credential paths
  (`proxy_service.rs:2505-2511, 1104-1110`). A user-to-platform switch changes
  what the request costs and who pays.
- **Approvals**: execution authority binds `user_service_id`
  (`execution_authority.rs:65,104,160`); approval identity compares
  `user_service_id` (`approval_service.rs:701`). A grant for the personal row
  does not cover the platform row. Also auto-connected rows suppress the
  implicit global default approval (`is_auto_connected`,
  `proxy_service.rs:1292-1296`), so the platform candidate may need *less*
  approval than the personal one. Both directions must be visible in preview.
- **Agent scope and bindings**: `ApiKey.allowed_service_ids` are `UserService`
  ids; `AgentServiceBinding` is keyed by `(api_key_id, user_service_id)`. The
  platform row is a different id.
- **Legacy guard counts unusable tokens as present**:
  `legacy_personal_provider_token_filter` accepts `expired` and
  `refresh_failed` (`proxy_service.rs:1638-1647`), so a dead legacy token
  blocks org fallback today. "Present" and "working" are already conflated.

### 2.8 Pins already exist

`?_nyxid_via=<user_service_id>` resolves one exact row and refuses a
cross-slug id (`proxy_service.rs:1894-1911`). Exact slugs are unique per
owner among active rows. The pin primitive does not need inventing; only the
canonical-slug selection is new.

## 3. Proposal

### 3.1 One evaluator, two modes

Add `slug_route_service::evaluate(actor, requested_slug, mode)` returning:

```
RouteDecision {
  requested_slug,
  catalog_identity: Option<{ catalog_service_id, catalog_slug }>,
  candidates: [ RouteCandidate ],      // ordered as they will be tried
  selected: Option<index>,
  unavailable_reason: Option<ReasonCode>,
  evaluated_at, health_scope: "pre_dispatch"   // never claims downstream health
}
RouteCandidate {
  kind: exact | personal | legacy | org | platform,
  user_service_id: Option, slug, owner: personal | org{org_user_id} | platform,
  eligibility: eligible | eligible_needs_refresh | ineligible(ReasonCode) | unknown,
  billing: user_owned | platform_resale | no_auth | node_managed,
  approval: none | required | granted,
  actions: [ enable | reconnect | continue_auth | rebind_node | ask_org_admin | update_key_scope ]
}
```

Execution mode wraps the existing resolver: evaluate eligibility read-only in
order (`ProxyCredentialResolution::read_only_snapshot()` already exists,
`proxy_service.rs:1367-1374`), materialize only the first eligible candidate
via `finish_resolution`, and if materialization itself fails (refresh
failure, decrypt failure) move to the next candidate. The boundary is "before
the first byte is sent downstream". Downstream responses never trigger
re-selection in any release; they may only update credential status through
the existing refresh-failure paths. Preview mode never refreshes, never
touches `last_used_at`, never rate-limits, and reports
`eligible_needs_refresh` honestly for OAuth keys with a refresh token and an
expired access token.

Fast path preserved: when the exact personal row is eligible, the cost is the
same single query as today.

### 3.2 Selection rules

1. **Exact wins.** If the requested slug matches an active `UserService`
   (or pool) for the actor, that is the exact candidate. A `_nyxid_via` id or
   an `AgentServiceBinding` is a pin: no fallback, ever. An exact custom slug
   with no `catalog_service_id` has no other candidates.
2. **Canonical identity from the exact row or the catalog.** If the exact row
   is unusable and carries `catalog_service_id`, or the slug is a catalog
   slug with no exact row, candidates are collected by `catalog_service_id`:
   other personal rows (deterministic order: slug equal to catalog slug
   first, then `created_at` asc), legacy personal, org rows in membership
   order with role/scope checks, then the platform candidate.
3. **Platform candidate eligibility** = `is_public_internal_master_credential_service`
   + actor-addressed operation policy + consent (all existing checks) +
   the user has not disabled the platform row (3.4). No new admin flag in
   release 1; the predicate is the policy. If admins later want per-row
   opt-in, add `DownstreamService.platform_fallback` with a backfill equal to
   the predicate so nothing changes on deploy.
4. **User-owned accounts never substitute for each other automatically.**
   If the canonical row is unusable and another personal row for the same
   catalog is usable, the decision is `unavailable` with reason
   `other_personal_connection_available` and the other slug named in the
   action. Same for org-owned rows in release 1 (keep the NyxID#209 order:
   personal presence still outranks org). Release 2 may add unusable-personal
   to org behind the same evaluator, with the org billing owner disclosed.
5. **API-key scope filters candidates**, it does not error. A candidate
   outside `allowed_service_ids` is `ineligible(out_of_scope)`.

### 3.3 One error

New variant `SlugRouteUnavailable { slug, decision }`, HTTP 400 for client
compatibility (today's failures are already 400), numeric code in a new
block 11800-11809, `details.candidates[]` with reason and action per
candidate. Replace the exits in 2.1 for slug-addressed requests. Keep 403
`OrgRoleInsufficient` when the only candidates are org rows the actor may
not use, since that is a permission statement, not an availability one.

Reason codes (one per existing state, no new states): `disabled`,
`credential_missing`, `pending_auth`, `expired`, `revoked`, `failed`,
`refresh_failed`, `node_offline`, `node_draining`, `node_deleted`,
`out_of_scope`, `org_role_insufficient`, `auto_provision_ineligible`,
`platform_policy_blocked`, `other_personal_connection_available`.

### 3.4 Disable intent (answering the primary's open question)

Keep Disable as a connection control; do not introduce a second control
also called Disable at the slug level (Rule 8 forbids lifecycle synonyms, and
"disable the route" and "disable the connection" would collide in the UI).

My first instinct was to express the opt-out by disabling the auto-provisioned
platform row itself. Verified against code, that does not work without a
guard change: `ensure_user_managed_service` (`user_service_service.rs:74-81`)
rejects every user mutation on `source = "auto_provision"` rows, including
`is_active`, and the `/user-services` and `/keys` routes document that 403.
So the opt-out needs its own small authoritative record (FI-004): a per-owner,
per-catalog preference document with `platform_mode: first_then_platform |
your_connection_only` and an optional `preferred_user_service_id`, unique on
`(user_id, catalog_service_id)`. The combined draft proposes the same shape
(`ServiceRoutePreference`); I agree with it, with two limits below in 4.2.

The platform candidate is then virtual: synthesized from the catalog row the
way the legacy path already does, never dependent on `list_keys` having run.
That also fixes the agent-only-user gap in 2.5.

Whichever is chosen: once the evaluator covers a slug, the handler must not
fall into the legacy `resolve_service_by_slug` path for it unless a legacy
connection exists. Otherwise `your_connection_only` is silently bypassed by
the fallthrough from 2.2.

Whichever is chosen: once the evaluator covers a slug, the handler must not
fall into the legacy `resolve_service_by_slug` path for it unless a legacy
connection exists. Otherwise disabling the platform row re-enables the
silent fallthrough from 2.2.

### 3.5 Visualisation

- **`/keys` list**: group cards by catalog identity. Each group header shows
  the verdict for the canonical slug: `Available via your connection`,
  `Available via platform (wallet-billed)`, or `Unavailable: <reason>` with
  the action button. Child cards keep today's per-connection badges. The
  `Disabled` and `Credential Missing` badges stay; they are candidate state,
  not slug state.
- **Detail page**: extend the existing Routing section
  (`components/dashboard/routing-section.tsx`) with the ordered candidate
  list and a "Check route" button that calls the preview endpoint. Show
  billing class and approval requirement per candidate.
- **API**: `GET /api/v1/keys/route/{slug}` returns `RouteDecision`. It is
  read-only and must live in the delegated-read allowlist, not the deny
  classes.
- **CLI**: `nyxid service route <slug>` prints the table; `nyxid service list`
  gains a `route` column derived from the same call, replacing the local
  `display_status` re-implementation.
- **MCP**: `nyx__list_connected_services` and `discover_services` include
  `available_via` and `unavailable_reason`; tool visibility follows the
  evaluator so MCP never hides a tool the proxy would serve via platform, and
  never advertises one it would refuse.
- **Response and audit**: every proxy response carries
  `X-NyxID-Route: personal|org|platform|legacy` and the selected
  `user_service_id`; the routing audit event records the requested slug, the
  selected candidate, and the skipped candidates with reason codes. No
  credential material, ever.

### 3.6 Smallest safe first release (challenge to the primary's bundle)

The primary's leaning bundles evaluator, fallback, preview, and UI. I would
cut it:

**Release 0, no routing change**: evaluator + structured error + preview
endpoint + all surfaces reading it + one determinism fix (ordered
`find_by_catalog_service_id`). Leave the legacy guard's "expired counts as
present" rule alone until Release 2; Release 0 only reports it as a candidate
with reason `expired`. Make the existing legacy platform fallthrough visible
via header and audit. This is shippable behind no flag because every request
resolves to the same row it does today.

**Release 1, one new behaviour**: present-but-ineligible personal row falls
to the platform candidate, with the platform-row opt-out from 3.4 shipping in
the same release. Do not ship fallback without the opt-out: fallback spends
wallet credits.

**Release 2**: unusable personal to org; "Make default for `<catalog slug>`"
as an atomic slug swap between two personal rows; per-agent preferences.

### 3.7 Objections and risks

1. **The premise may not match production data** (2.4). If no catalog row is
   both BYO-able and master-credentialed, "same slug, user then platform"
   has no instances and the real need is a catalog-identity alias between a
   user slug and a platform slug. Check before building Release 1.
2. **Fallback changes price and identity per request.** Acceptable only with
   the header, the audit record, the preview, and the opt-out. Without all
   four it is a billing surprise.
3. **Materializing after a failed candidate** can mark keys `failed` and
   fire expiry notifications for a credential the request did not end up
   using. That is correct (the credential is dead) but the notification copy
   should not imply the request failed.
4. **Node failover stays below the boundary.** `fallback_node_ids` is
   node-level retry for one selected `UserService`; do not fold it into the
   evaluator, and do not let a node dispatch failure trigger candidate
   re-selection (the body may already be streaming).
5. **Do not route on downstream 401/403** in any release. The reply may
   carry side effects, bodies are consumed, and provider errors are not
   reliably auth errors.
6. **Pools** already balance identical members but filter only `is_active`
   (`service_pool_service.rs:374-390`). If the evaluator gains credential
   eligibility, pools should use the same predicate for member viability or
   a pool will keep selecting an expired member while the evaluator would
   have skipped it.

## 4. Review of the combined draft (`slug-connection-resolution-proposal.md`)

The draft's evidence table is accurate; I re-checked every cited symbol
(`is_valid_master_credential_service` at `proxy_service.rs:308`,
`find_effective_service_owner` at 1992, `classify_credential`,
`resolve_member`). The product contract, the exact-vs-canonical split, the
pins, the no-replay rule, the preview/prepare/execute separation, and the
"a platform database row alone is insufficient" acceptance case are all
right and match my findings. What follows is only what I would change.

### 4.1 Contradictions

1. **Org fallback after a failed personal connection is both allowed and
   forbidden.** The flowchart (`Usable user connection? No -> Usable
   authorized org connection?`) falls to org unconditionally. The prose two
   paragraphs later says a failed personal account does not authorize a new
   account substitution, and the failure table says "only for a canonical
   route whose policy allows that substitution". Pick one and fix the
   diagram. My recommendation: Release 1 keeps today's rule (org only when
   the actor has no personal connection at all); unusable-personal-to-org is
   Release 2 and needs the org billing owner disclosed.
2. **"Disabled connections stay disabled; do not revive access through a new
   fallback" versus what the code does now.** Disabling or deleting the
   personal row on a master-credential catalog slug already revives platform
   access through the legacy path (section 2.2). The draft's connection-level
   Disable copy ("This service will use NyxID while this connection is
   disabled") describes that existing behaviour, so the acceptance case
   "existing disconnected intent survives migration" is ambiguous about
   which intent. State explicitly: the legacy fallthrough is preserved but
   made visible, and `your_connection_only` closes it.
3. **Default mode versus migration promise.** "The normal mode is Your
   connection first, then NyxID" and "do not silently opt existing BYOK-only
   users into new charges" conflict unless the default is conditional. Make
   the rule crisp: the fallback mode is the default only where the platform
   target is configured **and** the payer already holds applicable billing
   authorization for that route; everyone else starts in
   `your_connection_only` until they opt in from the routing panel.
4. **Failure contract HTTP code.** "No configured connection (proposed
   409)" changes the status every existing client sees on this path. Today
   these are 400s (`API key is expired`, `No credential stored...`,
   `proxy_service.rs:2650-2672`) and CLI, MCP, and SDK callers already handle
   400. Keep 400 and add the numeric code; 409 also reads as "conflict" to an
   agent. Likewise "temporary route unavailability (503)" must not remap the
   existing pool 502 (11403) or `NodeOffline` (8001).
5. **Canonical default for multiple personal rows.** The draft says
   "several exist without a preference: ask the user to choose". On the slug
   path the row whose slug equals the catalog slug is already an exact,
   deterministic match; forcing a chosen default there regresses every
   multi-connection user on day one with a `choose_default` error. The
   unordered `find_one` only affects the catalog-id path
   (`/proxy/{service_id}`). Rule: exact slug row is the default when usable;
   a stored preference is required only when that row is unusable and other
   personal rows exist.

### 4.2 Unjustified scope for the first releases

1. **Service-level Disable at the canonical header.** It is a third
   lifecycle verb next to the two Rule 8 allows, it needs the preference
   document to carry `is_active`, and nothing in the user's ask needs it: the
   mode selector plus connection-level Disable already answers "stop using my
   key" and "never use platform". Drop it, or rename and defer.
2. **Persisted credential-health observations** (verification state,
   `last_success_at`, epoch-bound invalidation, "last succeeded 2 min ago").
   That is a new subsystem with its own invalidation rules. The ask is
   pre-dispatch eligibility plus a clear error. Ship `Not yet verified` and
   the existing `last_used_at` (`touch_last_used`, `proxy_service.rs:2662-2667`)
   and defer health evidence.
3. **Operation allowlist on the platform target.** `DownstreamService`
   already carries `proxy_operation_policy` with the
   `PLATFORM_REQUIRE_OPERATION_POLICY` fail-closed switch (`docs/ENV.md:222`)
   for exactly the "platform credential must not reach account-bound
   operations" concern. Reuse it; do not add a parallel allowlist.
4. **Alias to a separate platform catalog row.** Only needed if production
   has no catalog row that is both BYO-able and master-credentialed (section
   2.4). Check the data first; same-row fallback is the smaller feature.
5. **"Explicitly allowed alternative IDs" on the preference document.** An
   ordered failover list of user connections is a second selection engine
   beside pools, which the draft says it will not add. Keep the document to
   `preferred_user_service_id` and `platform_mode` in v1.
6. **LLM ingress as separate rollout work.** `llm_proxy_request` already goes
   through `resolve_proxy_target_from_user_service` (see the call-site note at
   `proxy_service.rs:1671-1672`), so placing the evaluator inside the resolver
   covers it. No separate LLM step; just say so.

### 4.3 Gaps the draft should add

- The legacy guard treats `expired` and `refresh_failed` provider tokens as
  present (`proxy_service.rs:1638-1647`); under the draft's own "usable"
  definition they are not. Decide when that changes (I say Release 2).
- Auto-provisioned rows reject all user mutation
  (`user_service_service.rs:74-81`), so the opt-out cannot be "disable the
  platform row"; the preference document is required for it.
- `X-NyxID-Route` (or equivalent) on every proxy response, and the skipped
  candidates with reason codes in the routing audit event, so preview and
  actual can be compared during rollout as step 3 promises.
- The `provider_config_id.is_none()` clause of
  `is_public_internal_master_credential_service` is the existing structural
  proof that OAuth-provider services (GitHub, Google) are never platform
  candidates. Cite it; it is stronger than a policy statement.

## 5. Reconciliation with the primary (final dispositions)

Agreed by both parties on 2026-09-16. Where this section conflicts with
sections 3 and 4 above, this section wins.

1. **No fallback on server faults.** KMS/decrypt failures, database errors,
   and integrity violations (missing endpoint, dangling `api_key_id`) stay
   sanitized 5xx and never trigger candidate re-selection. Only typed
   pre-dispatch outcomes do: `credential_missing`, `pending_auth`,
   `expired`/`revoked`/`failed`/`refresh_failed`, terminal refresh rejection,
   `disabled`, `auto_provision_ineligible`, and unavailable permitted node
   routes. This narrows my section 3.1 "if materialization fails, move on":
   only a *typed* materialization failure moves on.
2. **Ordered catalog-id lookup is a routing change.** Replacing the
   unordered `find_by_catalog_service_id` changes which row a catalog-id
   call selects, so it is not part of the no-routing-change Release 0.
   Release 0 only reports candidates. Canonical selection by catalog
   identity lands with the fallback release, selects only when exactly one
   usable personal row exists, and otherwise stops with an advisory
   `choose_default` reason. This supersedes the "one determinism fix"
   wording in section 3.6.
3. **No numeric codes allocated here.** The 11800-11809 suggestion in
   section 3.3 is withdrawn. `backend/src/errors/mod.rs` is the registry and
   allocation happens at implementation.
4. **Platform mapping.** Same-row fallback reuses the existing validated
   master-credential predicate and authorization path. An admin-configured
   mapping from a logical user catalog row to a distinct platform catalog row
   is added only if production inventory confirms the two are distinct rows.
   In both cases operation compatibility stays strict via the existing
   operation policy; unknown compatibility fails closed.
5. **No atomic slug swap.** "Make default" is `preferred_user_service_id` on
   the preference document; existing slugs and proxy URLs are never renamed.
   Section 3.6 Release 2 "atomic slug swap" is withdrawn.
6. **Adopted from this review:** staged rollout; the existing silent
   master-credential fallthrough is preserved but made visible and closed by
   `your_connection_only`; the platform candidate is virtual and independent
   of `list_keys`; `_nyxid_via` and agent bindings are the pins; Disable
   stays connection-only with an explicit platform-mode opt-out;
   failed-personal-to-org and multiple-account fallback are deferred.
