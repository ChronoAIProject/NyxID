# Platform Services V1 — Implementation Spec

*2026-08-21. Branch `travel-allowlist`, based on `origin/main` (`7dcb6d5f`). This is the first
buildable slice of `docs/assistant/ONBOARDING_CAPABILITIES.md`. It is a build document: every
change names its file, function, current behaviour, new behaviour, the test that proves it, and
what a reviewer must check. Implement the items in the order they appear.*

**Scope:** the four safety changes (items 2–5) plus the activation runbook for one service
(X recent search). Item 1 (assistant tool use) is pending an in-flight review and is marked, not
specced. Voice, Twilio, Duffel booking, Reddit, and the constraint DSL are out of scope.

---

## How this is verified

- **Docker is unavailable on the implementation machine, so the backend suite cannot run
  locally.** CI is the verifier: a PR to `main` starts a single-node replica set and runs the
  suite with `NYXID_TEST_DATABASE_URL=mongodb://127.0.0.1:27017/?replicaSet=rs0&directConnection=true`.
- DB-backed tests follow the existing pattern in `backend/src/services/proxy_service.rs` tests:
  `connect_test_database("<name>").await` returning `Option<Database>`, with an early-return skip
  when no MongoDB is reachable (see `server_chosen_master_credential_requires_public_valid_row`,
  `backend/src/services/proxy_service.rs:3547`). New DB tests MUST use this helper so they run in
  CI and skip gracefully elsewhere.
- Before pushing: `cargo check` and `cargo clippy --all-targets` must pass locally (both work
  without Docker). `cargo test` compiles tests; DB tests will self-skip locally.

## Architecture facts the items rely on

Verified on this branch; a reviewer should re-confirm each once.

1. **All master-credential decryption is gated.** `AuthorizedMasterCredential` has a private
   constructor (`backend/src/services/proxy_service.rs:96-110`); the only producers are
   `authorize_master_credential` (`proxy_service.rs:149`) and
   `authorize_master_credential_server_chosen` (`proxy_service.rs:215`). Every actor-addressed
   path that spends a platform credential passes through `authorize_master_credential`:
   - legacy catalog resolution: `resolve_proxy_target` (`proxy_service.rs:1014-1021`)
   - lenient/node resolution: `resolve_proxy_target_lenient` (`proxy_service.rs:1153-1163`)
   - auto-provisioned UserService rows (used by `/proxy`, `/llm` gateway, MCP, the POC):
     `proxy_service.rs:2324-2329`
   The server-chosen producer is used only by `resolve_admin_proxy_target`
   (`proxy_service.rs:891`), which backs `TargetMode::AdminManaged`
   (`backend/src/handlers/proxy.rs:1937-1956`) — the assistant surface where NyxID, not the
   caller, picks the target (`backend/src/handlers/assistant.rs:693`).
2. **Policy rule evaluation happens at two call sites, both gated on policy presence:**
   REST proxy `backend/src/handlers/proxy.rs:2018-2030` and MCP
   `backend/src/services/mcp_service.rs:2931-2947` (`prepare_proxy_tool_call`). When
   `proxy_operation_policy` is `None` both sites skip evaluation entirely;
   `authorize_proxy_operation_fields` also maps `None => allow`
   (`backend/src/services/proxy_authorization.rs:227-233`).
3. **The `/llm` gateway does not rule-match.** It resolves credentials through the auto-provision
   path (item 2 therefore applies) but builds provider-shaped requests itself and never calls
   `authorize_proxy_operation` (`backend/src/handlers/llm_gateway.rs` has no call). A present
   policy's rules are not path-matched there; only the presence check from item 2 applies.
4. **`is_valid_master_credential_service`** (`proxy_service.rs:244-252`) is the platform-row
   predicate: active + `http` + `service_category == "internal"` + `!requires_user_credential` +
   non-empty `credential_encrypted` + `provider_config_id.is_none()`, with
   `master_credential_required` = `auth_method != "none"` (`proxy_service.rs:142-146`). It is
   currently private (`fn`, not `pub`).
5. **MCP platform publication** builds endpoints exclusively from active `ServiceEndpoint` rows
   (`backend/src/services/mcp_service.rs:1068-1076` filters `is_active: true`;
   `:1241-1260` publishes platform rows). A platform row with no rows and no generic fallback
   publishes zero operations; the POC additionally rejects the generic proxy endpoint
   (`backend/src/services/assistant_direct_agent_poc/tools.rs:363-364`).

---

## Item 1 — Assistant tool use: PENDING, do not build from this section

A review is in flight verifying whether enabling assistant tool use is a feature-flag flip or
real development. Facts already established on this branch:

- The machinery exists and is routed: `POST /api/v1/assistant/direct/agent`
  (`backend/src/routes.rs:137`), gated by `require_direct_chat_enabled`
  (`backend/src/handlers/assistant_direct_agent_poc.rs:35`) on the
  `experimental:direct-chat-engine` flag (`backend/src/services/feature_flag_service.rs:111`).
- The eligibility filter `is_poc_operation_eligible`
  (`backend/src/services/assistant_direct_agent_poc/tools.rs:363-383`) admits only operations
  whose **method** derives to `Read` — `derive_verb_from_method`
  (`backend/src/services/operation_descriptor.rs:144-150`) maps `GET|HEAD|OPTIONS => Read`,
  `DELETE => Destructive`, everything else `Write`. As written, `POST /v2/scrape` and
  `POST /air/offer_requests` are `Write` and are **not** admitted.

Two outcomes, either of which the review resolves:

- **If the filter is amended (or the review finds an admission path) for semantically-read
  POSTs:** v1 gains a flag flip (`set_platform_override` on `experimental:direct-chat-engine`)
  plus the end-to-end drive of the POC against the X row from the runbook below.
- **If not:** admitting POST-shaped reads is separate work with its own review; v1 ships with
  the X row reachable via REST proxy and MCP, and via the POC for GET operations only. The X
  activation in this spec uses `GET /tweets/search/recent`, which is admissible either way —
  item 1's outcome does not block anything below.

---

## Item 2 — Master-credential rows fail closed without a policy

### Current behaviour

- Absent `proxy_operation_policy` means passthrough. The model field documents this
  (`backend/src/models/downstream_service.rs:351-355`), `authorize_proxy_operation_fields`
  implements `None => allow` (`backend/src/services/proxy_authorization.rs:227-233`), and both
  rule-evaluation call sites skip entirely on `None` (fact 2 above).
- Existing test pinning the behaviour: `missing_policy_preserves_passthrough_behavior`
  (`proxy_authorization.rs:324-331`).

### Change

One check at the single choke point, so every actor-addressed transport (REST, MCP, `/llm`
gateway, node-routed, WS-upgraded HTTP resolution, POC) is covered without touching each
handler:

**File:** `backend/src/services/proxy_service.rs`, function `authorize_master_credential`
(line 149). After the `is_valid_master_credential_service` check (line 154-162) and before the
visibility match (line 164), insert:

```rust
// A row holding a platform credential must state what it may do. Absent
// policy is deny, not passthrough (V1_SPEC item 2). Present-but-empty
// policy also denies, at the operation layer.
if service.proxy_operation_policy.is_none() {
    tracing::warn!(
        service_id = %service.id,
        service_slug = %service.slug,
        reason = "master_credential_missing_operation_policy",
        "Catalog master credential authorization denied"
    );
    return Err(AppError::NotFound("Service not found".to_string()));
}
```

**Deliberately unchanged:**

- `authorize_proxy_operation_fields` keeps `None => allow`. Non-platform rows (BYOK
  connections, user services, no-auth rows) retain passthrough; the fail-closed rule is scoped
  to rows spending the platform credential, which all funnel through
  `authorize_master_credential`.
- `authorize_master_credential_server_chosen` (`proxy_service.rs:215-242`) is unchanged. The
  server-chosen surface (`execute_admin_proxy` / assistant direct) forwards requests the
  platform itself addressed; the caller never picks the target. Note the interaction: when a
  server-chosen row *does* carry a policy, `handlers/proxy.rs:2018` rule-matches its traffic
  too — see the deployment prerequisite below.
- Update the doc comment on the model field (`downstream_service.rs:351-354`) to read: empty
  policy denies every operation; missing policy preserves passthrough **except** on
  master-credential rows, where resolution itself denies (`authorize_master_credential`).

### Tests

In the existing `#[cfg(test)]` module of `proxy_service.rs`, `connect_test_database` pattern:

- `master_credential_without_policy_is_denied` — public, valid master row (mirror the setup at
  `proxy_service.rs:3620-3631`: `service_category = "internal"`, `auth_method = "bearer"`,
  `credential_encrypted = vec![1,2,3]`), `proxy_operation_policy = None`. Assert
  `authorize_master_credential(&db, &service, &actor).await.is_err()`.
- `master_credential_with_empty_policy_resolves` — same row with
  `Some(ProxyOperationPolicy { rules: vec![] })`. Assert resolution `is_ok()` (denial of every
  operation is the operation layer's job — pinned by the existing
  `present_empty_policy_denies_every_operation`, `proxy_authorization.rs:333-339`).
- `master_credential_with_policy_resolves` — same row with a one-rule policy. Assert `is_ok()`.
- **Update existing tests** in that module that call `authorize_master_credential` on
  credentialed rows without a policy (the block at `proxy_service.rs:3620-3700`, including the
  "public credentialed row should be allowed" assertion at `:3626-3631`): attach
  `Some(ProxyOperationPolicy { rules: vec![] })` so they keep asserting what they were written
  to assert (visibility/consent), not policy presence.
- `missing_policy_preserves_passthrough_behavior` in `proxy_authorization.rs` stays as-is: it
  pins that *non-master* rows keep passthrough.

### Reviewer checklist

- `grep -rn "AuthorizedMasterCredential::new\|AuthorizedMasterCredential(" backend/src` — the
  only constructors remain inside the two gate functions. If a third producer exists, item 2
  has a bypass.
- Confirm the auto-provision UserService path still copies the catalog policy onto the resolved
  target (`apply_catalog_proxy_authorization`, `proxy_service.rs:2285` and `:2345`) so
  present-policy rule matching at `handlers/proxy.rs:2018` still fires for those rows.
- Confirm no listing/`executable` computation calls `authorize_master_credential` (it must stay
  an execution-path-only gate; MCP `executable` for platform rows is computed at
  `mcp_service.rs:924` without it).

### Deployment prerequisite (production, before this code deploys)

Absent-policy master rows stop resolving the moment this deploys. Audit first:

```js
db.downstream_services.find({
  auth_method: { $ne: "none" },
  service_category: "internal",
  requires_user_credential: false,
  provider_config_id: null,
  is_active: true,
  proxy_operation_policy: { $exists: false },
}, { slug: 1, base_url: 1 })
```

For each row returned (expected: `chrono-llm-public`, possibly others), check the audit log for
actor-addressed traffic (proxy/MCP/`/llm` events naming the row) over the last 30 days, then:

- **No actor-addressed traffic** → leave the policy absent. Server-chosen (assistant direct)
  traffic is unaffected by item 2 and, with policy `None`, is also untouched by the rule
  matcher at `handlers/proxy.rs:2018`. Post-deploy, actor-addressed calls deny — the desired
  state.
- **Actor-addressed traffic exists** (e.g. users calling the public LLM through `/llm` or
  `/proxy`) → attach a policy whose rules cover **both** the observed actor paths **and** the
  paths the assistant surface forwards (assistant direct forwards chat-completions-shaped paths
  into `execute_admin_proxy`, `handlers/assistant.rs:693`; once a policy is present, AdminManaged
  traffic is rule-matched too). For a chat row with `base_url` ending in the API root, that is
  at minimum `POST /chat/completions` (plus `GET /models` if observed). Verify the assistant
  works in staging with the policy attached before production.

Do not skip the audit: attaching an empty policy to a row serving assistant traffic breaks the
assistant (rules evaluate and deny), and leaving policy absent breaks any real `/llm` gateway
users of that row. The audit decides which of the two configurations is correct per row.

---

## Item 3 — Reject master credential combined with the wrong credential mode

### Current behaviour

- Nothing inspects credential shape at admin write. `create_service`
  (`backend/src/handlers/services.rs:707`) accepts `credential` + `auth_method` +
  `service_category` (`CreateServiceRequest`, `services.rs:39-92`), derives
  `requires_user_credential = service_category == "connection"` (`services.rs:929`), and always
  stores `provider_config_id: None` (`services.rs:1092`). A credential supplied for a
  `connection`-category row is encrypted and stored even though the resolution path can never
  use it (`proxy_service.rs:1003-1013` takes the user-credential branch first) — a stored
  master credential lying dormant on a user-mode row.
- `update_service` (`UpdateServiceRequest`, `services.rs:248-310`) has **no** `credential`,
  `auth_method`, `service_category`, `requires_user_credential`, or `provider_config_id`
  fields, so the combination cannot be introduced after create. There is no other admin write
  path to these fields (routes: `backend/src/routes.rs:529-547`).
- Seeded `api-twitter` carries `provider_config_id: Some(...)` with `auth_method: "none"`
  (seed definition `backend/src/services/provider_service.rs:2618-2634`, seed writer
  `:3765-3840`, provider defaults including `tweet.write` near `:590`). It can never become a
  platform row (`is_valid_master_credential_service` rejects the provider link); platform
  variants must be new provider-less rows. This item makes that a checked invariant at write
  time instead of an emergent property of a private predicate.

### Change

**File:** `backend/src/handlers/services.rs`. Add a pure function next to
`derive_http_service_category` (`services.rs:494`):

```rust
/// A stored (master) credential makes a row platform-credentialed. That is
/// only coherent on an internal, provider-less, non-user-credential row —
/// hard error otherwise (V1_SPEC item 3).
fn validate_master_credential_shape(
    auth_method: &str,
    credential_present: bool, // request supplied a non-empty credential
    service_category: &str,
    provider_config_id: Option<&str>,
) -> AppResult<()> {
    if !credential_present {
        return Ok(());
    }
    if auth_method == "none" {
        return Err(AppError::ValidationError(
            "A stored credential requires an auth_method; auth_method \"none\" never injects it".into(),
        ));
    }
    if service_category == "connection" {
        return Err(AppError::ValidationError(
            "A stored master credential cannot be combined with a user-credential (connection) service; \
             create an internal-category row for platform credentials".into(),
        ));
    }
    if provider_config_id.is_some() {
        return Err(AppError::ValidationError(
            "A stored master credential cannot be combined with a provider_config_id; \
             platform rows must be provider-less".into(),
        ));
    }
    Ok(())
}
```

Call it in `create_service` immediately after `service_category` /
`requires_user_credential` are derived (`services.rs:929`), before the credential is encrypted:

```rust
validate_master_credential_shape(
    &auth_method,
    !credential.is_empty() && auth_method != "oidc",
    &service_category,
    None, // create_service never sets provider_config_id (services.rs:1092)
)?;
```

Exclude `oidc`: its `credential_encrypted` is a generated client secret
(`services.rs:895-921`), not an operator-pasted downstream credential, and its category is
forced to `provider` (`services.rs:498-500`), which the user-mode arm would otherwise not
cover. The `provider_config_id` arm is unreachable from `create_service` today; it exists so
any future write path reusing the validator inherits the rule, and so the rule is unit-tested.

**No update-side change:** `UpdateServiceRequest` cannot alter any input of this predicate. Add
no dead code there.

### Tests

Pure unit tests in the existing `mod tests` of `handlers/services.rs` (alongside
`derive_http_service_category_*`, `services.rs:3074+`):

- `master_credential_shape_allows_internal_credentialed_row` —
  `("bearer", true, "internal", None)` → `Ok`.
- `master_credential_shape_rejects_user_credential_category` —
  `("bearer", true, "connection", None)` → `Err`; assert the error is
  `AppError::ValidationError` (400).
- `master_credential_shape_rejects_provider_linked_row` —
  `("bearer", true, "internal", Some("prov-id"))` → `Err`.
- `master_credential_shape_rejects_credential_without_auth` —
  `("none", true, "internal", None)` → `Err`.
- `master_credential_shape_ignores_absent_credential` — `("bearer", false, "connection", None)`
  and `("none", false, "internal", None)` → `Ok` (BYOK catalog rows unaffected).

### Reviewer checklist

- Re-read `UpdateServiceRequest` (`services.rs:248-310`) and confirm it still has none of the
  five predicate inputs; if a future PR adds `credential` to update, the validator must be
  called there too.
- Confirm seeding is unaffected: seeds bypass this handler and write
  `encrypt(b"")` + `auth_method` from the seed table (`provider_service.rs:3744` onward);
  provider-linked seeds remain non-master via `master_credential_required` and the
  `provider_config_id` check in `is_valid_master_credential_service`.
- Confirm the frontend cannot hit the new 400 in normal flows: the admin create-service form
  schema has no `credential` field at all (`frontend/src/schemas/services.ts:148-170`), so only
  API/CLI callers can supply one.

---

## Item 4 — Stop the overlay sync resurrecting deactivated endpoints; trim the overlays

### Current behaviour

- `upsert_one_endpoint` (`backend/src/services/service_endpoint_service.rs:320-410`)
  force-sets `"is_active": true` in its update branch (`:336`) and returns `is_active: true`
  (`:406`). It serves both `bulk_upsert_endpoints` (`:263`, admin discover/reconcile) and
  `upsert_endpoints_additive` (`:304`, used by the startup overlay sync
  `backend/src/services/catalog_spec_sync.rs:58` and the zero-row spec-URL discovery `:184`).
  Result: the startup sync reactivates overlay-named endpoints on every boot; an operator
  cannot keep one deactivated.
- Shipped overlays advertise operations the platform surface must never expose (operation
  inventory verified from `backend/specs/catalog/*.openapi.json`):
  - `twitter.openapi.json`: `GET /users/me` (get_me), `GET /tweets/search/recent`
    (search_recent_tweets), `POST /tweets` (create_tweet), `GET /users/by/username/{username}`,
    `GET /users/{id}/tweets`, `DELETE /tweets/{id}` (delete_tweet)
  - `firecrawl.openapi.json`: `POST /v2/agent`, `GET /v2/agent/{id}`, `POST /v2/scrape`,
    `POST /v2/search`, `POST /v2/map` (map_site)
  - `elevenlabs.openapi.json`: text_to_speech, text_to_speech_stream, `GET /v1/voices`,
    `GET /v1/models`, and six `convai` operations (list_convai_agents, create_convai_agent,
    get_convai_agent, get_convai_signed_url, list_convai_conversations,
    get_convai_conversation)

### Change A — sync respects operator deactivation

**File:** `backend/src/services/service_endpoint_service.rs`.

```rust
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum EndpointSyncActivation {
    /// Admin-initiated reconcile: re-adding an endpoint reactivates it.
    ForceActive,
    /// Background/startup sync: never flip an operator's is_active choice.
    PreserveExisting,
}
```

- `upsert_one_endpoint(coll, service_id, input, now, activation)`:
  - update branch: with `ForceActive`, keep `"is_active": true` in `set_doc` (`:336`);
    with `PreserveExisting`, omit `is_active` from `set_doc` entirely and set the returned
    struct's `is_active` to `existing.is_active` (currently hardcoded `true` at `:406`).
  - insert branch: unchanged (new endpoints are created active) for both modes.
- `bulk_upsert_endpoints` passes `ForceActive` (admin discover-endpoints semantics unchanged,
  including its soft-delete-of-absent-names step `:280-293`).
- `upsert_endpoints_additive` gains no new parameter for callers; it passes `PreserveExisting`
  internally. Both its callers (`catalog_spec_sync.rs:58` startup overlay sync, `:184` zero-row
  spec discovery) want that mode — the second only ever runs against zero rows, so the mode is
  inert there but correct.

### Change B — trim the shipped overlays

Delete these operations (the whole path item when every method under it is dropped) from
`backend/specs/catalog/`:

- `twitter.openapi.json`: `POST /tweets` (create_tweet), `GET /users/me` (get_me).
- `firecrawl.openapi.json`: `POST /v2/agent`, `GET /v2/agent/{id}`, `POST /v2/map`.
- `elevenlabs.openapi.json`: `GET /v1/voices` and all six `convai` operations.

Retained on purpose — record, do not "fix":

- twitter `DELETE /tweets/{id}`, `GET /users/by/username/{username}`, `GET /users/{id}/tweets`;
  elevenlabs `text_to_speech`, `text_to_speech_stream`, `GET /v1/models`; firecrawl `scrape`,
  `search`. The overlays also feed **BYOK** catalog services where users act under their own
  credential; the platform X row below never mounts the overlay (its endpoint list is
  hand-curated), so retained-but-unused entries cannot leak into the platform menu.
- Consequence, accepted: fresh deployments' BYOK `api-twitter` loses the create_tweet / get_me
  MCP tools. Already-deployed databases keep their existing rows (the additive sync never
  deletes), so no production BYOK capability disappears on deploy.
- Existing deployments that want the trimmed rows gone from the *platform-relevant* templates
  deactivate them once via `PUT /api/v1/services/{service_id}/endpoints/{endpoint_id}` with
  `{"is_active": false}` (`UpdateEndpointRequest.is_active`,
  `backend/src/handlers/endpoints.rs:50`); with change A they now stay deactivated across
  restarts. MCP publication already filters `is_active: true`
  (`mcp_service.rs:1068-1076`), so deactivation removes the tool from every menu.

Update any unit tests that assert overlay operation counts or names
(`catalog_spec_registry` / `catalog_spec_sync` test modules) — run
`cargo test -p nyxid catalog_spec` and fix what the trim breaks. The drift workflow
(`.github/workflows/catalog-spec-drift.yml`, `scripts/check-catalog-spec-drift.py`) verifies
that overlay operations still exist upstream; removing operations cannot fail it.

### Tests

DB-backed, in `catalog_spec_sync.rs` or `service_endpoint_service.rs` test module,
`connect_test_database` pattern:

- `additive_sync_preserves_operator_deactivation` — seed a system service row whose slug has an
  overlay, run `sync_seeded_service_endpoints`, flip one produced endpoint to
  `is_active: false` directly in the collection, run `sync_seeded_service_endpoints` again.
  Assert the endpoint is still `is_active: false` and that its `updated_at`/definition fields
  were still refreshed from the overlay (definition updates and activation are independent).
- `bulk_upsert_reactivates_on_admin_reconcile` — deactivate an endpoint, call
  `bulk_upsert_endpoints` including its name. Assert `is_active: true` (admin semantics
  unchanged).
- `preserve_existing_returns_stored_activation` — assert the `ServiceEndpoint` returned by the
  additive path for a deactivated row reports `is_active: false` (guards the `:406` return-value
  bug class).
- `trimmed_overlays_exclude_retired_operations` — pure test: parse the three embedded overlays
  via the same path the sync uses and assert none of the retired operation names
  (`create_tweet`, `get_me`, `agent`, `agent_status`, `map_site`, `list_voices`, every
  `*convai*` name) appears, and that the retained names still do.

### Reviewer checklist

- Confirm no third caller of `upsert_one_endpoint` appeared; confirm `bulk_upsert_endpoints`
  soft-delete step is untouched.
- Confirm the returned struct in `PreserveExisting` mode reflects the stored `is_active` —
  callers log/serialize it.
- Confirm MCP and catalog endpoint listings filter on `is_active` (MCP: `mcp_service.rs:1068`;
  catalog `/{slug}/endpoints` and admin list — spot-check their queries) so a deactivated row
  is invisible everywhere, not just unexecutable.

---

## Item 5 — Per-user rate limit on platform credentials

### Current behaviour

- The per-agent limiter keys on API-key id and runs only when the key carries an explicit
  limit: `check_agent_rate_limit` / `check_agent_rate_limit_raw`
  (`backend/src/mw/rate_limit.rs:462-497`) no-op when `api_key_id` or `rate_limit_per_second`
  is `None`. Browser sessions have neither, so they bypass it entirely. Call sites:
  `handlers/proxy.rs:1755`, `handlers/mcp_transport.rs:759,881,954`,
  `handlers/llm_gateway.rs:240,568`.
- Nothing limits how much of a shared platform credential one user consumes.

### Change

A per-`(service, user)` token bucket enforced at the same choke point as item 2, so every
actor-addressed transport is covered once.

**File:** `backend/src/mw/rate_limit.rs`:

```rust
/// Per-(platform service, user) token bucket guarding shared master
/// credentials. Wraps PerAgentRateLimiter's keyed bucket with fixed limits.
pub struct PlatformUserRateLimiter {
    inner: PerAgentRateLimiter,
    per_second: u32,
    burst: u32,
}

impl PlatformUserRateLimiter {
    pub fn new(per_second: u32, burst: u32) -> Self { ... }
    /// false = throttled. Key is "{service_id}:{user_id}".
    pub fn check(&self, service_id: &str, user_id: &str) -> bool {
        self.inner.check(&format!("{service_id}:{user_id}"), self.per_second, self.burst)
    }
    pub fn cleanup(&self) { self.inner.cleanup() }
}

static PLATFORM_USER_LIMITER: OnceLock<PlatformUserRateLimiter> = OnceLock::new();

/// Install the process-wide limiter. per_second == 0 disables limiting.
/// Called once from main.rs at startup; second call is a no-op.
pub fn init_platform_user_rate_limiter(per_second: u32, burst: u32);
pub fn platform_user_rate_limiter() -> Option<&'static PlatformUserRateLimiter>;
pub fn cleanup_platform_user_rate_limiter();

/// Enforcement seam: unit-testable with an explicit limiter.
pub fn enforce_platform_user_limit(
    limiter: Option<&PlatformUserRateLimiter>,
    service_id: &str,
    user_id: &str,
) -> Result<(), crate::errors::AppError> {
    if let Some(limiter) = limiter
        && !limiter.check(service_id, user_id)
    {
        tracing::warn!(service_id, "Platform per-user rate limit exceeded");
        return Err(crate::errors::AppError::RateLimited);
    }
    Ok(())
}
```

`init_platform_user_rate_limiter(0, _)` leaves the `OnceLock` empty (disabled). Reuse
`PerAgentRateLimiter` (`rate_limit.rs:222-258`) unchanged — it is already a generic keyed
bucket taking rate/burst per call. Do not log `user_id` at warn (keep limiter logs
id-of-service only; the audit event, if any, carries attribution).

**Enforcement:** in `authorize_master_credential` (`proxy_service.rs:149`), after the item-2
policy check and before `Ok(...)`:

```rust
crate::mw::rate_limit::enforce_platform_user_limit(
    crate::mw::rate_limit::platform_user_rate_limiter(),
    &service.id,
    &actor.user_id,
)?;
```

(`EffectiveActor.user_id` is module-visible — same module.) The server-chosen gate is not
limited: it has no actor, and its surfaces (assistant direct) carry their own controls. The
anonymous/public proxy is untouched — it already has per-IP limits and daily quotas.

**Config:** `backend/src/config.rs` gains
`PLATFORM_SERVICE_RATE_LIMIT_PER_SECOND` (u32, default **2**) and
`PLATFORM_SERVICE_RATE_LIMIT_BURST` (u32, default **10**), `0` disables. Wire
`init_platform_user_rate_limiter(cfg.platform_service_rate_limit_per_second,
cfg.platform_service_rate_limit_burst)` in `main.rs` where the state limiters are built
(near `main.rs:710`), and add `cleanup_platform_user_rate_limiter()` to the existing limiter
cleanup task (`main.rs:815`). Document both variables in `docs/ENV.md` and the CLAUDE.md env
block.

Error surface: `AppError::RateLimited` (HTTP 429), the same variant the per-agent limiter uses
— callers already handle it.

### Tests

- `platform_user_buckets_are_isolated` (`rate_limit.rs` unit): limiter `new(1, 2)`; user A
  exhausts burst on service S (third `check("S","A")` returns false); assert
  `check("S","B")` is still true and `check("T","A")` is still true.
- `enforce_platform_user_limit_maps_to_rate_limited` (`rate_limit.rs` unit): with a `new(1, 1)`
  limiter, second call returns `Err(AppError::RateLimited)`; with `None` limiter, always `Ok`.
- `master_credential_rate_limit_uninitialized_is_unlimited` (`proxy_service.rs`, DB-backed):
  in the test process the `OnceLock` is never initialized; assert a valid policied master row
  authorizes repeatedly without throttling. (Do **not** initialize the global in tests — it is
  process-wide and would poison parallel tests. The throttle behaviour itself is covered
  through the seam above.)

### Reviewer checklist

- Confirm `init_platform_user_rate_limiter` is called exactly once, before the server starts
  serving, and that no test initializes the global.
- Confirm the limiter key uses `service.id` (UUID), not slug — slugs are operator-editable.
- Confirm the check sits *after* the policy/validity checks so a throttled caller learns
  nothing about row existence ordering (429 only on rows they could otherwise use).
- Trace one call per transport (REST `/proxy`, MCP tool call, `/llm` gateway) into
  `authorize_master_credential` to confirm coverage is real, not asserted.

---

## X search activation runbook

One platform row, one operation, created with credential and policy in the same call so it is
never live-but-unrestricted. Prerequisites: items 2, 3, 4 deployed (item 5 before external
users); the item-2 production audit completed.

### Step 0 — credential

An X **app-only** Bearer token (OAuth 2.0 Application-Only), from a ChronoAI-owned X developer
app used for nothing else. App-only tokens cannot act as a user; that property is what item 3
protects. Do not reuse the `twitter` provider config or `api-twitter` in any way.

### Step 1 — create the row (policy inline)

`POST /api/v1/services` (admin JWT):

```json
{
  "name": "X Search (Platform)",
  "slug": "x-platform-search",
  "description": "Platform-credentialed X recent search. Read-only; operation allowlist enforced.",
  "service_type": "http",
  "base_url": "https://api.x.com/2",
  "auth_method": "bearer",
  "credential": "<X_APP_ONLY_BEARER_TOKEN>",
  "service_category": "internal",
  "visibility": "public",
  "proxy_operation_policy": {
    "rules": [
      { "method": "GET", "path_template": "/tweets/search/recent" }
    ]
  }
}
```

Field-level notes (all verified against `create_service`):

- `auth_method: "bearer"` maps to backend `bearer` (`services.rs:812-820`), default
  `auth_key_name` `Authorization` (`services.rs:824-826`), injected as `Authorization: Bearer`
  (`proxy_service.rs:581`).
- `service_category: "internal"` is accepted for non-none auth (`services.rs:510`) and derives
  `requires_user_credential: false` (`services.rs:929`); `provider_config_id` is `None`
  (`services.rs:1092`). The row therefore satisfies `is_valid_master_credential_service`.
- The policy is normalized/validated at create (`services.rs:1056-1061`). `base_url` carries
  `/2`, so the template is `/tweets/search/recent` — **not** `/2/tweets/search/recent`. The
  externally-documented operation `GET /2/tweets/search/recent` and this rule are the same
  operation.
- After item 3, this body passes validation; the same body with
  `"service_category": "connection"` must 400.

### Step 2 — verify no spec was auto-attached

`create_service` runs docs discovery (`services.rs:928`) and, if a spec URL was found, spawns
endpoint auto-discovery. Check `GET /api/v1/services/{id}`: `openapi_spec_url` must be `null`.
If discovery attached one, clear it (`PUT /api/v1/services/{id}` with
`{"openapi_spec_url": ""}`) and deactivate any endpoint rows it created
(`GET /api/v1/services/{id}/endpoints`, then per-row `PUT ... {"is_active": false}`). The
platform row's menu must be hand-curated, not spec-derived.

### Step 3 — publish exactly the allowed operation

`POST /api/v1/services/{service_id}/endpoints` (admin;
`CreateEndpointRequest`, `handlers/endpoints.rs:21-34`):

```json
{
  "name": "search_recent_tweets",
  "description": "Search X posts from the last 7 days.",
  "method": "GET",
  "path": "/tweets/search/recent",
  "parameters": [
    { "name": "query", "in": "query", "required": true,
      "description": "Search query using X search operators.",
      "schema": { "type": "string" } },
    { "name": "max_results", "in": "query", "required": false,
      "schema": { "type": "integer" } },
    { "name": "next_token", "in": "query", "required": false,
      "schema": { "type": "string" } },
    { "name": "tweet.fields", "in": "query", "required": false,
      "description": "Comma-separated extra tweet fields.",
      "schema": { "type": "string" } }
  ],
  "request_body_required": false,
  "response": { "content_types": ["application/json"], "binary_artifact": false },
  "risk": "read"
}
```

This mirrors the overlay definition (`specs/catalog/twitter.openapi.json`,
`/tweets/search/recent`). `response.binary_artifact: false` + JSON content type + `GET` make
the operation POC-eligible (`tools.rs:363-383`); `risk` serializes snake_case
(`models/service_endpoint.rs:6-11`). The menu now equals the policy: one operation, and MCP
platform publication serves exactly this row set (`mcp_service.rs:1241-1260`).

### Step 4 — verification (staging, then production)

As a normal authenticated user with nothing connected:

1. `GET /api/v1/proxy/{service_id}/tweets/search/recent?query=nyxid&max_results=10` → 200 with
   X JSON. (Slug addressing works too unless the caller owns a same-slug UserService.)
2. `POST /api/v1/proxy/{service_id}/tweets` → 404 `"Service operation not found"` (policy
   denial, `proxy_authorization.rs:246-248`).
3. `GET /api/v1/proxy/{service_id}/users/me` → 404 (same).
4. MCP `tools/list` / `nyx__search_tools`: the service exposes exactly one operation.
5. Temporarily create a second internal row with a credential and **no** policy; any proxied
   call → 404 (item 2 live in the deployed build); delete the row.
6. Burst >10 identical requests inside a second as one user → 429 on the overflow; a second
   user still gets 200 (item 5).
7. Disable is reversible: `PUT /api/v1/services/{id}` `{"is_active": false}` → calls 400/404;
   re-enable restores.

If item 1's review lands on "flag flip": enable `experimental:direct-chat-engine` for a test
account and repeat step 1 through the assistant; the tool call must appear in the response
trace.

---

## Out of scope for v1 (named, with reasons — nothing else is deferred)

- **Item 1 implementation** — pending the in-flight review (section above); both outcomes
  pre-stated. The X activation does not depend on it.
- **Voice (ElevenLabs TTS row), Firecrawl row, Duffel search row** — same mechanism as X;
  activate later by repeating the runbook with their policies
  (`ONBOARDING_CAPABILITIES.md`, "The service configurations"). No code is missing for them.
- **Twilio / phone calls, SMS, Duffel booking/payment, Reddit, constraint DSL** — excluded by
  `ONBOARDING_CAPABILITIES.md` decisions; calls are a separately-designed product.
- **Rule matching for `/llm` gateway traffic** — the gateway constructs requests itself and
  only the item-2 presence check applies there (architecture fact 3). Platform rows in v1 (X
  search) are not LLM rows, so no v1 surface depends on gateway-side rule matching. Bringing
  the gateway under `authorize_proxy_operation` is follow-up work if an LLM-shaped platform
  row ever carries a selective (non-empty, non-covering) policy.
- **Per-user limiting of server-chosen assistant traffic** — no actor at that gate; the
  assistant surface has its own controls. Revisit if assistant-driven platform usage becomes
  unmetered in practice.
- **MCP `executable` flag accuracy for policy-less master rows** — such rows list as
  `executable: true` (`mcp_service.rs:924`) but deny at call after item 2. V1 ships no such
  row (the runbook forbids it); tightening the flag is cosmetic follow-up.

## Definition of done

- Items 2–5 implemented exactly as above; `cargo check` and `cargo clippy --all-targets` clean
  locally; full suite green in CI (replica-set run).
- Every named test exists with the stated assertion; the two updated legacy tests still assert
  their original subject.
- The item-2 production audit executed and its outcome recorded (which rows, which policy
  decision) before the deploy that contains item 2.
- X row live in production via the runbook, all seven verification steps recorded.
- Item 1 marked pending in the PR description with a link to the review outcome when it lands.
