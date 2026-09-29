# Service-account skill assignment through PUT /keys

Status: agreed design following independent Fable 5.1 review of current main and this plan. Fable's four required changes are incorporated: refs-only body, unchanged non-SA dispatch, authorization before body parsing, and an explicit write-response disclosure contract. No implementation or deployment is included in this planning turn.

## Outcome and evidence

The existing CatalogEditor service account must complete this workflow using its current authority:

1. Validate an ORNN package through `POST /api/v1/proxy/s/ornn-api/api/v1/skill-format/validate`.
2. Create, read, and update the ORNN package through the same authorized proxy.
3. Discover NyxID catalog services through `GET /api/v1/keys` and `GET /api/v1/keys/{catalog_uuid}`.
4. Assign or replace their recommended immutable skill references with the client's exact request:

   ```http
   PUT /api/v1/keys/{catalog_uuid}
   Content-Type: application/json

   {"recommended_skill_refs": [...]}
   ```

ORNN owns package content. This PUT changes the recommendations on a NyxID catalog service. Updating an ORNN package does not automatically advance a recommendation pinned to its older version. Assignment also does not change ORNN package visibility or bindings; those remain subject to ORNN's existing authoring contract.

The preceding live check reproduced PUT `/keys/{catalog_uuid}` returning 403, error code 1002, `Service accounts cannot access this endpoint`, with the supplied SA and a fresh token carrying `proxy catalog:skills:read catalog:skills:write user-services:read`. Both key GETs and an authorized no-op through the curation PUT succeeded. Earlier live ORNN validation and create/read/update checks succeeded. Those results establish the separate route gap; they do not establish completion of the requested workflow.

Source inspection on 2026-09-29 compared worktree `35e54066` with fetched `origin/main` at `8a38f0f6`. The relevant catalog authorization and mutation code is unchanged. Main has added human `icon_url` updates, which must be preserved. The preceding live health check reported `c0eba82eb509`, version 0.33.0. Refresh the implementation base and deployment identity when execution begins.

There are three code gaps: the PUT is mounted inside the human-only router, the CatalogEditor extractor allows key GETs only, and this route has no service-account request contract for `recommended_skill_refs`. Adding permissions to the existing SA cannot remove these gates. PR #1679 fixed ORNN proxy access; it did not add this PUT contract.

## Chosen API contract

Add an authenticated CatalogEditor branch to the existing PUT endpoint. Dispatch on verified caller identity before parsing the body.

| Caller | PUT target and behavior |
| --- | --- |
| Any non-SA principal admitted by the existing middleware | Existing personal/org UserService UUID or slug, existing request/response and authorization semantics, including third-party OAuth-app access tokens |
| Protected, active CatalogEditor SA | Exact catalog UUID, strict recommendation-reference replacement request |
| General or legacy Curation SA | Continue rejecting this PUT, including when scope strings resemble catalog scopes |
| API key, delegated token, relay token | Continue rejecting this PUT |

For scope-authorized editors, require `catalog:skills:write` in both the token and live account. Legacy CatalogEditors additionally retain their existing live global `nyxid:catalog:skills:write` role check. Assignment covers all existing and future catalog services, including disabled services, under the existing catalog authority; it is not restricted to the ORNN proxy target. The write response does not require catalog read, `user-services:read`, or proxy scope. GET authorization remains separate. No new scopes, roles, per-service grants, endpoint policies, schema migrations, or environment variables are required for this alias.

The SA DTO is exactly `#[serde(deny_unknown_fields)] { recommended_skill_refs: Vec<SkillReference> }`. A nonempty array replaces the complete list; `[]` stores empty refs and empty derived advisory names. Omitted/null is rejected. An empty array is distinct from absent refs: a later names-only admin change still requires replacement refs or the existing `clear_refs` operation.

Do not accept `base_revision`, `request_id`, `recommended_skills`, `clear_refs`, credential values, URLs, identity configuration, node selection, labels, icons, or arbitrary additional fields in this SA branch. Reject an entire mixed request, including forbidden fields with null values. The existing curation endpoint supplies the guarded/replayable form and retains its broader recommendation DTO with required revision/request ID.

Reuse existing reference bounds: exact immutable versions, lowercase SHA-256 shape, distinct names/references, dependency limits, and the 64 KiB skill-input limit. Enforce the curation router's 70,000-byte HTTP body cap on the SA branch only. Do not change the human body cap. NyxID validates reference structure; it does not fetch ORNN bytes or prove the supplied hash.

Return 200 with the existing safe catalog `KeyMetadataResponse` representation, including `resource_type: "catalog_service"`, catalog UUID, committed references, derived names, revision, and manifest digest. Use `Cache-Control: private, no-store`. This is a write acknowledgement available to the authorized writer; it performs no follow-up GET requiring read scope. Explicitly qualify the documentation's current statement that write alone does not grant reads: key GETs still require read authority, but a successful PUT also returns this entry's safe metadata (slug/name/label/type/active state). Never serialize a `DownstreamService` model or decrypt credentials for this response.

Keep ordinary Axum JSON extraction statuses: malformed JSON, incompatible/missing content type, invalid field types, unknown fields, and duplicate fields follow the existing extractor conventions. Document 400 semantic validation, 401 invalid/revoked token, 403 authority/route denial, 404 nonexistent catalog UUID or private-instance UUID, 409 concurrent-change conflict, 413 body cap, 415 content type, 422 DTO rejection, and 429 write budget. The alias has no client request-ID conflict semantics; preserve any existing engine conflict rather than retrying through it. Slugs and disallowed path/method combinations remain denied by SA route authorization. Preserve human error behavior and avoid logging rejected bodies or credentials.

Document intentional error changes caused by authenticating the admitted SA request: forged/expired SA-shaped JWTs now reach `AuthUser` and receive 401 instead of the old unverified-claim middleware's 403. Curation SAs may receive the extractor's Curation-specific message with the same 403/code 1002. General SAs keep `Service accounts cannot access this endpoint`. An SA token combined with an `x-api-key` header receives the API-key rejection. Test status, numeric code, and absence of mutation without unnecessarily freezing explanatory message text.

## Concurrency and replay

Use `catalog_skill_service::commit` unchanged. Preserve its transaction, live account/token/generation checks, legacy role fences, revision compare-and-swap, shared budget, history, durable operation receipts, and no-op behavior.

For the existing one-field client body, read the current catalog revision once after write authorization, generate one request UUID, then pass both fixed values into `commit`. Mongo transaction retries keep that captured revision. Do not automatically reload a newer revision and overwrite a competing change. A race after that snapshot returns 409.

This compatibility form replaces the list as of the server's observation. Repeating an identical request after a lost response is a no-op if nothing changed in between. It cannot detect a stale client list produced before the server's observation, or identify the previous operation across HTTP retries. A retry after an intervening edit can replace that edit. State this limit in examples; never advertise this form as protection from all lost updates.

Clients that need an observed-revision fence and durable replay of a committed change use the existing endpoint with the same catalog UUID:

```http
PUT /api/v1/catalog-curation/services/{catalog_uuid}/skills
Content-Type: application/json

{
  "recommended_skill_refs": [],
  "base_revision": 7,
  "request_id": "bf32757b-e188-4e43-9fc4-042e59adba41"
}
```

That endpoint already maps directly to the shared engine. Replaying the same committed request returns its recorded skill state/revision after current authorization checks. Reusing its ID for a different target, revision, or body returns 409. A stale revision requires a deliberate reread and decision, followed by a new ID; do not blindly retry. Do not add another optional-guard contract to the compatibility alias.

Preserve the current no-op rule: no revision increment, budget use, history, receipt, or changed-event audit. A no-op therefore does not establish durable replay. On the guarded curation route, retrying it after a later edit may return 409; on the alias, the later request takes its own server snapshot and can replace the later edit. Do not change receipt semantics for existing human or curation writes as part of this fix.

Build the safe response from `SkillCommit.service` plus explicit `SkillCommit.state` and `revision`. This also keeps internal duplicate-key replay handling correct: the service document can be current while the state/revision come from a receipt. The alias does not expose caller-controlled replay identifiers.

## Implementation sequence

1. Start a fresh implementation branch from current main, preserving the merged proxy fix and current human key features. Do not replay `7852a058`/`35e54066`; they already landed as squash `274cbe48` (#1679). Carry this plan forward as the review record. Capture the failing exact PUT as a router regression before changing routing.
2. Add a focused key-update dispatcher and a dedicated `key_update_routes` router merged into `api_v1`, following the existing ownership-router pattern. Move only the PUT method out of the human-only group; retain POST, DELETE, and key history there. Preserve the old middleware order minus SA rejection: `.layer(reject_delegated_tokens).layer(reject_api_key_tokens).layer(reject_relay_tokens)` (relay outermost). Shared key GETs keep their current layers. Branch only on verified `AuthMethod::ServiceAccount`; every other principal admitted by the middleware goes to the existing human handler verbatim. Inside the SA branch, reject every non-CatalogEditor purpose.
3. Extend `ensure_catalog_editor_route` for ordinary HTTP PUT to an exact catalog UUID detail path. Do not reuse a list-or-detail matcher without excluding the list. Keep WebSocket, nested paths, other methods, slugs, and private-instance access outside this exception. Reauthorize in the handler/service and existing commit transaction.
4. Recheck SA write authority before applying the body limit or parsing the body. A read-only/unauthorized editor receives 403 even for malformed/oversized input, with no body buffering. Preserve the original request and invoke the original `Json<UpdateKeyRequest>` extraction and `keys::update_key` for non-SA callers. Parse the SA DTO separately. Do not select a branch from JSON content or round-trip through a generic JSON object, which could change duplicate-field and null/omission behavior. Apply the authorized SA's limit before body collection.
5. Add a small snapshot-plus-commit function alongside `commit` in `catalog_skill_service`, which owns revision semantics. Keep the existing commit engine unchanged and metadata/identity mutation inputs empty. Add a safe `CatalogMetadata` constructor in `catalog_editor_catalog_service` taking the committed document/state/revision. Make `catalog_curation::audit_change` available within the crate and reuse it when changed. Put the dispatcher in a focused new handler module.
6. Update generated OpenAPI and prose in the same change. Keep the annotation on `keys::update_key` to preserve the existing operation ID and human schema name. Describe the caller-dependent target and response clearly. Use a small manual `PartialSchema` for request `anyOf[UpdateKeyRequest, CatalogSkillRefsUpdateRequest]`: the existing permissive human DTO makes an exclusive `oneOf` incorrect. Reuse `KeyReadResponse` for the 200 response. Encode and test the SA required-array, unknown-field, and null rules. Explicitly state that `recommended_skill_refs` is honored only for CatalogEditor callers; human requests continue to ignore this unknown field while processing their other supported fields normally.

Source locations: `backend/src/routes.rs`, `backend/src/mw/auth.rs`, a focused key-update handler and its module registration, `handlers/keys.rs` for the OpenAPI annotation, `services/catalog_skill_service.rs` for additive orchestration, `services/catalog_editor_catalog_service.rs` for response projection, and `handlers/catalog_curation.rs` for shared audit visibility. Keep the existing commit contract intact.

Update `docs/SERVICE_ACCOUNTS.md`, relevant key API descriptions, and the CatalogEditor note in `CLAUDE.md`. `AGENTS.md` is its symlink; edit the shared source once. Qualify old statements that all SA key writes are denied: General and Curation service accounts still cannot write keys, and their connection-metadata reads are unchanged. Check rendered/generated docs rather than editing only a comment. Existing CLI `commands/service.rs` and frontend key hooks send human instance updates; preserve those schemas and behavior. No SA reference client was identified in this repository, so the supplied direct-HTTP request is the compatibility acceptance contract. Do not claim an external client release or change a human command into a catalog writer.

## Required verification

Use the real Axum router and Mongo replica-set transactions for authorization and race checks. Mocking only the handler misses the present failure.

| Area | Required evidence |
| --- | --- |
| Exact client request | One-field PUT succeeds at revision zero and nonzero, persists references, derives names, and both key GETs return the catalog result; cover multiple catalog targets, a disabled target, and one created after token issuance |
| Package pin changes | Replace a v1 reference with v2/hash, verify revision/digest change; remove one reference and clear the list |
| Authority separation | Scope-authorized and genuine legacy editor positives; write-only token succeeds and GET remains denied; missing token/live write scope and removed legacy role deny before body parsing; General/Curation cannot acquire this alias via strings or grants; for CatalogEditors the alias and curation route have identical target allow/deny results |
| Token/account lifecycle | Expired/revoked token, disabled/rotated account, and permission revocation during a write deny/abort with no partial effects |
| Target and body boundaries | Private UserService UUID, missing UUID, slug, nested route, list PUT, other methods, upgrade, forbidden/mixed fields including null, malformed/oversized references and bodies; prove no catalog/credential side effects |
| Other credentials | API-key header and bearer variants, delegated and relay tokens still denied for writes; their allowed key GETs remain supported; test mixed SA/API-key headers and forged/expired SA-shaped JWT error changes |
| Human compatibility | Session and ordinary JWT personal/org UUID/slug updates, owner ACLs, credentials, platform binding, header omission/null/array, latest icon-only behavior, malformed body/content type, limits, and response shape retain existing behavior |
| Concurrency | Force two writers to share one base revision: one commits, the other conflicts; alias versus curation/human write shares the same fence; body-only form never silently rebases |
| Retry and no-op | Identical alias retry is a no-op when state is unchanged; document/test replacement after an intervening edit; existing guarded curation replay and conflicts still work; base_revision/request_id are rejected as unknown fields on the alias |
| Transaction effects | One changed history entry/receipt/budget increment, one eligible audit dispatch, rollback leaves none; alias and curation consume the same budget |
| Existing workflow | Curation DTO still requires its pair; ORNN proxy slug/UUID, signed SA identity, dedicated/master credential boundaries, explicit policies, owner isolation and live proxy revocation regressions pass |
| Contract | Generated OpenAPI accepts each documented request shape, rejects invalid SA shapes in its SA schema, retains human schema/operation ID, and documents safe responses/statuses |

Split the existing blanket `editor_cannot_mutate_keys_or_read_unrelated_account_routes` assertions: a read-only editor gets 403; an authorized writer sending `{}` gets 422; a valid refs body gets 200; POST/DELETE and HEAD/upgrade/non-UUID boundaries retain rejection. Add substantive mixed/forbidden-write cases. Reuse transaction collision hooks and authority-pause fixtures. Fixtures must persist real SA identities and avoid dumping bearer tokens, credentials, or fixture-derived routes into assertion diagnostics.

Run focused router/auth/catalog/curation/keys/proxy/docs tests first, then the repository's required backend CI, formatting, Clippy, and feature checks on the final implementation commit. Existing green CI from #1679 is a baseline, not verification of this new change. Run relevant CLI/frontend checks if their source changes; exercise their existing HTTP behavior through backend regressions regardless.

## Deployment and real workflow acceptance

The implementation is additive and uses existing fields/collections. Deploy all backend replicas before enabling reliance on the new PUT. During a mixed rollout, old replicas still return 403. Confirm every serving replica's build through deployment inventory; repeated public requests alone do not prove that all replicas were upgraded.

Use the supplied SA, its current roles/scopes, and an ordinary client-credentials token. Record method, path template, status, deployment build, revision/digest and sanitized correlation identifiers. Do not persist or print credentials/tokens.

Before any live writes, select an existing low-risk real QA catalog target and establish an authorized cleanup mechanism for a disposable ORNN package. The preceding probe used the disabled QA service `zz-aevatar-reftest`; reconfirm it is suitable and snapshot its exact names/refs and revision/digest. Snapshot package visibility/bindings too. Prefer this existing target; creating a new production catalog row would be a separate platform-admin action outside the SA's authority and requires appropriate execution authorization. A synthetic ORNN binding is not a substitute for catalog assignment. Do not overwrite unrelated recommendations on the production ORNN catalog entry.

Execute and record:

1. Validate a real valid package via the exact slug POST; assert the successful validation result. Submit an invalid package too, to confirm the upstream validation response is not being replaced or swallowed.
2. Create the disposable skill, read it, update its package/version, and read the changed version. Confirm ownership is the SA and the supplied role's delete/admin exclusions remain enforced using safe local/staging fixtures.
3. Obtain the immutable reference and verify its version/hash against ORNN's artifact contract. NyxID's shape validation alone is not proof of the package hash.
4. GET keys/list and the real QA catalog UUID; then send the exact one-field PUT with real refs. Assert 200 and reread through both GET surfaces.
5. Assign the earlier and newer pinned package versions in turn to prove replacement updates refs/revision/digest. An ordinary consumer's inheritance and explicit-override behavior must pass automated integration tests. Also verify the live consumer view if an existing QA consumer account is available; do not create human UserService fixtures in production for this check. Record live consumer verification as covered or unavailable separately.
6. Confirm an identical alias retry does not advance revision. Exercise the guarded curation request/replay and stale-revision behavior on the QA target. Do not revoke or rotate the user's working production SA to test a negative; lifecycle negatives belong to disposable test accounts.
7. Run cleanup below and verify final state. Only then report the deployed workflow as complete.

Do not equate 200 from the older curation route, a wiremock upstream, a synthetic binding, or historical tests with acceptance of the exact client workflow. If live ORNN validation returns a permission error again, retain its sanitized response/correlation ID and identify which server emitted it before changing permissions; this alias must not mask upstream failures.

Rollback reverts the new alias and its docs; existing persisted skill data remains readable by older code and the curation endpoint remains available. The exact client PUT will return its old 403 after rollback. No account reclassification, role expansion, secret rotation, or data backfill is part of rollout or rollback.

## Cleanup and completion criteria

- Restore the QA target's exact original skill state using the existing guarded restore path and the currently verified test revision. Restore is a new auditable revision, not deletion of history. On concurrent unrelated edits, stop cleanup of that resource and reconcile rather than overwrite them.
- Remove only disposable test packages/bindings/resources through an authorized cleanup actor. The production SA's create/read/update role need not gain delete/admin permissions. Establish cleanup authority before creating more immutable versions; package creation/update cannot simply be undone by reverting catalog refs.
- Reinspect the earlier fixture `sa-cru-check-e615afcb7ebf` (`cb18730e-5371-4b57-8a74-02cfd126088d`). Earlier synthetic binding/unbinding may have changed its visibility; verify actual visibility/binding state rather than assuming it is private. Resolve or explicitly inventory that fixture with appropriate ownership/authority.
- Verify the QA target, any available QA consumer state, bindings, and fixture inventory after cleanup. Declare intentional durable records: history revisions, operation receipts (including generated IDs, with no TTL), and `catalog_skills_updated` audit events. ORNN immutable versions remain outstanding disposable data until the janitor removes them; do not describe them as cleaned up prematurely. Keep a sanitized verification record and remove only task-owned temporary scripts/artifacts that are no longer needed; never delete shared files or evidence indiscriminately.
- Close stale tests/docs, review the final diff, confirm final-commit CI, and record deployed build plus the full live acceptance matrix. Any inaccessible cleanup action or unverified endpoint remains an explicit unfinished item, not a claim of “no leftovers.”

Completion means the exact request works under existing authority, existing callers retain their contracts, all required checks pass on the implementation actually deployed, and disposable test effects are accounted for and cleaned up. This document itself is a plan, not evidence that those implementation gates have passed.
