# NyxID Tools Registry and direct-provider migration

Status: product and engineering proposal, prepared on 2026-10-07. This is the design for the next implementation. Vendor signup, email verification, billing enrollment, and account setup remain manual. No sidebar code, runtime endpoint, provider credential, or published offering is changed by this document.

NyxID should add **Tools Registry** to the admin sidebar. It becomes the workspace for importing tool definitions, tracking manual provider setup, validating execution, and publishing offerings. Users get a **Tools** entry beside **AI Services**, where they can find ready-to-use platform offerings and grant them to their AIs without connecting a vendor account.

The registry builds on the existing catalog, OpenAPI contracts, encrypted master credentials, platform audiences, exact operation grants, proxy, and MCP. It stores onboarding progress and import provenance. The existing `DownstreamService` and `ServiceEndpoint` records remain authoritative for execution.

## 1. Navigation and product distinction

| Sidebar entry | Audience and route | Purpose |
| --- | --- | --- |
| AI Services | Existing user entry, `/keys` | Personal or organization connections, credentials, service pools, and Agent Keys |
| Tools | New user entry, `/tools` | Published platform offerings, topics, source details, price, availability, and AI access controls |
| Tools Registry | New admin entry, `/admin/tools-registry` | Import, configure, validate, publish, pause, and maintain offerings |
| Services and Providers | Existing admin entries, `/services` and `/providers` | Detailed catalog configuration and existing authentication-provider administration |

Use the current `hasAdminRead` and `canAdminWrite` role conventions: operators can inspect registry metadata and progress; admins can import, change setup, request execution checks, and publish. Credential entry remains in the existing privileged service editor. Ordinary users see only the published offerings allowed by their live service visibility and platform audience, with a separate explanation of whether their selected AI has a grant.

The boundary between the two user entries is whose account answers the call. An AI Service acts as the person or organization: their OAuth token, key, or endpoint, and the data that account can reach. A Tool acts as NyxID: a platform-held vendor account, or no account at all. The test is whether two different users calling with the same input reach the same thing. If they do, apart from quota and price, the operation is a Tool. If the result depends on whose account is connected, it is an AI Service.

Side effects, cost, and data scope do not define the boundary. They are per-operation classifications. Tools include POST searches, generation that creates new output, and jobs that change state. AI Services include read-only operations such as listing mail. "Read-only, public data, verified free" is the admission rule for the first Public Data bundle, not the definition of a Tool.

Two configurations need explicit handling so the boundary stays visible:

- Tools run on the same automatically provisioned platform-key rows that AI Services already shows under its auto-connected filter. A product-kind field on the catalog offering decides which entry lists it. The `/keys` listing must not present a Tool's internal provisioning rows as connections.
- One vendor can appear in both entries, for example Firecrawl with the user's key and Firecrawl with NyxID's key. Cards state "Your account" or "Provided by NyxID" so the two offerings do not look like duplicates.

Tools is broader than public research. Its categories can include Public Data, Enrichment, Generation, and Owned Resources. Public Data is the first release. Each operation needs its own data-scope, cost, and side-effect classification before it can join a public-data bundle. Account-management, communications, storage, browser, and machine operations need their own authority and ownership contracts.

Keep AI Services as the current label; no existing personal-service screen needs to be renamed for this feature. The Tools page explains the distinction with product copy: “Tools provided by NyxID. Choose which of your AIs can use them.” Cards show source, supported tasks, availability, user price, and limits. Show “Free” only when the NyxID user price is verified as zero, and explain any capped allowance separately.

## 2. Registry workspace

The MVP has three tabs: **Providers**, **Tools**, and **Imports**. A “Needs setup” filter on Providers supplies the setup queue; a separate workflow application is unnecessary.

| View | Information | Actions |
| --- | --- | --- |
| Providers | Vendor label, verified API operator when known, tool counts, contract progress, account setup, credential check, publication, next blocker | Open vendor documentation, configure services, continue setup, validate, publish selected tools, pause |
| Tools | Provider/path identity, native contract mapping, runtime requirements, category, data scope, upstream cost evidence, user price, selected catalog operation, validation state | Inspect schema and provenance, select offering, classify, review mapping, inspect blocker |
| Imports | Pinned source/version, inventory hash, added/changed/removed operations, source aliases, exceptions, reviewer, applied revision | Preview diff, accept selected changes, retain previous snapshot |

A provider detail has four sections: **Overview**, **Setup**, **Operations**, and **Validation & changes**. The setup panel records the responsible operator, vendor dashboard/documentation links, account-access evidence, billing choice, quota, and remaining task. It stores references and nonsecret notes; keys, passwords, card details, and login cookies stay outside registry records.

An example layout, showing planned states rather than configured integrations:

```text
Tools Registry                          [Import definitions] [Add provider]
Providers   Tools   Imports
Search providers…      Needs setup ▼      Category ▼

Provider    Contract work       Account setup        Published    Next step
TinyFish    Review source       Manual review        0            Verify account access
Firecrawl   Reconcile overlay   Check deployment     0            Choose existing service
TikHub      Obtain native spec  Identify API account 0            Find executable contract

Provider detail → Setup
  Vendor account / product access       [Open documentation]
  Billing and shared capacity           [Record limits]
  Catalog services / encrypted keys     [Configure service]
  Normal-user validation                [Create validation request]
  Selected operations                   [Review publication]
```

The registry tracks Monid provider identities initially. An identity such as a dataset brand may be served by a different API operator or aggregator. Do not translate 87 provider labels into an assumed 87 vendor accounts. Verify the API operator, access model, and native contract, then group offerings under the correct vendor while retaining all original import identities.

## 3. Manual setup flow

```mermaid
flowchart LR
    A[Import definitions as candidates] --> B[Review native contracts and tool scope]
    B --> C[Operator creates or selects vendor account]
    C --> D[Operator enables product access and billing if needed]
    D --> E[Configure existing catalog services and credentials]
    E --> F[Set audience, operations, user price and shared limits]
    F --> G[Validate as an authorized normal user]
    G --> H[Publish selected tools]
    H --> I[Users grant access to their AIs through existing grants]
```

1. **Import before account setup.** The importer creates candidate records and reviewed specifications without execution authority. A provider can wait in the setup queue while its contracts are prepared.
2. **Verify the actual supplier.** Record the native API operator and product documentation. Confirm that a platform account can provide the intended data access, and whether the access is free, limited, paid, or still unknown. A Monid price describes Monid's offering; it does not establish the native vendor's price.
3. **Complete signup manually.** The operator opens the vendor site, creates or selects the platform account, handles email/SSO verification, activates the relevant products, and enables billing if selected. The registry records “account ready” only as an operator milestone; that state is separate from credential validation.
4. **Configure service credentials.** Open the existing service editor. For an ordinary direct shared-key offering, use HTTP, a fixed vendor origin, `service_category = "internal"`, `requires_user_credential = false`, `provider_config_id = null`, the correct auth injection, and the existing encrypted master-credential field. Configure the live platform-key audience. Verified no-auth APIs use `auth_method = "none"` and no stored credential.
5. **Configure each execution boundary.** Split catalog services by origin and credential class. TinyFish search/fetch require separate services. One vendor account can supply both; configure credentials through each service's current encrypted field. The registry does not introduce a reusable secret-reference mechanism. Two-key products, Basic auth, or custom prefixes require an explicit verified mapping to the existing injector or a reviewed adapter.
6. **Choose what to offer.** Select reviewed operations, set the exact method/path policy, declare data scope and risk, set the NyxID user price or subsidy, and configure shared account quotas. A free upstream tier stops at its enforced capacity; reaching a limit does not silently enable paid execution.
7. **Validate execution.** Run contract parsing and fixture checks first. A bounded live request requires an explicit operator validation request with selected operations and a cost ceiling. For the first pilot, use a deliberately configured restricted test audience and a legitimate normal-user/agent grant. Validation uses the ordinary execution path; admin status is not an execution bypass.
8. **Publish the reviewed subset.** Recheck current configuration and validation evidence on the server, then publish that subset through existing catalog fields and operation policies. The resulting cards and MCP tools appear for eligible users; they receive no vendor key or vendor connection prompt.

The initial import and manual setup steps do not require browser/computer automation. Periodic catalog import, schema diffing, and configured contract checks are engineering tools; account creation and purchase decisions remain operator tasks.

## 4. Track independent states

A single “connected” badge hides too much. Store workflow milestones and compute live access facts independently:

| Dimension | Example states | Source of truth |
| --- | --- | --- |
| Contract progress | Inventory only, native contract needed, review needed, adapter needed, validated | Registry candidate, pinned spec, and validation receipt |
| Account setup | Not reviewed, action required, ready, access issue | Manual setup checklist and verified access evidence |
| Credential presence | Not required, missing, configured, unknown | Existing privileged credential-status calculation; never a persisted registry secret |
| Credential usability | Not checked, verified, failed, stale | Bounded validation receipt tied to current configuration |
| Publication | Draft, selected tools published, paused | Current catalog active flags and operation policy |
| Caller availability | Available, grant needed, quota exhausted, unavailable | Current actor visibility, audience, grant, billing, and capacity evaluation |

Configured does not mean usable. The existing `credential_configured` view is optional, and unreadable credentials can produce an unknown state. A previous success becomes stale when its relevant service configuration, credential, operation generation, spec, price, or limit changes. Record the operation generation, spec hash, nonsecret configuration revision, test identity, selected inputs after redaction, result summary, and check time. Add a nonsecret revision/invalidation hook to service writes where current timestamps are insufficient; do not hash or expose key material as a verification identifier.

## 5. Architecture and implementation boundary

### Vendor grouping is separate from authentication providers

`ProviderConfig` represents existing authentication/provider behavior. It must not become the vendor-registry model. The current service create/update validator rejects an operator-supplied master credential combined with `provider_config_id`. Its supported direct shared-key shape is an internal, provider-less service.

Create registry vendor metadata that references catalog services independently. Existing personal OAuth/provider services retain their native relationships. Reconcile an existing Firecrawl/OpenAI/Gemini/ElevenLabs contract when useful, but do not silently repurpose a personal connection or assume a deployed shared credential exists. When the current row cannot support the desired shared-key shape, create a dedicated internal offering after operator review.

### Existing execution machinery

| Responsibility | Reuse | Proposed addition |
| --- | --- | --- |
| Fixed destination and credential injection | `DownstreamService`, encrypted master credentials, platform-key ACL | Registry references and manual setup checklist |
| Typed operations | OpenAPI parser, hosted spec registry/sync, `ServiceEndpoint`, positive generations | Pinned importer, candidate mappings, staged diffs, native overlays |
| AI discovery/call | Existing MCP search/call and visibility resolver | Product categories, scope/cost/source metadata, user Tools browse view |
| Authorization | Exact service/operation grants, live audiences, approval revalidation | Shared public-data bundle later if selected; existing grants for MVP |
| Billing | Existing credential-class pricing and exact accounting | Separate upstream cost evidence and shared provider capacity admission |
| Audit | Existing append-only audit service | Metadata-only registry import/configuration/publication events |
| Jobs and resources | Existing infrastructure only where it meets the required contract | Durable tenant ownership, poll/cancel/result fences, resource lifecycle for selected providers |

For direct HTTP tools, keep the current MCP → authority checks → proxy → vendor path. For contracts with executable hooks, first decide whether a native OpenAPI operation provides the intended capability. If equivalent behavior requires transforms or multiple calls, add a reviewed backend adapter. An adapter must enter the same actor authority, platform credential, capacity, billing, and audit path; it cannot expose a generic caller-controlled upstream provider/endpoint selector.

Pooled accounts require ownership for jobs and resources. A valid GET path or provider job ID alone is insufficient. Persist local person/owner/agent/operation bindings and recheck them for polling, cancellation, list access, and results. Account-wide history and wallet views stay in operator workflows. Existing NyxID machine-node and channel contracts remain authoritative; importing a vendor machine/email endpoint does not grant access to those subsystems.

### Owned resources and jobs on a pooled account

Monid can offer state-changing operations to every caller because each effect stays inside the caller's workspace. A Monid API key is bound to one workspace, each run is charged to that workspace's wallet, and run and resource identifiers are checked against the workspace. The audited catalog has no operation that changes another tenant's data or a shared account.

The methods in the backlog follow that pattern. Most of the 758 POST tools are queries that use a body for structured input: `dataforseo` bulk lookups, `apollo /people/match`, `exa /contents`, and roughly 200 search and fetch operations. All 15 DELETE, PATCH, and PUT tools act on a resource the caller created: AgentMail inboxes, drafts, messages, and domains; Saperly numbers; SmolMachine machines; and TinyFish browser profiles and sessions. Thirteen of them are flagged by Monid's resource registry. The two AgentMail thread operations are unverified but address threads inside an owned inbox.

A NyxID platform account collapses every NyxID user into one upstream workspace. Monid would then accept user B's request to read, send from, or release user A's inbox. Query-style POST tools need only per-user billing. Owned resources and asynchronous runs need ownership enforced by NyxID.

Restricting PATCH and DELETE is insufficient. Ownership is known only if it was recorded at creation, and reads leak as much as mutations: reading another user's mailbox, sending from their inbox, or fetching their run result. Every operation that references a resource identifier must reference one the caller may access, whatever its HTTP method. Upstream list operations cannot pass through, because they return the whole pooled account.

The tracking design has six parts:

1. **Operation annotations.** Each operation declares a resource role (`create`, `use`, or `release`), the resource type, and the identifier location: a JSON pointer into the response for `create`, or a path, query, or body location for `use` and `release`. Import Monid's `GET /public/v1/resources` registry (65 affected endpoints) as review input. Carry the reviewed annotation in the overlay beside `x-nyxid-changes-existing` and into the matching `ProxyOperationRule`. An operation with an unannotated identifier parameter cannot be published to a pooled offering.
2. **Ownership records.** A proposed `tool_resources` collection stores a UUID v4 `_id`, the polymorphic owner, acting person, API key or agent, catalog service, resource type, upstream identifier, status (`pending`, `active`, `released`, or `unknown`), and BSON datetimes. A unique index on service, resource type, and upstream identifier prevents two owners from claiming one resource.
3. **Check before execution.** The evaluator shared by MCP and the HTTP proxy extracts referenced identifiers, loads their records, and authorizes through `org_service::resolve_owner_access`. Organization sharing follows existing owner semantics. Unknown or inaccessible identifiers return a not-found-shaped error before credential materialization.
4. **Record on creation.** Insert a `pending` record before the upstream call and complete it from the buffered response. Creating operations cannot stream. A lost response leaves the record pending for reconciliation and is never retried automatically.
5. **Owner-scoped listing.** Serve resource and run lists from local records rather than upstream account-wide lists.
6. **Lifecycle and reconciliation.** Mark records released after a successful release operation. Release or transfer resources when a user is purged or loses organization access. Settle recurring resource charges through the billing ledger, cap resources per owner against the shared account, and periodically compare upstream inventory with local records to find orphans.

Asynchronous runs use the same records with a run resource type. The mechanism therefore also supports the 131 polling-lifecycle endpoints. Annotation work is limited to about six providers: AgentMail, Saperly, SFS, SmolMachine, Browserbase, and the TinyFish browser operations.

Monid workspaces per organization are an alternative only as an AI Service. Monid's documented authenticated API cannot create workspaces or API keys, so NyxID cannot provision one per owner. An organization or person can connect its own Monid workspace key through the existing owner-scoped credential model. Monid then enforces isolation and bills its own wallet, at the cost of manual signup and funding. That path suits users who want the full Monid catalog and is outside the no-setup Tools offering.

### Minimal persistent registry records

These names are proposed, not existing models:

| Model / collection | Contents | Important boundary |
| --- | --- | --- |
| `ToolRegistryEntry` / `tool_registry_entries` | Vendor label and verified operator, provenance identities, docs/dashboard links, nonsecret manual setup, references to catalog offerings, revision | Vendor grouping only; no credential material or independent runtime permission |
| `ToolRegistryCandidate` / `tool_registry_candidates` | One imported identity, source/version/hash, native operation mapping, capability/classification review, workflow blockers, selected `ServiceEndpoint` reference, latest bounded validation receipt | Separate candidates allow thousands of tools without oversized embedded vendor documents |
| `ToolRegistryImport` / `tool_registry_imports` | Pinned source, inventory hash, progress, counts, reviewer, staged change references, result | Large specs and detailed diffs use versioned artifacts with hashes, not unbounded embedded blobs |

Keep specs in the existing hosted/in-tree registry initially. Mongo IDs use UUID v4 strings and the required BSON datetime helpers. Preserve the handler → service → model layering. The import key is a stable namespaced source identity, not a display name; index it uniquely with the source family. Aliases are explicit, versioned reconciliation decisions. A model-specific tool sharing another tool's upstream method/path needs either a merged native contract or distinct server-bound adapter operations.

Repeat imports are idempotent and stage changed definitions for review. They do not overwrite credentials, audiences, prices, operator labels, or existing grants. An admin-created dedicated offering owns its explicit runtime operation policy; background sync cannot enlarge that published policy. Removed tools produce a withdrawal proposal and block new publication. Existing live tools remain on their approved contract until a reviewed update or explicit withdrawal, subject to current execution checks.

### Publication must be an execution change

Imported candidates are metadata only. New runtime offerings remain unavailable until validation: use inactive service/endpoint state and an explicit empty `proxy_operation_policy` while staging. Current `POST /services` creates an active service, so the registry must add a reviewed transactional draft-creation path, or keep the candidate outside the catalog until it can be created safely. Do not create an active service and rely on a later update to hide it.

Publication validates the current spec/mapping, operation classification, fixed origins, credential/no-auth configuration, platform audience, pricing, quotas, and current test receipt. It updates the reviewed operation policy and runtime active state as one coherent change, with a metadata audit event. Pausing removes execution authority through the same runtime fields, including raw HTTP proxy access; hiding a card alone is insufficient. A retryable publication request uses a revision check and durable operation ID so partial completion can reconcile safely.

When a new shared-key offering must be temporarily live for normal-user validation, limit it to the explicit test audience and exact test operations. Broader publication remains a separate transition. If an existing service is reused, review the effect on its current users and bindings before changing its policy.

Contract or adapter binding changes use positive operation generations and the existing execution-authority/approval contract. New execution-affecting metadata must be incorporated through the repository's rolling-compatible authority design before activation. Registry workflow fields alone do not belong in that digest.

### Proposed management API and frontend placement

These routes are new proposals. Continue to use current service APIs for secret/configuration writes rather than add a registry credential API.

| Method and route | Purpose |
| --- | --- |
| `GET /api/v1/admin/tool-registry/entries` | Filtered, cursor-paginated provider/setup list |
| `POST /api/v1/admin/tool-registry/entries` | Create manual vendor metadata |
| `GET /api/v1/admin/tool-registry/entries/{id}` | Current setup, references, and derived status |
| `PATCH /api/v1/admin/tool-registry/entries/{id}` | Update nonsecret setup metadata with revision precondition |
| `GET /api/v1/admin/tool-registry/candidates` | Provider/category/blocker filters and bounded candidate pages |
| `PATCH /api/v1/admin/tool-registry/candidates/{id}` | Review classification and operation mapping |
| `POST /api/v1/admin/tool-registry/imports` | Create a bounded staged import from a supported pinned source |
| `GET /api/v1/admin/tool-registry/imports/{id}` | Import progress and change summary |
| `POST /api/v1/admin/tool-registry/imports/{id}/apply` | Apply selected reviewed changes with revision preconditions |
| `POST /api/v1/admin/tool-registry/entries/{id}/validations` | Request selected bounded checks; live checks declare identity, inputs, and budget |
| `POST /api/v1/admin/tool-registry/entries/{id}/publications` | Publish or pause a selected offering revision |
| `GET /api/v1/tools` and `GET /api/v1/tools/{offering_id}` | Metadata-only caller-filtered browse/detail; MCP remains the execution interface |

Use existing `AppError` responses and HTTP semantics: 201 for a new metadata record, 202 for queued import/validation, 409 for stale revisions, and existing validation/access errors where appropriate. New errors follow the authoritative error module. Paginate large inventories with capped limits and an indexed stable cursor. No management or listing GET creates connections, runs a provider check, returns secrets, or performs tool execution. Any later secret/streaming/execution-shaped GET must join the delegated-read deny classes before mounting.

Frontend implementation goes in the actual current structure: sidebar configuration, `router.tsx`, dashboard breadcrumb/compact-navigation maps, pages under `frontend/src/pages/`, per-domain hooks and Zod schemas, and API types. Reuse the service configuration dialog/editor and `useAppForm`. Check both the main sidebar and the dashboard's additional admin navigation renderer so the entry appears consistently.

## 6. Exhaustive migration scope

The completed audit contains **2,440 hosted tool identities / 87 providers** and **699 source definitions / 31 providers**. Of those source definitions, **637** match hosted identities after the two explicit provider aliases. The generated backlog accounts for:

- 637 hosted identities with a matching source definition;
- 1,803 hosted identities without a matching source definition;
- 62 source-only identities awaiting hosted-name reconciliation or separate direct import.

The resulting **2,502 backlog rows** are a union of audited identities, not a claim of 2,502 distinct live capabilities. Renamed source-only tools may later merge with hosted rows. The reported hosted total of 2,441 retains its documented StockAnalysis discrepancy; no definition is invented for the missing listing record.

The [provider backlog](provider-migration-backlog.csv) contains all 87 providers and their account/contract/ownership work. The [tool backlog](tool-migration-backlog.csv) gives every audited identity a wave, matching source, runtime class, draft state, and blockers. The [machine-readable summary](migration-backlog-summary.json) records the counts, provider membership, aliases, and input hashes. None of these rows is marked activated or credential-verified.

### Port by shared runtime requirement

| Wave | Providers | Hosted tools | Source definitions | Main work |
| --- | ---: | ---: | ---: | --- |
| 1. Search pilot | 3 | 18 | 12 | TinyFish, Firecrawl, Exa; choose a small synchronous public search/retrieval subset |
| 2. Source-backed data | 16 | 557 | 554 | Native wire review, schemas, query serialization, enrichment/news/SEO contracts; route async operations to the job dependency |
| 3. Data jobs | 5 | 75 | 77 | Apify, Clay, Cloro, Orbit, Ploid; start/poll/cancel/result ownership and deadlines |
| 4. Generation | 9 | 59 | 39 | Six source-backed generation providers plus existing-contract overlap candidates OpenAI, Gemini, ElevenLabs; model routes, artifacts, token/unit prices |
| 5. Owned services | 5 | 79 | 17 | Saperly, AgentMail, SFS, SmolMachine, Browserbase; owner bindings, lifecycle, mutations, per-resource costs |
| 6. Additional native contracts | 49 | 1,652 | 0 | Obtain executable native contracts and actual supplier access; includes TikHub, DeFiLlama, dataset brands, and aggregators |
| Total | **87** | **2,440** | **699** | All audited providers assigned once |

Waves prioritize integration work, not publication eligibility. Start wave 6 contract research during the foundation/pilot work, especially high-coverage providers such as TikHub and DeFiLlama. A provider in wave 2 can still have polling or owned-account operations that cannot ship until those capabilities are ready. A wave 6 supplier may offer a straightforward synchronous API and move forward once its contract and access are verified.

The 56 providers without public connector source are distributed across waves 4–6: three existing provider-contract overlap candidates, four hosted-owned-service providers, and 49 remaining providers. Existing overlap does not establish exact tool coverage or deployed credentials. The current source's 699 definitions have five mutually exclusive engineering classes: 267 plain HTTP request candidates, 49 request-transform adapters, 239 start-hook adapters, 131 polling-lifecycle adapters, and 13 resource-ownership adapters. Even plain candidates need response/error/cost and native-wire review.

### Repeatable provider port checklist

Every provider follows the same sequence:

1. Reconcile its hosted and source identities; preserve aliases and any source-only tools.
2. Identify the actual native API operator, fixed origins, credential classes, docs/specs, and product access. Record an explicit contract blocker if only public tool descriptions exist.
3. Extract a pinned native execution contract from MIT source or supplier documentation/specification; retain attribution for reused code. Do not treat Monid's hosted wrapper inputs as native vendor schemas without checking the mapping.
4. Classify each operation's data scope, side effects, query/body/response transforms, job/resource dependencies, native cost, and limits. Source lifecycle hooks are evidence to inspect, not an instruction to execute downloaded code inside NyxID.
5. Generate/review OpenAPI and any required adapter, exercise NyxID's parser and input validation, and record a change report. Verify arrays, combinators/workflow projection, method/path collisions, multipart/binary responses, timeouts, and error envelopes where applicable.
6. Complete manual account setup and configure the dedicated catalog offerings using the existing secret boundary.
7. Validate permitted operations through the normal caller path, including grants, revoked access, live audience, quota exhaustion, billing/subsidy, and tenant ownership where relevant. Paid checks stay bounded by the selected validation budget.
8. Publish the selected subset, capture operational evidence, and assign maintenance ownership. Refreshes produce staged diffs rather than silently replacing the current offering.

An x402 advertisement in the hosted inventory is not proof that a native provider requires x402. Investigate ordinary API-key access first. If a supplier truly requires a wallet payer or an aggregator contract, mark that dependency explicitly and implement it as a separately funded capability. Manual email/card setup alone does not supply such a runtime payment protocol.

### What “all ported” means

Track contract coverage, native execution coverage, and published coverage separately. Completion requires an executable native contract, valid supplier access, correct runtime behavior, and normal-user validation for each tool claimed as migrated. A missing contract, unsupported supplier, unavailable dataset entitlement, unfunded paid tool, or unfinished ownership model remains blocked with a named reason and next step. A reviewed equivalent or replacement needs an explicit mapping; it is not silently counted as the original tool.

The goal is complete accounting for every audited provider and tool, followed by validated direct coverage wherever the required access and contracts exist. The public listing cannot establish that all 2,440 tools can be directly called under accounts we can obtain. A central Monid supplier is a separate execution option described in the [audit](README.md); it is not required by this direct-provider plan.

## 7. Implementation milestones and estimates

| Milestone | Deliverable | Acceptance |
| --- | --- | --- |
| A. Registry foundation | Admin sidebar/route, provider list/detail, manual checklist, candidate mappings, imports view, existing editor links | All 87 provider entries and all audited identities are visible with honest setup states; operators read, admins write; no keys in registry responses |
| B. Import and draft workflow | Pinned-source importer, native spec intake, bounded staged diffs, safe catalog staging, idempotent apply | Reimport changes no keys/grants/prices; source mappings/counts reconcile; draft operations cannot execute through MCP or raw proxy |
| C. Direct pilot | TinyFish search/fetch first, then selected Firecrawl/Exa synchronous tools when configured | A normal user without a vendor connection can grant an AI access and call the exact selected tools; denial, limits, errors, and source evidence are verified |
| D. User Tools view and publication | Sidebar entry, browse/detail, grants UI, live publish/pause and stale-validation handling | Cards reflect current availability; pausing blocks both discovery and execution; no account-connect prompt for platform tools |
| E. Source-backed expansion | Provider/operation work from waves 2–4, serialization support, shared quota admission, reviewed adapters | Published coverage is measurable per operation; unsupported hooks or contracts remain visible blockers |
| F. Jobs/resources and full contract coverage | Tenant-owned lifecycle capabilities plus waves 5–6 native contracts | Users cannot access another owner's jobs/resources; every audited identity is migrated, explicitly reconciled, or blocked with evidence |

Use existing exact grants for the pilot and first registry release. The proposed dynamic public-data bundle is optional subsequent work, with a shared execution evaluator and rolling-compatible rollout. It does not need to block manual onboarding or the first user-visible Tools screen.

Preliminary effort for one engineer familiar with NyxID, with credentials and test infrastructure ready: **6–10 engineering days** for the registry/manual setup MVP, and **10–20 additional engineering days** for the reusable import/draft workflow plus a small direct pilot and user browse/publication path. That is roughly **3–6 engineer-weeks** for the initial combined scope. Shared job ownership is a further **10–20 engineering days** for the first provider/runtime pattern. These are planning ranges; account-access waits are excluded and staffing can change elapsed time.

Equivalent coverage of all 699 source definitions remains a multi-month integration program. Estimate the additional hosted tools after contract/access discovery for the 56 source-absent providers. Do not multiply a per-tool estimate by 2,440: bulk native API families can share tooling, while a few resource or machine operations can require major ownership work.

## 8. Reproduce and validate the planning artifacts

```bash
python3 docs/plans/monid-api-audit/build_migration_backlog.py
```

The offline generator asserts unique provider assignments, unique hosted/source identities after aliases, complete coverage of all 87/2,440/699 records, exact 637/62 matching counts, and per-provider totals. It performs no network calls, account setup, credential checks, or runtime activation. CSV rows use the original inventory identities so a reviewer can join them to the [hosted](hosted-endpoints.csv) and [source](source-endpoints.csv) evidence.

## References

- [Approach and Monid study](../2026-10-07-public-data-tools-approach.md), [exhaustive API audit](README.md), and [direct-provider engineering plan](DIRECT_PROVIDER_PLAN.md).
- [Provider backlog](provider-migration-backlog.csv), [tool backlog](tool-migration-backlog.csv), [coverage summary](migration-backlog-summary.json), and [backlog generator](build_migration_backlog.py).
- [Current sidebar](../../../frontend/src/components/dashboard/sidebar.tsx), [router](../../../frontend/src/router.tsx), and [dashboard navigation](../../../frontend/src/components/layout/dashboard-layout.tsx).
- [Master-credential shape validator](../../../backend/src/handlers/services.rs), [catalog model](../../../backend/src/models/downstream_service.rs), [operation model](../../../backend/src/models/service_endpoint.rs), and [platform credential access](../../../backend/src/services/platform_key_service.rs).
- [API discovery and spec sync](../../API_DISCOVERY.md), [platform credentials and prices](../../PLATFORM_KEYS_AND_INFERENCE.md), and [service configuration](../../SERVICE_CONFIGURATION.md).
