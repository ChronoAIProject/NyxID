# Service authorship and change history

Status: implemented on `org-service-edit-history` against main at `e3ff0150`. The agreed service-only scope, generic fallback, and backend-defined options contract are recorded below. The implementation and deployment reference is [SERVICE_HISTORY.md](../SERVICE_HISTORY.md).

The implementation includes atomic service mutation and history capture, current-scope authorization, shared-resource fan-out, retained deleted-service history, audit publication, and creator/latest-editor metadata. The admin form-safety contracts from main are preserved. Service detail drafts now survive transient read failures, and service or signed-in identity changes discard the previous editor and confirmation state.

Validation commands and deployment prerequisites are documented in [SERVICE_HISTORY.md](../SERVICE_HISTORY.md). The pull request records the final local results and CI status. Browser rendering and Docker container execution were unavailable locally and are not claimed as verified.

Requested outcome: org admins can see who created and last changed a service on its card, and can open a history of changes within the service. The same mechanism should support personal and org services.

## Product recommendation

Add this feature. Shared services control credentials, routing, and identity propagation, so attribution helps admins diagnose failures and understand configuration changes. The existing audit trail has useful building blocks but does not currently supply a complete service history.

The first release should cover service instances shown under AI Services (`/keys`): org and personal services, catalog-backed and custom services, HTTP and SSH, direct and node routing. Include platform-managed instances with verified system/app attribution and a read-only History entry point. Platform catalog templates are a separate resource; their history must be distinguishable from changes to a user's service instance. NyxID agent keys, service pools, and nodes are separate resources, not additional service-card variants in this release.

### Card and detail experience

- Add a compact footer at the bottom right of each authorized service card: `Created by Alice · 12 Sep 2026` and `Last edited by Ben · 2 hours ago`. Names and dates here are illustrative.
- Keep endpoint and proxy information legible. Let the footer wrap below that information on narrow screens. Provide full timestamps with timezone through an accessible tooltip and in history.
- On a newly created service without subsequent changes, show `No edits since creation`. For legacy services, use `Creator not recorded` and `Earlier edits not recorded` where necessary.
- Add a `History` tab alongside Overview and Advanced, including an equivalent entry point for platform-managed detail pages. Preserve table-view parity for the authorship metadata.
- Show newest changes first: actor, timestamp, action, and changed fields. Group local commits from one user action into one expandable row. Expand rows to reveal safe before/after values. Paginate groups on the server without splitting a logical action into duplicate rows across pages.
- Every captured configuration change has a readable entry even if no detailed description matches it. Use `Service updated` with the verified actor and exact timestamp as the generic fallback; the entry updates the card's last-edit summary in the same way as a recognized change.
- Record successful persisted changes. Opening, reading, using, or saving identical non-secret configuration does not create a change entry or move the last-edit footer. Intentional credential replacement remains a security event even if the caller resubmits the same secret; do not expose a secret-equality oracle.

### Events and safe details

| Change | History presentation |
| --- | --- |
| Created | Creator or explicit system actor, creation time, service identity |
| Label or endpoint edited | Field names changed; free-form values omitted by default |
| Routing changed | Direct/node transition; safe node identity only within the reader's permissions |
| Enabled or disabled | Previous and new enabled state |
| Auth or identity settings changed | Reviewed enum and boolean before/after values |
| Credential replaced, rebound, or reauthorized | Event type and safe connection metadata; no secret material |
| Credential source changed | User credential to platform credential, or the reverse; never platform credential material |
| OAuth scopes changed | Reviewed scope identifiers and grant result after completion |
| SSH configuration changed | Changed fields and reviewed safe values; no private keys or certificate bodies |
| Default headers or WebSocket auth rules changed | Field/header names and counts; no values, templates, payloads, or match expressions |
| Deleted | Durable deletion event retained after endpoint/credential deletion |
| System provisioning or configuration change | Explicit system actor and reason; never attributed to an arbitrary org admin |
| Other persisted service configuration change | `Service updated` with verified actor and exact timestamp; no unreviewed field names or values |

Routine OAuth refresh, last-used timestamps, health checks, node presence, proxy traffic, and discovery-cache updates are outside edit history. Failed attempts can remain operational/security audit events but must not appear as successful edits. A partially applied operation needs truthful outcome records for the parts that committed. Coverage means changes made through NyxID; direct database edits, upstream-provider console changes, and unreported node-local file edits cannot be reconstructed by this feature.

### Generic fallback for service changes

Detailed classification enriches an entry; it must never decide whether a real configuration change is recorded. The common service-mutation path captures the verified actor, service/owner IDs, committed time, and operation group even when no specific event formatter or safe field-detail mapping exists. Example: `Alice updated Firecrawl · 16 Sep 2026, 14:32 SGT`. When useful, add `Additional change details are unavailable`; never invent a reason or values.

- Detect an actual configuration change at the transaction boundary independently of the safe-detail allowlist. A field excluded from before/after display can still trigger a generic entry. A bumped operational timestamp or state version alone is insufficient evidence.
- Preserve the established exclusions for no-op saves, reads, usage, routine refreshes, and health updates. A successful HTTP status alone must not create a generic entry.
- If an action contains recognized and unclassified configuration changes, show the recognized details plus `Other service settings updated` in the same grouped entry. Do not add a duplicate fallback for a change already fully described.
- Store only the generic metadata when details have not been reviewed for safe exposure. Do not copy arbitrary request keys, values, URLs, or model snapshots into the fallback.
- Generic entries use the same transactional capture, actor attribution, retry deduplication, access controls, retention, audit publication, and last-edit projection as detailed entries.
- All in-scope mutation paths must use the common recording boundary. The fallback covers missing classifications, not writes that bypass logging; it does not reconstruct missing historical records.

### Access and historical accuracy

- Org admins may read history only for services within their current resource scope. Personal owners may read their own service history. Apply the same checks to the footer response fields and history API; hiding a tab is insufficient.
- Keep detailed history and attribution admin-only for org services in the first release. Existing member/viewer access to ordinary service details does not automatically grant audit-history access.
- Attribute the acting person/API key independently of the polymorphic service owner. Record API-key identity, an explicit system actor, and an application/assistant initiator where server-verifiable. Do not infer a human from an org-owned API key.
- Store stable actor IDs plus minimal display-name snapshots so departed users and deleted API keys do not erase attribution. Preserve service identity across slug changes and deletion. Define identity-data retention with the existing account-deletion policy.
- Existing timestamps can be preserved as known timestamps; they do not prove who made an edit. Never populate old creator fields from the present org admin or rewrite chained historical audit rows. Show when reliable tracking began.
- History is read-only; restoration/rollback, change approvals, notifications, and audit export are later features.

## Current repository evidence

Initial history review at `9bb33bcf`; options integration and affected service-model/write paths rechecked at `efc20d24` on 2026-09-17.

| Area | Evidence and implication |
| --- | --- |
| `frontend/src/pages/keys.tsx`, `KeyCardContent` and table rows | The screenshot is the unified service-instance card, not the platform catalog `ServiceCard`. |
| `frontend/src/pages/key-detail.tsx`, `KeyDetailPage` | Overview and Advanced already exist. Platform-managed instances take a separate rendering branch. |
| `backend/src/models/user_service.rs` | Stores `created_at`, `updated_at`, and `state_version`, but no creator/editor fields. |
| `backend/src/services/unified_key_service.rs`, `build_key_view` | Assembles service, endpoint, and credential data but exposes the service row's timestamps. |
| `backend/src/handlers/keys.rs`, `update_key` | A single request can update several records and perform node delivery. History cannot be inferred solely from the HTTP result or the submitted body. |
| `backend/src/handlers/user_endpoints.rs` and `user_api_keys_external.rs` | Endpoint and credential edits have separate routes. Shared credentials may affect several service instances. |
| `backend/src/services/user_service_service.rs`, `update_user_service` | Unconditionally advances the row timestamp/version on accepted updates, including unchanged configuration; currently audits header names, not general service edits. |
| `backend/src/handlers/assistant_action_effects_services.rs` | Assistant mutations include direct transaction-aware writes that would bypass handler-only logging. |
| `backend/src/services/audit_service.rs` | Supplies `AuditActor` and chained logging. Both asynchronous and awaited audit entry points currently describe persistence as best-effort relative to the operation. |
| `backend/src/services/audit_chain_service.rs` | Global sequence and HMAC chain already exist. Append currently has no caller-session interface. |
| `backend/src/handlers/services.rs`, `update_service` | Catalog updates already log changed field names. Header values are deliberately excluded even when marked non-sensitive. |
| `docs/AI_SERVICES_ARCHITECTURE.md` | Delete retains an inactive service tombstone while removing its endpoint and credential. Disabled services resolve for management by UUID, not slug. |
| `.github/workflows/ci.yml`, `docker-compose.yml` | CI exercises transactions against a replica set; basic Compose still starts standalone MongoDB. Any new transactional prerequisite must be explicit and tested. |
| `docs/OPTIONS_API.md`, `backend/src/services/options_service.rs` | Registered option-set framework exists; only `service-scope` is implemented. It exposes labels, descriptions, source, disabled state, content version, freshness, search and pagination. Scope suggestions are explicitly not an exhaustive enum or authorization policy. |
| `backend/src/handlers/options.rs` | Query validation and authorization currently assume service accounts before resolver dispatch. History requires its own context and owner/resource authorization. |
| `frontend/src/hooks/use-options.ts`, `schemas/options.ts`, `types/options.ts` | Reusable loading/version-reset pattern, but the schema and context are literal `service-scope`/`service_account`, and pagination bounds are scope-specific. Extend deliberately rather than pass history through this contract unchanged. |
| `docs/PLATFORM_KEYS_AND_INFERENCE.md`, `backend/src/services/unified_key_service/platform.rs` | New `credential_binding` supports platform/user choices with dedicated mutation paths. Automatic platform service cleanup can physically delete service rows; history must survive without relying exclusively on a `UserService` tombstone. |

## Implementation contract

### Authorship and event shape

Add optional `created_by` and `last_change` summaries to `UserService`. The last-change summary carries actor, timestamp, action, and change-group ID; it advances on a meaningful persisted configuration change or intentional credential replacement. Keep these separate from operational timestamps. All new optional BSON dates use the existing optional datetime helper; models remain plain serde structs and API responses remain dedicated response types.

Create `service_change_events` with UUID `_id`, `COLLECTION_NAME`, `service_id`, `owner_id`, `change_group_id`, affected entity type/ID, action, actor context, server timestamp, changed field names, and a bounded list of safe value changes. Store the originating server-known operation context; a client-provided User-Agent is not proof that an actor used the CLI or browser. Keep immutable event payloads separate from mutable audit-publication metadata (`audited_at`, `audit_log_id`). A shared backing-resource edit must be linked to every affected service without revealing other services outside the reader's scope. Keep events outside service documents; never embed an unbounded history array.

Actor variants cover people, API keys, service accounts, apps, and system components. Derive them from verified auth and carried initiation context. Store stable IDs and minimal display-name snapshots, without adding emails to history. Later reads can optionally enrich current names in one batched lookup; never issue one lookup per card. Account-deletion policy controls any required anonymization, while immutable audit retention remains explicit.

Use an explicit field allowlist before persistence. Default free-form and secret-bearing fields to names-only. Do not serialize whole request bodies or models and then try to mask them in the frontend. Exclude credential ciphertext, access/refresh tokens, passwords, private keys, arbitrary headers and frame payloads. URLs can carry secrets in userinfo, paths, or queries, so raw URLs are not automatically safe history values.

The initial value allowlist is `is_active`, `admin_only`, `auth_method`, `ssh_auth_mode`, `identity_propagation_mode`, `identity_include_user_id`, `identity_include_email`, `identity_include_name`, `forward_access_token`, `inject_delegation_token`, `node_priority`, authorized node IDs, reviewed OAuth scope identifiers, and `credential_binding` (`user`/`platform`). Use the effective normalized binding when comparing legacy absent values with explicit values; persisting an equivalent default alone is not a user configuration change. Other fields are names-only; header/rule summaries retain only reviewed names, counts, and direction. Do not enable generic serialization of new fields into history.

Preserve the existing `state_version` and credential-epoch semantics. `state_version_after` may be informational evidence but is not a unique whole-service history sequence: endpoint and credential changes have different write paths. Use event IDs and a stable group ID for correlation, and a separate history sequence if needed for committed ordering. Group identity covers the whole logical action while each event represents one actual local commit.

### Durable recording and audit integration

The key invariant is that every committed in-scope configuration change has durable safe history, with the correct actor and time. Detailed entries also record approved before/after state; unclassified changes have generic metadata. Calling best-effort audit logging after a mutation is insufficient for this invariant. An awaited write after commit still leaves a crash gap and misleading retry behavior.

Decision: use the dedicated transactional `service_change_events` journal as the authoritative product history. `created_by` and `last_change` are compact projections saved atomically with the relevant event. Publish a safe immutable copy of each event into the existing audit chain through a durable retrying relay. Keep global audit-chain sequencing outside the mutation transaction; its global sequence contention should not delay or abort ordinary service edits. Reuse the existing HMAC machinery rather than adding a second custom chain.

The journal itself supplies durable capture, not independent cryptographic tamper evidence. The audit mirror must contain the safe immutable event payload (or its canonical digest), not just its ID, if it is used to verify journal contents. Verification must compare the journal to its mirror; a valid chain alone says nothing about later changes to a separate journal row. Events waiting for publication are not yet covered by that mirror. Report publication backlog count/oldest age and retry failures; retain the audit chain's existing limits, including its unanchored-tail limitation.

Make publication idempotent with a stable audit UUID derived from or identical to the event UUID, plus an append-path check that a previously inserted row matches the intended event. A crash after append but before recording publication success must not produce duplicate mirror entries on retry. Relay markers alone do not provide this guarantee. Add journal indexes for owner/service/history ordering, group retrieval, and pending publication; do not add a history TTL or cascade service-event deletion when credentials/endpoints are removed.

The relay's append variant must distinguish duplicate `_id` from sequence collisions. On duplicate key, look up the intended audit `_id`: matching immutable payload/digest means publication already succeeded; different content is an error and must never overwrite a chained row. Only an absent `_id` permits treating the error as a sequence collision and retrying. The existing generic duplicate-key retry loop alone cannot implement this contract. Preserve the audit chain format and verification behavior.

The implementation must satisfy all of the following:

- Read committed before/after configuration within the same transaction. Use an atomic pre-image (`find_one_and_update` returning the previous document) or an equivalent transactionally fenced read, and compute the normalized committed post-image including sets, unsets, and pipeline effects. A separate pre-read and post-read can attribute another editor's values to the wrong person.
- Persist local mutations, event identity, and footer metadata together. Preserve IDs across transaction retries and deduplicate asynchronous publication. Converge the session and non-session service-write helpers onto a shared transaction-aware implementation.
- Use one transaction per local commit point. A `PUT /keys` request can cross several commit points with node delivery between them; do not put the whole handler or remote calls inside a retryable transaction. Mint `change_group_id` at the trusted operation boundary, propagate it through sub-writes/callbacks, and reuse existing operation receipts for retries where applicable. This grouping ships with the first release.
- Make the service-layer recording contract common to UI, CLI, public API, assistant actions, OAuth/device-code completion, node management, system provisioning, and lifecycle paths. Audit low-level direct database writes as well as named helper calls.
- Capture actual shared-resource impact, creation-time actor context carried through authorization callbacks, and deletion evidence before dependent records disappear.
- Keep external provider/node actions outside retryable database callbacks. Record their observed outcomes separately when necessary; a local commit must not be presented as confirmed node delivery.
- Require transaction-capable MongoDB and align deployment, local development, startup checks, and CI explicitly. Include authenticated replica-set initialization/internal keyfile handling for existing Compose credentials and a documented migration for existing volumes. Support a single-node replica set or mongos, as appropriate. Never silently downgrade to lossy history on standalone MongoDB or commit a change first and discover the missing prerequisite afterward.
- Preserve the existing audit chain, verification behavior, and legacy rows. Service history does not by itself add full database tamper protection or audit tail-truncation detection.

### API and queries

Expose the compact summary on existing key list/detail responses so cards do not make one history request per service. Provide a service-UUID-based history endpoint, provisionally `GET /api/v1/keys/{service_id}/history`, with bounded cursor pagination and stable ordering of change groups. Authorize against the owner and current service scope on every request. Existing member-readable key resolvers alone are insufficient for this admin-only history route; use explicit role/scope checks and a tombstone-aware history resolver.

Index by owner, service ID, and history ordering keys. Keep the history read path functional for disabled services and deleted tombstones without loading a deleted endpoint or credential. A deleted service's history should remain accessible to authorized admins through a retained link or audit entry. A reused slug must not inherit another service's history. Account/org deletion follows the existing audit retention/access policy.

Latest-main platform reconciliation also physically removes automatic service rows. Persist their removal event and retain the service UUID/owner identity in history before cleanup. Authorize archived-history reads using the retained `owner_id` through `org_service::resolve_owner_access` and current resource scope against the retained service UUID, rather than requiring a live `UserService` row. Org readers still require admin history access; no weaker archive permission is introduced. Re-provisioning a new instance must not inherit the old instance's history by slug. Classify automatic cleanup as a system action and do not attribute it to the user whose read triggered reconciliation.

### Shared enums through the existing `/options` endpoint

Decision: extend the existing registered-set design in `services/options_service.rs`, following `docs/OPTIONS_API.md`. Use typed backend history definitions as the shared source for event population and options responses. Do not add a parallel configuration file or duplicate the full enum/label map in the frontend. Existing model enums and validation constants should supply domain values such as SSH auth modes wherever applicable.

Proposed initial sets (new registrations, not available on current main):

| Option set | Backend source and purpose |
| --- | --- |
| `service-history-action` | History action definitions: stable code, label, description, and group. Examples: `service.created`, `service.updated` (generic fallback), `service.enabled`, `service.disabled`, `service.deleted`, `service.credential_replaced`, and `service.credential_binding_changed`. |
| `service-history-field` | Reviewed history field definitions with stable codes and display names, such as `auth_method`, `node_id`, `admin_only`, and `credential_binding`. Include only fields approved for field-name exposure. |

Add an enum-value set only when an actual UI consumer needs it; derive it from the corresponding backend domain definition. Do not treat service-account tokens such as `proxy` or previously configured custom scope strings as service-change enums. History actions are a closed server-authored vocabulary with a generic fallback. Missing current metadata must not make a historical code or a retired safe enum value unreadable.

Required integration work:

1. Dispatch query validation and authorization by registered option set. The current `OptionsQuery` requires `principal_type=service_account`, and `get_options` applies service-account management permissions before dispatch; these cannot be reused unchanged. The two initial history sets contain static product vocabularies, with no owner/resource data: require the route's existing authentication but no fabricated owner/service-account context. Future dynamic actor/node sets require owner/service context and the history role/scope checks, including archived identities. Do not overload `service_account_id` or call a service instance a service-account principal. Preserve the existing service-scope request, response, custom-input behavior, and ACLs. Reject unrelated context fields according to each set's query schema.
2. Reuse the `OptionItem` presentation shape and return `source: backend_definition` for registered actions/fields. Populate version/freshness from the history definition version and actual content. Static sets report `resources: static` through their own validated response variant; the existing service-scope variant retains `resources: live`. Keep `Cache-Control: private, no-store` and live request authentication/authorization. Do not copy the scope-specific version, configured-scope source limits, or 10003 pagination cap into new resolvers. Use bounded per-set pagination. Omit `selected_items` from history variants or return an empty array; service-account edit selections retain their existing semantics.
3. Extend backend OpenAPI schemas and frontend response/context types by option set. The current frontend Zod schema accepts only `service-scope` and `service_account`; use explicit discriminated response/context types rather than loosening all validation to arbitrary strings. Check returned option set and all security-relevant context against the request. Keep item action/field codes forward-compatible on the display path; an unfamiliar code should use the generic renderer rather than invalidate the full history response.
4. Extend `useOptions` with set-specific context serialization, enablement, and pagination checks while preserving identity/owner-separated query keys, immediate staleness, refetch/invalidation, and restart on version drift. Static-set queries require a signed-in identity but not `context.owner_id`. Validate that a non-null next offset advances and is below `total`, retaining a page-count safety bound; backend offset bounds remain per set. History filter pickers can reuse `AsyncOptionSelect` with custom input disabled for a closed action menu; the existing scope picker keeps custom input enabled. Extend the picker's context-reset logic alongside the hook so it no longer assumes every set carries service-account context.
5. Populate event codes and approved field details at the backend commit boundary using these shared definitions. The event writer calls local code; it never makes an HTTP request to `/options`. History entries still come from the history endpoint and retain actor, resource, committed time, schema version, and safe details. No client-provided placeholder, label, or selected action establishes that an event happened.

History responses also embed resolved action/field labels from the same backend definitions. Render the timeline from those responses; `/options` supplies filter choices and other consumers' metadata. Current options pages/searches are not a full label dictionary, and the options route currently rejects API-key, service-account, and relay credentials; history clients must not gain a new dependency on that route or broaden its authentication policy. Validate delegated management-read behavior using the existing verified extractor rules. If a future reader cannot call options, the history response remains self-contained. Adding static sets is part of the requested initial integration, while capture and timeline rendering remain independent of their runtime availability.

Backend authorization, transaction capture, redaction, and the safe-value allowlist remain enforced independently of presentation options. A new label cannot enable logging of secrets or suppress a required generic event. An unclassified committed configuration change is recorded as `service.updated` with trusted actor/time; no-op/operational exclusions still apply. Missing display metadata uses a generic label without rewriting the stored action code. Keep a small built-in generic display label for missing history labels. Authorization/context errors must clear unauthorized cached data, not retain prior-owner options or events under the fallback.

Stable action codes are never reused for a different meaning. Retain definitions for retired codes and keep them queryable as history filters; filtering a past action is not permission to perform it. Bump the definition version on metadata changes. Event `schema_version` versions the stored event format independently of options `definitions_version`. Preserve approved identity/display snapshots for renamed/deleted actors and resources; current dropdown data cannot reconstruct history. Dynamic actor/node filters are separate optional sets backed only by resources/history the reader is authorized to see. Neither arbitrary previously observed data nor external-provider metadata can expand the backend history enum or recording policy.

Tests should assert a specific action code for known specialized mutations and generic capture for otherwise unclassified meaningful changes. Creation, deletion, and generic events can legitimately have no field-value diff. Generic-use counts may help improve descriptions over time, but a missing specialized mapping must never suppress recording or reject a valid service edit.

## Delivery and acceptance

1. Inventory all writers and freeze the event, attribution, access, and safe-value contracts. Align transaction prerequisites and unify write helpers. Prove atomic capture and idempotent audit publication with failure injection.
2. Implement durable capture, audit publication, operation grouping, and compact summaries across the full service-instance scope. Add indexes and backward-compatible response fields. Include callbacks, assistant paths, shared resources, system actors, and deletion; these are first-release coverage requirements.
3. Add the two registered history option sets backed by shared definitions and extend the options query/response/frontend context contracts. Add card/table metadata and History with self-contained labels plus options-driven filters, loading/error/empty/legacy/permission states. Refresh both the summary and history after edits.
4. Verify and release the coherent service-instance feature. A subsequent extension can apply the same mechanism to platform catalog templates in a separately visible resource scope; retain existing catalog audit logging throughout. CLI history display and cross-service export are optional later surfaces; edits from the CLI must already be recorded in the first release.

The writer inventory must classify at least unified key management; direct service, endpoint, and external-credential routes; assistant transaction paths; OAuth/device-code/connect-link completion; automatic provisioning; SSH and node binding changes; credential rebind/reconciliation; platform/user credential-source switching and platform connection edits; automatic platform removal/re-provisioning; catalog identity propagation that persists into instances; and cleanup/deletion paths. For each, identify the mutation helper, actor source, grouping, and regression coverage, or justify why the write is operational and excluded. Raw textual source scans can assist review but cannot prove complete coverage; typed writer boundaries and behavior tests are the release gate. Do not advertise complete historical coverage before this gate passes.

Required validation:

- Correct creation/edit actor for personal users, org admins, scoped admins, API keys, assistants, and verified system actions.
- No cross-org, member/viewer, out-of-scope, or deleted-owner metadata leaks, including summary fields.
- Meaningful before/after values under concurrent writes; retries do not duplicate events; no-op configuration saves do not move the footer.
- A persisted configuration change with no recognized detailed mapping produces a generic entry and advances last-edit attribution. A mixed recognized/unclassified action retains both kinds of information within one group. Known detailed changes do not also produce redundant generic entries. Operational-only writes and no-ops produce neither kind.
- Endpoint-only edits, label-only edits, shared-key changes, credential replacement, successful OAuth reauthorization, SSH changes, Enable/Disable, and Delete are all represented.
- Abort after event insertion leaves no event, stamp, or entity change. A crash after local commit but before publication is recovered by the relay. A crash after audit append but before publication marking produces exactly one matching mirror row after retry. Concurrent edits have correct committed before/after values. External failures produce accurate partial outcomes.
- Secret fixtures never appear in stored history, audit payloads, API responses, or UI. Test URLs, headers, frame templates, and credential payloads.
- Legacy rows show uncertainty honestly. Disabled/deleted UUIDs remain accessible only under current authorization; slug reuse cannot merge histories.
- Responsive card footer, table parity, keyboard-accessible exact timestamps, paginated history, and platform-managed detail rendering.
- Missing/stale `/options` data, unknown or retired action codes, and renamed/deleted option entities preserve readable actor/time history. Presentation changes cannot expand recorded secret values or prevent generic capture. Definitions served to clients match the backend registry, with scoped dynamic choices.
- Static history option sets require existing route authentication and expose no owner data; resource history and future dynamic options enforce personal-owner/scoped-org-admin access. Scope-suggestion endpoint behavior remains compatible. Validate unrelated context fields, response-context mismatches, version drift, set-specific bounds, permission revocation, timeline labels independent of options search/pagination/availability, closed action filters including retired codes, and rejection of arbitrary action codes on any write input.
- Platform/user binding changes record the effective enum transition, automatic removal preserves history after physical row deletion, and re-provisioned UUIDs do not merge by slug.
- Real MongoDB transaction/integration tests plus relevant Rust checks, frontend tests, type checks, lint, and build. This proposal alone does not require executing product test suites.

This is a moderate backend feature with a small UI surface. Complete recording and accurate attribution are the substantial work; adding labels to cards is only the presentation layer.
