# Tools Manager coordinator review ledger

Status: implementation in progress; no completion or sign-off claimed.
Baseline: `10043f0b87f1db2ac4a966f598f543e1c758513b`.
Implementation: ordinary Heca session `01a133f0-a945-71e0-9b46-91aed94f543f`,
branch `tools-manager-astra-implementation`. No subagents used.
Runtime verified from the Codex turn record: `gpt-6-astra`, `xhigh`,
`approval_policy=never`, `sandbox_policy.type=danger-full-access`.
The installed daemon downgraded the first attempted session to high; that
session was stopped before implementation. The active session uses the
previously verified patched daemon in an isolated local data directory.

The first implementation session `01a133be-7ca4-7f40-b0ed-15659515c95b`
was stopped after slow progress and repeated patch-placement/path failures.
All edits were preserved in place and backed up as `first-session-wip.patch`
in the isolated daemon directory. A fresh ordinary Astra session continues in
the same worktree with a focused backend handoff and the full acceptance plan.
Only one implementation worker is active; no native subagents were spawned.
Effective settings were reverified from the new runtime turn record.

## Verification already performed

- Installed frontend dependencies with `npm ci --no-audit --no-fund`.
- Baseline frontend router, desktop navigation and capability tests: 3 files,
  33 tests passed (`npm test -- src/router.test.ts
  src/components/dashboard/sidebar.test.ts src/types/api.test.ts`).
- Started a dedicated ephemeral MongoDB replica set for feature tests. No
  existing local or production database is used.

## Review targets and implementation guidance

These are requirements to verify against the implementation, not findings
against an unfinished diff.

1. Initial shared credential and destination changes must race safely. The
   shared catalog metadata writer replaces its top-level `$or`; use a filter
   composition that preserves both the credential/destination fence and skills
   revision fence. Test both orderings and durable freeze after Admin clear.
2. Direct endpoint updates, discovery and automatic spec reconciliation must
   atomically remove approval when execution semantics change. Preserve no-op
   generations and handle a concurrent Admin publication.
3. An unpublished specific route blocks a broader published route. Deleting
   or changing the method/path of that blocker must not expose a previously
   blocked vendor request. Check deletion, direct edits and spec reconciliation.
4. Manager Tool source inheritance must exclude default headers, destination
   targets, shared credentials, provider links, pricing and audience. Verify
   private-source visibility and source changes while a twin is prepared.
5. Both-role callers must satisfy Tool authority for Tool rows and Catalog
   authority for provider/AI-service rows. Legacy rows remain Admin-only.
6. Management identity checks and all alternate mutation routes must preserve
   human-only authority. Adding a role must not enable Agent Key or delegated
   writes. Trace anonymous rules, OIDC helpers, ownership and provider links.
7. CLI dry run and validate do not access vendor credential env vars, prompt,
   discover or write. A complete rerun performs no writes. Retry of partial
   work does not rotate credentials. Test inactive and foreign rows.
8. Manifest unknown fields, inline secrets, URL userinfo, invalid auth settings,
   missing operations and variable errors fail without echoing secret values.
   Verify API error redaction as well as successful output.
9. Readiness must distinguish pending Admin setup from failure, must not infer
   ordinary-user visibility from Admin preview, and must report effective
   primary/component/operation prices including pending-sync behavior.
10. Frontend route, page, navigation and field controls agree with server
    capabilities; both-role users retain valid capabilities. First credential
    input stays write-only. No provider association is added to supply icons.
11. Icon upload uses the shared SVG validator and immutable serving path;
    ownership is rechecked on attachment, failed updates preserve the old icon.
12. Full fake-vendor E2E must prove credential injection, publication denial,
    live audience enforcement, acting-person billing and no-op reapply.

## Findings and disposition

R1 — Role slug collision: initial seed used `ensure_system_role`, which can
convert a pre-existing custom/client role into a system role and grant its
assignees new authority. Sent to Astra; require collision-safe seeding and
regression coverage. Follow-up: the existing precheck helper itself has a
check/read race, so require a conditional system-role update and safe handling
of insertion collisions. Pending verification.

The isolated implementation branch was corrected from stale local main to
`10043f0b` before continued work; all existing edits were preserved. Runtime
settings were reverified after reload.

R2 — No-auth Tool execution: listings already have a no-auth audience helper,
but `resolve_catalog_platform_target` always demands an encrypted credential.
Explicitly configured no-auth Tools need live audience enforcement with a
working no-auth target resolution, including existing user-service bindings.
Sent to Astra as an integration requirement; pending implementation/tests.

## Review round 1 — sent to Astra before backend handoff

Sent as a consolidated correction request on 2026-10-13. These may be fixed
before the increment is ready; all remain pending regression verification.

- `require_tool_row_writer` currently checks role/marker only, not creator,
  owner transfer, eligible internal/provider-less shape, or identity kind.
  The update handler skips `require_admin_or_creator` for marked Tools.
- `check_tool_update` calls Catalog Manager's checker unchanged, which rejects
  every credential. It needs a complete independent/exhaustive classifier.
- `check_tool_create` is non-exhaustive and its destination predicate always
  returns None. It must reject privileged SSH/identity/execution fields too.
- Manager twins still go through unrestricted `prepare_create` before checking
  authority and inherit Admin-only fields; sanitized preparation is pending.
- Initial manager platform config is `Default::default`, rather than explicit
  enabled/restricted/empty ownership policy. Check actual defaults.
- First-credential CAS currently checks only marker/kind/frozen; it must bind
  the observed destination, creator/current ownership, eligible shape and
  credential absence. Otherwise a URL edit that commits first redirects the
  credential from an earlier snapshot.
- Role helper now handles only duplicate-key but errors on an existing custom
  slug, which regresses the established skip-without-startup-failure behavior.

- Manager endpoint deletion was enabled without protecting more-specific
  unpublished blockers; reject manager deletion or implement safe transactional
  semantics. Relocation and auto-sync need equivalent safeguards.

## Review round 2 — fresh backend session

The fresh worker has corrected the creator/transfer and eligibility predicate,
separated the first-credential classifier, added snapshot predicates to manager
metadata writes, and composed catalog-skill predicates under `$and`. These
changes are source-reviewed but await regression tests.

Further findings sent:

- Keep existing delegated `account:read` linked-provider metadata GET access;
  human-only checks belong on mutation entry points, not every shared role read.
- Twin source comparison must tolerate absent defaulted fields in pre-feature
  documents. Serializing a full model and matching every field makes legacy
  sources fail with conflict because they lack new marker fields.
- Preserve non-Tool Admin publication behavior when adding a Tool-only
  transaction fence.
- Route-move demotion must validate all affected endpoint generations before
  incrementing them, including invalid/exhausted generation cases.
- The first worker copied a deletion prohibition into CREATE then UPDATE;
  latest source now allows CREATE/UPDATE and restricts DELETE, pending tests.
- No-auth direct proxy now uses the shared audience helper; existing no-auth
  UserService live ACL remains an explicit review/test requirement.

## Delivery status

No feature commit, remote push, PR or deployment exists from this run. Both
local feature branches still point to `10043f0b`; changes are in the isolated
Astra worktree. A remote PR cross-check through the NyxID skill could not run:
`nyxid service list --output json` reports the configured session has expired.
This does not block local implementation or tests. Heca lists Fable 5.1 and
Opus 5.5 xhigh, but no review invocation or sign-off has occurred.

## Independent compile check

Ran `cargo check -p nyxid --tests --locked` in Astra worktree, with output
at `/tmp/tools-manager-backend-check.log`. Finished exit 101 with six errors. Failure diagnostics sent to
Astra: wrong `crate::platform_key_service` module path; provider handler match
missing the new authority variant; four capability response test fixtures
missing `manage_tools`. This is a failed check, not feature test
coverage. Re-run after fixes and coherent backend handoff.

Role collision tests must create the production unique `roles.slug` index;
`connect_test_database` alone does not install it. Guidance sent.

## Final verification and model sign-offs

Pending implementation and coordinator review.

## Independent review continuation

The second independent `cargo check -p nyxid --tests --locked` completed
successfully (6m54s), with 15 test-code warnings from the then-current snapshot.
Warnings and final source still require clean verification. Astra is compiling
and running the dedicated MongoDB Tools Manager tests; no passing runtime
result is claimed yet.

Additional findings sent for correction/verification:

- Manager twin clones retain per-operation `target_id` even though destination
  targets are not inherited. Unsupported destination-bound operations must not
  be silently redirected to a different base URL.
- Manager twin `import_source.imported_at` uses current time before request
  fingerprinting; exact idempotency retry must preserve its fingerprint.
- Endpoint reconciliation omits data_scope/cost_class/execution comparison and
  writes; execution contract changes must demote publication consistently.
- Semantic demotions need meaningful chained audit evidence.
- Existing no-auth platform bindings still need live destination/server-routing
  parity assessment; current resolver keeps stored endpoint URL and node.
- Authoritative bulk discovery currently preserves omitted Tool operations,
  contrary to its soft-delete comment. Define and test safe omission behavior.

User-facing status remains: local uncommitted backend work; no feature push,
PR or sign-off created. CLI, frontend management, Tool icons and full ingress/
billing E2E remain required.
