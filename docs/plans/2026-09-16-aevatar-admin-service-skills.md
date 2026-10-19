# Autonomous Aevatar editing of admin service skills

Historical review status (2026-09-16): autonomous design reviewed with Fable. The user's subsequent clarification permits skill create/read/update plus assignment, reassignment, and unassignment; package/version deletion remains excluded. Supersedes the earlier per-change approval and no-unassignment designs. That design review preceded runtime implementation and performed no provisioning. The NyxID implementation is now present in this checkout; see the [authoritative implementation plan](2026-09-16-aevatar-admin-service-skills-implementation.md) and [current curation API](../../backend/src/handlers/catalog_curation.rs).

## Required behavior

Aevatar uses service skills to activate and operate the relevant services. Its integration has create, read, and update (CRU) authority for skill content on the authorized admin services. It can also assign, reassign, unassign, and reorder those services' recommended skills, including clearing a recommendation list. These assignment changes do not delete the underlying packages or historical versions, which remain outside the account's authority. Platform administrators authorize standing access during setup; ordinary content changes and service assignments use it without per-change review.

The previous proposal/apply-approved design did not meet this requirement and is superseded. There is no human approval queue or approved-manifest requirement in the normal editing path.

Use a dedicated Aevatar service account with bounded editorial authority. NyxID owns catalog recommendations; Ornn owns skill packages and versions. The grant must cover both actual package editing and recommendation maintenance. A recommendations-only writer is incomplete.

## Proportionate protection and accepted consequences

CRU does not guarantee useful or correct instructions: an update can replace good content with an empty or misleading version. This design accepts temporary disruption from bad editorial choices within the granted resources. It protects recovery by retaining earlier immutable content and assignment history, bounds the resources the account can change, and keeps service permissions enforced outside skill text. Basic validation, revision checks, write budgets, audit, revocation, and a working restore path are the core controls. A separate publication service, per-change human review, or a new semantic approval system is not required for v1.

Unassigning or assigning an irrelevant skill can leave a consumer without the guidance needed to configure or use a service; it does not itself delete the service, its credentials, or its permissions. A wrong skill can cause failed requests, incorrect results, unnecessary calls, or harmful actions that the consuming agent already has permission to perform. In particular, a skill-only editor can influence a more powerful runtime through instructions even though the editor cannot directly execute those operations. Backend permissions constrain which actions are possible, not whether an authorized action is appropriate.

Changing a shared package can affect consumers outside the edited service, and changes to recommendation names can invalidate catalog admission state. Restoring a skill or assignment repairs future guidance; it does not undo external writes, messages, data disclosure, or charges already caused by its use. The service account therefore provides bounded editorial authority and recovery, not a guarantee that every downstream effect is harmless. Recovery after a compromised writer is stopped must remain possible with platform-controlled credentials.

## Operating flow

1. Aevatar reads the allowed service's current skill references, content, and revision.
2. It selects an existing published skill, creates a new skill, or edits an existing skill within its standing grant. Selecting a published skill for assignment does not require permission to edit that package.
3. Automated validation checks package structure, sizes, references, dependency resolution, and the applicable contract checks. Any checks that execute service operations use already authorized operations and budgets.
4. If content changed, Aevatar publishes an immutable version to Ornn. It assigns, reassigns, unassigns, or reorders skill references using the expected current revision. Assignment-only changes require no new publication.
5. Its runtime loads that exact version and dependencies, verifies integrity, and uses the service under the existing execution permissions.
6. Failed validation leaves the previous working version active. Recoverable failures are retried; content errors can be repaired automatically with bounded attempts. Aevatar can restore an earlier content version or recommendation list under the standing grant; newer content and assignment history remain available.

No step asks for a human to review routine edits. A failed check returns an actionable machine-readable result; it does not create an approval task. Operators can inspect exceptions and disable the integration, but they are not in the normal publication loop.

## Standing authorization

A platform admin grants the dedicated service account:

- Exact catalog service IDs whose skills it may manage.
- Explicit rights to read, create, publish content versions, assign/reassign/unassign skills, update their pinned versions or ordering, and restore earlier content or assignments. No package/version deletion.
- Exact existing skill IDs eligible for content editing, plus service-scoped creation authority for new skills. This edit list does not restrict which existing published skills Aevatar may assign to an authorized service. The server derives allowed new names and binds created records to the permitted service; arbitrary client-supplied association labels cannot enlarge authority.
- Apply/write budgets and an active/revoked state, with the configured credential and grant lifecycle. No per-edit grant renewal.

The account does not receive general platform administration. Package/version deletion, archiving or unpublishing packages, ownership transfers, ACL/sharing changes, arbitrary tag manipulation, and unrelated service settings are outside this editorial grant. Administrators choose publication visibility during provisioning; routine publishing must not silently change it. These exclusions apply to packages Aevatar creates as well as existing packages. Content updates append a version rather than destroying earlier bytes. Removing a reference from a service's recommendation list is explicitly permitted and preserves the package and assignment history.

The platform issues and revokes these grants through its human admin control plane. Organization admins cannot create platform authority by supplying a scope string or role ID. Every operation checks the verified service-account identity, token validity, live grant, exact resource, and permitted action. A missing or empty resource bound grants no access.

For v1, a single optional grant embedded in the service-account record is sufficient: grant ID, exact service IDs, issuance attribution, expiry, bounded write allowance, and optional Ornn proxy target. Dedicated platform-admin-only grant routes issue or revoke it; ordinary account create/update fields cannot modify it. A persistent curation purpose and platform-protected flag remain after the grant is revoked. This reuses the account's live lifecycle and does not require a general grant framework.

Persist a platform-protected flag on the account so ordinary owner/org fallback cannot rotate or manage its credential, even after all editing grants expire or are revoked. Platform admins manage this identity and its standing grants. It should not lose platform-issued authority merely because the employee who originally provisioned it leaves; disable/revoke and ownership succession are explicit platform operations. Credential lifecycle automation remains separate from model-selected arguments.

Use the existing client-credentials provisioning and rotation flows and store credentials in the runtime's secret store. Never include credentials in skill content, model prompts, history responses, or audit metadata. Protection of management APIs does not invalidate a secret an administrator previously received: offboarding must rotate exposed credentials, revoke old tokens, and remove that person's access to the runtime secret store. This is an operational credential responsibility, not a per-edit approval step.

A new secret-store delivery integration and automatic employee-demotion recovery subsystem are deferred. Close the verified secret-rotation race with a credential generation: issuance captures the generation with the validated secret; rotation atomically increments it with the secret hash; authentication requires a matching current generation in the token claim and stored token record. Missing, revoked, expired, or mismatched token records fail closed. Legacy missing generation means zero only while the account is still generation zero. The standing workload grant remains independent of the original administrator's current role.

The runtime holds the credential and offers typed, bounded tools. The model supplies content and resource arguments; text inside a skill cannot widen the tool's grant.

Limit the editing credential's proxy access to Ornn, plus the explicit NyxID skill-management authority. The documented `proxy:<service_id>` scope is not implemented today, and generic proxy scope also admits the LLM gateway. In the verified service-account extractor, the curation purpose allows only the dedicated curation router and ordinary HTTP to the exact granted Ornn catalog UUID through `/api/v1/proxy/{id}`. Require a live grant; reject other routes, proxy slug forms, WebSocket upgrades, and provider self-management. Keep existing General-account/API-key routing semantics. Platform-admin-selected role IDs remain available for Ornn's downstream identity checks, never NyxID platform authorization. Aevatar uses its existing execution identity for the relevant downstream service operations. Compromising the editor must not turn it into a general service-execution account.

## Assignment of skills to services

Assignment means maintaining the admin service's NyxID recommendation list. Within the granted service IDs, Aevatar decides which skills are relevant and can add, replace, remove, reorder, or clear assignments without per-skill approval. It can assign an existing skill it did not author, a newly created skill, or a different version of an assigned skill. Assignment authority is bounded by the destination service; the content-edit allowlist is a separate permission and must not become an implicit assignment allowlist.

The standing assignment grant delegates content selection to Aevatar. It may select published Ornn packages that the consuming runtime is authorized to read. When supplying exact references, the pipeline resolves and verifies versions, hashes, and dependencies and checks runtime availability. Advisory name-only assignments remain supported without a deterministic-version claim. NyxID enforces the target service grant and reference/write contract; it does not claim to verify Ornn access or package bytes. Assigning a skill neither grants permission to edit its package nor makes a private package readable to other users.

The recommendation PUT accepts the complete ordered name or reference list, base revision, and stable operation ID. Replacing an entry reassigns the service to another skill; omitting an entry unassigns it; an empty list clears the recommendations. Every change records the prior list so it can be restored. Aevatar resolves legacy names when converting to exact references; NyxID stores the supplied references and does not perform registry lookups. Personal/org instance overrides remain authoritative for their instances. Changing assignments on two services requires authority for both.

For example, Aevatar can attach an existing setup skill to a granted service, update it from version 1 to version 2, replace it with a better setup skill, and remove an obsolete troubleshooting skill. These operations change the service's recommendations without deleting any Ornn package or version.

This NyxID assignment does not require changing Ornn service bindings, ownership, or visibility. Those registry operations have separate effects and permissions. Assigning existing published skills can ship without Ornn's new content-editor capability; editing existing platform-owned packages still needs that capability.

## Enforcement in NyxID and Ornn

NyxID needs a dedicated machine API for grant-scoped discovery and recommendation updates. Keep the general service/admin routers closed to machine tokens. A strict update body accepts an ordered advisory-name list or optional immutable references, an explicit clear-refs flag, a base revision, and a stable operation ID. Check authority over the target service and commit the update together with its history.

Ornn needs a resource-and-action checked CRU editor capability for these platform-owned skills and for new skills Aevatar creates. Do not use the broad `ornn:admin:skill` permission or transfer ownership as a shortcut. The current documented `ornn:skill:update` permission also covers privacy and ACL changes, so assigning it alone cannot implement a content-only boundary. Existing shared-edit support, if present, must be verified against its actual action checks; otherwise add a narrow content-publication action. Package/version deletion and changes to access or ownership must be rejected by Ornn itself, including calls made through the generic proxy; removing a Delete tool from Aevatar's UI is insufficient. This is resource/action authorization, not a claim that Ornn can determine whether every content edit is safe.

Aevatar can create a service-specific copy and reassign an authorized service to it if its credential remains restricted to CRU on the new package. Raw owner credentials with broader management or delete powers are not an acceptable fallback under this contract. The original package and assignment history remain available. The complete solution supports editing authorized existing skills through Ornn's narrow editor capability.

The grant must remain enforceable on every route, including ordinary Ornn skill update, refresh-from-source, binding, permissions, and tag routes. An alternate route must not let the editorial identity bypass a check. The account's granted creation authority automatically enrolls new skill records into the same permitted service scope; creating an arbitrary skill does not grant authority over arbitrary existing skills.

Cross-system grant enforcement needs an authenticated, server-controlled mechanism. Ornn must validate the editor grant and resource bounds rather than trust client-provided service IDs or generic role strings. Select the mechanism when inspecting Ornn: reuse a suitable live grant/ACL check if it exists, otherwise implement a narrow authenticated grant check. A durable copy with no revocation protocol would not provide live revocation.

Actual skill content remains in Ornn. NyxID does not become a second skill registry or a generic proxy for authoring operations.

## Automated quality and versioning

Validation establishes structure, integrity, and compliance with the standing grant. It does not certify that newly authored instructions are semantically correct. This is intentional editorial delegation to Aevatar, with bounded access and recovery.

Mandatory checks include bounded package size, safe archive paths, required metadata, exact version/hash references, resolvable dependency closure, and target-service eligibility. Contract smoke checks should be limited to the service operations and spending/test budgets already authorized. An additional model review or security score can inform automated repair, but it cannot enlarge authority or claim certainty.

Package checks run in Aevatar's publication pipeline and Ornn. NyxID validates the recommendation reference shape, exact versions rather than mutable tags, hashes, grant bounds, and revision preconditions; it does not fetch Ornn packages to verify their bytes or dependency closure. The writer verifies before submitting references, and the consuming runtime verifies before use.

Support optional structured references containing source, immutable skill ID, name, version, and content hash. Aevatar resolves and freezes dependency versions/hashes when it supplies exact references. Consumers claiming exact-version use must load the declared revisions and verify hashes; a missing version or hash mismatch cannot silently fall back to latest. Hashes identify the published content; they are not evidence of a human review. Name-only consumers retain their existing latest-following behavior.

Service-specific skills keep edits local to the managed package. Reading a shared dependency does not grant permission to edit it. Publishing changes to a shared package requires explicit editorial authority for that shared resource; otherwise Aevatar may create a service-specific copy under its CRU creation grant and reassign its authorized services to the copy. This bounds writable resources, not the audience of public packages: external consumers may already follow an existing public skill name.

Publishing a version under an existing Ornn name advances latest. In the autonomous design this is a legitimate live publication when the account has authority for that package; run required checks before that effect. When checks need an uploaded artifact, use an isolated candidate first. Candidate cleanup cannot use Aevatar's CRU credential to delete artifacts. Do not publish into an existing package whose edit authorization is uncertain.

For existing name-following consumers, editing an already recommended skill can take effect on the next fetch without any NyxID recommendation change. A NyxID write is needed to change which skills are recommended or to update exact references. Locally installed copies are not automatically refreshed or rolled back; the consuming runtime must implement that behavior. Ornn's editorial authorization and publication budgets therefore protect content changes even when no NyxID write occurs.

## Commit, compatibility, and recovery

- Add a skill-list revision and compare-and-swap updates. All writers, including the existing admin form and `PUT /services/{id}`, use the same update path. On conflict Aevatar re-reads the current state and recalculates the change; it cannot blindly overwrite intervening edits.
- Use stable operation IDs. Reusing an ID for the same change returns the recorded result; using it for different content is rejected. An uncertain outcome is resolved before sending another mutation.
- Commit the conditional NyxID update, change-history record, and idempotency outcome in one MongoDB transaction. Include actor, grant, old/new references, revision, and operation ID. Transactions are already used for Agent Key approval.
- Emit tamper-evident audit entries through the existing best-effort chained append path. The transactionally recorded change history is the durable proof of effect; no new audit delivery subsystem is required for this feature.
- Ornn publication and the NyxID update are separate effects. Publish/verify any new content first, then update recommendations. Assignment-only changes need no publication. If the reference write fails, retain the old references and retry/rebase safely; an unused candidate is acceptable. When publishing under an existing live name, name-following consumers may already see the new version even if the later NyxID update fails, so report and recover that actual state.
- Rollback restores earlier content references or an earlier recommendation list as a new revision, without another human approval. It may remove a newly added assignment or restore an unassigned skill; it never deletes packages, versions, or change history. Reference rollback only controls consumers that honor those references. For legacy latest-followers, recovery republishes known-good content as a new version through the authorized content-publication path, subject to verification of Ornn's behavior. A bad newly assigned skill can be unassigned immediately within the same service grant.
- Maintain an independent platform-controlled Ornn recovery identity. Revoking the Aevatar grant stops future edits but does not undo a published version, and its revoked credential cannot perform the recovery. Automated platform recovery can use the independent path; operators are an exception path, not routine reviewers.
- Preserve personal/org endpoint overrides. A platform recommendation edit changes only the inherited default.
- Derive legacy `recommended_skills` names when structured references exist. A human name-only update must explicitly clear structured references; it cannot leave stale references attached to a different list. Add per-entry validation to legacy string writes.
- Preserve the existing `catalog_digest` contract. Recommendation-name changes can invalidate catalog/exact-approval state; avoid no-op updates and bound write loops. Use a separately versioned `skills_manifest_digest` for version/content changes and make the runtime re-observe it. Do not silently change broad digest construction during a rolling deployment.

Legacy names remain compatible, but do not provide deterministic version selection or content rollback. Aevatar's runtime must honor exact references before claiming that behavior. Consumers can migrate additively; their current latest-following behavior must be stated accurately. NyxID's bundled CLI skill installer is not a general installer for service-recommended Ornn packages and does not need to become one for this feature.

## Skills and service activation

Aevatar can update the instructions it uses to activate or operate the granted services and immediately use the new version. It should not encounter a new approval gate merely because it repaired or published a skill.

Skill-edit authority and service-execution authority remain separate backend checks. The editor cannot grant itself a missing credential, OAuth scope, billing allowance, or permission by writing new instructions. Existing authorized service operations proceed under the runtime's existing permissions. If activation means provisioning a new connection or enabling a management action that the runtime cannot call today, that needs its own narrowly defined standing permission; general admin access must not be inferred from skill editing.

Do not claim NyxID's existing durable operation grants already solve this service-account path. Their current contract targets scheduled-invocation API keys and exact endpoint/body bounds, not arbitrary skill-driven activation.

## Minimal complete implementation

1. Verify the deployed Ornn service-account identity path and inspect its resource/action ACL support. Establish CRU-only editing of existing platform-owned skills and scoped creation of new ones, with package/version deletion and access/ownership changes denied for both. This is required for actual content editing, not an optional later phase.
2. Add the platform-managed service account grant, protected custody and actual proxy confinement, plus NyxID grant-scoped reads and recommendation updates for assignment, reassignment, unassignment, and ordering. Reuse existing credential provisioning/rotation and operational offboarding. No proposal, review, or approval collection.
3. Wire Aevatar's read -> select or edit -> validate -> publish if needed -> assign -> use/repair loop. Include immutable versions, history, conflict handling, idempotent recovery, and automatic rollback. Provide credentials only to the runtime.
4. Verify Aevatar's exact-reference loading, dependency integrity, and scoped service operations. Enable autonomous editing for the authorized services; expand the permitted resource set through platform administration when needed.

Illustrative machine operations are GET service skills, create a service-bound skill, publish a new version of an authorized skill, PUT the service's recommendation list, and restore earlier content or assignments. The content operations belong to Ornn; recommendation operations belong to NyxID. No package/version delete action is authorized. Exact API names are an implementation detail to settle after verifying Ornn's existing surface.

Release evidence should include a complete authorized create/edit/publish/use flow with no human approval, denied edits to an unrelated skill/service, denied ACL/ownership changes, revocation on both systems, rejected org self-granting, protected secret rotation, automatic handling of validation failure, concurrent edits, lost responses, shared dependencies, failed cross-system steps, and successful rollback for the Aevatar consumer. Credential tests must cover management by a demoted owner, old secrets/tokens after rotation, missing/mismatched token records, and issuance racing with rotation. Protect the Ornn-only proxy bound against LLM, unrelated service, WebSocket/node, and provider self-management paths.

Assignment evidence must also cover adding an existing skill outside the package-edit allowlist to a granted service, assigning a newly created skill, updating its pinned version or ordering, reassigning to another skill, unassigning, clearing the list, and restoring the prior list. Reject writes to ungranted services, package/version deletion, and archive/unpublish/visibility bypasses, including for Aevatar-created packages and through the generic proxy. Verify assignment changes do not mutate Ornn packages or bindings, grant package edit/read permissions, or overwrite instance overrides. Verify recovery after a bad content update and after unassignment, with all historical versions and changes retained.

## Evidence recorded during the design review

Local source inspected at `9bb33bcf`:

- `routes.rs` keeps `/services` and `/catalog` under a router that blocks service-account and API-key credentials. Service update authorization loads a `User`, not service-account roles.
- `service_account_service.rs` accepts free-form scopes, while `admin_service_accounts.rs` lets org admins manage org-owned accounts. Scope text and existing role assignment do not establish platform authorization. The direct-owner fallback also means a special credential-custody boundary is needed.
- `service_account_service::rotate_secret` returns the raw replacement secret and calls `revoke_all_tokens` after separately updating the secret hash. Managed delivery and demotion-triggered recovery are not existing guarantees or v1 deliverables.
- `mw/auth.rs::scope_allows_rest_proxy` recognizes `proxy` and `proxy:*`, not the documented `proxy:<service_id>` scope. `scope_allows_llm_proxy` also accepts generic proxy scope. Existing configured-service scope checks compare `UserService` IDs, while the legacy catalog proxy rejects scoped callers; curation confinement requires explicit treatment of both paths.
- `identity_service.rs` and `handlers/proxy.rs` explicitly propagate a service account's identity and role-derived permissions to downstreams. This is a foundation, not a proof of Ornn acceptance.
- `mcp_service.rs` resolves instance skill overrides before catalog defaults and includes recommendation names in the broad catalog digest.
- `audit_service.rs` provides chained append helpers; existing business updates and audit appends are separate. `auth_agent_key_login_service.rs` provides an existing transaction precedent.

The connected Ornn manual `ornn-agent-manual-cli` v1.5 was read on 2026-09-16. It documents immutable versions, hashes, mutable latest/tags, service bindings, and broad existing update permissions. A system-skill label means a service association, and its audit verdict is not a publication gate. The manual is not live proof of service-account authorization or fine-grained shared editing.

Checks identified by the 2026-09-16 design review included Ornn's deployed editor authorization, machine identity and callback compatibility, version retention/rollback behavior, revocation across systems, NyxID's actual proxy/route confinement, and Aevatar's consumer contract. At that historical review stage, the work comprised design documentation; it had not yet changed runtime code, credentials, or registry packages.


Implementation follow-through: the [converged implementation plan](2026-09-16-aevatar-admin-service-skills-implementation.md) is authoritative for the NyxID checkout. The observations above describe the 2026-09-16 review, before runtime implementation. NyxID's [transactional recommendation service](../../backend/src/services/catalog_skill_service.rs) and [router/security regressions](../../backend/src/handlers/curation_tests.rs) are now implemented and verified. Current API and operational behavior are documented in [Service accounts: catalog skill curation](../SERVICE_ACCOUNTS.md#catalog-skill-curation). Ornn/Aevatar content CRU remains a distinct external integration requiring its own repository and executable evidence.
