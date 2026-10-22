# Issue #1949: authority before slug collisions in connection selection

Status: implemented on branch `nyxid-1949-fix` (planned against `main`
618c627b, the revision the reporter inspected; rebased onto current `main`).
See the Review log for rounds and the final status.

## Problem

An organization-owned specialist granted organization connection `O1`
(`api-github-pat`) cannot discover or call it when the acting member also owns a
personal connection `P1` with the same slug. Discovery meanwhile offers the
member's ungranted personal connection `P2`, whose tool then fails with
`Unknown tool`.

## Root cause

Both symptoms come from one ordering error in the canonical MCP catalog loader:
it resolves tool-name (slug) collisions over every connection the actor can
reach, and only afterwards applies the caller's authority.

1. `services/mcp_service.rs::load_callable_user_services` collects personal rows,
   then rows of every organization membership, and drops any later row whose
   slug was already seen (`seen_slugs`). Personal `P1` therefore removes `O1`
   before any grant is considered.
2. `load_operation_catalog` applies `ServiceScope::Allowed(grants)` only after the
   loader returned (`visible.retain(service_scope.permits)`), so `P1` is then
   removed as ungranted and `O1` is already gone. The same happens for every
   other `Allowed` caller: restricted API keys and OAuth tokens, delegated
   catalogs, durable-grant endpoint loading by ID.
3. MCP discovery (`handlers/mcp_transport.rs::load_all_services_for_meta_tools`,
   shared by `nyx__search_tools` and `nyx__list_connected_services`) loads chat
   callers without any service scope and with an unrestricted node scope, while
   `tools/list`, `nyx__call_tool` and direct tool dispatch use
   `mcp_service_scope`, which narrows an organization agent to its grants. The
   ungranted personal `P2` is discoverable but absent from the execution catalog,
   hence `Unknown tool`.
4. REST `GET /api/v1/mcp/config` (`handlers/mcp.rs`) computes yet another scope
   (`Unrestricted` for every chat key), so it disagrees with MCP `tools/list` for
   organization agents although `docs/API.md` states both use the same scoped
   catalog.

The same "collision before authority" defect also reaches:

- a personal specialist granted an organization connection that shares a slug
  with an ungranted personal one (the ungranted row wins, and calling it asks for
  a card for the wrong connection);
- NyxBot's `nyxid__grant_subagent` target resolution by explicit connection ID
  (`assistant_team_service::resolve_each_target`), which cannot find a shadowed ID;
- assistant readiness evidence by connection ID
  (`assistant_readiness_service::execution_state_for_user_service`);
- REST `/proxy/s/{slug}` for organization-bound keys
  (`service_pool_routing::select_slug_metadata`), which picks the personal row
  first and is then refused by the final organization fence.

Execution already keys on the exact `UserService.id` (`McpToolSource::UserManaged`)
and the final fence (`org_agent_service::authorize_service_with_access`) already
refuses personal and other-organization rows, so no unauthorized execution or
credential substitution occurred. The defect is selection, discovery and parity.

## Design

### D1. One selection authority, applied before collisions

`ServiceScope` becomes the single description of which connections a caller may
see, and the loader applies it **before** collision resolution:

| Variant | Candidates | Collision preference | `service_scope_restricted` |
| --- | --- | --- | --- |
| `Unrestricted` | every reachable connection (unchanged) | personal, then organizations | false |
| `Allowed(ids)` | only `ids` (UserServices only, unchanged) | personal, then organizations | true |
| `Preferring(ids)` (new) | every reachable connection | listed IDs first, then personal, then organizations | false |
| `Organization { access }` (new) | only the bound organization's connections that the acting person's live access permits | none needed (one owner, slugs unique per owner) | true |

- `Organization` uses the request's authentication snapshot
  (`org_agent_service::RequestAccess`), never a second membership resolution, as
  `docs/ORG_AGENTS.md` requires. Its ACL is exactly the existing organization-key
  rule (owner match, `allows_resource`, `admin_only` requires write): factor it
  into one `RequestAccess` method reused by `key_services`,
  `authorize_service_with_access` and the loader so the three cannot drift.
- Personal connections, other organizations' connections and platform
  (`DownstreamService`) catalog services are never candidates for an
  organization agent; organization agents cannot be granted them
  (`validate_grants`).
- Collision preference is identity/authority based, never executability based:
  a granted but temporarily unavailable connection still owns its slug and shows
  `executable: false`, instead of silently switching the tool name to a
  different credential.
- Organization order for unrestricted callers follows the documented proxy
  cascade (`docs/site/shared/concepts/organizations.md`): `primary_org_id` first,
  then earliest membership `created_at`, then organization ID. This replaces the
  current unordered membership iteration so MCP tool names and
  `/proxy/s/{slug}` choose the same connection.
- All pre-existing authority filters (live membership role/scope, `admin_only`,
  pinned-primary node scope) keep running before collisions, as today.

### D2. One caller-scope function for every surface

Add `mcp_service::caller_service_scope(...)` (services layer, pure) taking the
actor, bound owner, request access snapshot, chat authority,
`allow_all_services` and `allowed_service_ids`:

1. bound owner present and different from the actor:
   - chat key with a matching snapshot: `Organization { access }`;
   - anything else: `Allowed(allowed_service_ids)` (fail closed);
2. chat key: orchestrator (NyxBot) `Unrestricted`; any other chat key, including
   guest turns, `Preferring(allowed_service_ids)`;
3. otherwise `Unrestricted` when `allow_all_services`, else
   `Allowed(allowed_service_ids)`.

`mcp_service_scope` (MCP) and `/api/v1/mcp/config` (REST) both call it.
MCP discovery uses the same service scope **and** the same node scope
(`mcp_node_scope`) as execution. Personal chat keys always allow all nodes, so
only organization agents (whose nodes are the organization's) change.

Resulting contract for an organization specialist: `nyx__list_connected_services`,
`nyx__search_tools`, `tools/list`, `/mcp/config`, `nyx__call_tool` and direct
dispatch all see exactly the bound organization's connections the acting member
can use. Granted ones report `chat_access: granted` and execute; ungranted
same-organization ones report `chat_access: acknowledgement_required` and, when
called, reach the existing `service_gate` (owner card when routed and the person
maintains the agent, `organization_grant_required`, `access_request_denied` or
missing-access), exactly as `docs/ORG_AGENTS.md` describes. They no longer end in
`Unknown tool`. Execution authority is unchanged: grants are still enforced by
`chat_service_gate`/`ensure_service_in_scope`, and the final organization fence
still runs.

### D3. ID lookups are never defeated by collisions

Every caller that looks a connection up by ID through the catalog scopes the load
to that ID (or prefers it):

- `resolve_each_target`: `Preferring(requested targets)`, so an explicit
  connection ID always resolves and a slug still follows normal precedence;
- readiness `execution_state_for_user_service`: `Allowed([id])`;
- durable grants and delegated catalogs already pass `Allowed`, and are fixed by D1.

The implementer audits every `load_operation_catalog` / `load_user_tools*`
caller and records the disposition of each in this document.

### D4. REST slug routing for organization-bound keys

`select_slug_metadata` gives an organization-bound caller (snapshot present) the
bound organization's service or pool with that slug first, under the snapshot ACL.
Only when the organization has no usable match does the existing cascade run
unchanged, so the documented implicit model-transport exception keeps working and
the final organization fence still refuses everything else. Other callers keep
the documented cascade (personal always wins; `_nyxid_via` remains the explicit
selector).

### D5. Explain what is deliberately not shown

- `/api/v1/mcp/config` diagnostics gain `shadowed_connections`: the number of
  caller-authorized connections omitted because another connection with the same
  slug takes precedence. It is computed after authorization, so it never counts
  resources the caller may not use.
- `nyx__list_connected_services` rows gain `owner_kind` (`personal` or
  `organization`) for UserService rows; `nyx__search_tools` service matches gain
  `service_id`. Both meta-tools match a query equal to a connection ID exactly,
  so the reporter's exact-ID discovery works.
- `CHAT_ACCESS_HINT` stops calling every `user_service` "the user's own
  connection" and points to `owner_kind`.

## Explicitly unchanged

- Unrestricted humans and NyxBot keep personal-first precedence. A human with
  both `P1` and `O1` still sees `P1` as `api-github-pat__*` (now explained by
  `shadowed_connections`) and selects `O1` explicitly by ID on REST
  (`--via-service`, `_nyxid_via`). Disambiguated tool names would rename existing
  tools for every client and are not part of this fix.
- Machine job declarations (`machine_gateway_service::services`) already filter by
  grants before slug de-duplication; covered by a regression test only.
- The implicit model-transport exception and `resolve_implicit_for_request`.
- No persisted data, migration, index, environment variable or error code change.

## Work packages

1. **Loader**: thread `ServiceScope` into `load_user_tools_with_grants` and
   `load_callable_user_services` (and `load_user_tools_all_scoped`); apply
   candidate selection, then one explicit collision-resolution function; return
   the shadowed count to `load_operation_catalog`. Keep the post-load `permits`
   retain as defense in depth (`Organization` permits only UserManaged rows of the
   bound owner). Work that cannot affect results for scopes that exclude platform
   services may be skipped, but outputs must be identical.
2. **Scope function** (D2) and its use in `mcp_service_scope`, MCP discovery
   (service and node scope) and `/mcp/config`.
3. **ID lookups** (D3) and **REST slug routing** (D4).
4. **Discovery metadata** (D5).
5. **Docs**: `docs/ORG_AGENTS.md` (discovery/execution contract),
   `docs/API_DISCOVERY.md` (new "Connection selection and slug collisions"
   section), `docs/API.md` (`/mcp/config` diagnostics and meta-tool fields),
   `docs/site/shared/concepts/organizations.md` (MCP follows the same order), and
   one concise invariant in `CLAUDE.md`.

## Regression coverage (all against MongoDB, through the real auth paths)

Organization specialist thread key (authenticated via `x-api-key`):

1. Personal `P1` and organization `O1` share a slug; `O1` granted. `O1` appears
   in `nyx__list_connected_services` (also by exact ID), `nyx__search_tools`,
   `tools/list` and `/mcp/config`; `P1` never appears.
2. Ungranted personal `P2` never appears on any surface; calling its tool name
   returns `Unknown tool` (consistent with discovery).
3. Same slug in two organizations (the other organization joined earlier and is
   the primary): the bound organization's connection is selected; the other never
   appears.
4. Ungranted same-organization connection: listed with
   `acknowledgement_required`; calling it reaches `service_gate` (refusal or card,
   no execution, no `Unknown tool`).
5. Live access: membership scope narrowing and `admin_only` for a Member remove
   the connection from every surface and refuse a stale tool name; membership
   revocation refuses the next request at authentication.
6. End to end: `nyx__call_tool` on `O1`'s repository-read operation against a
   local mock upstream succeeds; the upstream receives `O1`'s credential (not
   `P1`'s) and the `mcp_tool_call` audit records `service_id = O1`.
7. REST `/proxy/s/{slug}` with the organization key routes to `O1`.

Other callers:

8. Restricted API key allowed only `O1` with personal `P1` present: `O1` is
   published in `tools/list` and `/mcp/config`.
9. Personal specialist granted `O1` with ungranted `P1`: `O1` owns the slug.
   NyxBot (orchestrator) and unrestricted humans still get `P1`, and
   `/mcp/config` reports `shadowed_connections = 1`.
10. `resolve_each_target` resolves `O1` by ID; readiness reports `O1` executable;
    durable endpoint loading by `O1`'s ID succeeds.
11. Cross-organization precedence for unrestricted callers follows
    `primary_org_id`, then membership `created_at`.
12. Unit tests for `caller_service_scope` covering every branch, including the
    fail-closed bound-owner cases.

Update any existing test that encoded the old organization-agent behavior
(`tools/list` granted-only, `Allowed` scope assertions) to the new contract, with
the reason in the test.

## Gates

CI-equivalent, run in full (never subsets): `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets -- -D warnings`, and
`cargo nextest run -p nyxid -p nyxid-service-adapters --profile ci` with
`NYXID_TEST_DATABASE_URL` pointing at the dedicated replica set. Plugin/sync
checks only if those files change.

## Rollout

No data change. Affected callers (organization agents everywhere; other callers
only where a slug collision exists) see a different catalog and therefore a
different `catalog_digest`; during a mixed-version rollout replicas may disagree
for those callers until all are upgraded. Execution authority never widens: every
newly visible organization connection is still gated by grants and the live
organization fence.

Post-deploy check (operator): repeat the reporter's steps 4–6 with the
specialist's own conversation; `O1` must be discoverable and its repository read
must succeed through `O1`.

## Review log

Implementer: GPT-6.1-Sol (xhigh). Reviewer: Claude Opus (PM), five rounds plus
early feedback, iterated to zero open findings.

### Implementation audit (2026-10-19)

The D3 caller audit covers every production and test call found by `rg`:

| Caller | Disposition |
| --- | --- |
| `handlers/mcp.rs::get_mcp_config` | Fixed by D2; uses `caller_service_scope` with the auth snapshot. |
| `handlers/mcp_transport.rs::handle_tools_list`, `dispatch_service_tool`, and `handle_meta_call_tool` | Fixed by D2; all use `mcp_service_scope`; meta discovery also uses the matching node scope. |
| `handlers/mcp_transport.rs::load_all_services_for_meta_tools` | Fixed by D2; service and node scope are applied before discovery collisions. |
| `services/assistant_team_service.rs::resolve_each_target` | Fixed by D3; uses `Preferring(requested target IDs)`. |
| `services/assistant_readiness_service.rs::execution_state_for_user_service` | Fixed by D3; uses `Allowed([user_service_id])`. Platform readiness remains unchanged because it resolves a catalog service ID. |
| `services/durable_operation_grant_service.rs` | Fixed by D1; existing `Allowed` scope now filters before collision resolution. |
| `handlers/delegation.rs` and `services/exact_service_approval_service.rs` | Fixed by D1; existing restricted `Allowed` scopes now select before slug de-duplication. |
| `services/assistant_service_execution/async_watch.rs` | Fixed by D1; its existing organization `Allowed` scope now cannot lose the exact service to a personal slug collision. |
| `handlers/async_service_operations.rs` and all other exact service operation paths | Fixed by D1; existing catalog scope is applied in the loader before collision resolution. |
| `services/mcp_service.rs` public `load_user_tools`, `load_user_tools_scoped`, `load_user_tools_all`, and `load_user_tools_all_scoped` callers | Unchanged call signatures and intentional unrestricted behavior; fixed by D1 because their shared loader now resolves authorized candidates before collisions. |
| `services/mcp_service.rs` internal catalog and listing tests | Unchanged fixtures; unrestricted and explicit `Allowed` test scopes continue to exercise the same contracts, with D1 applied before slug de-duplication. |
| `handlers/proxy_concurrency_tests.rs`, `handlers/async_service_policy_tests.rs`, and `services/platform_key_service/tests.rs` calls to the public `load_user_tools*` helpers | Unchanged test callers; these intentionally exercise unrestricted listing behavior and inherit the D1 loader fix. |
| `billing_integration_tests.rs`, `handlers/async_service_operation_tests.rs`, `handlers/mcp_config_routes_tests.rs`, `handlers/mcp_path_parameter_tests.rs`, `services/google_auto_activation_tests.rs`, `services/user_token_service.rs`, and `services/destination_routing_tests.rs` catalog loads | Unchanged fixtures; each uses `Unrestricted` or the existing caller scope for the operation under test, so D1 now applies authority before collisions. |
| `services/channel_adapters/aurinko_tests.rs` catalog loads | Unchanged explicit `Allowed` scopes; D1 fixes ID-scoped collision selection while preserving the adapter's authority assertions. |
| `services/assistant_service_execution/*_tests.rs` catalog loads | Unchanged test callers; their unrestricted or explicit scopes remain intentional and inherit the shared loader fix. |

### Review log entries

- D1–D2: added `Preferring` and request-snapshot-backed `Organization`
  scopes; retained the post-load `permits` fence as defense in depth.
- D3: exact target and readiness lookups now prefer or restrict by ID before
  slug resolution. No execution authority was widened.
- D4: organization-bound REST slug metadata selects a live ACL-permitted
  organization service/pool first, then retains the documented cascade.
- D5: added `shadowed_connections`, `owner_kind`, exact-ID search matching,
  and corrected the chat-access source hint.
- Follow-up F1: removed connection IDs from ordinary search text and changed
  list/search ID handling to trimmed, case-insensitive exact matching. The MCP
  transport preserves exact-ID service matches through its native-tool merge;
  unit coverage proves UUID substrings such as `add` do not affect ordinary
  discovery or relevance order.
- Follow-up F2: restored the no-extra-read listing path and centralised the
  proxy cascade order in `OwnerGrants::from_memberships`; both the timeout
  membership query and the in-memory snapshot use `primary_org_id`,
  `created_at`, then `org_user_id` as a deterministic tie-breaker. Added a
  Mongo-backed ordering regression.
- Regression coverage added or strengthened for the organization collision
  path: exact-ID list/search, personal and other-organization shadowing,
  ungranted same-organization acknowledgement/service-gate behavior,
  unknown personal tool behavior, restricted-key selection, and bound-org
  slug routing. Existing live ACL, target/readiness, durable-loading and
  final-fence tests continue to exercise their respective callers.
- No data migration, error-code, index, or environment change was needed.

### Round 1 (2026-10-20)

| Finding | Disposition |
| --- | --- |
| R1-1 | Fixed. `Preferring` is now a membership class; personal ownership remains the next tie-breaker, independent of preferred-ID order. Added a reversed-order same-slug regression. |
| R1-2 | Fixed. Bound-org REST service and pool candidates require the request snapshot, live ACL, an optional allowed-ID match, and the shared authority/node eligibility helper. Added real proxy slug tests for granted and ungranted bound-org rows. |
| R1-3 | Fixed. Exact connection matching is centralized, exclusive, trimmed and case-insensitive; ordinary search never indexes IDs and the transport no longer appends native or unrelated matches. Added UUID-fragment description coverage. |
| R1-4 | Fixed. Organization scope skips the personal query; allowed rows are filtered before API-key collection, node-route precomputation, and platform-pinned blocking. Organization rows are filtered before the same batches. |
| R1-5 | Fixed. Removed the duplicate scoped loader and unrestricted branch, passed the actor directly to connected-service listing, trimmed target IDs before `Preferring`, and removed the stale OwnerGrants comment. |
| R1-6 | Fixed. The unrelated NyxBot and channel-delivery rewrites were reverted; the current diff has no changes in those files. |
| R1-7 | Fixed. Coverage now includes primary/earliest other-org precedence, exact denial/card service gates with no execution audit, member scope/admin-only narrowing across discovery/config, both MCP execution paths with credential and audit assertions, restricted keys, personal-specialist/NyxBot/human precedence, exact target/readiness/durable ID loading, machine declarations, and the live revocation fence. |
| R1-8 | Fixed. Discovery and organization docs describe restricted visibility, specialist preference, and bound-org REST fallback precisely; the invariant now lives under Streamlined Services in `CLAUDE.md`. |

The round introduced no design deviation. The only implementation choice worth
calling out is that readiness and durable endpoint coverage invoke their
ID-scoped loaders directly in their service-module tests, matching the private
read models used by production callers while avoiding a second public API.

### Round 2 (2026-10-20)

| Finding | Disposition |
| --- | --- |
| R2-1 | Fixed. Bound-organization service slug selection now uses identity, the request snapshot ACL, and the granted-ID allow-list only; credential and route availability are left to execution. The proxy regression also proves an unavailable granted O1 remains selected over personal P1. Pool branches retain the shared availability/authority eligibility helper. |
| R2-2 | Fixed. Search tool definitions are built by one helper shared by exact-ID and ordinary ranked search paths, preserving async descriptions and generic/typed schemas. |
| R2-3 | Fixed. Non-preferring scopes use a zero preference rank, exact-ID queries use an early return, candidate loops rely on the pre-load service filter, connected-service ranking takes an actor string, and the chat hint documents `owner_kind`. |

Round 2 introduced no design deviation.

### Round 3 (2026-10-20)

| Finding | Disposition |
| --- | --- |
| R3-1 | Fixed. Replaced the artificial REST slug test with a real organization specialist conversation key, live grant changes, and local labeled downstreams. It covers granted O1 over personal P1, an unavailable granted O1 remaining selected, and an ungranted bound-organization row falling through to the personal cascade before the final organization fence. The inference-catalog variant was not added because the existing no-auth UserService fixture does not carry catalog inference metadata into the executable target; adding that would require an unrelated credential/platform setup change. The proxy handler now passes an organization specialist's effective grant list to slug metadata selection even when the persisted key has its allow-all bit set. |
| R3-2 | Fixed. The cross-organization ordering regression now uses the existing organization as the earlier membership and a distinct, later-joined primary organization, asserting each different winner as the primary field is set and unset. |

Round 3 introduced no design deviation.

### Round 4 (2026-10-20)

| Finding | Disposition |
| --- | --- |
| R4-1 | Fixed. The proxy routing comment now states that organization-bound callers use the effective grant snapshot, independently of the `allow_all_services` flag and consistently with `caller_service_scope`. |
| R4-2 | Fixed. The discovery documentation now says an unavailable granted connection remains selected, and the cascade runs only when no granted ACL-permitted bound-org service or eligible pool owns the slug. The duplicate restricted-key paragraph was merged. |
| R4-3 | Fixed. Organization-agent documentation now covers REST slug ownership for granted model/inference connections and the unchanged fallback/implicit model exception, and names direct `tools/call` dispatch precisely. |

Round 4 introduced no design deviation.

R4-4 (P2): organization and allow-list scopes still returned Phase-1 platform
catalog services from the discovery loader, so `nyx__search_tools` and
`nyx__list_connected_services` offered platform tools that execution rejected
with `Unknown tool`. Fixed by applying `ServiceScope::permits` once, at the end
of `load_user_tools_with_grants`, for every loader output. Regression: a
credential-free auto-connected platform service is absent from every
organization-specialist surface and its tool name is `Unknown tool`.

### Round 5 (2026-10-20)

| Finding | Disposition |
| --- | --- |
| R5-1 | Fixed. The interim optimization that skipped Phase 1 for restricted scopes changed the catalog recommended-skill fallback for those callers. It was removed; Phase 1 is identical to `main`, and the single end-of-loader `permits` retain is the only scope filter for platform rows. `restricted_scopes_preserve_catalog_skill_fallbacks` proves identical `recommended_skills`, `recommended_skill_refs` and `skills_revision` under unrestricted, allow-list and organization scopes. |

### Final status (2026-10-20)

- Every plan item (D1–D5, WP1–5, regression tests 1–12) is implemented and
  covered; the targeted regression set (9 tests) passes on Rust 1.98.1 against
  MongoDB 8.0; `cargo fmt --check` passes.
- A local full `nextest` run reached 2,658 passing tests before it was stopped
  for R4-4; its only failure was a 300 s timeout in
  `handlers::catalog_editor_tests::key_updates::key_put_revocation_during_transaction_leaves_no_partial_write`,
  outside this change, which passes in isolation (63 s). Per the owner's
  decision, the full suite (clippy, nextest, coverage) runs in GitHub CI on the
  pull request.
