# Tools Manager and Catalog Manager CLI: PM gap/acceptance plan

Owner: Opus 5.5 xhigh (PM; ordinary Heca session; no subagents). Implementation
and verification: GPT-6-Astra xhigh. Personal review: the user/coordinator.
This document plans and audits. It does not change source, and it confers no
publication authority. Do not commit, push, create a PR, merge, deploy, or use
production credentials or services.

Inputs read for this audit: `AGENTS.md`/`CLAUDE.md`, the preserved plan
`2026-10-13-tools-manager-cli.md` (SHA256 `79cf53cf…aec84`, unchanged),
`2026-10-13-tools-catalog-delivery-handoff.md` (`9cd4d685…cd8c`), the
coordinator-owned `2026-10-13-tools-manager-review.md` (`44c95d6c…dfc2`,
not modified), `2026-10-13-tools-manager-verification.md`, `docs/TOOLS.md`,
`docs/CATALOG_MANAGER.md`, and the uncommitted diff on baseline `10043f0b`:
66 modified and 13 new files. I traced the authority, endpoint, proxy, icon,
CLI, and E2E paths cited below in source. I ran no builds or tests because
those belong to Astra and the 16 GB host must not run overlapping compiles.

## 0. Evidence vocabulary (applies to every row below)

| Level | Meaning | Who produces it |
| --- | --- | --- |
| **S** (source) | Code exists and the PM traced it at this snapshot | PM/Astra/reviewer |
| **L** (local proof) | Automated test or E2E with a recorded command, exit code and log, run against loopback fakes on an owned replica set | Astra |
| **R** (real-service) | Real vendor app/account, or an outside user's own vendor account, confirmed by an authorized Admin | Admin only |

- **Ready is configuration readiness, never Verified.** `validate` may report
  `ready` from metadata alone. A row is **Verified** only after its applicable
  R gates (the real-vendor API smoke test and, for user-credential services, the
  Admin's confirmation of an outside user's connection). No CLI output, test,
  or report may print or imply `Verified`.
- **Fake proof never stands in for vendor evidence.** A loopback fake-vendor
  E2E demonstrates the software flow only. Never run tests against production
  to close an R gate, and never fabricate R evidence.
- **No new verification database.** Evidence lives in secret-free manifests
  (scope source, revocation applicability, smoke operation), structured CLI
  reports, the Lark Platform Accounts row, and redacted notes in the
  verification report. Nothing in the playbook requires server-side storage.

## 1. Applicability: which items apply to which row type

The six playbook items (C1–C6), the smoke test, and the outside-user gate are
defined for **OAuth catalog services**. Other row types receive explicit
applicability instead of invented OAuth URLs.

| Item | OAuth2 provider service | Device-code provider service | API-key provider service | Provider-less Tool, shared credential | No-auth Tool |
| --- | --- | --- | --- | --- | --- |
| C1 NyxID-managed client ID/secret, no leak | Required (ID + secret) | Required (ID; secret only if the provider requires one) | N/A: users supply keys | Equivalent: **first shared credential**, set once (hidden/env), then frozen | N/A (`not_required`) |
| C2 Default scopes + retained source | Required: vendor CLI login scopes, else documented minimum for exposed operations; `supports_oauth_scopes:false` is N/A with a source | Same as OAuth2 | N/A | N/A (vendor-side key scope; may note in `auth_notes`) | N/A |
| C3 Authorization/token/revocation URLs | Authorization + token required; revocation configured **or** documented `vendor_unsupported`/`nyxid_incompatible` with source | Device-code + device-token URLs (and any URL the server requires); revocation as OAuth2 | N/A | N/A: never invent OAuth URLs | N/A |
| C4 `credential_mode` admin (default) or both | Required; `user` = **not met** (connectable but fails playbook) | Same | N/A | Equivalent: Admin-only credential replacement plus restricted-empty audience until Admin approval | Same audience rule |
| C5 OpenAPI URL **or** explicit operations | Required (≥1 active operation) | Required | Required | Required: selected operations; Admin publishes | Required |
| C6 Static SVG icon | Provider icon via `admin catalog icon` (Catalog Manager: own providers; Admin otherwise) | Same | Same | `admin tools icon` (service-owned asset) | Same |
| Smoke test (API) | Required: declared read-only GET; L via fake, R via real vendor | Required | Required | Required: ordinary non-admin user through the published Tool, with a billing row observed (R) | Required (no billing for no-auth) |
| Outside-user connection confirmed by Admin | Required (R) | Required (R) | Required (R): an outside user connects their own key | N/A: no user vendor account. Covered by the ordinary-user smoke test | N/A |

Provider-less non-Tool catalog services, such as internal no-auth rows, are
outside the six-item OAuth playbook. Validate reports them as
`not_applicable: provider-less non-Tool service`; this delivery adds no new
icon route for them.

## 2. Catalog Manager CLI gap matrix (updated six items + gates)

Code: `cli/src/commands/admin_catalog.rs` (unchanged from `10043f0b`),
`cli/templates/catalog/zoho-mail.yaml`.

| Item | Current state (S) | Gap | Required change | L proof | R gate |
| --- | --- | --- | --- | --- | --- |
| C1 client, no leak | Wizard asks for the ID visibly and the secret hidden (`read_client`, :1241); env via `--client-id-env/--client-secret-env` and slug-derived names; provider responses expose only `has_client_*`. | **C-F1:** secret-bearing requests use the default client, which follows up to 10 redirects (`build_cli_http_client`, `cli/src/api.rs:99`); 307/308 replays the body. Plain HTTP to non-loopback hosts is allowed. Secrets are plain `String`. API error chains from secret-bearing POST/PUT are propagated verbatim. **C-F2:** `apply --dry-run` and `add` read env secret values before planning (:262, :315 → `clients_from_env`). | Every request carrying client values (`provider create` with client, `provider set-credentials`, `catalog add`/`apply` when clients are present) uses `ApiClient::from_auth_with_credential_transport` (HTTPS or exact loopback only, no redirects). Hold client secrets in `Zeroizing`. Map failures of secret-bearing calls to a fixed redacted message (as `admin_tools::set_credential`). Dry run performs **no env reads** and reports only the env *names* required at apply time. Apply reads values just in time, immediately before the create/PUT that needs them. A visibly typed client ID is acceptable (RFC 6749 §2.2) but must never be printed, saved, or logged. | wiremock: a 307 from a secret-bearing PUT is not followed, and the secret never reaches the redirect target; a non-loopback `http://` base URL is refused before any request; dry run with poisoned env values succeeds without reading them (env-lock test asserting no access, e.g. unset vs. set yields identical output); error bodies echoing a canary are redacted from stdout/stderr | — |
| C2 scopes + source | The wizard prompt says "matching the vendor's own CLI"; the Zoho scope source exists only as a YAML comment. | No structured, retained scope-source evidence; validate cannot report it. | Add a CLI-only provider `playbook.scope_source` `{kind: vendor_cli \| documented_minimum \| no_scopes, url, note?}` (see D1). The wizard asks for it, and `--save` keeps it. Validate reports item C2 as `met` only when scopes match the manifest and a source is declared. | Manifest parse/strip tests; validate JSON snapshot for met/missing/no_scopes | Admin checks the cited source when confirming Verified |
| C3 URLs | Wizard covers authorization/token/revocation; validate compares authorization/token/scopes/params/auth method/PKCE but **not revocation**. The Zoho template has no revocation. | Revocation is neither validated nor reported, and vendors without revocation have no explicit applicability. | Validate compares `revocation_url`/`revocation` against the manifest. `playbook.revocation` `{kind: configured \| vendor_unsupported \| nyxid_incompatible, source, note?}`; `configured` requires the row fields. Implementer verifies Zoho's documented revoke endpoint and request style against NyxID's supported styles (`rfc7009` form etc.) from the vendor docs. If the style is incompatible, record `nyxid_incompatible` with the source rather than inventing a config. | Validate tests for configured/missing/declared-unsupported; template parse test | — |
| C4 credential_mode | Wizard default is `admin`; server default is `admin` (`providers.rs:377`). Validate marks `user` as a *note* while status stays `ready` (:1061). | **C-F4:** `user` mode passes validation. | Keep `status` semantics for compatibility; set playbook item C4 = `missing` (reason `credential_mode_user`) for `user`, and `met` for `admin`/`both`. A `both`-mode provider still needs the managed client for C1. | Validate tests across admin/both/user | — |
| C5 spec or operations | Services pass `openapi_spec_url` through, and existing auto-discovery runs on create/update. The manifest **cannot author explicit operations**. Validate checks neither. | **C-F8:** no explicit operations (Zoho Mail publishes no official OpenAPI document; verify). Validate does not report the operation count. | Service-level `operations:` list (allowlisted endpoint fields: `name, method, path, description, parameters, request_body_schema, request_content_type, response_description`), mutually exclusive with `openapi_spec_url`. Reconcile by name through the existing `/services/{id}/endpoints` routes: create missing, update drifted fields, **never delete** omitted rows (report them as `unmanaged`). Validate requires ≥1 active endpoint. Rerun reports `unchanged`. | wiremock create/update/no-op/omitted tests; validate C5 tests | — |
| C6 SVG icon | `admin catalog icon <provider>` exists (#1857); validate ignores icons. | Item not reported; the template omits the step. | Validate reports C6 `met` iff the linked provider has an `icon_url` served from `/catalog-icons/`. Mark `requires_admin: true` when the caller lacks write authority for the provider (seeded/foreign provider). Docs/template add the icon step. | Validate tests | Admin inspects the light/dark rendering |
| Smoke test | Validate prints `connect_command`; docs show `nyxid proxy request …` ad hoc. | **C-F9:** no declared smoke operation. **C-F6:** `connect_command` is always `… --oauth` (:1107), including for API-key providers. | Service-level `smoke_test: {operation: <explicit op name>}` or `{method: GET, path: <literal path>}`: GET only, no query values or headers, path restricted to `[A-Za-z0-9/._-]`. Validate prints the exact `nyxid proxy request <slug> <path> --method GET`. Build `connect_command` per provider type using the existing `service add` syntax (OAuth/device-code vs API key). **The CLI never executes the smoke test**; `nyxid proxy request` already does. | Catalog fake-vendor E2E runs the printed command (WP-G) | Admin/Catalog Manager runs it with a real test account; record date, actor, HTTP status class only |
| Outside-user confirmation | Not represented. Lark `remarks` say `validation: ready`, which reads as Verified. | Ready/Verified conflation. | Every validate row gets `verification: {status: "not_verified", pending_gates: ["api_smoke_test_real_vendor", "outside_user_connection_admin_confirmed"]}` (only the gates applicable per §1). Lark remarks say "configured; not verified". The Admin records confirmation manually in the Lark row's Verified field and as a redacted line in the verification report (date, Admin, slug, "outside user", no account identifiers or tokens). | Validate JSON snapshot | **R only** |
| Inactive rows (handoff defect) | Apply lists active rows only. Services' slug unique index is **partial on `is_active:true`** (`db.rs:429-440`) and the create check filters `is_active:true` (`services.rs:1238`), so applying over a deactivated service **creates a duplicate active row**; `CATALOG_MANAGER.md` wrongly says the create fails. Providers have a full unique slug index, so creation fails opaquely. `include_inactive` is Admin/Tools-Manager only. | **C-F3** | See D5: scoped `include_inactive` for `/services` and `/providers`. CLI matches inactive rows and reports `inactive` with the exact reactivation handoff; never creates. Fix the doc sentence. | Backend authorization tests (Admin all, Catalog Manager own, foreign hidden); wiremock no-create test | — |
| Template | Zoho has no revocation, spec/operations, icon step, smoke op, or structured scope source. | **C-F7** | Update `zoho-mail.yaml` with `playbook`, explicit read-only operations (accounts/folders/messages) taken from Zoho Mail API docs, `smoke_test`, and an icon step in comments/docs. Do not commit vendor SVGs without confirming brand terms; the icon step stays a documented operator action. | Template parse + validate test | Real Zoho run is R |

## 3. Tools acceptance matrix (original plan §0–§3)

Status reflects source traced at this snapshot. "S" rows still need fresh L
evidence after the fixes in §4. The last recorded L results predate the
latest edits (handoff: CLI run15, E2E13, frontend 5,143, Clippy7).

| Requirement | State | Evidence / gap |
| --- | --- | --- |
| Seeded role/permission, Operator prerequisite, capability, grant/revoke | S | Collision-safe seed (`role_service.rs` `ensure_unclaimed_system_role`); `manage_tools` (`users.rs`); E2E grants through ingress. L pending rerun. |
| Server-owned marker, creator/owner/shape fence, exhaustive classifier | S | `require_tool_writer`, `check_tool_create/update` (`catalog_authority.rs:511-627`); transactional `fence_tool`/`fence_authority` (`tools_manager_service.rs`). |
| Human-only writes, no Agent Key/delegated management | S | Plus an Admin-wide tightening (**T-F6**). |
| Twins sanitized; destination-bound ops refused; private source visibility | S | `prepare_create_for_actor`; `services.rs` refuses inherited `target_id` and clears execution/async. Run5 twin failure reportedly fixed; L pending. |
| Restricted-empty initial audience; Admin audience/price/publish | S | `services.rs` create forces `restricted`/`[]`; publication fence in `tool_publication_service.rs`. |
| First credential CAS, URL race, durable freeze after Admin clear | S | `commit_manager_update` binds `updated_at`/`base_url`/credential/freeze; `tools_credential_frozen` set on any Tool credential write. |
| Published-edit demotion, generation, no-op preservation | S, **gaps** | Contract edits demote. **T-F1** (discovery omission) and **T-F2** (Admin hard delete) are open. |
| HTTP/WS/MCP publication gates; no-auth live ACL; redirects disabled | S | E2E covers HTTP/MCP draft and paused, plus WS paused. No-auth ACL extends to provider-linked `auth_method=none` rows: **T-F7**. |
| Icons: shared validator, immutable serving, live recheck, failure preserves old icon | S, **gap** | `catalog_icons::upload_tool`. **T-F3** (attach checks syntax only). |
| Frontend capability/route/nav/page parity | S | `canManageTools`, `canWriteTool`, guards, editor, mobile tests. L pending after any API change. |
| CLI add/apply/validate/icon, flags, variables, typed manifest, secret-free save | S | `admin_tools.rs`. **T-F8** (rerun discovery), **T-F9** (topic list drift), **T-F10** (stale comment). |
| Dry-run/validate read-only, no env access | S | `plan()` reads only; validate uses `without_token_refresh`. |
| Readiness: pending_admin vs failure, effective/operation/component price, eligibility, Lark | S, **gap** | Missing `verification` block (**T-F11**) and pricing advisory (**T-F12**). |
| Firecrawl template, new-API doc, wrapper script | S | `cli/templates/tools/firecrawl.yaml`, `scripts/tools/add-tool-twin.sh`, `docs/TOOLS.md`. |
| Local fake-vendor E2E | L (stale) | E2E13 passed on an earlier snapshot; E2E14 interrupted. Must rerun after fixes with the new assertions in WP-G. |
| Full backend suite | **Failing/unknown** | Run5: seven failures, interrupted (handoff list). No full green exists. |
| Remote CI, model sign-offs, commit/PR/merge | Not run | Withheld by the user / external (see §8). |

## 4. Findings ledger (PM IDs; fix all, at every severity)

Severity: H = exposure or authority bypass; M = correctness/compatibility
risk; L = hygiene. Each needs a regression test unless marked doc-only.

| ID | Sev | Location | Finding (traced) | Required fix and test |
| --- | --- | --- | --- | --- |
| T-F1 | H | `service_endpoint_service.rs:341` (`if !tool` skips omission); `handlers/endpoints.rs` manager discovery builds changes only for names present | Authoritative Tool discovery never revokes omitted operations. A published operation the vendor spec dropped or renamed stays **published and live**. `docs/TOOLS.md` calls this "additive". (Also reported by the coordinator.) | On Tool discovery (Admin and manager), demote omitted **published** rows to `draft`, `is_active:false`, generation +1, in the same transaction, with chained audit `tool_operation_withdrawn` (`reason: absent_from_spec`). Keep the rows as blockers; do not delete. Return the withdrawn names. Update the docs. Tests: Admin and manager discovery with a dropped op and a renamed op (old path is no longer reachable via HTTP/MCP); a no-op rediscovery preserves publication. |
| T-F2 | H | `handlers/endpoints.rs:466-481`, `service_endpoint_service.rs:281` | Admin hard delete of a Tool endpoint is a plain `delete_one`, with no demotion and no chained audit. Deleting a paused/draft specific route can expose a **broader published route** without a publication decision. (Also reported by the coordinator.) | For Tools: delete inside a transaction that calls `demote_tool_in_session` (consistent with conservative contract-change demotion) and appends a chained audit entry (`tool_operation_deleted`, ids/names only). Tests: a paused narrow route under a published broad route; after delete, the broad route is demoted and the request returns 12700 with no vendor hit. |
| T-F3 | M | `services.rs:6389` `validate_service_icon_url` | `icon_url` on create/update/manifest checks only the `{BASE_URL}/catalog-icons/{uuid}` syntax, not that the asset exists. (Also reported by the coordinator.) | Look up the asset through `catalog_icon_service` before attaching (inside the manager transaction where applicable). Reject a nonexistent id with a validation error. Tests: nonexistent UUID rejected; existing asset accepted; the failed attach keeps the old icon. |
| T-F4 | M | `services.rs:975-990` | `GET /services?include_inactive=true` gives Tools Managers **all** inactive non-private rows, including AI services, not just Tools. | Admin: all rows. Tools Manager: inactive rows with `offering_kind: tool`. Catalog Manager: see C-F3/D5. Union for both roles. Tests per role. |
| T-F5 | M | `services.rs:39, 469, 1080, 1092` | `#[serde(deny_unknown_fields)]` on shared Admin Create/UpdateServiceRequest is an API behavior change, and the error "Invalid service request fields" hides the cause. | Keep it (it is fail-closed for manifests) **only after** a recorded caller inventory: frontend payload builders, `service add/update --catalog-admin`, `admin catalog apply` pass-through manifests and templates, scripts, SDK/docs examples. Add a test per first-party payload. The error names the unknown field(s) **without values**: surface serde's `unknown field` message, and use a generic message for type errors, which can echo values. Narrow to Tool-manager requests if the inventory finds an extra-key caller that cannot be fixed. |
| T-F6 | M | `catalog_authority.rs:66` | `require_catalog_writer` now requires a first-party human for **Admin and Catalog Manager** provider/service writes. That tightens existing behavior; previously these routes were outside `api_v1_human_only`. | Accepted as intentional (security-positive; the playbook is human-run). Add tests rejecting API key, Agent Key login credential, delegated, SA and OAuth-app tokens on POST/PUT `/providers` and `/services`. Document in `CATALOG_MANAGER.md`, including that `nyxid login --agent-key` profiles cannot run catalog admin. Confirm the Catalog Editor/Curation paths (`/keys/{uuid}`, `/catalog-curation`) are unaffected. |
| T-F7 | M | `proxy_service.rs` (three `auth_method == "none"` branches now call `platform_key_service::require` when `platform_key` is set) | Platform ACL now applies to every no-auth row with a platform config, including provider-linked rows that store `auth_method=none` (the run10 regression class). | Regression tests: a provider-linked `auth_method=none` row with restricted platform audience and a user's **own** connection is not denied; a provider-less no-auth Tool is denied to non-allowlisted users on HTTP, WS and MCP; public audience is allowed. |
| T-F8 | L | `admin_tools.rs:751` | Apply rediscovers whenever endpoints are empty. A spec that yields zero operations makes every rerun write, which violates "complete rerun = no writes". | When discovery returns zero endpoints, fail with an actionable error ("spec has no operations") instead of reporting success. Test with a mock spec of zero ops. |
| T-F9 | L | `admin_tools.rs:92` | CLI hardcodes `TOPICS`, which can drift from the server vocabulary. | Fetch the public `GET /tools/topics` during planning and validate against it. Unit-test with a mock. |
| T-F10 | L | `admin_tools.rs:745` | Stale comment ("Switch to HTTPS/loopback-only … before secret access") describes work already done at client construction. | Replace it with an accurate comment. Optionally add a debug assertion that the credential transport is active. |
| T-F11 | M | `admin_tools.rs` `readiness()` / Lark row | Tool `status: ready` plus Lark `status` can be read as Verified. | Add a `verification: {status: "not_verified", pending_gates: ["ordinary_user_smoke_real_vendor"]}` block (no-auth: the same gate without billing), and rename the Lark status semantics to configuration. Snapshot test. |
| T-F12 | L | `admin_tools.rs` readiness/handoff | An authenticated Tool with no synced price is effectively free and gets no Admin pricing handoff. | Add the non-blocking reason `pricing_unconfigured_effective_free` and an Admin handoff line pointing to the existing pricing command or field. Status is unaffected (free may be intentional). |
| T-F13 | L | `service_endpoint_service.rs:574-584` | Pipeline surgery through a chain of `unwrap()` calls panics if the pipeline shape changes. | Build the Tool merge document explicitly, or return `AppError::Internal`. No `unwrap` in this production path. |
| T-F14 | L | `catalog_icons.rs` `upload_tool`; `admin_tools.rs` Icon | The audit actor drops IP/UA. The CLI icon command skips session pre-validation (`from_auth` instead of `from_auth_checked`). | Use `AuditActor::from_auth_user`; use the checked client. |
| T-F15 | L | `tools_manager_service.rs:84, 200` | `fence_authority` increments `tools_write_fence` on the user, group and shared system-role documents on every Tool write, serializing Tool writes platform-wide and adding an unmodeled field to identity documents. | Accept for Operator-scale volume. Document the fence in `docs/TOOLS.md`, and confirm no change-stream consumer of `users`/`roles`/`groups` reacts to it (grep `watch(`). Doc-only unless a consumer is found. |
| C-F1…C-F9 | H/M | `admin_catalog.rs` | See §2 (C-F1 secret transport and redaction H; C-F2 dry-run env M; C-F3 inactive duplicate M; C-F4/5/6/8/9 M; C-F7 L). | As in §2. |

## 5. Resolved design decisions

- **D1 Catalog manifest additions (CLI-only, stripped before every API call,
  excluded from drift).** These sit alongside the existing pass-through fields:

  ```yaml
  providers:
    - slug: zoho-mail
      # … existing POST /providers fields …
      playbook:
        scope_source: {kind: documented_minimum, url: https://www.zoho.com/mail/help/api/, note: Zoho publishes no Mail CLI}
        revocation: {kind: configured, source: https://…}   # or vendor_unsupported | nyxid_incompatible
  services:
    - slug: api-zoho-mail
      # … existing POST /services fields; operations XOR openapi_spec_url …
      operations:
        - {name: list_accounts, method: GET, path: /accounts, description: List mailbox accounts}
      smoke_test: {operation: list_accounts}
  ```

  Unknown keys inside `playbook`, `operations`, or `smoke_test` are rejected.
  Every `url`/`source` is validated as HTTPS without userinfo. Secret-field
  rejection is unchanged.
- **D2 Validate output.** Keep the `status` values and exit codes (exit 1 only
  on `failed`) for compatibility. Add `playbook: {c1…c6, smoke_test: declared |
  missing, complete: bool}`, where each item is `{state: met | missing |
  not_applicable, reason, requires_admin}`. Add the `verification` block from §2.
  Table output gains a Playbook column. No new flags.
- **D3 Smoke test.** The CLI declares the operation and prints the command; it
  never stores results. L = fake-vendor E2E executes it. R = an authorized human
  runs it against the real vendor.
- **D4 Outside-user confirmation.** R only: a Lark row plus a redacted
  verification-report line. No API, database, or CLI attestation flag.
- **D5 Inactive rows.** `GET /services?include_inactive=true` and a new
  `GET /providers?include_inactive=true`. Admin sees all rows. A Catalog Manager
  sees inactive rows they created (`catalog_manager_created` and `created_by`).
  A Tools Manager sees inactive Tool rows. The first-party human guard applies
  to the inactive view. The CLI never creates over a matched inactive row; it
  reports `inactive` and the reactivation command. Server create semantics stay
  unchanged, so foreign inactive slugs remain a documented limitation.
- **D6 Secrets in Catalog CLI.** Credential transport, `Zeroizing`, redacted
  errors, and just-in-time env reads (§2 C1). Dry run reads no env values.
- **D7 Tool contract safety.** Discovery omission (T-F1) and endpoint deletion
  (T-F2) follow the existing conservative demotion pattern: fail closed, Admin
  republishes, chained audit.
- **D8 Compatibility tightenings.** T-F5 is retained only with a clean caller
  inventory and field-name-only errors. T-F6 is accepted and documented.

## 6. Prioritized execution plan for Astra

Run heavy work serially: one large compile or one index-heavy suite at a time.
Use a fresh owned replica set; never touch PID 31485, port 27017, or another
actor's processes. Record every command, exit code, and log path in
`2026-10-13-tools-manager-verification.md`. Recompute the inventory hash at the
end.

1. **WP-A (P1) Tool exposure fixes:** T-F1, T-F2, T-F3, T-F4, T-F13, plus
   docs. Done when the new tests pass and the existing Tools suite
   (`tools_manager_*`, publication, endpoints) passes.
2. **WP-B (P1) Catalog secret hygiene:** C-F1, C-F2. Done when the wiremock
   redirect/HTTP/redaction/no-env tests pass and no CLI test regresses.
3. **WP-C (P1) Catalog playbook support:** D1, D2, C-F4, C-F5, C-F6, C-F8,
   C-F9, the C-F7 template update, the `CATALOG_MANAGER.md` rewrite (six items,
   applicability table, Ready ≠ Verified, R gates, fixed inactive-row sentence),
   and the wizard asking for the scope source and revocation applicability. Done
   when validate JSON snapshots cover OAuth2 admin/both/user, device-code,
   API-key, and seeded/foreign providers (`requires_admin`), and an explicit
   operations rerun is write-free.
4. **WP-D (P2) Inactive lookup:** D5 backend plus CLI for Catalog (C-F3),
   reusing the T-F4 scoping. Done when the role tests pass and a wiremock test
   shows apply never creates over an inactive match.
5. **WP-E (P2) Compatibility proofs:** T-F5 inventory and tests, T-F6 tests and
   docs, T-F7 regression tests.
6. **WP-F (P2/P3) Tools CLI polish:** T-F8, T-F9, T-F10, T-F11, T-F12, T-F14,
   T-F15 docs.
7. **WP-G (P1) Local proofs:**
   - Rerun the Tools E2E with added assertions: discovery withdrawal (T-F1) and
     delete demotion (T-F2). The ordinary user must still be the billing owner.
   - Add a **Catalog fake-vendor E2E** in the same harness with a loopback fake
     OAuth vendor (authorize, token, revoke, one GET API):
     1. A Catalog Manager runs `admin catalog apply` from a manifest with
        `playbook`, explicit `operations`, and `smoke_test`. The client comes
        from env through the credential transport.
     2. Upload the icon with `admin catalog icon`.
     3. `validate` shows the playbook complete and `not_verified`.
     4. A second ordinary user completes OAuth through ingress: the fake
        authorize step redirects to the callback with a code, and the token
        exchange receives the managed client and scopes.
     5. The printed smoke command reaches the fake vendor with the user's
        bearer.
     6. Disconnect calls the fake revoke endpoint.
     7. Re-apply is write-free.
     8. Dry run with poisoned env performs no env reads.
     9. stdout/stderr/audit contain no client-secret canary.

   This is software-flow proof only; it is not R.
8. **WP-H (gate) Full verification:**
   - Investigate each run5 failure in isolation on the final source (pool billing
     timeout, pool overall deadline, twin contract copy, linked shared-service,
     assistant one-shot timeout, auth-device approval/denial audits).
   - Record the root cause and fix. Never call a failure environmental without
     log evidence (e.g. MongoDB errors).
   - Then run, in order: `cargo fmt --all --check`, workspace Clippy
     `-D warnings`, the full CLI suite, the full backend suite (single run, owned
     DB), adapters/permissions, both E2E scripts, frontend test/build/lint,
     `git diff --check`, and the CI helper checks listed in the verification
     report.
   - Finalize the report with a per-file hash inventory.

Coordinator review happens after each WP; findings return to Astra, and
affected checks rerun.

## 7. Definition of done for this local delivery

- Every row in §2–§4 is fixed with passing L evidence, or explicitly
  dispositioned here with the user's agreement. No open finding of any
  severity.
- The full backend, CLI, frontend, Clippy, format, and both E2E runs pass on
  one recorded final source digest, with no skipped test counted as passing.
- Docs (`TOOLS.md`, `CATALOG_MANAGER.md`, `API.md`, `CLAUDE.md` invariants)
  match the behavior. The preserved plan, handoff, and coordinator review
  hashes are unchanged.
- All reports say `not_verified` until R gates are recorded by an Admin.

## 8. Genuine blockers and external gates (not closable by Astra)

- **R gates:** real vendor OAuth apps/API keys and outside users are not
  available to the implementation session. They must be performed later by an
  authorized Admin per service. They remain pending; this is not a blocker for
  local completion.
- **Model reviews** (Fable 5.1 review, Opus 5.5 xhigh final sign-off), **remote
  CI**, commit/PR/merge/deploy: withheld by the user or not yet run. Never claim
  them.
- **Environment parity:** local Rust 1.96.0 and MongoDB 9.0.2 versus CI's
  1.98.1 and 8.0; the release GCP-KMS artifact inspection needs CI artifacts.
  Local results do not establish the CI matrix.
- No evidence-based blocker prevents implementing §6 locally.
