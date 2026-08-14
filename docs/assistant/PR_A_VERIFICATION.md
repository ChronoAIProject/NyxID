# PR-A verification — design-author sign-off

**Verdict: GREEN — raise the PR.** Branch `travel-booking`, commits `fcc93df7` + `1b426418` + `f086a726`, verified 2026-08-14 against `docs/assistant/TRAVEL_BOOKING.md` §A.1 by the design's author. Opus's three-cycle adversarial review (`PR_A_REVIEW.md`) asked "is there a path to a wrong outcome"; this pass asked "is it the thing we designed, and does it actually run." One defect found and fixed in this commit: the design document itself (below).

## Checks

1. **Code matches §A.1 — yes.** Both deliverables are present and complete: every catalog master-credential decrypt in the tree routes through `authorize_master_credential` / `authorize_master_credential_server_chosen` (strict, lenient, server-chosen, auto-provision, `_nyxid_via`, WS, node, LLM gateway, MCP — the MCP executor resolves through the same functions), and the upstream error-body preview is gone with status/size/request-id logging in its place. Implementation exceeds spec in two places (both good): `EffectiveActor` is sealed harder than specified (private field *and* private constructor), and the `master_credential_required` predicate carries the rationale comment for why identity propagation is additive. One spec item is intentionally partial: §A.1's per-entry-point test matrix ("private-row denial on UUID/slug/lenient/WS/MCP") exists as function-level coverage plus the admin-path e2e plus the cross-resolver delegation test, not as per-route drives. Deferral accepted — the census + function-level tests carry the property; entry-point drives are hardening for PR-B's test budget.
2. **The document's claims are now exactly true — after a fix made here.** Of cycle 1's two structural overclaims: the synthetic-actor claim (**delivered** — `proxy_service.rs:85-96`, private field, private `from_user_id`, no `Default`/`From`; construction outside the module does not compile); the "enforced by the type system / only constructor" claim (**neither delivered nor corrected through cycle 3** — `EncryptionKeys::decrypt` is still public and `handlers/services.rs:2033` decrypts a catalog row's OIDC secret outside the newtype). `TRAVEL_BOOKING.md` §9 has been reworded in this commit to the module-boundary guarantee the code actually provides, with the OIDC exception named. The document no longer overstates its own enforcement.
3. **The gate genuinely ran.** All five targeted tests executed against Mongo on `:27018`: `5 passed; 0 failed; 0 ignored` in 0.58 s, and three of the five `panic!` rather than skip when Mongo is absent (`proxy_service.rs:3327, 3446, 3488`; `handlers/proxy.rs:7006`), so a silent no-op run is impossible. Also run here: `cargo fmt --all -- --check` clean, `cargo build -p nyxid` clean, `cargo clippy --workspace --all-targets -- -D warnings` clean.
4. **The tests test the thing — proven by mutation, not by reading.** I reverted `master_credential_required` to the cycle-2 predicate in a scratch edit and re-ran `delegated_identity_does_not_suppress_master_credential_across_resolvers`: **FAILED (0 passed; 1 failed)**, then restored the tree (`git status` clean). The B5 regression test cannot pass against the pre-fix code. The redaction test is a real end-to-end proxy call against a stubbed 422 carrying a sentinel passenger name, asserting absence from captured tracing.
5. **Deferrals are stated, not silent.** From cycle 1: **N1** — the non-empty-credential clause measures ciphertext length, so `encrypt(b"")` rows pass it and inject an empty bearer (latent; no current wrong outcome; fold into PR-B or a comment). **N2** — a private credentialed row with no `developer_app_ids` is denied with no creator/admin escape hatch; this matches the design as written but is a behavior change operators must hear about (in the PR body below). **N7/N10** — `EffectiveActor::from_user_id` is private, so the gate is uncallable outside `proxy_service`; correct sealing for PR-A, but **PR-B as designed cannot reach the credential gate from a new handler module**. PR-B must either add a `pub fn EffectiveActor::for_auth_user(&AuthUser)` beside the gate (keeping the field private) or host the `/api/v1/resource-tokens/exchange` logic inside `proxy_service`; decide at PR-B design time, not mid-implementation. Also carried: **N14** — the twin auto-provision predicate in `unified_key_service.rs:211-221` is spelled differently from `master_credential_required`; a comment naming each other (or one shared fn) belongs in the next touch of either file.
6. **Before deploy (not before merge):** run the prod readback — `db.downstream_services.find({requires_user_credential: false, auth_method: {$ne: "none"}}, {slug:1, visibility:1, service_category:1, provider_config_id:1, is_active:1, credLen: {$binarySize: "$credential_encrypted"}})`. Every returned row must satisfy public + internal + `provider_config_id: null` + non-empty credential + active + http, or it will 404 after deploy. Specifically confirm the live `aevatar` row (if it is `auth_method: "none"` it never reaches the gate and is unaffected), and triage any private credentialed row without `developer_app_ids` as an N2 casualty. Full `cargo nextest run -p nyxid --profile ci` in CI is the remaining merge gate; only targeted tests plus fmt/clippy/build were run here.

## PR description draft

**Title:** `fix(proxy): gate catalog master-credential access and redact upstream response bodies`

**Body:**

Closes a structural credential-authorization gap in the proxy, independent of any travel work.

**What changed.** Before this PR, any authenticated caller who addressed a catalog service by UUID or slug could have its platform master credential decrypted and injected with no visibility or consent check — the caller-addressed resolvers loaded the row and decrypted unconditionally. Now every path that can produce a catalog master credential (strict, lenient, server-chosen/assistant, auto-provisioned, `_nyxid_via`, WebSocket, node-routed, LLM gateway, and the MCP executor) routes through a single authorization gate: public rows resolve as before; private rows require a valid, unexpired consent to one of the row's `developer_app_ids` OAuth apps; the server-chosen path can never serve a private credentialed row; every denial is `NotFound`-shaped (no existence leak) and logs a reason discriminator. The credential ciphertext is now namable only inside the gate functions, and the actor type cannot be constructed outside the module — no synthetic "system" actor is possible. Separately, the proxy no longer writes any upstream response-body bytes to logs (previously the first 1 KiB of every non-success upstream body was logged at error level, for every service); it logs status, size, and the upstream request id instead. A 2 KiB request-body preview in the shared forward path was also removed.

**Exposure calibration — structural, not live.** The ~30 provider-seeded catalog rows were excluded from the exposure on two independent counts: they carry `provider_config_id` (which the gate predicate rejects) and `auth_method: "none"` (which returns before the credential block is ever reached), and their stored credential is an encryption of the empty string. No seeded row could leak. The gap mattered for admin-created platform-credentialed rows and for rows added in the future — including the planned travel integration, which is why it was found — so this is a hardening of a structural path, not an incident response.

**Verified by:** three adversarial review cycles (Opus — verdict SHIP, `docs/assistant/PR_A_REVIEW.md`: full decrypt-site census, strictly-narrowing behavior proof, consent fail-closed semantics, seeded-row regression analysis) plus a design-author verification pass (`docs/assistant/PR_A_VERIFICATION.md`: spec conformance, doc-claim accuracy, mutation-tested regression coverage — the key test fails against the pre-fix predicate — and live-Mongo execution of the suite; fmt/clippy/build clean).

**Behavior changes to know about:** (1) a *private* catalog row holding a platform credential is now callable only by users holding an OAuth consent for one of its `developer_app_ids`; a private credentialed row with none configured is not callable by anyone, including its creating admin — flip it public or wire an OAuth app if you own such a row. (2) Rows with an empty stored credential now 404 at authorization instead of 500 at decrypt.

**Pre-deploy checks:** run the readback query in `docs/assistant/PR_A_VERIFICATION.md` §6 against prod: confirm the `aevatar` assistant row's shape (expected `auth_method: "none"`, unaffected; if it carries a credential it must be public + internal + no `provider_config_id`), and triage any other returned row against the six predicate clauses — each is a row that resolved a master credential before and must satisfy the predicate after.

**Deferred, tracked in `docs/assistant/PR_A_VERIFICATION.md`:** ciphertext-length vs secret-presence check (N1), private-row escape hatch (N2, by design), `EffectiveActor` constructor exposure for PR-B (N7 — blocks the resource-token exchange route as designed; resolve at PR-B design time), twin-predicate comment (N14).

🤖 Generated with [Claude Code](https://claude.com/claude-code)

## Post-rebase census against `origin/main` (`62add35b`)

Re-verified after rebasing the eight PR commits from the stale `21297220` base. The
intervening mainline work changed both proxy files substantially, but introduced no new
read or decrypt of a catalog master credential. The current-tree census is:

- **Six production reads of `DownstreamService.credential_encrypted`:** two are the
  authorization constructors (`proxy_service.rs:211,242`); three only test ciphertext
  presence for master-credential/auto-provision eligibility
  (`proxy_service.rs:252,1957`, `unified_key_service.rs:198`); one is the documented,
  admin/creator-gated OIDC client-secret endpoint (`handlers/services.rs:2090`). The
  OIDC endpoint requires `auth_method == "oidc"`, returns the client secret to its
  authorized administrator, and is not a proxy credential-producing path.
- **One raw proxy master decrypt:** `AuthorizedMasterCredential::decrypt`
  (`proxy_service.rs:110`). Its ciphertext field and constructor are private. The only
  production call chain into it receives an `AuthorizedMasterCredential`; the other raw
  decrypts in the module are a legacy user's `UserServiceConnection` credential
  (`proxy_service.rs:125`) and a user-owned `UserApiKey` credential
  (`proxy_service.rs:2667`), not catalog master credentials.
- **Four catalog-master plaintext-producing resolver branches, all gated:**
  server-chosen/admin calls `authorize_master_credential_server_chosen` before decrypt
  (`proxy_service.rs:751-753`); strict legacy calls `authorize_master_credential`
  (`:874-881`); lenient legacy does the same (`:1013-1021`); auto-provisioned
  `UserService` resolution does the same in `finish_resolution` (`:2173-2180`). No
  branch reads or clones `service.credential_encrypted` outside the gate.
- **Entry-point convergence:** REST UUID/slug, `_nyxid_via`, direct WebSocket, and node
  fallback resolve through the strict, lenient, or `finish_resolution` branches
  (`handlers/proxy.rs:647,708,869,930,1195,1218`). Both LLM gateway paths use
  `finish_resolution` then strict fallback (`handlers/llm_gateway.rs:292,328,653,726`).
  MCP user-managed and platform execution uses `finish_resolution` or strict/lenient
  legacy resolution (`mcp_service.rs:3084,3226,3234`). Assistant and direct-chat use
  the server-chosen resolver through `execute_admin_proxy`
  (`handlers/assistant.rs:693`, `handlers/assistant_direct.rs:170`,
  `handlers/proxy.rs:1626`). Public proxy explicitly forces `auth_method = "none"` and
  an empty credential (`handlers/public_proxy.rs:145-160`).
- **Established properties survived:** `master_credential_required` remains exactly
  `auth_method != "none"` and is shared by validity/auto-provision checks
  (`proxy_service.rs:144,247,1952`); additive delegation/access-token flags cannot
  suppress the catalog credential. `EffectiveActor.user_id` and `from_user_id` remain
  private with no `Default`/`From` constructor (`proxy_service.rs:86-95`). Upstream
  error logging accepts only service id, status, response size, and request id
  (`handlers/proxy.rs:117-130`); no response-body preview or `/responses` request-body
  preview exists in either proxy file. The end-to-end sentinel test remains at
  `handlers/proxy.rs:7544-7623`.
