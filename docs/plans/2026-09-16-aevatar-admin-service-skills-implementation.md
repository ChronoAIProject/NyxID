# Aevatar admin service skills implementation

Status: design and implementation plan converged with Fable on 2026-09-16 after the corrections recorded below. Implements the autonomous scope in `2026-09-16-aevatar-admin-service-skills.md`.

## Delivery contract

A platform administrator grants a dedicated service account access to exact catalog service IDs. Aevatar can read their recommended skills, assign existing/new skills, reassign, unassign, clear, and restore recommendations without per-change approval. Actual package creation and updates belong to Ornn, with immutable old versions retained and package/version deletion denied. Skill text never grants service execution authority.

The account can make incorrect editorial decisions. V1 limits writable resources and supports recovery; it does not add a semantic approval engine, publication controller, or human review queue. Authorization must be enforced by the APIs, including alternate routes and generic proxy access.

The implementation has two independently testable parts:

1. NyxID: live account authority, bounded recommendation API, version history/restore, existing-writer participation, management/client support, and tests. This checkout owns these changes.
2. Ornn/Aevatar: resource/action scoped content CRU and consumption of published changes. Ornn source is `ChronoAIProject/Ornn`, with companion content-only publication tracked in [#1247](https://github.com/ChronoAIProject/Ornn/issues/1247). Aevatar deployment consumption still requires verification. The complete deployed content-editing integration is not considered verified until those boundaries have executable evidence. Do not substitute broad Ornn admin permissions or claim a recommendation-only feature edits packages.

## Simplifications for the first release

- Store the live curation grant on the service-account record rather than create a general grant framework. The grant has a UUID, exact catalog service IDs, issuance attribution, optional expiry, and an update budget. Absence means no authority. Revocation removes/disables the grant; a separate sticky protection flag remains.
- Reuse the current platform-admin service-account area for issuance, inspection, and revocation, with dedicated `POST/DELETE /admin/service-accounts/{id}/curation-grant` routes that require platform admin. General create/update request structs do not accept grant fields; explicit service-layer `$set` updates cannot clobber the embedded grant. Expose grant management through the existing admin UI and CLI.
- Use the existing client-credentials flow and operational secret storage. A new secret-store delivery integration and automatic employee-demotion rotation workflow are not prerequisites for this feature. Protected account management, token revocation, and reliable rotation remain required; document offboarding rotation of any secret a former administrator could retain. The standing workload grant is not permanently conditional on its original issuer retaining a role.
- Preserve name-based recommendations. Add optional exact references for writers/consumers that support them; reference presence is not a content-approval claim. Do not require a general Ornn installer to be built into the NyxID CLI as part of recommendation management.
- The transactionally recorded history is the authoritative record of changes. Emit through the existing chained audit helper; do not build a new audit delivery subsystem just for curation.

These choices require corresponding edits to the design document before implementation so the two documents express the same scope.

## 1. Account authority and credential boundary

Files: `backend/src/models/service_account.rs`, `services/service_account_service.rs`, `handlers/admin_service_accounts.rs`, `mw/auth.rs`, and their callers/tests.

Add serde-defaulted `platform_protected`, `purpose: general | curation`, and optional curation-grant data. The grant optionally names exactly one `ornn_proxy_service_id` for content authoring. Define exact `catalog:skills:read` and `catalog:skills:write` scopes and require both the verified token scope and live account grant. Grant service IDs resolve only to catalog `DownstreamService` rows; no user/org instances, wildcard, empty-as-all, or arbitrary client labels. Validate cardinality, UUIDs, existence, expiry, and budget limits.

Issuance rejects org-owned accounts and sets curation purpose plus protection atomically. An org administrator cannot set or replace the grant, unset protection, alter scopes/roles to gain platform authority, or manage a protected account through the direct-owner fallback. Revocation unsets the grant while purpose/protection remain. Protected management and provider-credential attachment require platform administration. Do not start resolving service-account role IDs as NyxID platform roles; leave role IDs to platform-admin discretion for downstream identity assertions and prove Ornn's effective role permissions separately.

The currently documented `proxy:<service_id>` scope is not implemented: `scope_allows_rest_proxy` accepts only `proxy` or `proxy:*`, and `proxy` also admits the LLM gateway. For curation accounts, restrict scopes to the two curation scopes plus `proxy`, which is permitted only with the explicit Ornn target. In the verified SA extractor, mirror `ensure_api_key_purpose_route`: allow only `/api/v1/catalog-curation` paths and plain HTTP `/api/v1/proxy/{ornn_proxy_service_id}` paths matched on exact segments. Require a live unexpired grant for either surface. Deny all WebSocket upgrades, proxy slug/list forms, other service IDs, LLM/OpenAI-compatible gateway, provider/connection self-management, node, oracle, trigger, and other routes. Credential issuance remains on its existing client-credentials path.

Keep the existing proxy projection for this purpose so the catalog legacy branch can execute after the strict route gate. Do not project catalog IDs into API-key `UserService` ID checks or broaden ordinary proxy semantics. Test the real target resolution so caller-owned configuration or alternate forms cannot select another service. Ordinary General service accounts retain their route behavior. Admins connect the Ornn provider credential through existing platform-admin-only provider-management routes. Fix the stale per-service scope documentation to explain the actual curation route/target restriction.

Add `credential_generation: i64` (legacy default zero) to the account and token record and optional `sgen` to SA JWT claims. Issuance records the generation read with the validated secret. Rotation increments generation atomically with the secret-hash change, then revokes old rows through the existing helper. Authentication requires an existing token record with matching account/jti/scope, unexpired and unrevoked, and matching current credential generation in both record and claim. For legacy SA tokens, missing generation means zero and is accepted only while the account remains generation zero. Other token types do not carry this field. This closes issuance-after-rotation races regardless of row insertion order. Never return/log secret material from grant/history APIs.

Admin account responses expose purpose, protection, grant summary, and credential generation without secret material.

The approved integration with current main reuses its `proxy_operation_policy`: a Curation
Ornn target must have an explicit policy at grant issuance and runtime (empty means
deny all). Use a separate catalog endpoint with the exact skills read/upload/version
routes in `docs/SERVICE_ACCOUNTS.md`, not the shared endpoint for other clients.
The SA's downstream role has only Ornn read+publish; exact existing object write
grants bind package edits to its UUID. Ornn's auth-only assistant/audit routes make
the endpoint policy necessary in addition to the role. No new policy framework is added.

## 2. Recommendation state and history

Files: `backend/src/models/downstream_service.rs`, new small model/service modules, `backend/src/db.rs` indexes and existing service serializers.

Add `skills_revision` with legacy default zero and optional `recommended_skill_refs`. References contain source, immutable skill ID/name, exact version, SHA-256, and bounded dependency pins where supplied. Validate shape, lengths, duplicates, count and payload limits; reject mutable tag/version selectors for exact references. NyxID does not fetch package bytes.

When exact refs exist, derive legacy `recommended_skills` names in their order. A caller may manage advisory names without pretending they are verified references. Changing names on a service with refs requires replacement refs or an explicit clear-refs operation; never silently keep stale pins. Treat an empty list as intentional unassignment of all defaults. Preserve instance overrides.

Create an append-only `catalog_skill_revisions` collection recording service, old/new revision and values, actor kind/id, grant ID when applicable, request ID/fingerprint, and timestamp. Use a unique `(service_id, revision)` index and a unique actor/request identity for retry outcomes. Record legacy baseline values so restoring the state before the first edit works. No history-delete API or short TTL.

## 3. One conditional commit path

Create a service-layer operation shared by the machine API and human admin writes. Its input is validated recommendation state, expected revision, stable request ID, and verified actor authority.

Within one MongoDB transaction:

1. Validate current live authority for a machine actor. Resolve an existing matching actor/request result before budget reservation; identical content replays the original result, changed content or target returns conflict. Never replay across callers or bypass current authorization.
2. Compare the service's current revision, including missing legacy revision as zero. Identical desired state is a successful no-op at the current revision: no history, revision increment, or budget consumption. A no-op makes no durable mutation/replay claim; document this distinction in the API contract.
3. Reserve the bounded write allowance with a conditional update on the account/grant record inside the transaction. Use a persisted fixed window so concurrent requests and multiple server replicas share the limit; counting history rows is insufficient. Apply only skill fields for machine callers, deriving names when refs exist, and increment revision.
4. Insert the history and retry outcome in the same transaction. No history row may claim an effect that did not commit.

Return a clear conflict on stale revision and a bounded rate-limit error when exhausted. Transaction aborts leave all state unchanged. Unknown commit responses are retried with the same request ID. Use existing transaction/error helpers where suitable. Emit chained audit metadata after commit; history remains the effect record if best-effort audit delivery fails.

Restore uses the same operation with an earlier recorded state and the current expected revision. It creates a new revision, may undo assignment/reassignment/unassignment, and never deletes package bytes or history. Serialize with current human writes so restoration cannot silently overwrite a newer edit.

## 4. Narrow machine router

Add `/api/v1/catalog-curation` outside the human-only router. Layers reject delegated tokens, relay tokens, and API keys. Reject human sessions/access tokens at handler-level by requiring `AuthMethod::ServiceAccount`; there is no existing reject-human middleware. Then require the exact scope and live grant. Keep existing `/services`, `/admin`, and unrestricted `/catalog` access closed to service-account tokens.

Proposed routes:

- `GET /services`: minimal discovery of only granted service IDs/names and skill revision; no service credentials or configuration.
- `GET /services/{id}/skills`: advisory names, optional refs, current revision, and skill digest.
- `PUT /services/{id}/skills`: complete desired recommendation state, `base_revision`, `request_id`, and optional explicit clear-refs flag. Reject unknown fields such as base URL, credentials, ACL, role, or ownership changes.
- `GET /services/{id}/skills/history`: bounded, paginated history for that granted service.
- `POST /services/{id}/skills/restore`: revision to restore, current base revision, stable request ID.

Disallowed service IDs must not expose their current content or history. GETs are read-only. Machine POST/PUT responses contain no secrets. Add `catalog-curation` explicitly to `delegated_read_denied_path` per CLAUDE.md rule 5, even though the router also rejects delegated tokens.

## 5. Existing writers and consumer compatibility

Files: `handlers/services.rs`, `handlers/services_helpers.rs`, `services/catalog_service.rs`, `services/mcp_service.rs`, `handlers/{catalog,mcp,keys}.rs`, and relevant frontend types/forms.

Route admin create/update skill fields through shared validation and the conditional/history path. Preserve existing authorization of human service creators where applicable; machine grants cannot touch personal/org instances. For forms, send the observed skill revision and a stable request ID for skill changes; avoid sending unchanged skill fields during unrelated edits. A legacy skill update without a revision means expected revision zero, never an unconditional overwrite. Display/refuse conflicts rather than retrying stale values.

For mixed metadata/skill writes, preserve one atomic service update: after existing handler validation builds the non-skill fields, apply those fields together with the skill CAS and history inside the same transaction. Machine callers cannot supply those additional fields. Include the entire validated mixed mutation in the request fingerprint; do not replay a skill result and apply different metadata under the same ID. If skills are unchanged, commit changed metadata without incrementing the skill revision, recording skill history, or consuming curation budget. Preserve the existing identity-change optimistic filter. Keep identity propagation/reconciliation, Lago sync, OIDC redirect updates, and endpoint discovery after commit; never call external systems from the transaction. If restructuring the current update path reveals a blocker, return it for review instead of silently accepting partial writes.

Add optional refs/revision to appropriate read responses and MCP config. An instance name override must suppress inherited catalog refs to avoid attaching pins to different names. Existing consumers continue receiving name lists.

Preserve the existing `catalog_digest` algorithm. Add a separately versioned `skills_manifest_digest` for exact reference changes; name changes still affect the existing digest. Verify old digest fixtures stay unchanged when only new optional fields are present. Document next-fetch propagation and the fact that locally installed copies do not update themselves.

## 6. Usable management and documentation

Extend the current service-account CLI/admin management surface to issue, inspect, and revoke curation authority with explicit service IDs and bounded budget. Provide machine API examples covering token acquisition, discovery, assign, reassign, unassign, conflict recovery, and restore. The grant APIs must work without inventing a new UI workflow; frontend changes should keep existing service editing compatible and show protected-account/grant state where appropriate.

Update `docs/SERVICE_ACCOUNTS.md`, relevant API docs, and the service-skill authoring instructions. State separately which capability is NyxID recommendation management and which requires Ornn's scoped content identity. Explain rollback limits: restored instructions cannot undo external effects already executed by a consumer.

## 7. Verification and personal review

Use the repository's MongoDB replica-set test helper (`connect_transaction_test_database`) for transactional behavior; never silently skip those tests. Use local/test credentials only and do not provision a production account or publish live packages as a test.

Required proof:

- authorized read/assign/reassign/unassign/clear/restore, including legacy state and exact refs;
- denial for every wrong token class, missing scope/grant, wrong service, expired/revoked grant, revoked/missing token, inactive account, and forged org/admin inputs;
- protected account management/rotation remains platform-only after grant expiry/revocation; missing token rows fail closed and old-generation issuance racing rotation cannot authenticate afterwards;
- no broader credential/proxy authority and no metadata writes through the skill body; route matrix covers allowed exact Ornn HTTP path, denied other IDs/slug forms/WS upgrades, LLM and OpenAI-compatible gateway, providers/connections, oracle/triggers/nodes, and unchanged General SA behavior;
- two concurrent writers, repeated request, changed request under the same ID, no-op with zero budget/history/revision effects, failed transaction, lost response/replay, budget exhaustion and cross-replica concurrency;
- matching admin form behavior, no stale name/ref combinations, instance override precedence, unchanged legacy digest fixtures, history and restore correctness;
- real Ornn/Aevatar contract evidence once their source/deployment test surface is available: scoped content create/read/update, denied package/history deletion and unrelated resource/settings writes, retrieval/use of new content, and recovery.

Run formatting, backend/CLI compilation and focused tests, affected frontend type/lint/tests, then the applicable broader CI checks. The primary agent reviews the final diff personally, records each finding, sends fixes to the same implementation session, and reruns the affected evidence until no identified issue remains. Do not delegate code review or create worker subagents. Unavailable external evidence is a remaining task, not a passing check.

## Execution

After Fable and the primary agent agree on this plan and its corresponding design edits, use one Heca Codex session on this worktree with model `gpt-6-astra`, effort `xhigh`, and mode `full-access`. The implementation session must not spawn agents. No live provisioning, registry publication, merge, or deployment is part of this local implementation task.

Review convergence: Fable accepted the embedded grant, optional names/refs, transactional budget reservation and admin history, and the need to fix the proven rotation race. The purpose route gate above incorporates Fable's follow-up correction to its initial allowlist suggestion. Mixed writes use a single transaction instead of introducing a partial-commit recovery contract.
