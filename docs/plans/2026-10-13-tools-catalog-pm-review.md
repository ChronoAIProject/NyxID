# Tools and Catalog delivery: PM coordinator review ledger

Owner: Opus 5.5 xhigh (Claude Heca session `01a13546-1783-7941-baeb-c4ce32ac4889`),
now coordinator and personal reviewer at the user's request. The previous
coordinator session (`01a1319a`, GPT-6.1-Sol) has exited. Implementation owner:
ordinary Heca Astra `01a13552-5971-7893-8f83-973dca3d1165` on the patched
daemon. Its rollout records `gpt-6-astra`, effort `xhigh`, `approval_policy:
never` and `danger-full-access`, which I read independently. No subagents. No
commit, push, PR, merge, deploy, or production credentials/services.

Binding inputs, which are preserved and never edited by anyone:
- the original plan;
- the delivery handoff;
- the PM matrix `2026-10-13-tools-catalog-pm.md`;
- the coordinator review `2026-10-13-tools-manager-review.md`;
- the personal ledger `2026-10-13-tools-catalog-personal-review.md` (P1–P13
  plus its follow-through notes).

All of their open items remain required. This ledger adds the reviews I make
from now on. IDs are `R<n>`.

Status values: `open`, `fixed-untested`, `fixed-verified` (with log path), `closed`.

**Implementer model change (user request, 2026-10-13 22:12 +08).**
- Method: `AgentSetModel` on the same session, so the Codex thread and its context are retained.
- Requested: `gpt-6.1-sol`. The turn ending 14:11:31Z (Astra's clean checkpoint and handoff) still ran `gpt-6-astra`.
- PM-verified rollout `turn_context` at 14:11:52Z: `model=gpt-6.1-sol`, `effort=xhigh`, `approval_policy=never`, `danger-full-access`.

## Review rounds

### Round 1: source snapshot of 2026-10-13 21:40 (+08), pre-checkpoint

Reviewed in source (no tests run by the PM):
- `catalog_mutation_service.rs` (new) and `tools_manager_service.rs`;
- `service_endpoint_service::commit_mutation` and the legacy upsert paths;
- ownership transfer access (P7) and the icon asset checks (T-F3);
- the Catalog CLI changes in progress (WP-B).

P7's Tool gate across preview, commit and replay looks correct. T-F1 withdrawal keeps the withdrawn rows as draft blockers. T-F2's Admin-only delete demotes conservatively and is audited. Findings:

| ID | Sev | Location | Finding | Required resolution and test | Status |
| --- | --- | --- | --- | --- | --- |
| R1 | M | `tools_manager_service::fence_role_authority` (via `catalog_mutation_service::fence`, now on every service/endpoint mutation) | Every mutation increments `tools_write_fence` on the actor's user doc, **all** of the actor's groups, and **all** role docs, including the immutable shared system roles (`admin`, `operator`, the default role). This makes a platform-wide write hotspot: concurrent edits by unrelated users conflict and retry. It is also unnecessary, because system-role permissions cannot change at runtime (`role_service.rs:421`) and platform Admin/Operator status depends only on `user.role_ids`. | Fence the user doc always. Fence only **non-system** roles that carry the required permission, and only the groups whose `role_ids` include such a role. Rename the counter to something neutral (e.g. `authority_fence`), since it now covers non-Tool catalog writes, and document it. Tests: system-role docs are untouched by catalog/Tool writes; editing a custom role's permission concurrently still conflicts; revoking a user role still conflicts. | open |
| R2 | M | `catalog_mutation_service::fence` | Full-document equality between the live and observed snapshots now gates **all** service and endpoint mutations, including legacy non-Tool AI services. Background writers (billing price-sync status, catalog identity reconciliation, concurrency config, skills) create spurious `409 Service changed` errors for Admin and creator edits. Before this change, unrelated fields were last-writer-wins. | Fence an explicit authority/contract projection: kind, owner, creator, both markers, visibility, `is_active`, provider link, type/category, `base_url`, auth method/key, credential bytes and freeze, destinations, identity/forwarding/delegation, token exchange, default headers, WS injections, anonymous rules, `requires_user_credential`. Keep the stricter `updated_at` binding only for Tool first-credential/destination writes. Tests: a billing sync-status write between fetch and commit does not fail an Admin description edit; a kind, owner or `base_url` change does. | open |
| R3 | L | `catalog_mutation_service::commit_update` | Duplicates the conversion check already in `require_desired_kind`. The icon comment claims "legacy remote non-Tool icons" keep a contract, but service `icon_url` is new and only uploaded assets are allowed. The in-transaction check parses with `contains`/`rsplit`. | Remove the duplicate, correct the comment, and reuse one strict prefix+UUID parser in the handler and the transaction. | open |
| R4 | L | `services.rs::validate_service_icon_url` | An icon cannot be cleared: `""` is rejected, while the other metadata URLs treat `""` as clear. | `""` → null through the same fence and audit (Admin and owning manager), with CLI manifest and frontend parity, plus tests. | open |
| R5 | M | `service_endpoint_service::commit_mutation` (`contract_changed = true` for any field diff) | A description- or schema-only edit to one Tool operation demotes **every** published operation of that Tool. That is an availability outage requiring full republication, and only route-shape changes can expose a broader route. | Demote all published operations only for route-affecting changes (method, path, `target_id`, deletion, name changes affecting identity). Other contract edits demote and advance only the edited row. A new draft insert never demotes. Keep no-op preservation. Tests: a description edit demotes one operation; a path edit or delete demotes all. | open |
| R6 | M | `catalog_spec_sync` → `upsert_endpoints_additive` → per-row `initial_publication` | Background/startup spec sync still reads the kind per row outside any parent fence. An Admin converting AI service→Tool while sync runs can insert a **published** operation on a Tool (P10's background note). | Route the additive sync through `commit_mutation(Reconcile { authoritative: false }, None)` with the observed parent snapshot. Test: a deterministic flip race creates no published Tool operation. | open |
| R7 | L | `service_endpoint_service::{create_endpoint, update_endpoint, delete_endpoint, bulk_upsert_endpoints}`, per-row `initial_publication`, legacy `commit_tool_contract` | Unfenced legacy mutation functions now survive only for tests. Any future production caller would bypass the parent/authority fence (FI-007). | Delete them and port their tests to `commit_mutation`. | open |
| R8 | L | `admin_catalog.rs::post_provider`, `set_client` | Every failure maps to a fixed redacted message, even when the request carries no client values, and the HTTP status and NyxID numeric `error_code` are dropped. Operators lose the authority/conflict/validation distinction. | Redact only requests that carry secrets, and for those keep the status and numeric `error_code` (never body text). Non-secret requests keep the normal error rendering. Wiremock tests for both. | open |
| R9 | M | `handlers/providers.rs` `list_providers` include_inactive (Catalog Manager branch) | The filter matches `catalog_manager_created: true`, but `ProviderConfig` has no such field. A Catalog Manager's own inactive providers are therefore never listed, so the provider half of C-F3/D5 does nothing. | Match `created_by == caller` (Catalog Managers own the providers they created, as `check_provider_update` does). Tests: a manager sees their own inactive provider and not a foreign one; Admin sees all; a non-writer gets 403; CLI apply reports `inactive` instead of a failed create. | open |

### PM decision D9: Zoho `Zoho-oauthtoken` scheme (resolves the C-F7 caveat)

Evidence: Zoho documents `Authorization: Zoho-oauthtoken <token>`, and NyxID
only injects `Bearer`. Bearer acceptance cannot be verified without a real
vendor test, and the delivery may not ship a known 401 caveat, so NyxID needs
a bounded way to send a documented non-Bearer scheme.

1. **Field.** Add optional `auth_scheme` to the catalog `DownstreamService`. It
   is valid only with `auth_method: bearer`. Accept an RFC 9110 `token` (tchar
   only, 1–64 bytes). An absent value means `Bearer`, with byte-identical
   behaviour for existing rows. No `UserService` override.
2. **Injection.** One shared helper builds the header value. Every
   **server-side** bearer injection of catalog-backed vendor credentials uses it:
   direct HTTP, direct WS, node-routed HTTP/WS where the server injects, and MCP
   proxy calls. Verify each path in code and list them in the report.
   Node-owned credentials and NyxID's own delegated/forwarded tokens are out of
   scope; record that applicability.
3. **Authority.** Classify the field in `catalog_authority.rs` exhaustive
   destructuring:
   - Catalog Manager: may set it on create and on own marked rows.
   - Tools Manager: may not set it (keep the Tool allowlist).
   - Admin: may set it.
   - For Tools it is a contract field: changing it demotes operations.
4. **Approvals.** Add it to the execution-authority projection only when it
   is present, so existing digests stay stable during a rolling upgrade.
5. **CLI and template.** The manifest passes it through and validate compares
   it. The Zoho template sets `auth_scheme: Zoho-oauthtoken` and drops the 401
   caveat. Docs describe the field.
6. **Frontend.** Add an optional input on the service create/edit forms for
   bearer services (Admin/Catalog Manager), with the Zod schema.
7. **Tests.**
   - The loopback mock observes the `Zoho-oauthtoken` value on HTTP, WS and MCP.
   - The default stays `Bearer`.
   - Invalid schemes (whitespace, CRLF, over-long) are rejected.
   - Authority-matrix rows cover the field.
   - Digests are unchanged for rows without the field.

The real-Zoho smoke test remains an R gate. Fake proof never closes it.

### Round 2: source snapshot of 2026-10-13 22:20 (+08), after the Astra handoff checkpoint

Source-reviewed fixes, still awaiting tests:
- R1: the user doc is always fenced; only permission-granting mutable roles and the groups that grant them are fenced; system roles are untouched; the counter is `authority_fence`.
- R2: an explicit authority/contract projection replaces full-document equality.
- R5: all operations are demoted only for route-shape changes (`name`, `method`, `path`, `target_id`) or deletion.
- R6: seeded and background sync go through `commit_mutation` (non-authoritative).
- R8: client-bearing failures are redacted with the HTTP status and numeric `error_code` kept.
- R9: the provider inactive filter uses `created_by`.

The status of R1–R9 stays `fixed-untested` until their tests run. New findings:

| ID | Sev | Location | Finding | Required resolution and test | Status |
| --- | --- | --- | --- | --- | --- |
| R10 | L | `services_helpers::service_to_response_with_eligibility`, used per row by `list_services` and provider listings | `eligible_manager_tool_in_db` runs one `service_provider_requirements` query per eligible Tool row, an N+1 on list paths. CLAUDE.md's catalog-listing rules require batched lookups. | One batched `$in` query per list request feeding a pure eligibility function. Single-row paths may keep the direct query. Test: list eligibility matches single-row eligibility, including for a Tool with a provider requirement. | open |
| R11 | L | `catalog_spec_sync.rs:84, 234` (`audit_chain_hmac_key().unwrap_or(&[])`) | Background sync substitutes an **empty** HMAC key when the chain key is not initialized. That contradicts the audit-service invariant ("never substitute an unkeyed digest") and would append chain entries that fail verification. Production initializes the key first (`main.rs:422` before `:581`), but the fallback is still unsafe. | Fail closed when the key is missing (return an error, or skip only non-Tool sync without an audit append) instead of passing `&[]`. Test: sync with no key appends no chained entry and does not mutate Tool publication. | open |

### PM decision D10: Tools acceptance gate (user clarification, 2026-10-13)

The user clarified that "Tools" means API services that NyxID lets users call.
That is the existing `offering_kind: tool` concept: NyxID-held shared
credential or no-auth, billed to the acting user, Admin-published. The
real-service gate is therefore the Tool counterpart of the Catalog
outside-user gate.

- **Gate:** an Admin confirms that an outside (non-staff) user called the
  published Tool through NyxID against the real vendor and that the usage was
  billed to that user. For no-auth Tools, only the call is confirmed.
- **Readiness output** (refines T-F11): Tools `validate` reports
  `verification: {status: "not_verified", pending_gates: ["outside_user_call_real_vendor_admin_confirmed"]}`.
  It never prints Verified.
- **Recording:** the Admin records the confirmation in the Lark row and as a
  redacted line in the verification report, the same as D4. No database or
  attestation flag.
- **Fake proof:** the local fake-vendor E2E remains software-flow proof only.

### PM decision D11: Tools serve both credential lanes, as the platform does today (user clarification)

Current platform behaviour (baseline `docs/TOOLS.md`): Tools are provider-less
and `requires_user_credential=false`, and they support two lanes:

- **Platform lane:** NyxID-held master credential or no-auth, live audience ACL, `platform_key_pricing`.
- **BYOK lane:** a user binds their own API key to the Tool row through AI Services (`/keys`), with optional `byok_pricing`.

OAuth "connect your own vendor account" remains on provider-linked Catalog
services. Tools stay provider-less; no new platform capability. This refines D10.

1. **Preserve BYOK on manager-prepared Tools.** Add a regression where a user
   binds their own key to a published manager Tool with a restricted-empty
   audience. Expected:
   - the call succeeds and the vendor observes the user's key, not the shared one;
   - usage is billed to that user on the BYOK lane (free if unpriced);
   - the platform lane is still denied to non-allowlisted users;
   - unpublished or paused operations are denied on both lanes.
   Confirm that none of the new fences, the no-auth ACL, eligibility, or the
   Tool mutation paths blocks BYOK or removes BYOK bindings.
2. **Manifest field.** Add an optional, CLI-only Tools field
   `lanes: [platform, byok]`: unique, non-empty, default `[platform]`. `byok`
   is invalid with `auth_method: none`.
3. **Readiness.** `validate` reports `lanes.platform` (credential/no-auth,
   audience, platform pricing) and `lanes.byok` (an authenticated method,
   publication, effective BYOK pricing and sync status, and the existing CLI
   command a user runs to add their own key) separately. Overall `ready`
   requires every declared lane to be ready.
4. **Gates (per declared lane).** `pending_gates` contains:
   - `outside_user_platform_call_admin_confirmed` (billed to the caller), and/or
   - `outside_user_byok_call_admin_confirmed` (the user connects their own vendor key and calls; BYOK lane billing).
   Never Verified.
5. **Handoff.** If `byok` is declared and the BYOK lane is unpriced, add a
   non-blocking Admin pricing advisory. Free stays allowed.
6. **Docs and E2E.** `docs/TOOLS.md` describes both lanes and their gates; the
   Lark row carries the lanes. Extend the Tools fake-vendor E2E with a second
   user calling through their own fake key: the vendor observes that key, and
   the billing owner is the user.

### Delivery authorization (user, 2026-10-13)

Once PM sign-off and every check are complete with no leftovers:
- raise the PR, squash-merge it when possible, and babysit it until CI is green and no merge conflicts remain;
- local commits on `tools-manager-cli-wizard` are permitted for integration; push only after sign-off.

The PM performs push, PR, babysitting and merge. The implementer never pushes,
opens PRs or merges. Fable 5.1 review from the original plan is superseded by
the user's instruction that the PM review personally without spawning sessions.
Recent PRs (#1870, #1872, #1873) were squash-merged by the user's account while
review was still pending, so a human approval is not expected to block the
merge; required CI must be green.

### R12 (H): integrate latest main now; #1872 overlaps semantically

`origin/main` is `4faec86b`, 5 commits past the baseline. #1872 ("let Catalog
Managers complete shared catalog rows") changes `catalog_authority`, the
`services.rs` update path, `endpoints.rs`, `providers.rs`, `provider_service`,
`catalog_icons`, `admin_catalog.rs`, `CATALOG_MANAGER.md` and `CLAUDE.md`. 23 of
our files overlap. Integrate before more Catalog work.

**Method.** Back up `git diff --binary HEAD` and the untracked files to `/tmp`.
Make a local WIP commit (no push), run `git merge origin/main`, resolve the
conflicts, then commit the merge locally. Any later changes stay uncommitted
until final delivery.

**Reconciliation must preserve both sides:**

1. **Shared-row rule in the fence.** `catalog_mutation_service::fence` must
   apply #1872's `require_catalog_row_writer` rule against the **live** row
   inside the transaction:
   - a shared row is public, unowned, HTTP, not internal and not a Tool;
   - the live Catalog Manager permission is fenced;
   - #1872's field limits and destination guard still apply;
   - the creator rule stays for other rows; Tool rows are unchanged.
2. **`is_active`.** Catalog Managers cannot change `is_active` on rows they
   did not create.
3. **Endpoints.** `commit_mutation` allows Catalog writers to create, update,
   delete and discover on shared non-Tool rows, and to publish/pause non-Tool
   operations. Tool operations stay Admin-published. Catalog writers see paused
   and draft operations in listings.
4. **Providers.** Keep the `update_provider_if_unchanged` version guard and
   client re-entry. #1872's `provider update --with-client` must use our
   credential transport, `Zeroizing` and secret-only redaction (C-F1/R8).
5. **Icons.** Catalog Managers may upload any provider's icon; Tool icon
   behaviour is unchanged.
6. **Docs and the playbook.** Merge the docs. The playbook `requires_admin`
   flag reflects #1872: seeded and shared rows are editable; client-bound
   settings are editable only before anyone connects and with client
   re-entry; replacing or clearing someone else's client, or `is_active`, stays
   Admin-only. D9 `auth_scheme` takes `auth_method`'s field class.
7. **Tests.** Rerun fmt, check and Clippy, the #1872 and Catalog Manager
   tests, the focused Tools tests and the CLI tests. Record everything in the
   verification report.
