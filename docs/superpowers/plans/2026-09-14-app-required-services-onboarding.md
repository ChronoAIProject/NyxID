# App-Required Services Onboarding (design plan v2, not implemented)

Status: DRAFT v2, 2026-09-14. Revised after an adversarial review by Astra
(Codex gpt-6-astra, xhigh): 30 pre-plan attacks, then 6 BLOCKER + 17 MAJOR
findings against v1. All blockers are addressed below; the v1 -> v2 diff is in
section 12. Nothing here is built.

## 0. Rollout gate (decided 2026-09-14: internal-only for rapid prototyping)

The feature ships dark and is enabled only for developer apps owned by our
own org. Abuse potential if opened publicly (apps forcing users through
credential collection, probe amplification, brand impersonation) is high, so
the gate is layered the same way the broker rollout is
(`BROKER_REQUIRE_SENDER_CONSTRAINT` / `BROKER_REQUIRE_ADMIN_CAPABILITY`: env
default, MongoDB override set by platform admins, replicas refresh on the
existing short interval).

```
APP_CONNECT_ROLLOUT=disabled            # disabled | allowlist | public ; default disabled
APP_CONNECT_ALLOWED_ORG_IDS=            # comma-separated org user ids honored in allowlist mode
```

- **Per-app capability**: `OauthClient.app_connect_capability_enabled: bool`,
  settable only by platform admins (same posture as
  `broker_capability_enabled` under `BROKER_REQUIRE_ADMIN_CAPABILITY`). Never
  self-grantable by the developer app owner, never via DCR scope.
- **Effective check** (`app_connect_rollout::is_enabled_for(client)`):
  `rollout != disabled` AND `client.app_connect_capability_enabled` AND
  (`rollout == public` OR `client.created_by` resolves to an org in
  `APP_CONNECT_ALLOWED_ORG_IDS`). Evaluated at manifest publish, at
  `/oauth/authorize` before any gate logic, at App Connect Link creation, at
  `/app-requirements/status`, and at the hosted page context mint. When the
  check fails every one of those behaves exactly as today: authorize ignores
  the stored manifest, the app-facing routes are not-found-shaped, and the
  developer UI hides the Requirements and White-labeling cards.
- **Kill switch**: flipping the DB override to `disabled` stops new links and
  gating immediately; in-flight links finish or expire but no new probes run.
- **Audit**: `app_connect_capability_granted` / `_revoked` and
  `app_connect_rollout_changed` metadata-only events, actor = platform admin.
- **Admin UI**: Admin -> OAuth Clients -> App Connect Rollout Policy (mirrors
  Broker Rollout Policy) plus a per-client capability toggle.
- `public` mode is not a v1 target. Opening it requires a separate review
  that revisits Astra's finding 13 (branding as endorsement) and finding 19
  (probe DoS across many apps), plus per-app probe quotas.

## 1. Problem

Apps that use NyxID as their credential backend (ORNN, CMA, Aevatar, heca)
need a guarantee that a user has certain services connected **and working**
before the app can proceed. Example: CMA needs (a) at least one LLM provider,
(b) GitHub connected with the user's own GitHub OAuth authorization, (c) ORNN.

What exists today, and why it is not enough:

- `OauthClient.default_service_catalog_slugs` is a consent-time preselection
  hint. Unmatched slugs render as prose with no connect affordance
  (`frontend/src/pages/oauth-consent.tsx:446-456`).
- RFC 8707 `resource` fails closed with `RequiredServiceNotConnected` (3007)
  when the user has no matching service
  (`services/oauth_resource_service.rs:342`). Error page, not a connect flow.
- Hosted Connect Links are single-service, post-login, orchestrated by the app
  one at a time, and their app-bound callback must pass
  `oauth_service::validate_client` (`services/connect_link_service.rs:1277`).
- "Connected" everywhere means "we hold bytes with status=active". Nothing on
  the server probes a downstream except `cloud_credential_verify::verify_aws_sigv4`.
  The per-service probe recipes live in the frontend
  (`frontend/src/lib/proxy-probe.ts`) and there they test the **Agent Key**,
  not the downstream credential (the module says so explicitly).
- No "LLM provider" category: `/llm` uses a `^llm-` slug regex plus
  `provider_config_id != null` (`services/llm_gateway_service.rs:327`).
- `OauthClient` has no branding fields; the login page is app-agnostic.
- `proxy_service::forward_request_with_extra_outbound_headers` carries an open
  TODO(SEC-H1): it does not re-validate the resolved IP at send time.

## 2. Goals and non-goals

Goals

1. An app publishes an immutable, versioned **requirement manifest**. NyxID
   enforces it per user, per manifest version.
2. First login from App A: an app-branded Connect Link page (App A logo and
   title, "Secured by NyxID" badge, login embedded), then a checklist that
   walks the user through only what is missing, then the standard OAuth
   redirect with a code (success) or a correctly typed OAuth error.
3. Returning users hit the same app link. NyxID evaluates **local** state and
   fresh evidence; stale evidence sends the user to the hosted validation
   step; fully met users keep today's silent-SSO path.
4. Already-authenticated apps can query status and open a **repair session**
   without restarting login.
5. Satisfying a requirement binds the chosen concrete services into the
   consent grant (`allowed_service_ids`) through a server-authoritative
   channel. No new grant type.
6. Every validation result is bounded evidence with a validator id/version,
   an execution-authority digest, and an expiry. It never replaces the
   proxy's own execution-time checks.

Non-goals for v1

- Pure Agent Key clients (no browser authorize). The session engine keeps a
  second entrance for them, but the client-binding contract is a later phase.
- Editing an existing consent in place. Widening still needs interactive
  consent.
- Ongoing readiness webhooks for reused connections. Apps poll status.
- Probe-driven global credential death. Refresh-driven death stays as is.
- Catalog tags as authorization selectors.

## 3. User flow

### 3.1 First visit from App A

```
App A --302--> /oauth/authorize?client_id=A&redirect_uri=...&scope=...&state=...
                 |
                 | unauthenticated -> /connect/app/start/{ctx}  (app-branded shell,
                 |                    not the generic /login page)
                 v
   [App-branded shell]   (Composio Connect Link style handoff)
        +---------------------------------------------------------+
        |  [App A logo]  App A                                     |
        |  App A uses NyxID to securely store the accounts it      |
        |  connects on your behalf. Sign in or create a NyxID      |
        |  account to continue.                                    |
        |                                                          |
        |  [ email / password / Google / GitHub / device login ]   |
        |                                                          |
        |  Secured by NyxID  ·  app-a.example.com                  |
        +---------------------------------------------------------+
                 name + logo come from a short-lived, server-minted
                 authorization context (client active, redirect_uri valid);
                 the same shell hosts the checklist and consent steps so
                 the user never "leaves App A" for an unexplained NyxID page.
                 |
                 v
   authorize_inner: LOCAL evaluation only (no provider I/O)
                 |-- every required item has fresh positive evidence and the
                 |   stored consent already covers those services
                 |     -> today's silent redirect with code
                 |
                 |-- prompt=none and anything not locally satisfiable
                 |     -> OIDC interaction_required, no session created
                 |
                 |-- otherwise -> create AppConnectLink (origin =
                 |   authorize, holds validated authorize params) and
                 |   302 -> /connect/app/{app_connect_link_id}#t=<one-time capability>
                 v
   [Connect Link page]  "App A needs these to continue"
        (checks run only on an explicit click: Connect, Re-check, Continue;
         nothing is probed on page load)

        [x] LLM provider   Anthropic  verified 40s ago              [Change]
        [ ] GitHub         Not connected                            [Connect]
        [~] ORNN           Checking...                              [Re-check]
        [ ] Google Drive   Connected, missing scope drive.readonly  [Reauthorize]
        ---------------------------------------------------------------
        [Continue to App A]  (enabled only when all non-optional items are Met)
        [Not now]            -> cancelled

        Connect     -> child ConnectLink whose return target is the parent
                       session (server-derived, not app callback validated).
        Reauthorize -> child ConnectLink in reauthorize mode carrying the
                       required downstream scopes (existing reauth machinery).
        Change      -> pick among the user's eligible services for an any-of
                       item, then validate the pick.
        Re-check    -> one probe, rate-limited, joins an in-flight attempt.
                 |
                 v  Continue  (link -> ReadyForConsent)
   /oauth-consent  with a fresh short-lived consent token that embeds
                  app_connect_result_id; the selected services render as
                  "Required by App A - verified" and cannot be deselected.
                 |
                 v  Allow -> server loads the result, unions its mandatory
                            ids with RFC 8707 resources, rejects omissions,
                            rechecks local authority, issues the code.
                            link -> Completed (only here).
   redirect_uri?code=...&state=...
```

### 3.2 Returning visit

Same link. `authorize_inner` re-runs the **local** evaluator. Positive
evidence newer than the gate window (60 s) is accepted. Older evidence sends
the user to the onboarding page, which runs bounded asynchronous checks with
the user's prior permission on record; if everything passes the page
continues automatically to consent (already granted -> straight to the code).
A dead credential (refresh failed, prior probe rejected) opens the checklist
with only that item unmet.

### 3.3 Failure and abandonment

Outcomes go to the `redirect_uri` that `validate_client` accepted at
authorize time and that is stored in the server-side session. Nothing on the
onboarding page supplies a callback.

| Outcome | OAuth `error` | `nyx_connect_status` |
|---|---|---|
| user clicked Not now, or denied consent | `access_denied` | `cancelled` |
| a required item is unsatisfiable (catalog service inactive, org policy denies, no connect method) | `access_denied` | `failed` + `nyx_connect_reason=<code>` |
| provider or NyxID could not check within budget and the user chose "Try later" | `temporarily_unavailable` | `unavailable` |
| authorize transaction expired (user came back after TTL) | `invalid_request` | `expired` |
| `prompt=none` and gate not locally satisfiable | `interaction_required` | (none) |

`app_connect_link_id` and `state` are always included. Apps must
correlate `state` before interpreting the vendored parameters. A closed tab
produces no redirect; the sweep only expires the session and (later phase)
emits a webhook. Apps that need to know must poll status.

### 3.4 App-side status and repair (authenticated app, no login restart)

With a user access token bound to `client_id`:

- `GET /api/v1/app-requirements/status` returns local evaluation only:
  `requirements_version`, `result_id`, and per requirement
  `{state, user_service_id, resource_uri, owner_id, validated_at, valid_until, credential_health, granted_to_caller}`.
  `granted_to_caller` is computed against the token's own
  `allowed_service_ids`, so the app cannot mistake "Met for the user" for
  "usable with this token".
- `POST /api/v1/app-connect-links` creates a **repair session**
  (origin = app) bound to the verified client, user, manifest version,
  caller-supplied `state`, and a callback validated by `validate_client`.
  The app sends the user to the returned hosted URL. Repair completion means
  "connections satisfied"; it does not issue tokens. If the newly satisfied
  services are outside the app's stored grant, the terminal callback carries
  `grant_update_required=true` and the app re-runs authorize with
  `prompt=consent` (RFC 8707 resources for the new services), which is the
  existing widening rule.

## 4. Data model

### 4.1 Manifest: `app_requirement_manifests` (new collection, immutable rows)

```rust
pub struct AppRequirementManifest {
    pub id: String,                 // uuid
    pub oauth_client_id: String,
    pub version: u32,               // monotonic per client
    pub enforcement: Enforcement,   // Gate | Advise
    pub requirements: Vec<ServiceRequirement>,   // max 25
    pub compiled: CompiledManifest, // frozen catalog resolution at publish time
    pub published_by: String,
    pub published_at: DateTime<Utc>,
}

pub struct ServiceRequirement {
    pub id: String,                 // [a-z0-9_-]{1,32}, stable across versions
    pub label: String,
    pub any_of_catalog_slugs: Vec<String>,   // 1..=25; a single slug = exact
    pub any_of_catalog_prefix: Option<String>, // sugar, e.g. "llm-"; expanded to slugs at publish
    pub owner_policy: OwnerPolicy,           // PersonalOnly | PersonalOrOrgAllowed
    pub accepted_credential_types: Vec<String>, // e.g. ["oauth2"]; empty = any user credential
    pub allow_master_credential: bool,       // internal/platform-credentialed services
    pub allow_no_credential: bool,           // identity-forwarding services (ORNN case)
    pub required_downstream_scopes: Vec<String>, // OAuth providers only
    pub validator: ValidatorSelection,       // Profile { id } | StoredOnly
    pub optional: bool,
}

pub struct CompiledManifest {
    // slug -> catalog service id, resolved when published; unknown slugs
    // rejected; category "provider" rejected; inactive rejected. A prefix
    // is expanded here to the active seeded slugs at publish time, so the
    // membership is frozen per version (new llm-* seeds join on republish).
    pub catalog_service_ids: BTreeMap<String, String>,
    pub validator_versions: BTreeMap<String, u32>,   // profile id -> version frozen
}
```

`OauthClient` gains `current_manifest_version: Option<u32>` and the branding
fields in 4.5. Publishing a new version never alters running sessions or
already-issued tokens; it is a setup gate, not a revocation mechanism.

CMA's example compiles to:

```
llm:    any_of_prefix "llm-"  (expands to the active llm-* slugs at publish)
        owner PersonalOrOrgAllowed, validator Profile{llm_models_v1}
github: any_of [api-github], owner PersonalOnly, accepted ["oauth2"],
        allow_master_credential=false, scopes [repo], validator Profile{github_user_v1}
ornn:   any_of [ornn-api], allow_no_credential=true, validator StoredOnly
```

"The user's own GitHub authorization" is expressed as personal ownership +
`oauth2` + no master credential. It deliberately does **not** reject
`credential_source = "platform"`: that field records which OAuth *app*
obtained the grant (`models/user_api_key.rs:53-66`); a token obtained through
NyxID's shared GitHub app is still the user's own authorization.

### 4.2 Validator profiles (code-owned registry, not admin-authored)

```rust
pub struct ValidatorProfile {
    pub id: &'static str,             // "github_user_v1", "llm_models_v1", "slack_auth_test_v1", ...
    pub version: u32,
    pub catalog_slugs: &'static [&'static str],   // which seeds it applies to
    pub method: Method,               // GET or POST as the provider documents
    pub target: ProbeTarget,          // Relative(path) | AbsoluteAllowlisted(url)
    pub body: Option<&'static str>,   // fixed, no templating
    pub max_body_bytes: usize,        // bounded parse
    pub classify: fn(&ProbeResponse) -> ValidationOutcome,   // reviewed per provider
    pub claim: &'static str,          // what success proves, shown to users/apps
    pub billable: bool,               // v1: all false
}

pub enum ValidationOutcome {
    Authenticated,        // the documented identity/readiness contract was met
    PermissionDenied,     // valid credential, insufficient permission/scope/SSO
    CredentialRejected,   // documented invalid/revoked/expired credential signal
    ConfigurationError,   // request assembly problem (e.g. Twitch Client-Id mismatch)
    BillingBlocked,
    RateLimited { retry_after: Option<Duration> },
    TransportUnknown,     // timeout, 5xx, connection error
    Unsupported,          // no profile for this service
}
```

v1 profiles and their classification rules (each backed by provider docs and
recorded fixtures):

| Profile | Services | Rule |
|---|---|---|
| `github_user_v1` | api-github, api-github-pat | GET `/user`; 200 with `login` -> Authenticated; 401 -> CredentialRejected; 403/404 with `X-GitHub-SSO` -> PermissionDenied; other 403 -> RateLimited/PermissionDenied by body |
| `llm_models_v1` | llm-anthropic, llm-openai, llm-google-ai, llm-mistral, llm-cohere, llm-deepseek | GET `/models`; 200 with a non-empty list -> Authenticated; 401 -> CredentialRejected; 403 -> PermissionDenied; Google 400 `API_KEY_INVALID` -> CredentialRejected; billing-shaped bodies -> BillingBlocked |
| `openrouter_key_v1` | llm-openrouter | GET `/key` (its `/models` is public); 200 with key data -> Authenticated |
| `slack_auth_test_v1` | api-slack, api-slack-bot | POST `auth.test`; `ok:true` -> Authenticated; `invalid_auth`, `token_revoked`, `token_expired`, `account_inactive` -> CredentialRejected; `missing_scope` -> PermissionDenied |
| `lark_user_info_v1` | api-lark, api-feishu | GET `authen/v1/user_info`; `code == 0` -> Authenticated; documented auth codes -> CredentialRejected; other codes -> ConfigurationError/TransportUnknown |
| `telegram_get_me_v1` | api-telegram-bot | GET `getMe`; `ok:true` -> Authenticated; `ok:false` with 401 -> CredentialRejected; honor `retry_after` |
| `twitch_users_v1` | api-twitch | GET `users`; 200 -> Authenticated; 401 with Client-Id message -> ConfigurationError; 401 otherwise -> CredentialRejected |

Phase-0 amendments accepted on 2026-09-14:

- `llm-cohere` uses the absolute allow-listed target
  `https://api.cohere.com/v1/models`; its shared seed base remains `/v2`.
  Other `llm_models_v1` targets remain relative `models`. The Cohere success
  fixture is explicitly doc-derived from the documented `models` array;
  an authenticated capture is required on the phase-2 fixture checklist.
- Direct validation accepts seeded origins and base paths. Changing a base
  path cannot turn a public endpoint into authentication evidence.
- Node requests add `follow_redirects`, defaulting to `true`. Validation
  sets it to `false` and requires the existing status-update capabilities
  to advertise `no_redirect_proxy`; otherwise return `Unsupported` with
  `node_agent_upgrade_required` without sending a probe. Node DNS belongs
  to the owner's network boundary and is not pinned by NyxID. Server-side
  deadlines and received-body limits still apply. Offline nodes return
  `TransportUnknown`; `node_managed` and `ssh_certificate` are unsupported.

Everything else (openai-codex, openclaw, firecrawl, tiktok, lark-bot,
feishu-bot, aws-cost-explorer, google-cloud, custom endpoints) is
`Unsupported` in v1 and can satisfy a requirement only with
`validator: StoredOnly`, which reports `evidence: stored`.

Probes use a dedicated **validation transport**: DNS-pinned at each attempt
(same pattern as `api_docs_service::fetch_spec_json`), redirects disabled,
4 s deadline, decoded body capped by the profile, URL credentials rejected,
path composed only from the profile constant, provider origin allow-listed
for seeded services. Custom user endpoints are validated only through
`StoredOnly` in v1. Node-routed services are validated by dispatching the
same profile request through the node path; an offline node is
`TransportUnknown`.

### 4.3 Evidence: `service_validation_records` (new collection)

```rust
pub struct ServiceValidationRecord {
    pub id: String,
    pub user_service_id: String,
    pub owner_id: String,
    pub validator_id: String,
    pub validator_version: u32,
    pub execution_authority_digest: String, // from services/execution_authority.rs
    pub api_key_id: Option<String>,
    pub credential_epoch: Option<i64>,
    pub attempt_id: String,                 // fence; a stale attempt cannot overwrite
    pub outcome: ValidationOutcome,
    pub checked_at: DateTime<Utc>,
    pub valid_until: DateTime<Utc>,         // 60 s for gates, 5 min for display
    pub caller_context: CallerContext,      // Human { session } | App { client_id }
}
```

One record per (user_service_id, validator_id) is kept current via CAS on
`attempt_id`; a losing CAS discards its observation. Evidence for no-key,
master-credential, and node-routed services is stored against the actual
execution target, which a `UserApiKey`-level field could not express.
`UserApiKey.status` is **never** written by validation in v1. The existing
coordinated refresh path retains its own lifecycle policy. Phase 0 records
also track `completed` for pending attempts, a metadata-only `reason_code`,
and a digest of encrypted credential material/scopes to invalidate evidence
when OAuth refresh changes material without incrementing `credential_epoch`.
The display window is five minutes; TTL retention extends one day beyond
`valid_until`. Admission and in-flight ownership use MongoDB coordination
leases and slots.

### 4.4 App Connect Link: `app_connect_links` (new collection; distinct from single-service `connect_links`)

```rust
pub struct AppConnectLink {
    pub id: String,
    pub oauth_client_id: String,
    pub user_id: String,                        // bound at creation, after login
    pub manifest_id: String,
    pub manifest_version: u32,
    pub origin: AppConnectOrigin,
    //   Authorize { authorize_params: ValidatedAuthorizeParams, consent_nonce: String }
    //   App       { callback_url: String, state: String }
    pub items: Vec<AppConnectItem>,
    pub status: AppConnectStatus,   // InProgress | ReadyForConsent | Completed | Cancelled | Expired | Failed
    pub capability_hash: String,    // one-time page capability, redeemed once, then cookie-bound
    pub created_at, expires_at, completed_at,
    pub failure_reason: Option<String>,
    // terminal outbox fields identical to ConnectLink (later phase)
}

pub struct AppConnectItem {
    pub requirement_id: String,
    pub state: ItemState,   // Unmet | Connecting | Reauthorizing | Validating | Met | Unknown | Failed | Skipped
    pub connect_link_id: Option<String>,
    pub user_service_id: Option<String>,        // explicit user selection is authoritative
    pub validation_record_id: Option<String>,
    pub attempt_id: Option<String>,
    pub reason_code: Option<String>,
}
```

Rules: `InProgress` and `ReadyForConsent` are non-terminal; TTL 30 min,
extended 15 min per item reaching Met, capped at 2 h. Authorize params are
persisted server-side (the signed consent request TTL of 15 min is not
stretched); a fresh consent token is minted at `ReadyForConsent`. The page
capability is redeemed once into a subject-bound association and scrubbed
from the URL; resume is by authenticated read. Two tabs share one session;
item attempts are fenced by `attempt_id`. Child link completion is a durable
observation the parent reconciles on read, not only an in-process callback.

`ConnectLink` gains `parent_session_id` and `requirement_id`; for such
children the return target is derived server-side from the parent and does
not go through the app callback validator. Children also gain
`reauthorize_user_service_id` + `required_scopes` to drive the existing
connection-specific reauthorization path instead of a second connect with
the same insufficient scopes.

### 4.5 White labeling on `OauthClient`

```rust
pub logo_asset_id: Option<String>,   // uploaded, decoded, size-limited, re-encoded, served by NyxID
pub homepage_url: Option<String>,
pub handoff_blurb: Option<String>,   // <= 160 chars, plain text, shown under the app name
pub branding_revision: u32,          // bumped on any name/logo/homepage/blurb change
pub branding_verified_revision: Option<u32>,   // optional admin mark; adds a "Verified" chip only
```

Decision (Calvin, 2026-09-14): any active registered client gets its name and
logo in the shell. The verified mark is an optional chip, not a gate on
showing the logo. The destination host (or "desktop app registered as App A"
for custom schemes) and the "Secured by NyxID" badge are always rendered and
cannot be customized, which is the same posture as Composio's badge.

## 5. Backend logic

### 5.1 `app_requirements_service` (local evaluator, no provider I/O)

`evaluate_local(db, manifest, user_id, caller) -> RequirementsReport`

Per requirement:

1. Candidates = active `UserService` rows via
   `list_user_services_with_sources` whose `catalog_service_id` is in the
   compiled set, filtered by `owner_policy` (org rows require
   `CredentialSource::Org { allowed: true }`), `accepted_credential_types`,
   `allow_master_credential`, `allow_no_credential`, and
   `required_downstream_scopes` against `UserApiKey.token_scopes`.
   Disabled/tombstoned rows are never auto-enabled; they surface as
   "Disabled, enable to use".
2. Prior explicit selection for (user, client, requirement_id) wins if still
   eligible; otherwise the candidate with the freshest `Authenticated`
   record; otherwise most recently used.
3. State from the latest validation record: fresh `Authenticated` -> Met;
   `StoredOnly` -> Met iff the readiness `ConnectionState` is Connected;
   `CredentialRejected` -> Broken; scale-out to other eligible candidates
   before concluding; expired/no record -> Unknown; no candidate -> Unmet;
   no active catalog candidate -> Unsatisfiable; scope shortfall -> NeedsReauth.
4. Output includes `granted_to_caller` when the caller is an app token.

### 5.2 `service_validation_service` (probe executor)

`validate(state, caller, user_service_id, profile, attempt) -> ServiceValidationRecord`

The human route first uses `resolve_key_read_owner` for GET-equivalent,
not-found-shaped disclosure. It then runs the snapshot under the **actual
caller**, never the owner. Read access shows the key; validation requires
proxy permission (`role.can_proxy()` and the existing `admin_only` rule).
A read-only caller receives the proxy resolver's existing 403 before any
credential materialization, probe, or evidence write. The shared resolver
is unchanged.

1. `read_proxy_authority_snapshot_by_user_service_id` for the read-only
   snapshot and the execution-authority digest; refuse if the caller's own
   grant/node restrictions (for app callers) exclude the service.
2. If the freshest record for (service, profile) is inside its window and
   `!force`, return it. If an attempt is in flight, join it.
3. Materialize credentials through the existing coordinated OAuth refresh
   path (per-key lease, revision compare) so a probe never races the sweep
   or spends a rotating refresh token independently.
4. Send via the validation transport (4.2). Discard the body after the
   profile's bounded parse.
5. Classify with the profile; CAS-write the record fenced by `attempt_id`;
   a losing CAS discards the observation.
6. Never write `UserApiKey.status`. Log and audit metadata only
   (`service_validation_checked`: ids, profile, outcome, http status class).

Admission: one probe per (credential, profile, execution digest) per 60 s;
two concurrent probes per session; 32 active probes per deployment,
coordinated through a MongoDB lease collection, not a process-local
semaphore; 4 s probe deadline; 10 s foreground evaluation round with no
automatic retry inside it; `Retry-After` honored. Billing: a new
`BillingRoutePolicy::Exempt("service_validation")` entry in
`services/billing/route_inventory.rs`, allowed only for profiles with
`billable == false`. Provider rate limits still apply and the UI says so.

### 5.3 `app_connect_link_service`

- `start_from_authorize(client, user, manifest, validated_params, report)`.
- `start_from_app(client, user, manifest, callback, state)` after
  `validate_client(callback)`.
- `connect_item`, `reauthorize_item`, `select_item`, `revalidate_item`:
  each re-checks the live ACL for the specific resource
  (`org_service::resolve_owner_access`) but **never** substitutes org write
  access for ownership of the session. Session access requires the bound
  subject with a human session; API keys, delegated, relay, and
  service-account tokens are rejected before the handler.
- `ready(session)`: all non-optional items Met within a bounded evidence age
  spread -> `ReadyForConsent`; mint the consent token embedding
  `app_connect_result_id` and the selected service ids.
- Terminal transitions are atomic CAS on `status`; a late probe success
  after `Cancelled` is discarded by the attempt fence.
- Sweep: expires sessions past TTL; authorize-origin sessions have no
  redirect to deliver; app-origin sessions get `app_connect_link.expired` in a
  later phase.

### 5.4 `/oauth/authorize` integration

After `validate_authorize_request` and login, before the consent check:

```
if client.current_manifest_version is Some and manifest.enforcement == Gate:
    report = evaluate_local(...)
    if report.all_required_met_fresh:
        mandatory = report.selected_service_ids ∪ rfc8707_resolved_ids
        continue to consent_requires_prompt(mandatory)
    else if prompt == none:
        return interaction_required
    else:
        session = start_from_authorize(...)
        302 -> /connect/app/{link.id}#t=<capability>
```

Consent decision (`authorize_decision`): when the consent token carries
`app_connect_result_id`, load the result, verify it belongs to this user,
client, and manifest version, union its service ids with the RFC 8707
resolution, reject any omission from the form, run
`validate_grantable_service_ids`, recheck client activation and local
resource authority, then issue the code. This replaces the v1 idea of
pushing `required_service_ids` into the consent URL, which is display-only
(`handlers/oauth.rs:807-828` recomputes mandatory ids from resources).

### 5.5 Routes

Developer (existing developer-app router):

- `GET /api/v1/developer/oauth-clients/{client_id}/requirements` (list versions)
- `POST /api/v1/developer/oauth-clients/{client_id}/requirements` (publish new version; validates and compiles)
- `PATCH /api/v1/developer/oauth-clients/{client_id}` accepts `homepage_url`; `POST .../branding/logo` uploads the asset
- Admin: `POST /api/v1/admin/oauth-clients/{client_id}/branding/verify`

Hosted (human-only router, per-IP limits, capability in body after first load):

- `POST /api/v1/app-connect-links/{id}/redeem` (capability -> subject-bound)
- `GET  /api/v1/app-connect-links/{id}`
- `POST .../{id}/items/{requirement_id}/{connect|reauthorize|select|validate}`
- `POST .../{id}/ready`, `POST .../{id}/cancel`

App-facing (developer-app user access token; delegated, relay, service-account rejected):

- `GET  /api/v1/app-requirements/status`
- `POST /api/v1/app-connect-links` (repair session)
- `GET  /api/v1/app-connect-links/{id}` (app-bound: `oauth_client_id` must match the token's `client_id`)

Public:

- `GET /oauth/authorize-context?ctx=<short-lived token>` -> `{ client_name, logo_url?, homepage_url?, verified, destination }`;
  the token is minted by `authorize` when it redirects to login, so the
  login page never trusts `client_id` from the URL.

User-facing:

- `POST /api/v1/keys/{id}/validate` -> `validate` with a human caller
  context. The existing Agent Key verify step is **kept** (it tests the
  key's allowed and denied services, a different property).

### 5.6 Error codes (new block 12000-12010)

`12000 AppRequirementsInvalid` (400), `12001 AppConnectLinkNotFound` (404),
`12002 AppConnectLinkExpired` (410), `12003 AppConnectLinkCompleted` (409),
`12004 AppConnectLinkCancelled` (409), `12005 RequirementNotSatisfiable` (409),
`12006 RequirementNotMet` (409), `12007 ServiceValidationRejected` (422),
`12008 ServiceValidationUnavailable` (503), `12009 ServiceValidationRateLimited` (429),
`12010 AppConnectResultMismatch` (400, consent binding failed).

## 6. Frontend

- `/connect/app/start/$ctx`: the app-branded shell (`ConnectLinkShell`) rendered
  from `/oauth/authorize-context`: app logo, name, handoff blurb, the
  existing `AuthFlow` (email, social, device login) embedded inside it, and
  a fixed "Secured by NyxID · <destination>" footer. On success it continues
  the authorize transaction; the generic `/login` page is untouched.
- `/connect/app/$linkId`: same shell; redeems the fragment capability, then works
  from the session read. Checklist with the states in 4.4 and inline
  connect/reauthorize using the
  connect-link page pieces extracted into `components/connect/`
  (`CredentialForm`, `OAuthSetupForm`, `DeviceCodePanel`, `TerminalPanel`,
  `ConnectShell` are page-private today). 750 ms click throttle; no
  request on mount beyond the session read.
- Consent page: rows from the onboarding result are non-deselectable and
  badged "Required by App A - verified <age>".
- Developer app detail: manifest editor that publishes versions (slug
  picker from the include-all catalog, owner policy, credential types,
  scopes, validator, optional), version history, enforcement switch,
  branding upload with "verification pending".
- `/keys`: "Check connection" calls the server validate route and shows the
  profile's `claim` text.

## 7. SDK and docs

- `@nyxids/oauth-core`: `resource` on `buildAuthorizeUrl` /
  `loginWithRedirect`; `resource` on the token set; `client.requirements.status()`,
  `client.appConnectLinks.create()`; typed `NyxAppConnectError`
  parsed **after** `state` verification (today the SDK throws on `error`
  before checking state, `sdk/oauth-core/src/index.ts:171`).
- `docs/API.md` third-party connector section, developer-apps guide,
  `skills/nyxid/references/oauth-consent.md`, CLAUDE.md rule + error block.

## 8. Phasing and gates

| Phase | Deliverable | Gate to ship |
|---|---|---|
| 0 | validator profiles + validation transport + `service_validation_records` + `POST /keys/{id}/validate` (observation only) | provider fixtures for all 7 profiles; SSRF egress tests; no `UserApiKey.status` writes |
| 1 | manifests (publish/compile), local evaluator, `/app-requirements/status`, repair sessions, hosted page, SDK | cross-user/org access tests; scope-repair round trip; no-key ORNN case |
| 2 | authorize gate + consent result binding + login context + branding upload/verify | forged consent form, `prompt=none`, PAR, token narrowing, stale-probe-after-refresh, cancel-vs-late-probe, app disablement mid-session |
| 3 | onboarding terminal webhooks, Agent Key client binding, CLI parity | explicit app/requirement/selection relationship model |

`Advise` is the only enforcement available until phase 2 ships; publishing a
`Gate` manifest before then is rejected. All phases run under the section 0
rollout gate in `allowlist` mode for our org only; `public` is out of scope.

Vocabulary (aligned with Composio Connect Link so app developers recognize
it): **Connect Link** = hosted page a user completes; **App Connect Link** =
the multi-requirement run described here; **connected account** = the
user-facing name for a `UserService` + credential on the hosted page;
**White labeling** = the developer-app logo/title/blurb settings;
**Secured by NyxID** = the fixed badge; callback carries `status` +
`app_connect_link_id`, matching Composio's `status` + `connected_account_id`
and NyxID's existing `status` + `connect_link_id`.

## 9. Security notes

- Probe targets: profile constants + allow-listed provider origins; DNS
  pinned per attempt; no redirects; bounded body; custom endpoints excluded
  from live validation in v1.
- Probes never change global credential status; they produce fenced,
  expiring evidence.
- Probes run only on an explicit user click on the hosted page (never on
  load, never from the app-facing status route); app-facing status is
  local-only and disclosure-limited to the manifest's services.
- Session access = bound subject + human session; per-resource ACL checked
  separately; org write access never substitutes for session ownership.
- Consent binding is server-side (result id in the signed token, form
  omissions rejected); the consent URL carries display hints only.
- Redirect targets only from the validated authorize params or a
  `validate_client`-checked repair callback; children never use the app
  callback validator for internal returns.
- Login branding from a server-minted context token, immutable uploaded
  assets, verification tied to a branding revision.
- Records, sessions, and audits carry ids, profile ids, outcomes, and
  reason codes only.

## 10. Test matrix (minimum, per Astra finding 23)

Forged consent selection omitting an onboarding service; App B token reading
App A's session; org admin acting on a member's session; stale probe result
landing after a successful refresh; profile version bump invalidating
evidence; execution digest change (endpoint/node/override) invalidating
evidence; app deactivated mid-session; child link cancelled while probe in
flight; `prompt=none` with unmet gate; PAR-initiated authorize through
onboarding; token narrowing after onboarding; failure callback without
matching `state`; no-key ORNN requirement; Slack `ok:false` on HTTP 200;
Lark non-zero `code`; Twitch Client-Id 401; Agent Key denied-service test
still present.

## 11. Open decisions for Calvin

1. Resolved 2026-09-14 (Calvin): LLM providers are the `llm-*` seeds.
   Manifests take `any_of_catalog_prefix: "llm-"`, expanded to a frozen slug
   list at publish. No catalog tags.
2. Resolved 2026-09-14: Composio-style app-branded handoff shell with a
   "Secured by NyxID" badge; verification is an optional chip, not a gate.
3. Resolved 2026-09-14 (Calvin): no separate permission prompt. Checks run
   only on explicit clicks on the hosted page.
4. Resolved 2026-09-14 (Calvin): repair sessions ship in v1.
5. Resolved 2026-09-14 (Calvin): services that need no user OAuth or API key
   (platform-credentialed like `chrono-llm-public`, no-auth, identity
   forwarding) are not onboarding steps. If a manifest names one, it is
   auto-provisioned through the existing `auto_provision_no_auth_services`
   path and shown as "Included", never as a step the user must act on. The
   checklist is only ever OAuth or API-key connections.

Still needed from Calvin: the org id/slug for `APP_CONNECT_ALLOWED_ORG_IDS`,
and whether `ornn-api` should be seeded for local dev.

## 12. What changed from v1 (adversarial review outcome)

Accepted (blockers): server-side consent binding instead of URL hints;
`owner_policy` + credential types + master flag instead of `PersonalByo`;
provider-specific validator profiles with body parsing instead of
status-only recipes; evidence records keyed by service + validator +
execution digest instead of a field on `UserApiKey`; session ownership
never delegated to org write access; dedicated pinned validation transport.

Accepted (majors): two-entrance session engine; no provider I/O inside
authorize and none for `prompt=none`; 60 s gate / 5 min display windows and
the probe budget; typed OAuth errors instead of blanket `access_denied`;
`ReadyForConsent` before `Completed`; scope reauthorize path; no-credential
and master-credential requirement support for ORNN; immutable compiled
manifests; server-persisted authorize params; keep the Agent Key verify UI;
release gates and a test matrix.

Cut from v1: `AnyOfTag` and catalog tag editor; admin-authored probe
recipes; probe-driven credential death; remote logo URLs and the
partnership lockup; readiness webhooks and `requirement_id` on
`connection.expired`; deleting `proxy-probe.ts`.

Overridden by Calvin (2026-09-14): Astra's "avoid app-first branding on the
NyxID login page" finding. The concern is the opposite one, a user arriving
from App A being dropped on an unexplained NyxID page. The shell is
app-first with NyxID as the visible security provider, matching Composio's
Connect Link white-labeling (logo + app title + "Secured by" badge).

One correction to my own brief: GitHub SSO-restricted
orgs answer 403/404 with `X-GitHub-SSO`, not 401.
