# Consolidated Services: Fable adversarial review (pass 1)

Status: read-only design review of the seed design and of
`consolidated-services-flow.md` (25 Sep 2026). Checked against main `1b031c77`.
No code changed. Line references are to that commit.

## 1. Verdict in one paragraph

Ship **one row per callable address** with stateless provider dividers, not a
grouped "service block" hierarchy. A row is something an agent can actually
call today: a configured service slug or a pool slug. The catalog identity is a
sort key and a divider label, never a container with its own status, counts of
"connections", or a family-level "uses now". The inspector is a Sheet with three
fixed tabs, **Route / Activity / Details**, and it is the same component as the
full detail page. Before any of that is truthful, two backend facts must exist
that do not exist today: a per-request record that names the concrete service,
the verified caller and the outcome in one row, and a compact `last_request` /
`last_success` projection on `UserService`. Everything in the draft that reads
"Would use", "checked N seconds ago", "Check: POST /responses", "Use a specific
connection" or "As: application" depends on a resolver and policy store that
are still proposals; those controls must not appear in the first release.

## 2. What the data can and cannot say (evidence)

| Question the UI wants to answer | What exists | Consequence |
| --- | --- | --- |
| Which concrete service handled a request? | `proxy_request` audit stores `service_id` = **catalog id** for catalog-backed rows (`proxy_service.rs:3658`), plus method, path, `response_status`, `acting_client_id`, `connection_id` (`handlers/proxy.rs:4300`). The concrete `user_service_id`, `routed_via` and pool `chosen_user_service_id` are in a **separate** `proxy_routed_via_personal/org` row (`handlers/proxy.rs:599-650, 660-680`). No shared request id between the two rows. | "Who last used *this* service" cannot be computed for catalog-backed rows without a new field. The Agent Keys usage dashboard already keys `top_services` by that catalog id (`handlers/api_keys.rs:867-900`) and so cannot distinguish two OpenAI services. |
| Who was the caller? | `AuditLog.user_id`, `api_key_id`, `api_key_name` (`models/audit_log.rs`), `acting_client_id` in event data. `AuthUser.oauth_client_id` and `api_key_credential_id` exist (`mw/auth.rs:60,84`) but are **not** written to the proxy audit. | Person, agent key and delegated app are attributable from verified auth. Ordinary app tokens and Agent Key child credentials are not. |
| Last use? | `last_used_at` lives on `UserApiKey`, touched fire-and-forget at credential materialization **before dispatch** (`proxy_service.rs:2993-3000`) and also for agent override credentials (`3355-3362`). One credential can back several services (journal fan-out, `SERVICE_HISTORY.md`). | It is "credential last prepared", not "service last used" and not "success". Must be relabelled or hidden. |
| Last success? | Only derivable by scanning audit rows with `response_status < 400` (`handlers/api_keys.rs:862-865`), window clamped to 30 days. `usage_meter` rows carry `actor_user_id`, `api_key_id`, `credential_class`, `billing_owner_id`, `service_id` but only when `BILLING_ENABLED`. | No per-service projection. A list page cannot scan audit per row. |
| Who last changed it, and what? | `service_change_events` journal with per-service sequence, `service_slug` snapshot per event, actor kinds Person/ApiKey/ServiceAccount/App/System, safe field diffs; `created_by` / `last_change` summaries on the row; archived deleted histories (`handlers/service_history.rs:343-379`). | Solid. Readable only when `access.can_write() && access.allows_resource(id)` (`service_history/read.rs:47`), so personal owner or scoped org admin; members and viewers get nothing. |
| Lineage / spin-off? | `UserService.rotation_predecessor_id` points to the previous **UserApiKey**, not a parent service (`handlers/keys.rs:621-629`). `source` values: `user_created`, `auto_provision`, `codex_import`, `channel_onboarding`, `connection`, `telegram_*`; `source_id`, `source_app_id` + resolved `source_app_name`. No service-to-service edge. | There is no spin-off relation to render. Only origin facts and computed relations (same credential, same catalog, pool membership, agent binding). |
| What will the catalog slug do? | Resolver: exact personal slug, then personal pool, then legacy personal guard, then org walk in `primary_org_id` order (`proxy_service.rs:1766-1770`, mirrored by the approval-owner lookup at `2285-2330`); an unusable personal row errors with 400 instead of continuing (`2981-2986`); a missing row falls to the legacy platform master-credential path silently. `find_by_catalog_service_id` is an unordered `find_one` (`user_service_service.rs:788-802`). | The only deterministic statements today: an exact slug row executes itself; a pool executes one enabled active member chosen at request time; a catalog slug with no active row and `platform_key_available` falls to the platform key. |
| Is the credential working? | `status` (`active/expired/revoked/failed/refresh_failed/pending_auth`), OAuth `connection_status` derived from expiry (`unified_key_service.rs:4548`), `credential_missing`, `node_status`. Codex only: `metadata.verification_status` `saved/usable/reconnect_required` with `verified_at`, bound to token version, epoch, service version and endpoint (`codex_connection_service.rs:291-306`, `CODEX_CONNECTION.md`). AWS SigV4 probe at creation is not persisted. | Vocabulary is fixed by data: **Saved**, **Expired/Revoked/Failed**, **Missing**, **Verified <date>** (Codex only), **Not verified** (everything else). Never "Ready". |
| Payer? | `credential_class` and `billing_owner_id` on the meter; `BillingOwnerResolver::resolve_for_execution`; platform-key usage is billed to the acting person even on org-owned rows (CLAUDE.md Rule 5). | Payer must come from the server. Inferring it from the row owner is wrong for platform-bound org services. |
| Pools? | `ServicePool { user_id, slug, strategy: RoundRobin|Weighted, members[{user_service_id, weight, enabled}], rr_counter }`; `resolve_member` increments `rr_counter` on every resolution (`service_pool_service.rs:363-410`). `list_pools` is by owner id only. | A preview must never call `resolve_member`. Member array order already persists, so a `Priority` strategy is a small backend addition. |
| Agent pinning? | `AgentServiceBinding (api_key_id, user_service_id) -> user_api_key_id`; `ApiKey.allowed_service_ids`, `allow_all_services`, `allow_auto_connected_services` (server-expanded at auth time). | Scope membership and credential override are exact client-side facts. The auto-connected expansion is not. |
| Same slug twice? | `/keys` can return a disabled row and an active row with the same slug (`AI_SERVICES_ARCHITECTURE.md:279-282`); known gap 1 (E11000 on re-enable) and gap 2 (tombstone revival zombie). | Rows must be keyed by UUID and the disabled duplicate labelled. |

## 3. The grouping decision, resolved

**Choose: one default row per callable address, sorted by catalog identity,
with stateless provider dividers. "Flat" removes the dividers. No nested
"service block" and no cards-versus-grouped toggle.**

Why the draft's grouped-by-catalog default loses:

1. **Pools break the hierarchy.** A pool can mix members from different
   catalog services. The draft says "real pool routes appear once" but never
   says under which service block. Any answer is either wrong or an "Other"
   bucket, which proves the container is not a real grouping.
2. **Custom services become one-row families.** Every custom service would
   carry a header with "1 route, 1 connection". That is chrome, not
   consolidation.
3. **The block invites the exact claim the user forbade.** "Latest visible
   request" and "resolution for the visible caller" at the family level are
   aggregates over routes with different owners, payers and privacy rules. An
   org member would see Acme's latest caller; the draft's own privacy section
   forbids that.
4. **"Automatic" mode does not exist.** The draft's Route has modes Automatic,
   Direct, Pool. Only Direct and Pool are objects today. A family header is
   the natural place for the Automatic verdict, so the hierarchy will grow one
   before the resolver ships.
5. **The user's actual scan target is the address.** An agent config, a CLI
   call and an MCP tool all name a slug. The list should be scannable by the
   thing that appears in those configs.

What the row-first list keeps from the draft: OpenAI still appears once as a
divider, its three addresses are adjacent, and each row expands to the
existing card. Consolidation is achieved by sort order, not by a container.

## 4. Vocabulary (fewer words than the draft)

The draft renames the catalog capability to "Service" and the `UserService`
row to "Connection". That inverts existing product language: Rule 8 lifecycle
verbs act on a *service*, `/keys/{service_id}/history`, "Deleted service
history", `nyxid service list`, and "connection" already means the OAuth
connection (`connection_id`, `connection_status`, Reconnect, Codex
connection). Users and the CLI would need two dictionaries.

Use instead:

| Term | Meaning | Backed by |
| --- | --- | --- |
| **Service** | A configured row you enable, disable, delete and call by its slug. | `UserService` |
| **Catalog service** | The template a service was created from. Not callable by itself. | `DownstreamService` |
| **Address** | The slug a caller uses. A service has one; a pool has one. | `UserService.slug`, `ServicePool.slug` |
| **Credential** | The stored key or OAuth grant a service executes with. | `UserApiKey` |
| **Pool** | An address that picks one member service per request. | `ServicePool` |
| **Origin** | How a service came to exist. Write-once. | `source`, `source_id`, `source_app_id`, `created_by` |

Drop "Route" as a noun, "Connection" as a noun for services, and "spin-off"
entirely. "Route" survives only as the inspector tab title, meaning "what a
call to this address does".

## 5. Final flow

### 5.1 Services home

```
Services                        [Search]  Owner ▾  Attention ▾  Kind ▾  Grouped|Flat  [+ Add]

OPENAI · catalog llm-openai ─────────────────────────────────────────── 3 addresses
  Address             Owner   Credential                  Transport   Last request              Last change
▸ llm-openai          You     OpenAI key · Saved          Direct      2m · coding-agent · 200   3d · Alice
▸ llm-openai-2        Acme    Acme key · Saved            Direct      1h · you · 200            12 Sep · Ben
▸ llm-openai          You     NyxID platform · Disabled   Direct      —                         5 Sep · you
                              (slug reserved by the active row above)
ANTHROPIC · catalog llm-anthropic ──────────────────────────────────── 1 address
▸ llm-anthropic       You     OAuth · Expired             Direct      Reconnect needed          …
POOLS ────────────────────────────────────────────────────────────────── 1 address
▸ llm-pool            You     2 members · round robin     per request 5m · coding-agent · 200   …
CUSTOM ───────────────────────────────────────────────────────────────── 1 address
▸ internal-api        You     Bearer · Saved              node lab-1 (online)  —                Creator not recorded
```

Rules:

- Divider = catalog name, catalog slug, count. Nothing else. Not clickable.
- **Owner** is who owns the row (You / org name). **Credential** is whose key
  executes (your key / Acme key / NyxID platform). These differ for
  platform-bound rows and must be separate columns.
- **Last request** is the row's `last_request` projection (see 7.1), shown as
  age, verified caller, outcome class. For org rows a member sees only their
  own ("your last request"); an admin sees all. Absent: "No requests since
  <projection start>" or "Not recorded".
- **Last change** is `last_change` when the reader may see it; otherwise the
  column is blank for that row, not "never". Legacy: "Creator not recorded".
- Expanding a row reveals the existing card content unchanged. The card/table
  ViewToggle stays for the mobile-card split; there is no third view.
- A disabled row that shares its slug with an active row is labelled as such.
  Rows are keyed by UUID.
- The `CodexConnectionSection` banner moves into the Codex-linked row's Route
  tab as a "Verified <date>" line. It is per-credential evidence, not page
  state.
- Search covers address, label, owner, catalog name. Caller search comes only
  after the projection exists.
- **Attention** filter is defined exactly: `credential_missing`, `status` in
  expired/revoked/failed/refresh_failed/pending_auth, `connection_status =
  expired`, `node_id` set and `node_status != online`, or `is_active = false`.

### 5.2 Inspector (Sheet on desktop, page on mobile, same component)

```
llm-openai                                     You · Direct        [Open full page]
[Route] [Activity] [Details]

ROUTE
  Calls to /api/v1/proxy/s/llm-openai execute this service (exact slug).
  Credential    OpenAI key · Saved 12 Sep · Not verified
  Transport     Direct
  Payer         You · your key (server-computed)
  Evidence      Last successful request 2m ago · coding-agent · 200
  Note          This is the catalog slug. If this service is disabled or deleted,
                calls to llm-openai use the NyxID platform key and platform pricing.
  Viewing as    [You ▾]   coding-agent: in allowlist · overrides credential "work key"

ACTIVITY   (Requests | Changes)
  2m   coding-agent · agent key      POST /v1/responses   200   this service
  1h   you · session                 GET  /v1/models      200   this service
  3d   Alice changed label "Prod" → "Production"                        [expand]
  ─ Recorded since 20 Sep 2026 ─

DETAILS
  Endpoint, auth method, headers, identity propagation, node routing (existing)
  Origin     Added by you · CLI · 12 Sep 2026
  Related    Same credential: none
             Same catalog: llm-openai-2 (Acme), llm-openai (NyxID platform, disabled)
             Pools: llm-pool (member 1 of 2)
             Agents bound: coding-agent → "work key"
```

Pool inspector Route tab:

```
ROUTE
  Calls to /api/v1/proxy/s/llm-pool pick one enabled member per request.
  Strategy      Round robin · 2 of 3 members eligible now (excluded: llm-openai-2, disabled)
  Members       1  llm-openai      You    OpenAI key · Saved      weight 2
                2  llm-openai-2    Acme   Acme key · Disabled     weight 1   (skipped)
                3  llm-anthropic   You    OAuth · Expired         weight 1   (skipped)
  Evidence      Last request 5m ago ran member llm-openai (coding-agent, 200)
  [Strategy ▾ Round robin | Weighted | Priority]   drag handles appear only under Priority
```

Rules:

- The three tabs are fixed. The full page is the same three tabs; today's
  Overview/Advanced/History become Route/Details/Activity. No fourth tab, no
  "Connections" tab, no "Related" tab.
- **Route** shows facts and evidence, never a verdict, until the evaluator
  ships. "Would use", "checked N seconds ago", "Check: POST /responses",
  "Ready via" and an operation picker are out of the first release.
- **Viewing as** lists You and the reader's agent keys. For an agent it prints
  only exact facts: in allowlist / not in allowlist / all services, credential
  override label, rate limit. No route verdict "as agent" until the evaluator
  accepts an actor. "Application context" is dropped; nothing can simulate it.
- **Payer** is a server field. The client never derives it from Owner.
- The platform fallback **Note** appears only when the row's slug equals its
  catalog slug and `platform_key_available` is true. It is the one place the
  legacy fallthrough is disclosed, and it is the "paid fallback" answer.
- **Activity** is one chronological feed with two filter chips. Requests come
  from the per-request record (7.1); Changes from the journal. Members of an
  org see "Changes: available to org admins" instead of an empty list.
- **Details → Related** is computed from `api_key_id`, `catalog_service_id`,
  pool membership and bindings. Each relation is labelled with what it is; none
  implies inheritance. The journal already fans out shared-credential edits to
  every referencing service, so no "include related events" option is needed.
- Pool member drag exists only when strategy is Priority, only inside that
  pool, with keyboard equivalents. The preview never calls `resolve_member`.

## 6. Critique of the draft, itemised

### 6.1 Accepted as written

- One workspace; pools in it; Agent Keys stays a separate destination.
- No family-wide selection claim; no invented canonical route; omit absent
  platform sources; unavailable records shown below with a repair reason.
- Deep-link by immutable id; preserve list filters and scroll.
- Three tabs.
- Pin is strict; multiple accounts in a tier need an explicit choice; org order
  is not database order.
- No fallback after dispatch; a resolver/database failure is not permission to
  try another identity.
- Last attempt and last success tracked separately; transport completion is
  not business success; "Not recorded" and "Recorded since" wording.
- Do not infer caller from User-Agent, provisioning app, creator or owner.
- Relationship labels instead of "spin-off"; rename and credential rotation are
  history on the same service; slug reuse never merges histories.
- Create variant as a future write-once origin link.
- Round robin / Weighted say "selects per request"; previews never advance the
  counter; Priority is a real strategy before it has drag handles.

### 6.2 Blocking (the draft cannot be built truthfully until fixed)

| # | Draft claim | Problem | Required change |
| --- | --- | --- | --- |
| B1 | "Expanded Requests feed shows the requested route, actual connection, source/owner and result"; "same execution ID" for pool and direct calls | No single record has actor + concrete service + outcome; catalog-backed audit stores the catalog id; the two audit rows share no id; ordinary app `oauth_client_id` and child credential id are not audited. | Add `user_service_id`, `route_kind` (exact / pool_member / org / legacy_platform), `pool_id`, `credential_class`, `oauth_client_id`, `api_key_credential_id` and an outcome class to the `proxy_request`, `llm_proxy_request` and `proxy_request_denied` events, plus one request id shared with the routing row and the meter's `billing_request_id`. `DestinationAudit` (`destination_routing.rs:375-400`) carries service/target/origin but no request id, so one must be minted at the proxy entry and threaded through all three writers. |
| B2 | "Latest request … on the row" | Reading audit per row is a 30-day scan per service; the existing agent-key dashboard already pays this. | Add `last_request {at, actor, outcome_class, route_kind, request_id}` and `last_success {…}` projections on `UserService`, updated fire-and-forget after the response is written, same pattern as `last_change`. Outcome classes: `dispatched_ok` (upstream 2xx/3xx), `dispatched_error` (upstream 4xx/5xx), `not_dispatched` (denied or failed before upstream). `last_success` advances only on `dispatched_ok`. |
| B3 | `last_used_at` shown as usage | It is credential preparation time on a possibly shared credential, including agent override use. | Relabel to "Credential last prepared" inside Details, or hide. Never feed Last request or Last success from it. |
| B4 | "Would use … checked 24 seconds ago", "Ready via", "Check: POST /responses", "Use a specific connection", "As: application" | No resolver projection, no preflight endpoint, no per-service route preference, no actor-parameterised evaluation. The example's fallback to Acme after an expired personal key is not current behaviour: today that request returns 400. | Remove from release 1. Route tab shows facts and evidence. Label the §3 example as future-state. |
| B5 | Service / Route / Connection vocabulary | Inverts existing product terms and collides with OAuth "connection". | Adopt section 4 terms. |
| B6 | "Latest visible request" on the service block; caller names in the feed | Org privacy is stated in §5 but violated by the block aggregate; the rule is not concrete. | Concrete rule: personal rows show everything to the owner; org rows show all callers to org admins, only the reader's own requests to members, nothing to viewers. The list column obeys the same rule. |
| B7 | "Actual payer/pricing" before saving a policy; payer in resolution | Client cannot derive payer; platform-bound org rows bill the acting person. | Payer is a server-computed field on the key view; UI renders it verbatim or omits it. |
| B8 | History for "the visible caller context" | `read.rs` requires `can_write()` plus resource scope; members and viewers get no authorship or journal. | Activity → Changes must render an explicit "available to org admins" state for members; do not promise members edit history. |
| B9 | One row per address | `/keys` can return two rows with one slug (disabled + active); tombstones can be revived into zombies (known gaps 1 and 2). | Key rows by UUID; label the disabled duplicate; fix gap 1 (409 at create) before shipping the grouped sort, or the sort will show an impossible pair without explanation. |
| B10 | Agent "As:" simulation | `allow_auto_connected_services` expansion is server-side; org role scopes apply per membership. | Show only allowlist membership, binding override and rate limit as facts. No verdict. |

### 6.3 Challenged as control-heavy or unusable (drop or defer)

- **Cards toggle for individual connections** as a second presentation of the
  same list. Row expand shows the card. Keep only the existing card/table
  ViewToggle for the responsive split.
- **Search by authenticated caller** before B1/B2 exist.
- **"Changes can optionally include related-service events"**: the journal
  already writes shared-credential edits into every referencing service's own
  history. The option adds a control and a scope-leak risk for no new data.
- **Operation picker ("Check: POST /responses") in the Sheet**. If an
  operation-scoped check ever ships, put it on the full page and the CLI, not
  in the list inspector.
- **Saving a route/pin policy from the inspector** ("show who owns the setting,
  who it affects"). No policy store exists. The only writes in release 1 are
  the existing ones: Enable/Disable, node routing, pool strategy and members,
  credential replacement.
- **"Release dashboard · Application · on behalf of Alice"** cannot be produced
  for ordinary app tokens today (not audited). Show it only when
  `acting_client_id` (delegated) or the new `oauth_client_id` field is present.
- **"API key ending …7K2"**: the audit row has `api_key_id` and name, not the
  prefix; a lookup is needed, and Agent Key child credentials are invisible.
  Render the key name; add the child credential id in B1 if per-login
  attribution matters.

### 6.4 Where I disagree with the seed

- "Service family" as the hierarchy: no server object, misleading at the
  family level, breaks on pools and custom rows. Divider only.
- Four inspector tabs (Overview, Connections, Activity, Related): "Connections"
  duplicates the list, "Related" is a section of Details, "Overview + caller"
  is a Route tab with a Viewing-as control.
- "Uses now": unsafe for pools (counter), unavailable for catalog slugs (no
  resolver), and dishonest for anything requiring a probe. Replace with
  "executes this service" (exact), "picks per request" (pool), and evidence
  with age.
- "Spin-off": no data. Origin (write-once) plus computed relations.
- Actor from creator or User-Agent: rejected; the audit row's verified
  `user_id` / `api_key_id` / `acting_client_id` is the only source.

## 7. Backend additions the flow depends on (smallest set)

1. **Per-request record fields** (B1) on the existing proxy audit events, plus
   a shared request id. No new collection.
2. **`last_request` / `last_success` projections** (B2) on `UserService` and
   on `ServicePool` (the pool's last request also names the member that ran).
3. **`payer` on the key view**, computed by the existing billing owner resolver
   without reserving anything.
4. **`PoolStrategy::Priority`**: first enabled active member in array order,
   error if none; member reorder endpoint already implied by member edits.
5. **Gap 1 fix**: 409 when creating a service on a disabled row's slug.
6. **Read endpoint for a row's recent requests** scoped by the privacy rule in
   B6, cursor-paginated, 30-day cap like the agent-key dashboard.

Release 2 (only after the evaluator from `slug-connection-resolution-proposal.md`
exists): canonical catalog-slug rows with mode Automatic, the Route tab
candidate order, a read-only "Check route" button, Viewing-as verdicts, and
per-service source preference. Release 3: Create variant with an immutable
`origin.derived_from_service_id`.

## 8. Edge cases the final flow must render

| Case | Rendering |
| --- | --- |
| No requests ever, no journal | Last request "Not recorded"; Last change "Creator not recorded"; Activity shows "Recorded since <projection start>". |
| Legacy row with edits before tracking | Footer "Earlier edits not recorded"; timeline starts at first journaled event. |
| Deleted service | Hidden from the list; "Deleted services" filter lists archived UUIDs with slug and last change; inspector opens read-only with Activity only. |
| Slug recreated after delete | New UUID, new history. Details → Related: "This slug previously belonged to a deleted service (last changed <date>)", labelled as slug reuse, never as lineage. |
| Disabled row and active row share a slug | Both listed; disabled one labelled "slug reserved by the active row"; Enable on it is expected to fail until gap 1 is fixed, so the button explains why. |
| Multiple orgs | Each org row shows org name and the reader's role. No ordinal until the evaluator exposes membership order; the Route tab of a catalog-slug row says "org order: primary org first" only when the resolver reports it. |
| Two personal accounts for one catalog service | Two rows, two addresses (`llm-openai`, `llm-openai-2`); the catalog-slug row carries the fallback Note; the other carries none. No "default" control until the preference store exists. |
| Agent pinned to a credential | Viewing-as shows "overrides credential X"; Activity rows from that agent show the override in the route column once B1 lands. |
| Viewer role on org row | Row visible, reduced opacity, no Last request, no Last change, inspector Route tab facts only. |
| Member role on org row | Row visible; Last request = member's own; Changes tab "available to org admins". |
| Platform absent | No platform row, no Note, no option anywhere. |
| Platform configured but `platform_key_available = false` | Explicit platform-bound rows show Credential "NyxID platform · Unavailable" with the catalog reason; no fallback Note on the catalog-slug row. |
| Paid fallback live (catalog slug, platform key available, user row exists) | The Note in the Route tab, worded with "platform pricing". Disable confirmation repeats it: "Calls to llm-openai will use the NyxID platform key while this service is disabled." |
| Pool with mixed credentials or providers | Members listed with their own owner and credential; pool sits under the POOLS divider, never under a catalog divider. |
| Node-routed service, node offline | Transport "node lab-1 (offline, 2 fallbacks)"; Attention filter catches it; no request is claimed possible. |
| Codex-linked service | Route tab Credential line "Verified <date> · usable" or "reconnect_required"; the only service kind allowed to say Verified. |

## 9. Recommendation

Adopt the row-per-address list with stateless dividers, the three-tab
Route/Activity/Details Sheet shared with the full page, and the vocabulary in
section 4. Treat B1 and B2 as the definition of done for "who last used this
service"; without them the Last request column must not ship. Keep every
resolver-dependent phrase out of release 1 and label the draft's §3 example
as future behaviour. The draft's sequence (contracts, server, UI) is right; the
contracts are smaller than the draft implies, and the first UI release is a
truthful metadata view, not a routing product.

---

# Pass 2: closing disposition on the rewritten draft

Reviewed `consolidated-services-flow.md` at its 25 September rewrite (412
lines). Line numbers below refer to that file. Verdict: **converged once the
six wording fixes in P2.2 are applied.** They change sentences, not the design.

## P2.1 Accepted, and pass-1 positions withdrawn

The draft's structure stands: one entry per real callable address, stateless
catalog dividers, one inspector with Route / Activity / Details shared with the
full page, Service vocabulary, correlated execution activity, origin rather
than inferred lineage, explicit List/Cards with no third mode.

Withdrawn from pass 1 after the user's corrections and re-checking code:

- "Priority = first enabled active member in array order." Wrong; a pool
  member must pass the same eligibility as an Automatic candidate. Draft
  lines 212 and 235 are right.
- "Metadata-only release 1, resolver later." The target includes Automatic
  resolution; the draft's delivery order (contracts and resolver first, UI
  last) replaces my release split.
- The platform fallback Note derived from slug plus `platform_key_available`.
  Draft line 366-368 is right: fallback is a server decision, never
  reconstructed from availability metadata.
- "Member's own latest by filtering the row's latest." Needs an actor-scoped
  projection; draft line 283-284 is right.
- Pools must stay same-owner (`resolve_member` filters `user_id: owner_id`,
  `service_pool_service.rs:380-384`); draft line 206-207 is right.

Checked and found consistent with code, no change needed:

- No replay after dispatch (lines 189-192, 385). A node that accepted the
  request and then failed returns `DurableOperationOutcomeUncertain` with no
  retry (`handlers/proxy.rs:3282-3287`); `fallback_node_ids` is consulted
  only before a node accepts. Node failover therefore already sits below the
  source boundary.
- Streaming and Codex-transport audits record `response_status` at header
  time (`handlers/proxy.rs:3697-3712`, `3290-3300`), which is exactly why step
  2 (line 349) must separate protocol completion from headers. Evidence, not
  a blocker.
- Exact-call identity for the same-slug candidate exists in two forms today:
  the UUID address (`KeyResponse.proxy_url`, resolved by
  `resolve_proxy_target_by_user_service_id`, `handlers/proxy.rs:1059`) and the
  pin `?_nyxid_via=<user_service_id>` on the slug form
  (`handlers/proxy.rs:902`). Line 75 can name them.
- Auto-provisioned and platform-bound rows reject user mutation
  (`user_service_service.rs:76-83`), so the opt-out lives in the source editor
  as the draft says (line 148).

## P2.2 Blockers: six line fixes

**F1. Lines 115-133 and 172-182: state the eligibility rule.** The draft
never says whether a fresh, unverified credential may enter Automatic. Code
answers it: the read-only snapshot counts an expired OAuth token with a refresh
token as materializable without refreshing (`credential_is_materializable`,
`proxy_service.rs:3536-3545`; `read_only_snapshot`, `1650-1657`), and
execution refreshes during preparation. The only alternative, requiring
verification first, would exclude every newly added key from its own address
and there is no safe generic probe (Codex and the at-creation AWS probe are
the only supported checks). Insert after line 133:

> Eligibility is preparation, not verification. A candidate enters the
> usable order when authority, configuration and credential preparation
> succeed at evaluation time; a refreshable credential is eligible with a
> pending refresh gate. Verification evidence changes the label, never the
> order. A candidate leaves the usable order only on a classified terminal
> failure (missing, revoked, failed, terminal refresh rejection, disabled,
> no permitted node route) or an explicit user action (Disable, pin). An
> upstream 4xx, 429 or outage is recorded as that request's outcome and
> never demotes a candidate; repair is a human action. The first completed
> execution through a candidate is its verification evidence; no business
> request is sent to manufacture one.

This also closes the "actual valid and working" question: "working" is a
dated outcome shown in Activity, "eligible" is a preparation result, and the
UI never conflates them with a badge.

**F2. Lines 120 and 133: a pending gate above blocks "Would use" below.**
With F1, a personal candidate in "Refresh required" is still ahead of Acme.
The example at line 98 is fine because "Needs reconnect" is terminal, but the
rule is missing. Replace the "Would use" table row evidence with:

> Server evaluation for this caller, operation and policy version in which
> every higher-priority candidate is in a classified terminal state. If a
> higher candidate is in a pending gate (refresh, verification, approval),
> show **Would use Personal after refresh, otherwise Acme**; never name the
> lower tier alone.

**F3. Line 148 and line 376: state the default for platform inclusion.**
"Users may exclude optional org or platform fallback" and "only if allowed"
leave the default undefined, and the default decides whether migration opts
BYOK users into charges. Insert after line 149:

> Platform is included in Automatic by default only where the offering is
> available to this caller and the payer already holds billing authorization
> for that route; otherwise it is excluded until the user opts in from this
> editor with payer and pricing shown. Migration never enables platform
> charges for an existing user without that opt-in. Excluding a tier removes
> it; it never reorders the remaining tiers.

**F4. Line 172 and line 189: define when a canonical entry exists, and
close the no-row path.** "An enabled canonical Automatic address" is never
defined, so the list could show every catalog entry with a public platform
key, or hide a live path. Insert before line 172:

> A canonical entry exists for a viewer when the evaluated candidate set is
> non-empty: at least one personal service for that catalog identity, one
> org service reachable through an active membership with proxy rights, or
> a platform offering that is included under F3. With no candidate, the
> catalog page is the entry point and calls to the address return the
> structured unavailable error; the legacy catalog-slug path is not consulted
> for a canonical address unless a legacy pre-migration connection exists.

The last clause is the guard from the earlier reconciliation; without it
"no rows plus platform excluded" leaks through `resolve_service_by_slug`.

**F5. Lines 55-57 and 73-79: Owner and Latest request on canonical
entries.** A canonical entry has no record owner; "You" there is the policy
scope, while "Acme" on the row below is a record owner. Same column, two
meanings. Fix the value, not the column name: on canonical entries Owner
reads **Your calls** (or **Acme's calls** for an org-scoped policy that an
org admin opens). Add to line 77:

> A canonical entry's Latest request covers requests **to this address** by
> the viewer's scope; the nested candidate card's Latest request covers
> executions **through this service** by any permitted entry point. Both
> labels are shown. When the same-slug candidate is an auto-provisioned
> platform row, the nested card offers no Disable; exclusion lives in the
> source editor.

**F6. Line 100: the payer in the example is wrong.** With "Would use Acme ·
Work OpenAI" on a BYOK org credential, the billing owner is the resource
owner, not the acting person: `resolve_for_execution` charges the acting
person only for `NyxidManagedMaster` and otherwise resolves the resource
owner (`billing/owner_resolver.rs:53-67`). Change the line to **Paid by Acme
(org wallet) · provider charges to Acme's key** and add one sentence under
the label table:

> Paid by always follows the selected candidate: org credential, org wallet;
> platform key, the acting person; personal key, you. While selection is
> pending, show **Payer depends on source** rather than a guess.

## P2.3 Recommended, not blocking

- Line 57: the list's source cell ("Automatic · Personal") needs a batched
  read-only evaluation for the page. Say so, or show only **Automatic** in
  the list and evaluate on open. The footnote's age is invisible in a
  four-column row anyway.
- Line 75: name the two exact-call forms (UUID address, `_nyxid_via` pin).
- Lines 210-212: say that Round robin and Weighted also select among
  *eligible* members under F1, replacing today's `is_active`-only filter
  (`service_pool_service.rs:374-390`), and that a member failing preparation
  is skipped before dispatch within the member set.
- Line 166-167: "a stable saved order" for multiple orgs is a new per-user
  preference; list it in section 7 under versioned policy.
- Line 264: add "denied before dispatch" as an outcome with no executed
  target, matching line 350.

## P2.4 Status of F1-F6 after the user's edits

F2 (pending higher candidate blocks a definite preview), F5 (Owner on
canonical entries, "Through this address" versus "Handled by this service",
policy journal versus concrete journal, back breadcrumb) and F6 are addressed
in the final text. F3 and F4 remain as written above and are still required.
F1 is superseded by the user's explicit working-only choice; the closing
assessment of that choice follows.

---

# Pass 2 closing: the final text (460 lines)

Assessed as final. Two blockers remain, both inside the new working-only
paragraph (lines 151-163) and its acceptance rows (422-423). Everything else
is converged.

## G1. The working-only selection rule contradicts the draft's own skip rule and deadlocks the common case

The paragraph says unverified candidates "do not enter the working set" and
row 423 says "do not ... dispatch an Automatic/Priority request". Three
consequences, each traced to the final text:

1. **Internal contradiction.** Step 3 (line 219-221) permits skipping "only
   classified candidate failures". Row 422 skips a saved personal key because
   no evidence exists. Absence of a record is not a classified failure. Line
   197 forbids moving "between unchosen accounts"; row 422 moves the caller
   from their own configured account to Acme's on that same absence.
2. **Deadlock for slug callers.** A user with one personal key for a provider
   without a supported check (every provider except Codex and the at-creation
   AWS probe), no org and no platform, whose agent is configured with the
   canonical slug: the slug is Automatic (line 195-197), the candidate is
   unverified, no safe check exists, so every call returns **Verification
   required**. The only bootstrap is an exact UUID or `_nyxid_via` call (line
   161-162), which the agent never makes. The Connect flow (line 204-205)
   cannot progress past Saved for that provider. Pools of fresh keys under
   Priority deadlock the same way.
3. **Day-one migration.** The execution record that produces "qualifying
   recorded success" does not exist yet; today's audit rows cannot be
   backfilled per service (catalog id, no join). At rollout every existing
   BYOK candidate is unverified. Under row 422 their traffic shifts to org or
   platform accounts on the first call; under row 423 it stops. Either is a
   silent identity or availability change for every current user.

Keep working-only for **claims and preview**; change **selection** so
missing evidence never substitutes an identity. Replace lines 157-160 with:

> Execution may perform a supported safe check before selection, at most
> once per credential and configuration version, and records the result as
> evidence. When the top-ranked candidate has no evidence and no supported
> check, Automatic and Priority execute that candidate as its **verification
> attempt**: the caller's own request, on the requested operation, with no
> other identity substituted because evidence is missing. Preview shows
> **Would try Personal · unverified**, never Would use or Ready. The outcome
> becomes that candidate's version-bound evidence; a classified failure then
> removes it from the working set on the next call. A policy may opt into
> **Require verified sources**, in which case an unverified top candidate
> returns **Verification required** naming the available actions (Verify,
> Use exactly once, Switch source) and no other account is used.

Rewrite rows 422-423 accordingly:

| Scenario | Required result |
| --- | --- |
| A personal key is merely saved and org has current valid evidence | Check personal safely when supported; otherwise Automatic executes personal as its verification attempt and records the outcome. Org is used only after a classified personal failure, or when the policy requires verified sources and the user chose Switch source. |
| Every configured credential is unverified and cannot be safely checked | Automatic and Priority execute the top candidate as a verification attempt; nothing is marked ready. Under Require verified sources: Verification required, no dispatch. |

If the user keeps strict exclusion instead, the minimum to make it operable
is: (a) the single-candidate bootstrap (a canonical call whose candidate set
has exactly one unverified member executes it as an exact request), (b) a
migration rule that pre-rollout candidates receive one verification attempt,
(c) Connect and the row both stating **Unverified · Automatic will use Acme
until verified** before any traffic shifts. Without all three the rule ships
either an outage or a silent account switch.

## G2. Evidence definition omits OAuth authorization and lets time expire eligibility

Line 153-156: "supported validation/refresh, or a qualifying recorded
success ... Freshness limits ... defined by the provider adapter."

- A just-completed OAuth authorization produces a valid token with no
  refresh performed and no request yet made. Under the text it is
  unverified. Most catalog services are OAuth. Add: **a completed OAuth
  authorization or successful refresh is validity evidence for that token
  version** (`last_authorized_at`, `write_oauth_tokens_to_key`).
- Adapter-defined freshness means an idle service with a success eight days
  ago can drop out of the working set with no change to anything, flipping
  Automatic to another account and filling Needs attention with healthy
  rows. Replace with: **evidence is invalidated by a credential epoch change,
  an endpoint or configuration version change, or a classified failure; not
  by elapsed time. Age is always displayed.** If an adapter needs a
  time-bound re-check, it downgrades the label to **Re-check recommended**
  and never changes selection.

## Recommended, not blocking

- Line 55-63: Owner **—** on the canonical row shares a column with blank
  cells on restricted rows and "Not recorded" elsewhere, so a dash reads as
  unknown. **Your policy** (or **Acme policy** for org scope) scans cleanly
  and matches footnote ².

## Verdict

Converged on structure, vocabulary, inspector, activity scope, lineage,
pools and privacy. Not converged on G1 and G2: apply the replacement
wording, or keep strict exclusion with the three operability conditions.
No further investigation is needed either way.

---

# Final disposition on G1 and G2 (primary draft at 524 lines)

**Closed.** The rewrite keeps working-only selection and removes the three
risks by making absence a pending gate rather than a terminal or substitutable
state. Checked against the final text:

- **Deadlock.** Lines 159-163 return **Verification required** instead of
  skipping; lines 177-183 give an explicit **Use exact service** path with a
  generated exact URL/agent configuration and a user-requested first real
  operation that produces version-bound evidence; line 183 requires setup to
  surface that path. Lines 250-256 offer Automatic only after candidates are
  reviewed. A slug-configured agent with one unsupported key is no longer
  trapped; it is redirected. Opportunistic first-use verification was my
  preference; explicit setup is a legitimate tradeoff, not an open gap.
- **Migration.** Lines 250-252 preserve existing exact/custom/pool contracts
  until an explicit migration whose preview names addresses, agents, policy,
  evidence gaps and payer, and cannot enable Automatic while verification is
  unresolved. Row 476 rules out a first-day outage or paid fallback. Closed.
- **Identity.** Lines 161-163 and row 474 forbid substituting an org or
  platform account for missing evidence; lines 171-173 forbid time alone from
  marking a credential broken or changing identity; Priority stops at an
  unassessed higher member unless explicitly excluded (167-168); Round
  robin/Weighted use an explicitly approved working set disclosed before
  activation (165-166). Closed.
- **G2.** OAuth authorization and refresh count as evidence (154-155);
  freshness is a re-check, never a demotion by age. Closed.

Three one-line residuals, none blocking:

1. **Implicit no-row platform path.** Lines 246-249 retire the hidden legacy
   platform source for canonical contracts with no usable candidate. Callers
   who rely on it today (no rows, never opened `/keys`) are not covered by
   row 476. Name this as a deliberate consent cutoff in the migration
   section and measure it first: audit rows with `routed_via: "personal"`
   and `user_service_id: null` are exactly that path
   (`handlers/proxy.rs:632-640`).
2. **Copy target.** While a service is unverified and its canonical address
   is Automatic, the card's copy action and agent config must hand out the
   exact URL (`KeyResponse.proxy_url`), not `proxy_url_slug`, or the trap
   returns through the existing copy button.
3. **Freshness loophole.** Line 171-174 lets an adapter require a re-check
   and, if unavailable, "stop with an actionable state". Add: an adapter may
   require a freshness re-check only when it supplies a safe check; otherwise
   version-bound evidence does not expire. Without this, an idle key for an
   unsupported provider can move from working to stopped by time alone,
   which line 172 says must not happen.

Review closed. No further pass requested.
