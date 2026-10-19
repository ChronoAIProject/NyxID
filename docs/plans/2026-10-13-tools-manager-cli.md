# Tools Manager, manifest CLI, and wizard execution plan

## Objective and baseline

Implement the Tools proposal on top of `10043f0b` (latest fetched `origin/main`
on 2026-10-13). This includes #1849, management-page access from #1858, and
provider SVG uploads from #1857. Preserve existing
single-step commands and legacy authorization. Tools are internal, provider-less
HTTP catalog rows with `offering_kind=tool` and `requires_user_credential=false`.
Reuse existing encryption, publication, platform ACL, metering, and billing.

Publishing policy: **Admin publishes; Tools Manager prepares, validates, edits,
and pauses**. This is the proposal's recommended default; the user has been asked
and may override it. Do not silently broaden publication authority.

Execution: one ordinary Heca implementation session using `gpt-6-astra`, `xhigh`,
`full-access`, in this worktree. No subagents. The coordinating agent personally
reviews every implementation diff and sends corrections back to that session.
After implementation and coordinator review, run the proposal's Fable 5.1 review,
then obtain the user's explicitly requested Opus 5.5 xhigh final sign-off on the
final code. Neither reviewer may spawn subagents. Resolve every actionable
finding, including low-severity issues and nits, and repeat review after fixes.
Review results must identify the reviewed revision and actual verification;
never claim a model sign-off or check that did not run.

## 0. Catalog Manager parity audit and acceptance contract

Audit baseline: `10043f0b87f1db2ac4a966f598f543e1c758513b`.
The feature branch was fast-forwarded from `d3dfa30d` to this revision. At the
time of this audit the branch contains this plan only: no `tools_manager`
backend authority or `admin tools` wizard/apply/validate implementation exists.
The completed Heca model-effort fix is a separate repository change and does
not constitute completion of this feature.

Parity means equivalent onboarding, management, discoverability and automation
where the concepts apply. Shared vendor spending retains the Tool-specific
authority boundaries below. Provider OAuth registration, user OAuth consent,
provider linking and redirect URIs do not apply to provider-less Tools.

| Current Catalog Manager functionality | Tools acceptance requirement | Verification |
| --- | --- | --- |
| Seeded role; Operator prerequisite; Admin grant/list/revoke CLI and role UI | `tools_manager`, `nyxid:catalog:tools:write`, and `capabilities.manage_tools`; authority is lost immediately on role or Operator removal | Role matrix, revocation, plain user, both-role and capability tests |
| Server-owned creation marker, creator scope, exhaustive field classification | Own marked Tool rows only; no legacy-row takeover, role union bypass or writable provenance | Backend request/route matrix and concurrent-write tests |
| `/services` and `/providers` access for Catalog Managers (#1858); desktop/mobile navigation | `/admin/tools` and owned Tool detail/edit access for Tools Managers; desktop and mobile links; provider administration continues to require Catalog Manager or Admin | Router, navigation and actual page/control tests |
| Interactive `add`; `--file`, `--template`, `--save`, `--yes`, `--skip-credentials` | Equivalent Tool wizard flags, with `--credential-env` replacing OAuth client flags; no secret in saved files; skip results in explicit pending readiness | Parser, wizard decisions and noninteractive mock-server tests |
| YAML/JSON manifests with `variables`, `${name}`, `--set`, defaults and allowed values | Same variable precedence and constrained non-secret substitution for `add`, `apply`, `validate`; validate expanded typed document before writes | Template/variable/schema tests; malformed input makes zero writes |
| Idempotent apply, field-level drift, dry run and immutable-field checks | Same create/update/unchanged reporting; add inactive-row lookup, partial recovery, no secret reads in dry run, no-op discovery/publication | Mock-server mutation counts and repeat E2E |
| Rich service descriptions, docs, capabilities and operational notes | Typed manifest/wizard support for the safe descriptive metadata listed below; low-level commands remain usable | Round-trip, drift and field-authorization tests |
| Hidden OAuth client entry and environment inputs | Hidden shared vendor credential entry or environment name, first-set only for manager; skip for no-auth; durable freeze after first set | Redaction, first-set race, post-clear and URL-race tests |
| Endpoint authoring and lifecycle management | Discover, list, create/edit, validate and pause own operations; Admin publishes; execution edits demote published operations atomically | Discovery/sync, single/bulk endpoints and HTTP/WS/MCP tests |
| Provider SVG upload through `admin catalog icon` (#1857) | `admin tools icon <tool> --file logo.svg`, backed by a Tool-owned icon; no artificial provider association | Shared SVG validator, creator/revocation, render and CLI tests |
| Structured readiness, test command and Lark fields | Stable JSON/table readiness, safe proxy test command, exact Admin handoff, Lark fields, effective price and real audience eligibility | Snapshot/schema and live user verification |

Source references checked: `docs/CATALOG_MANAGER.md`,
`cli/src/commands/admin_catalog.rs`, `cli/src/cli.rs`,
`backend/src/handlers/catalog_authority.rs`,
`backend/src/handlers/catalog_icons.rs`,
`backend/src/services/catalog_icon_service.rs`,
`backend/src/services/catalog_service.rs`, `frontend/src/router.tsx`,
`frontend/src/pages/admin-tools.tsx`, `frontend/src/pages/service-edit.tsx`,
and the desktop/mobile navigation components.

Do not inherit known Catalog CLI limitations as parity requirements: its current
apply matches active rows only and loads credential environment variables before
dry-run planning. Tools must meet the stronger inactive-row and secret-free
dry-run requirements in this plan. Tests should prove both properties.

### Frontend parity

- Add a fail-closed `canManageTools` capability helper with Admin fallback.
  Retain `canManageCatalog` for provider and AI-service administration. A Tools
  Manager role alone must not unlock provider writes or AI-service edits.
- Update the route guard, the page's internal Admin check, desktop sidebar and
  mobile navigation together. Gate Tool detail/edit by the actual row type and
  authority; simply opening the shared `/services` parent is insufficient.
- Show create/twin, safe metadata, first credential, discovery, operation edits,
  validate and pause controls where permitted. Show pending Admin publication
  and setup clearly. Hide Admin-only mutations, including provider controls in
  the shared Tools workspace. Mixed-role users retain each role's permitted
  functionality without transferring authority between row types.
- Preserve existing operator read access. Read `DESIGN.md` before UI changes;
  reuse existing components and form patterns. Cover direct URLs, page controls,
  ordinary users, Operators, each manager role, both roles, Admins and revocation.

### Provider-less Tool icons

The current catalog icon is derived solely from `ProviderConfig.icon_url`;
`DownstreamService` has no icon field, and `admin catalog icon` resolves a
provider. Reusing that command alone cannot supply Tool icon parity.

- Add optional, serde-defaulted service icon metadata and expose it through
  dedicated service/catalog/Tool responses and frontend types. Preserve existing
  provider icon inheritance for AI services. Provider-less Tools use their own
  icon and retain the existing fallback when absent.
- Add an authenticated Tool icon upload route that resolves live Tool write
  authority and ownership before storing an asset. Reuse `catalog_icon_service`
  validation/storage and `/catalog-icons/{id}` serving, its immutable URL and
  security headers. Keep the 128 KiB static-SVG/viewBox contract.
- Attach the uploaded URL through the normal authorized, audited metadata path;
  recheck authority on attachment, including revocation during upload. Failed
  attachment preserves the existing selected icon and gives a retryable error.
  No credential, publication, destination or provider change is involved.
- Add `admin tools icon <slug-or-id> --file <svg>` with normal auth/output flags.
  Include optional `icon_url` metadata in manifests; local SVG file contents stay
  in the dedicated upload command. Test size/UTF-8/unsafe SVG failures, foreign
  rows, legacy rows, both roles, immutable serving and frontend display.

### Tool metadata and wizard details

In addition to slug/name/supplier/topics/spec, support description,
`homepage_url`, `repository_url`, `issues_url`, `examples_url`, safe existing
`capabilities`, `auth_notes`, `known_limitations` and `required_permissions`.
Advanced non-secret metadata may be supplied by manifest while the wizard keeps
the initial questions concise. Classify `asyncapi_spec_url` and
`custom_user_agent` explicitly against the same backend rules; neither may
introduce an alternate credential destination or bypass publication.

Use the Catalog CLI's variable contract (`--set` > interactive answer > default,
allowed values and restricted characters), rejecting undeclared/unresolved
variables. Do not use substitution to load credentials. Keep validation errors
free of raw supplied values. `--file` and `--template` are mutually exclusive;
`--credential-env` overrides only a single-Tool manifest. `--skip-credentials`
permits draft onboarding without a secret and never means ready. A noninteractive
`add` requires `--yes` plus sufficient file/template data. `apply` and `validate`
never prompt; `--yes` does not authorize backend-forbidden actions.

Implementation is complete only when every applicable row of the parity table
has code and passing evidence, alongside the security and delivery gates below.

## 1. Backend authority and safe state transitions

- Seed system role `tools_manager` and permission `nyxid:catalog:tools:write` using
  the existing role service. It confers authority only on Operators; Admins keep
  their current authority. Expose `capabilities.manage_tools` and ensure the
  existing user-role UI/CLI can grant/revoke the seeded role.
- Add serde-defaulted, server-owned `tools_manager_created` provenance to catalog
  rows. Only manager-created rows get the marker. No backfill and no user-settable
  ownership/marker fields. Both creator identity and current role are required on
  every manager mutation. Legacy rows remain Admin-only for Tool administration;
  Catalog Manager alone gains no Tool permissions. Check both-role callers too.
- Extend or compose the exhaustive request-field classifier in
  `backend/src/handlers/catalog_authority.rs`, keeping all new fields classified.
  Require a first-party human account identity wherever existing catalog writes
  require it; avoid accidentally enabling delegated, service-account or Agent Key
  management through the new role.
- Allow twins and new HTTP APIs. Force internal/provider-less Tool shape, disable
  identity forwarding/delegation and arbitrary privileged config. Allow descriptive
  fields, supplier, controlled topics, supported auth method/key name at creation,
  base URL and spec URL. Managers cannot flip existing rows, delete rows, change
  ownership, billing/inference, audience/grants, destination targets, identity,
  token exchange, anonymous rules, default secret-bearing headers or WS injection.
- Manager twins must use a sanitized allowlist, not the Admin twin's entire
  inherited object. Never copy credentials, destination targets, source ACL,
  pricing or other authority-bearing configuration. Preserve safe provenance and
  draft/inactive copied endpoints transactionally. Source visibility/eligibility
  must be enforced before copying, including private sources.
- Use an explicit server-selected initial audience policy: no manager-authored
  audience/grants. Default manager Tools to restricted with no allowed owners
  until an Admin approves audience, avoiding legacy absent-config public fallback.
  Admin may configure audience/prices and then publish using existing commands.
  Readiness must distinguish this pending Admin setup from an implementation error.
- Set the first nonempty shared credential through a dedicated atomic conditional
  write or equivalent transactional path. Filter/fence current creator, marker,
  eligible shape, absence of a prior credential, and destination version. Never
  overwrite a credential on a racing request. Replacement and clearing stay
  Admin-only. Track a durable first-set/frozen marker if needed so Admin clearing
  does not reopen manager replacement or destination modification authority.
- Freeze `base_url` once a credential has been set, including credential and URL
  changes in the same request and races between them. Audit all other indirect
  ways credentials could be redirected: auth injection config, token-exchange
  destinations, default headers, destination targets, WS injections and redirects.
  Retain existing outbound URL/SSRF validation for discovery and execution.
- Manager operation writes cover only own marked Tool rows. Draft/validated/paused
  transitions are allowed; published is Admin-only, including bulk and alternate
  routes. Pause is permitted without republish authority. Editing execution or
  contract fields of a published operation must atomically demote it to draft and
  inactive and advance generation; no stale Admin approval can authorize an edited
  operation. Review discovery/spec sync for the same edit-while-published bypass.
  Prevent generic `is_active`, service reactivation, or endpoint deletion from
  restoring/bypassing an Admin publication decision.
- Keep HTTP, WS and MCP publication gates authoritative. No change may bypass
  existing platform ACL or exact billing-owner resolution. Audit through the
  chained append service; log authority and field names/status only, never secrets.

## 2. CLI contract

Add `nyxid admin tools add|apply|validate`, reusing common catalog helpers where
that reduces duplication without coupling Tool secrets to provider credentials.

```yaml
version: 1
tools:
  - slug: tools-firecrawl
    name: Firecrawl
    twin_of: api-firecrawl
    supplier: Firecrawl
    topics: [web-search, page-fetch]
    credential_env: TOOLS_FIRECRAWL_CREDENTIAL
    operations: [search, scrape]
```

For new APIs replace `twin_of` with `base_url`, `auth_method`, optional
`auth_key_name`, and `openapi_spec_url`. Exactly one creation mode is required.
The manifest is a typed allowlist, with unknown fields rejected. Values for
credentials, authorization headers, provider links, billing and audience are not
accepted in this onboarding file; it may contain only an environment-variable
*name*. Reject duplicate slugs/operation names, invalid topics/URLs/auth combinations,
unsupported versions, and structurally malformed documents before any write.
Validate errors without echoing secret-bearing supplied values or YAML source.

- `apply --file ... --dry-run` performs reads and local planning only: no writes,
  discovery mutation, secret prompt or credential access. Report creates, changed
  fields, discovery needs, credential presence and publication/Admin work.
- `apply --file ...` performs minimal reconciliation by slug, verifies existing
  row shape and twin identity, safely resumes partial work, and sets a credential
  only if never configured. Never rotate/clear a credential from a rerun. Use env
  value only when necessary, held in zeroizing memory and absent from logs, error
  bodies, saved manifests, process arguments and output. Do not echo upstream
  response bodies for secret-setting failures if they can reflect the secret.
- A complete rerun reports `unchanged` and performs no writes, including no
  discovery, generation bumps or redundant publication. Detect changed desired
  spec/config without undisclosed destructive reconciliation. Match inactive rows
  explicitly so a rerun never attempts an accidental duplicate create.
- Desired `operations` is an explicit selected set. Admin apply publishes selected
  operations and pauses other live operations; manager apply prepares/validates
  selected operations, may pause deselected ones, and reports pending Admin
  publication without attempting forbidden writes. Missing operation names are
  actionable failures, never ignored. Omitted selection must have a documented
  non-destructive meaning; empty selection must not silently activate anything.
- `validate --file ... --output json` is read-only and does not prompt, set secrets,
  discover or publish. Report credential readiness, operation states, effective
  price including sync status/operation prices, audience readiness, ordinary-user
  `/tools` eligibility and Lark row values. Do not confuse Admin preview visibility
  with ordinary-user visibility or infer restricted membership from admin reads.
  Use explicit status/reasons (ready, pending_admin, pending_credential, failed),
  stable JSON shape and meaningful documented exit codes.
- Wizard asks twin/new, metadata, URL/auth/spec as appropriate, then shows its
  plan. Prompt for a missing credential once with hidden input or use an env name.
  Discover, list operations, collect selection, and apply the same planner. Print
  a truthful readiness report and exact Admin handoff commands when required.
  Support `--template firecrawl`, `--file`, `--set`, `--yes`,
  `--skip-credentials`, `--credential-env`, usual profile/base URL/output options,
  and safe noninteractive use; no hanging prompts without a terminal. Any `--save`
  path must serialize only the secret-free manifest.
- Provide a Firecrawl template and documented new-API example. Replace
  `scripts/tools/add-tool-twin.sh` with the supported CLI workflow and update its
  references. Retain all existing low-level commands.

## 3. Verification

Backend tests use real MongoDB transactions where state/race behavior matters.
Cover Admin, Operator+Tools Manager, Operator without role, plain user with role,
Catalog Manager only, both roles, revoked roles, foreign creator, legacy unmarked
Tool and malformed/spoofed marker requests. Cover every allowed and forbidden
field/operation, secret initial-set race, URL/credential race, post-clear freeze,
published-edit demotion, sync/discovery, bulk atomicity/audit, no-auth rows, and
HTTP/WS/MCP draft/paused denial. Ensure secrets are absent from Debug/API/audit.

CLI mock-server tests cover create/update/no-op, dry-run zero writes and no env
access, partial retry, existing incompatible/foreign/inactive row, missing secret,
secret failure redaction, selection drift, manager handoff, JSON/exit status,
templates, bad manifests and noninteractive behavior. Exercise the wizard's
decision logic without brittle terminal screenshots.

Build an executable local E2E against an ephemeral replica-set database and fake
vendor API: Admin grants the role; manager creates a twin and a new API, sets a
fake credential once, discovers/validates; unauthorized publication and credential
replacement fail; Admin configures audience and a price and publishes; ordinary
user calls through the real proxy; vendor observes injected credential; metering
and exact billed ownership/cost are verified. Draft/paused requests must not reach
the vendor or accrue usage. Reapply is a write-free no-op. Persist reproducible
commands and a redacted evidence report. Do not use production secrets/services.

Run formatter, relevant backend and CLI suites, frontend capability/router/page
and navigation tests, frontend build/lint, and repository CI. Frontend work and
provider-less SVG support are required parity deliverables. Investigate failures;
do not disable tests or
label a skipped E2E as passing. Update `docs/TOOLS.md`, CLI examples, role docs,
and applicable `CLAUDE.md` invariants.

## 4. Review and delivery gates

1. Implementation session produces code/tests/docs and an evidence report.
2. Coordinator personally traces all changed execution/authority paths, reviews
   every diff, runs necessary independent checks, and maintains a findings ledger.
3. Return all findings to GPT-6-Astra, verify fixes and repeat until no unresolved
   actionable findings of any severity remain.
4. Fable 5.1 review from the proposal, then Opus 5.5 xhigh final sign-off; repeat
   affected reviews when fixes change the reviewed revision. No subagents.
5. Commit on the feature branch, create a PR with concrete behavior/test evidence,
   wait for green required CI, address CI/review failures, then squash merge as
   requested by Delivery. Do not merge while any required check/review is missing.
6. Report the actual PR/merge revision, checks, model reviews and any genuine
   external blocker. A promise of mathematically zero future risk is not a test
   result; completion means all identified issues have been resolved and all
   specified gates actually passed.
