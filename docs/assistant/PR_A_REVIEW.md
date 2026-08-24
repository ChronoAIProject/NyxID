# PR-A adversarial review — `fcc93df7` "fix(proxy): gate catalog credentials and redact upstream bodies"

Reviewed against `docs/assistant/TRAVEL_BOOKING.md` §A.1 (contract) and Part I §9. Branch `travel-booking`, worktree `golden-badger`. 261 insertions across `backend/src/services/proxy_service.rs` and `backend/src/handlers/proxy.rs`.

Everything below was read in the tree at `fcc93df7`. Two targeted test runs were executed for real (Mongo reachable on `:27018`); results are marked inline. The full gate (fmt/clippy/boundary/build/suite) was not run here — it is being run separately.

---

## Verdict

**SHIP-WITH-FIXES.**

The security direction is right and, on the substance, the fix works: I enumerated every path in the tree that decrypts a catalog row's `credential_encrypted` and every one of them now routes through `authorize_master_credential` or `authorize_master_credential_server_chosen`. I could not find a resolution path — strict, lenient, server-chosen, auto-provisioned, `_nyxid_via`, WebSocket, node-routed, LLM gateway, or the MCP executor — that reaches a master-credential decrypt without the gate. The change is strictly narrowing everywhere: there is no input for which the new code decrypts something the old code refused. The redaction change is correct and complete for the REST response path.

What holds it back from SHIP is not a bypass. It is that the two *structural* guarantees the contract sells — "bypass is a compile error", "a synthetic actor cannot compile" — are both unmet (B1, B2), the new authorization function has **zero** test coverage (B3), and one live hot path can be silently taken to a total outage by a row state nobody has verified (B4). None of these needs a redesign; all four are bounded work.

Severity note, per the calibration given: the ~30 provider-seeded `internal` rows are excluded from the exposure on two independent counts — they carry `provider_config_id: Some(...)` (`provider_service.rs:3659`) and `auth_method: "none"` (`provider_service.rs:3598`), and the latter means the strict/lenient resolvers early-return at `proxy_service.rs:668` / `:804` before the credential block is ever reached. So the pre-existing exposure was structural, not live on seeded data, and this commit's behavioral blast radius on seeded rows is exactly nil. I have not inflated anything on that basis.

---

## Blocking findings

### B1. The newtype is not a compile-time gate; the invariant is a duplicated boolean two lines apart, and desyncing it produces an ungated decrypt that compiles

**Consequence:** the contract's central claim — "a single authorization function is the only constructor of a decryptable catalog credential ... so bypass is a compile error rather than something review must catch" (`TRAVEL_BOOKING.md:114`, A.1:188) — is not delivered. A future edit can reintroduce exactly the bug this PR closes, and neither the compiler, clippy, nor `scripts/check-rci-backend-boundary.sh` will say a word. Review must still catch it, which is the thing the design bought the newtype to avoid.

**Mechanism.** `EncryptionKeys::decrypt(&[u8])` (`crypto/aes.rs:372`) was not changed and is still `pub`. Nothing in the type system stops a caller passing raw catalog ciphertext to it. In `resolve_proxy_target` the protection is two separate `if service.requires_user_credential` branches that must agree:

```rust
// proxy_service.rs:689
let authorized_master = if service.requires_user_credential { None } else { Some(authorize_master_credential(...).await?) };
// proxy_service.rs:703
let credential_encrypted = if service.requires_user_credential { user_conn... } else { Vec::new() };
// proxy_service.rs:718
let decrypted_bytes = Zeroizing::new(if let Some(authorized) = authorized_master {
    decrypt_authorized_master_credential(encryption_keys, &authorized).await?
} else {
    encryption_keys.decrypt(&credential_encrypted).await?   // <-- raw Vec<u8> path, still live
});
```

**Failure scenario.** A developer adds a third credential class (say, an org-shared platform key) and edits `proxy_service.rs:703` to `service.credential_encrypted.clone()` in the else arm without touching `:689`. `authorized_master` is `None`, the `else` arm at `:721` decrypts the catalog master credential with no authorization, and it compiles clean and passes every test in the tree. That is the pre-`fcc93df7` vulnerability, restored, with the newtype still sitting in the file looking like it prevents it.

A second, already-in-tree instance of the same class: `handlers/services.rs:2031-2034` decrypts `service.credential_encrypted` outside the newtype entirely. It is gated by `require_admin_or_creator` and restricted to `auth_method == "oidc"`, so it is not a live hole — but it means the A.1 audit item "repo-wide `credential_encrypted` read census, every site through the newtype" is not satisfied, and it is proof by example that the "only constructor" property is a convention.

**Fix shape (not implemented here):** either collapse `:689`/`:703`/`:718` into a single `match` that produces one `enum { Master(AuthorizedMasterCredential), User(Vec<u8>) }` so the two conditions cannot desync, or add a repo lint to the boundary script rejecting `decrypt(` applied to a `DownstreamService` field outside `proxy_service.rs`. The latter is cheap and covers `handlers/services.rs` too.

### B2. `EffectiveActor` can be forged from any module in one line, so the server-chosen prohibition is a convention, not a construction

**Consequence:** A.1's "`EffectiveActor` has no default/system constructor — a synthetic actor cannot compile" (`TRAVEL_BOOKING.md:193`) is false as written. The next person who hits the type checker on a server-side surface can write `EffectiveActor { user_id: "system".to_string() }` and it compiles from anywhere in the crate — which is precisely the escape hatch §9 says is "handled by prohibition".

**Mechanism.** `proxy_service.rs:84-87`:

```rust
#[derive(Clone, Debug)]
pub struct EffectiveActor {
    pub user_id: String,      // pub field, pub struct, no #[non_exhaustive]
}
```

Rust struct-literal construction is available to every module that can name the type. `services::proxy_service` is a public module (other modules already call `proxy_service::resolve_proxy_target`), so the literal is legal crate-wide.

**Failure scenario.** PR-B adds `/api/v1/resource-tokens/exchange`, which A.1:221 specifies must "mint the Duffel component client key server-side via the credential gate". That route has a session user, so it is fine — but the same PR's reissue path, or any future scheduled/worker surface, has no user. `authorize_master_credential_server_chosen` refuses private rows, so the author reaches for `EffectiveActor { user_id: NIL_UUID }` instead. Against a **public** credentialed row that synthetic actor passes the gate outright (`proxy_service.rs:117` — `"public" => {}`, actor never consulted), and the server-chosen prohibition has been routed around with no compile error and no reviewer signal. Against a private row it happens to fail closed (consent lookup on a nonexistent user returns empty at `unified_key_service.rs:1738-1752`), which is luck, not design.

Today the type is not abused — I checked: `EffectiveActor` is constructed at exactly three sites (`proxy_service.rs:696, 854, 2010`), all from a real caller id. The finding is that nothing keeps it that way.

**Fix shape:** private field + a constructor that takes the id from `AuthUser`/`proxy_resolution_user_id()`, or `#[non_exhaustive]` plus `EffectiveActor::for_user(&str)` in the same module as the gate.

### B3. The new authorization function has no test at all — a change that denied every private row, or allowed every private row, would ship green

**Consequence:** the deny decision is the entire product of this PR, and nothing tests it. A regression in either direction is invisible: allow-everything reopens the exposure; deny-everything is a silent outage on private credentialed rows. Both pass CI today.

**Evidence.** The commit adds two tests. `handlers/proxy.rs:4808` covers redaction (see N4). `proxy_service.rs:3270` covers only `authorize_master_credential_server_chosen`. There is **no** test of `authorize_master_credential` anywhere in the tree — no private-row denial, no allowed-with-consent, no revoked-consent, no expired-consent, no empty-`developer_app_ids` denial, and no resolver-level test on any of the paths A.1:193 enumerates ("private-row denial on UUID/slug/lenient/WS/MCP/server-chosen; allowed with consent; public and auto-provision regressions"). Zero of nine listed cases exist.

The one test that does exist has three further problems:

- It exercises only 2 of the 6 predicate clauses (`visibility`, `service_category`). `provider_config_id.is_some()`, `credential_encrypted.is_empty()`, `!is_active`, and `service_type != "http"` are untested — and `provider_config_id` is the clause that excludes the entire seeded catalog, so it is the one most likely to be "simplified away" by someone who does not know why it is there.
- It is gated on Mongo (`connect_test_database` → early `return` at `proxy_service.rs:3272-3275`) for a function whose `db` parameter is literally named `_db` and never touched (`proxy_service.rs:148`). On a CI leg without Mongo the only authorization test in the PR silently passes without executing a single assertion.
- `dummy_service()` leaves `auth_method: "none"` (`models/downstream_service.rs:404`), so the fixture is not shaped like any row that actually reaches the gate through a resolver.

**Ran for real:** `cargo test -p nyxid -- server_chosen_master_credential_requires_public_valid_row upstream_error_log_excludes_response_body --nocapture` → `2 passed; 0 failed`, 0.19s, no skip message on stderr, so the Mongo-gated test did execute against `:27018`.

### B4. The server-chosen path now hard-denies on four row properties nobody has checked against the live Aevatar row — if any is wrong, every assistant chat turn 404s with no log line explaining why

**Consequence:** a total, undiagnosable outage on a live hot path. `resolve_admin_proxy_target` is the assistant chat pass-through (`handlers/proxy.rs:1551-1556`, `TargetMode::AdminManaged`), and it now requires the `aevatar` row to satisfy **all** of: `visibility == "public"`, `service_category == "internal"`, `provider_config_id.is_none()`, `credential_encrypted` non-empty, `is_active`, `service_type == "http"` (`proxy_service.rs:151` → `:159-166`). Before this commit it required none of them.

**Failure scenario.** The row is `visibility: "private"` (a defensible choice for a platform-only row an admin does not want in the user catalog), or it was seeded through the provider path and so carries `provider_config_id: Some(...)` (`provider_service.rs:3659` sets this on every seeded row). Either way `authorize_master_credential_server_chosen` returns `NotFound("Service not found")`, every `/api/v1/assistant/*` turn returns 404, and — because the gate emits no `tracing` event and no audit record, and the admin path has no `proxy_request_denied` wrapper the way the caller-addressed path does at `handlers/proxy.rs:1128-1146` — the logs show nothing distinguishing this from a bad service id.

This is aggravated by an error-shape inconsistency: `resolve_admin_proxy_target` deliberately returns `AppError::Internal` for every other misconfiguration, with the reason spelled out at `proxy_service.rs:532-533` ("A misconfigured platform row is a server fault, not a caller error: the caller had no say in which service this is") and applied at `:535, :541, :547, :553`. The new gate at `:577` breaks that convention and returns a caller-shaped 404 with no server-side detail. The not-found shape is right for `authorize_master_credential` (existence must not leak to a caller who named the row); it is wrong here, where the caller could not have named anything.

**What I could not verify:** the actual state of the live `aevatar` row. The repo says only that it is "admin-seeded" (`services/assistant_service.rs:30-38`) and resolved by slug; it asserts `requires_user_credential == false` at `:54-59` and nothing else. Admin-created rows get `visibility: "public"` by default (`handlers/services.rs:494-509`) and `provider_config_id: None` (`:1050`), and legacy rows missing the field deserialize to `"public"` (`models/downstream_service.rs:161-162, 357`), so the likely case is fine — but "likely" is not a merge gate for a 100%-outage failure mode.

**Merge gate I'd require.** Against prod, before this lands:

```js
db.downstream_services.find(
  { requires_user_credential: false, auth_method: { $ne: "none" } },
  { slug:1, visibility:1, service_category:1, provider_config_id:1, is_active:1,
    service_type:1, credLen: { $binarySize: "$credential_encrypted" } }
)
```

Every returned row is one that used to resolve a master credential and now must satisfy the predicate. Confirm `aevatar` is `public` + `internal` + `provider_config_id: null` + non-empty credential, and triage the rest.

---

## Non-blocking findings

### N1. `!credential_encrypted.is_empty()` measures ciphertext length, not whether a secret exists — the clause does not do what its name implies

**Consequence:** rows with *no* credential still pass the "non-empty credential" check and get authorized; the proxy then injects an empty bearer token and the caller gets a confusing upstream 401 instead of a NyxID-side refusal.

`encrypt(b"")` returns a non-empty ciphertext blob. Three write paths produce exactly that: `provider_service.rs:3578` (`let empty_credential = encryption_keys.encrypt(b"").await?`), `unified_key_service.rs:1132`, and `handlers/services.rs:884` (admin create encrypts whatever `credential` string was supplied, including `""`). So `proxy_service.rs:164` is satisfied by every seeded row and by every admin row created with a blank credential field. If the intent is "this row actually holds a platform secret", the check has to happen after decrypt, or the write path has to store `Vec::new()` for absent credentials.

### N2. There is no escape hatch for a private credentialed row with no `developer_app_ids` — including for the admin who created it

**Consequence:** an admin who today runs an internal tool on a private, platform-credentialed catalog row loses access to it at deploy time, with no configuration that restores it short of flipping the row to public (which reopens the exposure to every account) or wiring up an OAuth client + per-user consent.

`proxy_service.rs:119-125`: private with `developer_app_ids` absent or empty → `NotFound`, unconditionally. `created_by` is not consulted, nor is admin role. This matches the design as written (A.1:193, and it mirrors the pre-existing auto-provision rule at `unified_key_service.rs:1582-1585`), so it is a contract consequence rather than a bug — but it is unannounced, has no migration, and the operator gets a 404 with no hint. At minimum the PR body should state it, and the inventory query in B4 should be run to find affected rows.

### N3. A 2 KiB **request**-body preview is still logged at info level in the shared forward path

**Consequence:** the "proxy logs no bodies" property is not true of the forwarding function itself. `proxy_service.rs:3113-3136` logs up to 2048 bytes of the outbound request body whenever `url.contains("/responses")`, at `info`, for every executor that calls `forward_request` — REST, MCP, node direct-fallback, and public/anonymous proxy.

Duffel is not affected (no Duffel path contains `/responses`), and A.1:195 scopes the redaction to the response-body preview, so this is outside the letter of PR-A. It is inside the spirit of Part I §9's "no bodies in logs" and it is a substring match on the whole URL, so a downstream whose `base_url` happens to contain `/responses` leaks unrelated request bodies. Worth folding in while the file is open.

For completeness, same class, further out of scope: `user_token_service.rs:1319, 1380, 1755, 1770` log full upstream OAuth token-endpoint bodies at error level — `:1770` specifically fires on "response missing access_token", which is exactly the case where a provider returned tokens under a nonstandard key. That is credential material in logs. Not PR-A's job; flagging so it is not lost.

### N4. The redaction test asserts against a function that structurally cannot leak, so it would pass against a broken call site

**Consequence:** the test gives false assurance. A.1:195 asks for "a sentinel passenger name in a stubbed 422 body never appears in captured tracing" — an end-to-end assertion through the proxy. What landed (`handlers/proxy.rs:4808-4835`) calls `log_upstream_error` directly, a function whose body parameter is `_response_body` and is therefore incapable of emitting it. If someone reintroduces a `body = %preview` line beside the `log_upstream_error(...)` call at `handlers/proxy.rs:2996`, this test still passes.

The mismatched arguments make the point: the test passes a 22-byte sentinel with `response_size: 37` and asserts `response_size=37`, i.e. the size assertion is decoupled from the body it claims to describe. The existing in-file `oneshot`-style proxy tests (`handlers/proxy.rs:4779+` has the machinery) could carry a real stubbed-422 version.

### N5. `log_upstream_error`'s dead `_response_body` parameter is an invitation

`handlers/proxy.rs:105-120` takes `_response_body: &[u8]` and never uses it. It exists only so the test can hand it a sentinel. It also makes `response_size` redundant with `response_body.len()` at the call site (`:2996-3001`). A future maintainer reading a function that *accepts* the body and logs a size will reasonably conclude logging the body is in scope. Drop the parameter and derive the size inside, or drop the size and keep the body-as-length.

### N6. Gate denials are unobservable on two of the four paths

`handlers/proxy.rs:1128-1146` wraps the strict/lenient resolvers and emits `proxy_request_denied`, so those denials are audited. The server-chosen path (`:1551-1556`) and the `finish_resolution` paths reached via `resolve_proxy_target_from_user_service` / `..._by_user_service_id` (`:583`, `:643`, `:803`, `:863`) propagate with bare `?` and no audit or log. For a security gate, a denied attempt on a private credentialed row is exactly the event you want in the audit trail — and per B4, its absence is also what makes an aevatar misconfiguration undiagnosable.

### N7. `AuthorizedMasterCredential` is `pub` but unusable outside `proxy_service`, which blocks the PR-B design as written

`decrypt_authorized_master_credential` (`proxy_service.rs:99`) is module-private, so an external caller can obtain an `AuthorizedMasterCredential` from the two `pub` authorize functions and then do nothing with it. A.1:221 requires `/api/v1/resource-tokens/exchange` to "mint the Duffel component client key server-side via the credential gate" from a new handler module — which is not expressible against this API today. Either export a `decrypt` method on the newtype (keeping the field private) or plan for the exchange route to live inside `proxy_service`.

---

## Verified correct

- **Coverage across every resolution path.** I traced every reader of a catalog row's `credential_encrypted` in the tree. The master-credential decrypts are `proxy_service.rs:577` (server-chosen), `:693` (strict), `:851` (lenient), `:2007` (auto-provisioned `UserService`) — all four gated. Everything else that greps as `credential_encrypted` is either a `UserApiKey`/`UserServiceConnection` field (a per-user credential, correctly outside this gate), a write path, a test fixture, or the OIDC-secret read at `handlers/services.rs:2033` covered under B1.
- **MCP executor is covered.** `mcp_service::execute_tool` does resolve independently of the REST path, exactly as §9 warns — but it resolves *through these same functions*: `resolve_proxy_target_by_user_service_id` (`mcp_service.rs:2866`), `resolve_proxy_target_lenient` (`:3006`), `resolve_proxy_target` (`:3014`). No separate catalog decrypt exists in `mcp_service.rs` or `mcp_transport.rs`. Gate applies.
- **WebSocket and node-routed paths are covered.** Both share `execute_proxy_inner`'s resolution; the WS branch and the node branch consume an already-resolved `ProxyTarget`. `_nyxid_via` (`handlers/proxy.rs:583`, `:803`) goes through `resolve_proxy_target_by_user_service_id` → `finish_resolution`. LLM gateway (`handlers/llm_gateway.rs:291, 326, 650, 722`) likewise.
- **Anonymous/public execution is unaffected.** `handlers/public_proxy.rs:144-158` builds its target with `credential: String::new()` and forces `auth_method: "none"`; it never touches the master credential, so the gate cannot break it and never could have leaked through it.
- **The newtype genuinely cannot be constructed externally.** `pub struct AuthorizedMasterCredential(Vec<u8>);` at `proxy_service.rs:91` — tuple field is private, no `Default`, no `From`, no `pub fn new`, no `#[cfg(test)]` helper anywhere in the tree. External struct-literal construction is a compile error. (This is the half of the structural claim that *is* delivered; B1 is about the decrypt API, not the type.)
- **Consent semantics fail closed on every axis I checked.** `developer_app_ids` absent or empty → `NotFound` (`proxy_service.rs:119-125`). Unknown/unexpected `visibility` string → `NotFound` (`:137`). DB error inside `load_valid_app_consents` → propagated with `?`, never treated as "no consent found but allow" (`:127-132`). The consent lookup itself requires an `is_active` OAuth client and a non-expired consent (`unified_key_service.rs:1720-1752`), and revocation *deletes* the consent row (`consent_service.rs:143-181`), so a revoked grant fails closed rather than lingering behind an unchecked flag. Denials are `NotFound("Service not found")` throughout — existence does not leak.
- **No synthetic actor was introduced.** Three construction sites (`proxy_service.rs:696, 854, 2010`), all fed a real caller id — `user_id` from `AuthUser::proxy_resolution_user_id()` on the resolvers, `effective_owner_id` on the auto-provision path. (B2 is that the type permits one, not that one exists.)
- **The change is strictly narrowing — no input gains access.** Old code decrypted unconditionally on all four paths; new code decrypts on a strict subset. On the auto-provision path the new call at `:2007` is redundant with the pre-existing `is_public_internal_master_credential_service` check at `:2001` (which already requires `public`), so that path is behaviorally unchanged — the gate adds nothing there, but subtracts nothing either.
- **Seeded-catalog regression risk is nil.** All 30 provider-seeded `internal` rows default to `auth_method: "none"` (`provider_service.rs:3598`), so `resolve_proxy_target` / `_lenient` return early at `:668` / `:804` and never reach the gate. Their `provider_config_id: Some(...)` and `encrypt(b"")` credential are therefore moot. User-created SSH rows (`unified_key_service.rs:1133-1157`) are `visibility: "private"` + `service_category: "internal"` + non-empty ciphertext — they would otherwise land in the private-denial branch, but `service_type: "ssh"` excludes them at `proxy_service.rs:161` and the HTTP proxy already rejects them upstream at `:620`.
- **Rows with an empty `credential_encrypted` degrade from 500 to 404.** Previously `decrypt(&[])` failed all format probes and returned `AppError::Internal` (`crypto/aes.rs:519-543`); now the predicate rejects first. Strictly better.
- **Redaction is complete for the REST response path.** The one call site (`handlers/proxy.rs:2996`) is in the buffered branch; the streaming branch (`:2904-2950`) logs only `service_id`, transport error, and idle-timeout — no bytes, captured or otherwise. The node-routed branches never logged bodies. `mcp_service.rs:3285, 3314` convert node response bytes to text but *return* them as tool output rather than logging them. Node WS chunk handling (`node_ws_manager.rs:2454`) buffers SSH exec output into the response, not into tracing. The design's "all services, not just Duffel" requirement is met — the removed preview was unconditional and its replacement is too.
- **No log-injection via the new correlation id.** `upstream_request_id` is upstream-controlled (`x-request-id`), but `tracing_subscriber`'s default field visitor renders non-message fields through `Debug`, so embedded CRLF is escaped rather than forging log lines.

---

## Could not verify

1. **The live `aevatar` row's `visibility` / `service_category` / `provider_config_id` / credential state.** This is the B4 merge gate. No repo artifact pins it; the code asserts only `requires_user_credential == false` (`services/assistant_service.rs:54-59`). Query given in B4. Needs a prod readback, not a code read.
2. **Whether any live row other than `aevatar` currently resolves a master credential through the strict/lenient path.** Same query. The tree tells me seeded rows cannot (verified above) and admin-created rows can, but not which admin-created rows exist in prod.
3. **The full verification gate.** Only the two new tests were run here (`2 passed`, real execution against Mongo on `:27018`). `cargo fmt`, `cargo clippy --workspace --all-targets -- -D warnings`, the boundary script, and the full `nextest` suite are being run separately per the brief. One clippy note to watch: `authorize_master_credential_server_chosen` is `async` with no `.await` in its body (`proxy_service.rs:147-157`) — harmless under the default lint set, and it keeps the two authorize functions call-compatible, but `clippy::unused_async` would flag it if that lint is ever enabled.
4. **Whether `#[ignore]`/feature-gated tests were disabled by this commit.** I found none added and none removed — `git show fcc93df7` touches no `#[ignore]` and the diff adds only the two tests discussed. I did not audit the pre-existing suite for tests that this change should have broken but silently skips (the Mongo-gated pattern at `proxy_service.rs:3272` is widespread in this file and predates the commit).

---
---

# Cycle 2 — re-review of `1b426418` "fix(proxy): harden master credential gate and redaction"

380 insertions / 164 deletions across the same two files, on top of `fcc93df7`. Cycle 1 review above is unchanged; this section is appended.

Four targeted tests were run for real and all executed against Mongo on `:27018` — `master_credential_authorization_covers_visibility_and_consent`, `assistant_shaped_server_target_without_credential_resolves`, `upstream_error_body_is_redacted_end_to_end`, `server_chosen_master_credential_requires_public_valid_row`: **4 passed, 0 failed, 1.12s**. Two of them `panic!` rather than skip when Mongo is absent, so their passing is itself proof the DB was reachable. The full gate is running separately.

---

## Verdict

**SHIP-WITH-FIXES** — one blocking item, narrower than cycle 1's.

Most of cycle 1 is genuinely closed, and not cosmetically: the two-boolean desync is gone because `service.credential_encrypted` no longer appears anywhere in either resolver's credential block (`rg 'service\.credential_encrypted' proxy_service.rs handlers/proxy.rs` returns only the two authorize functions, the two predicates, and test fixtures); `EffectiveActor` is sealed outside the module; the authorization test fails in both directions and refuses to skip; the request-body preview is deleted outright; the redaction test is a real end-to-end proxy call against a stubbed 422. That is substantive work, not a paper response.

The blocker is that the B4 fix introduced a new defect on the same hot path it was meant to protect. `master_credential_required` treats `inject_delegation_token` as evidence that a row needs no platform credential — but the delegation token is injected into its own header, additively, and has nothing to do with whether the upstream API needs a bearer key. The predicate is then applied at three of the four gate call sites and omitted at the fourth, so one row shape now produces three different failures depending on which resolver you arrive through. And the regression test named for the B4 condition exits at a pre-existing early return without ever reaching the new code, so none of this is caught.

One correction to my own cycle 1 work, because it changes how urgent that finding was: B4 asserted the gate applies to the assistant row, but `resolve_admin_proxy_target` has always returned early for `auth_method == "none"` (`proxy_service.rs:649`, unchanged since before `fcc93df7`) — I read that branch and did not fold it into the finding. If the live `aevatar` row is `auth_method: "none"`, the cycle-1 gate never applied to it and there was no outage risk. The prod readback is still worth doing, but for the reason in B5 below rather than the one I gave.

---

## Blocking findings

### B5 (new, introduced by this commit). `master_credential_required` treats delegation-token injection as "no credential needed" — a row that legitimately does both now silently loses its platform credential, 500s, or 404s depending on which resolver reached it

**Consequence:** an active catalog row configured with a platform bearer credential *and* `inject_delegation_token: true` — a normal, supported combination — breaks in three different ways with no shared symptom. Through the caller-addressed proxy it silently sends `Authorization: Bearer ` (empty) and the user sees an upstream 401. Through the assistant/server-chosen surface it returns HTTP 500. Through an auto-provisioned `UserService` it returns 404. This is a regression created by the B4 fix, on the hot path the B4 fix existed to protect.

**Mechanism.** `proxy_service.rs:143-147`:

```rust
fn master_credential_required(service: &DownstreamService) -> bool {
    service.auth_method != "none"
        && !service.inject_delegation_token      // <-- wrong premise
        && !service.forward_access_token
}
```

The premise is false for `inject_delegation_token`. That flag causes a *separate, additive* header to be added — `handlers/proxy.rs:1984-2007` pushes `X-NyxID-Delegation-Token` into `identity_headers` and never touches `Authorization`. It is orthogonal to `auth_method` injection by design (CLAUDE.md Rule 6 states identity/frame injection is "additive, separate from HTTP `auth_method` injection"). So "this row injects a delegation token" carries no information about whether the upstream needs a platform bearer key, and the predicate reads one as the other.

The three divergent outcomes come from applying the predicate at three sites and not the fourth:

| Path | Site | `inject_delegation_token: true` + `auth_method: "bearer"` + credential |
|---|---|---|
| strict (`/proxy/{id}`, `/proxy/s/{slug}`, WS) | `proxy_service.rs:787` | `else { String::new() }` → empty bearer sent upstream, **no log, no error** |
| lenient (node-routed, MCP platform) | `proxy_service.rs:926` | same silent empty credential |
| server-chosen (assistant) | `proxy_service.rs:220-232` | `AppError::Internal("platform service does not require a master credential")` → **500** |
| auto-provisioned `UserService` | `proxy_service.rs:2055` | **no `master_credential_required` guard**; `authorize_master_credential` → `is_valid_master_credential_service` (`:245-246`) → `NotFound` → **404** |

The fourth row is the one that surprises: `finish_resolution` calls `authorize_master_credential` unconditionally after passing `is_public_internal_master_credential_service` (`:2049`), and that older predicate says nothing about delegation tokens. Folding `master_credential_required` into `is_valid_master_credential_service` therefore silently added a new denial to the streamlined `/keys` path that nobody guarded.

**Failure scenario.** An admin has a public internal catalog row — platform bearer key to the vendor API, `inject_delegation_token: true` so the vendor can call back into NyxID as the user. Users have auto-provisioned `UserService` rows against it. Deploy this commit: every one of those users gets `404 Service not found` on every call, with a `tracing::warn!` whose `reason` is `invalid_master_credential_service` — which points at the visibility/category clauses, not at the delegation flag that actually caused it. The same row addressed by UUID on the legacy path instead sends an empty bearer and returns the vendor's 401.

**Secondary case, same clause.** `forward_access_token: true` is more defensible — that path does overwrite `Authorization` (`proxy_service.rs:3151`, `request.bearer_auth(token)`) — but only when `caller_token` is `Some`. An API-key caller with no forwardable JWT previously fell back to the master credential and now gets an empty bearer.

**Fix shape (not implemented here):** the question the predicate is trying to answer is "will this request inject a master credential", and the honest form of that is `auth_method != "none"` and the row actually stores one — not "the row has no other identity feature". Drop the `inject_delegation_token` clause. Then apply whatever predicate survives at *all four* sites including `finish_resolution:2055`, so one row shape cannot produce three outcomes.

### B6 (carry-over, B4 not closed). The regression test named for the assistant shape never reaches the code it is named for

**Consequence:** the claim "an assistant-shaped row — delegation-token injecting, no stored credential — still resolves" is untested. The test passes for a reason unrelated to this commit, so it will keep passing if the rescoping is broken, reverted, or (as in B5) wrong.

**Mechanism.** `assistant_shaped_server_target_without_credential_resolves` (`proxy_service.rs:3444`) builds a row with `auth_method: "none"`, `inject_delegation_token: true`, `visibility: "private"`, `credential_encrypted: Vec::new()`, and calls `resolve_admin_proxy_target`. That function returns at `proxy_service.rs:649` — `if service.auth_method == "none" { return Ok(ProxyTarget { credential: String::new(), .. }) }` — which predates `fcc93df7` entirely. `authorize_master_credential_server_chosen` is never called.

The test proves this itself: the row is `visibility: "private"` with an empty credential. If it reached the gate, `authorize_master_credential_server_chosen` would reject it twice over (`:231` visibility, `:251` empty credential) and the `.expect("delegation-only assistant row should resolve")` would panic. It does not. Therefore the gate was not reached. (This is a code read plus the test's own assertions; I did not re-run the test against `fcc93df7` to confirm it passes there too, which would be the other way to show it.)

**Consequences beyond the test.** If `auth_method: "none"` is the real `aevatar` shape, the entire `master_credential_required` rescoping was unnecessary for the assistant — the early return already covered it — and it bought a new failure mode (B5) for no gain. If instead the real row is `auth_method: "bearer"` with a credential *and* `inject_delegation_token: true`, this commit takes it from cycle-1's 404 to a 500. Either way the prod readback from cycle 1 B4 is still the thing that resolves it, now with an extra column:

```js
db.downstream_services.find(
  { requires_user_credential: false, auth_method: { $ne: "none" } },
  { slug:1, visibility:1, service_category:1, provider_config_id:1, is_active:1,
    inject_delegation_token:1, forward_access_token:1,
    credLen: { $binarySize: "$credential_encrypted" } }
)
```

Any row returned with `inject_delegation_token: true` or `forward_access_token: true` is a B5 casualty. A test that actually covers this needs `auth_method != "none"`.

---

## Non-blocking findings

### N8. `TRAVEL_BOOKING.md:114` still claims type-system enforcement the code does not provide, and the doc was not touched

`git log -- docs/assistant/TRAVEL_BOOKING.md` shows `b786c8eb` as the last commit; `1b426418` changed only the two backend files. Line 114 still reads "**One gate in front of every master-credential decrypt**, enforced by the type system: a single authorization function is the only constructor of a decryptable catalog credential". After this commit that is closer to true but still overclaims: `EncryptionKeys::decrypt(&[u8])` (`crypto/aes.rs:372`) is unchanged and public, `decrypt_user_credential` (`proxy_service.rs:120-129`) is a live raw-bytes decrypt in the same file, and `handlers/services.rs:2033` still decrypts a catalog row's `credential_encrypted` outside the newtype. No lint or boundary check backs the property.

Line 193's "`EffectiveActor` has no default/system constructor — a synthetic actor cannot compile" **is** now true as stated for every module except `proxy_service` itself (see verified-correct below), so that one can stand. Line 114 should be reworded to what the code delivers: *the catalog master credential is unreachable outside `proxy_service`, and within it the only decrypt path for a catalog row goes through `AuthorizedMasterCredential`.* That sentence is defensible; the current one is not.

### N9. The new authorization test does not cover the predicate clause that just broke things

`master_credential_authorization_covers_visibility_and_consent` (`proxy_service.rs:3325`) covers visibility and the full consent lifecycle well, but exercises none of `master_credential_required`, `provider_config_id.is_some()`, `!is_active`, `service_type != "http"`, or the empty-credential clause. The first of those is exactly B5's blast radius: no test asserts what a `bearer` + `inject_delegation_token: true` row should do, in any of the four resolvers. Adding that one case would have caught B5 before it landed.

Coverage is also still function-level. `TRAVEL_BOOKING.md:193` asks for "private-row denial on UUID/slug/lenient/WS/MCP/server-chosen"; there is still no test that drives a private credentialed row through a resolver. The end-to-end machinery now exists in `proxy_resolution_integration_tests` (the redaction test builds a real service row, a real upstream, and calls `execute_admin_proxy`), so this is cheap to add.

### N10. N7 regressed: the gate is now completely uncallable from outside `proxy_service`

`EffectiveActor::from_user_id` (`proxy_service.rs:89-95`) is private. `authorize_master_credential` is `pub` but requires an `&EffectiveActor`, which no other module can construct — so the public function has no callable form outside its own module. `AuthorizedMasterCredential::decrypt` was made `pub` (`:108`), which is the half that no longer matters, since you cannot obtain the value to call it on.

This is the right default for sealing (and it is why N8's line-193 claim is now true), but it means `TRAVEL_BOOKING.md:221`'s requirement that `/api/v1/resource-tokens/exchange` "mint the Duffel component client key server-side via the credential gate" from a new handler module is still not expressible. PR-B will have to either add `pub fn EffectiveActor::for_auth_user(&AuthUser)` or host the exchange inside `proxy_service`. Worth deciding now rather than discovering it mid-PR-B.

### N11. N1 and N2 are unchanged and undocumented

`!service.credential_encrypted.is_empty()` (`proxy_service.rs:251`) is still a ciphertext-length test, satisfied by `encrypt(b"")` (`provider_service.rs:3578`, `handlers/services.rs:884`). Private rows with no `developer_app_ids` are still denied with no creator/admin escape hatch (`:169-177`). Both are defensible as deferrals — N1 is latent and N2 is what the design specifies — but neither is noted anywhere, so the next reader re-derives them. A comment on the predicate and a line in the PR body would close both.

### N12. The silent branch in B5 is the only unlogged outcome left

Every denial in `authorize_master_credential` and `authorize_master_credential_server_chosen` now emits a `warn`/`error` with `service_id`, `service_slug`, and a `reason` discriminator (`:156-161, :173-178, :189-194, :198-205, :221-227, :232-239`), which fully closes cycle 1's N6 for the gate itself. The one path that produces a wrong outcome with no log at all is the new `else { String::new() }` at `proxy_service.rs:798` / `:930` — the B5 silent-empty-credential case. If B5's predicate is kept in any form, that branch needs a `debug!` or `warn!` saying the row was resolved without its master credential and why.

### N13. The redaction test's capture is thread-local and would silently stop catching a moved log line

`upstream_error_body_is_redacted_end_to_end` (`handlers/proxy.rs:7004`) uses `tracing::subscriber::set_default`, which is thread-local, under a default `#[tokio::test]` (current-thread runtime) — correct today, because `log_upstream_error` is called inline in `execute_proxy_inner`. If that call were ever moved into a `tokio::spawn`ed task, the event would go to the global subscriber and the assertion would pass vacuously. A `with_default` over a `try_init`-free global, or an explicit assertion that *some* expected field was captured, would harden it. The test already asserts `output.contains("response_size")` and `output.contains("upstream-redaction-1")`, which mostly covers this — noting it only so the property is deliberate rather than incidental.

---

## Verified correct

- **B1 substantially closed — the desync is gone, not relocated.** Both resolvers now compute the credential in one `if / else if / else` chain (`proxy_service.rs:775-796`, `:917-932`) instead of two independently-editable booleans. More decisively: `service.credential_encrypted` no longer appears anywhere in the credential-resolution code. A repo grep over both changed files returns it only at `:210` and `:241` (inside the two authorize functions), `:251` and `:1840` (the two predicates), and test fixtures at `:3302, :3362, :3453` and `handlers/proxy.rs:7040`. There is no expression in either resolver that could be edited into an ungated catalog decrypt without first reaching into the gate functions themselves. The residual is `EncryptionKeys::decrypt` remaining public and `decrypt_user_credential` taking raw bytes — which is why N8 says the doc still overclaims, but the practical property the design wanted is now delivered inside this file.
- **B2 closed.** `EffectiveActor`'s field is private and `from_user_id` is a private associated function (`proxy_service.rs:85-95`). Struct-literal construction and construction-by-constructor are both impossible outside `services::proxy_service`; there is no `Default`, no `From`, no `pub` constructor, no test helper. The three construction sites (`:790`, `:928`, `:2058`) all pass a real caller id. `TRAVEL_BOOKING.md:193`'s claim is now accurate.
- **B3 closed — the test fails in both directions.** `master_credential_authorization_covers_visibility_and_consent` (`:3325`) asserts `Ok` for a public credentialed row and `Ok` for a private row *with* valid consent, and `Err` for: private without consent, expired consent, deleted consent, `developer_app_ids: None`, and `developer_app_ids: Some(vec![])`. A deny-everything regression fails the two `Ok` assertions; an allow-everything regression fails the five `Err` assertions. It also inserts a real `OauthClient` and a real `Consent` and mutates `expires_at` in Mongo rather than stubbing, so it exercises `load_valid_app_consents`'s actual query semantics. And it `panic!`s when Mongo is unavailable (`:3327`) instead of returning early, so it cannot silently no-op in CI. This is a genuinely good test.
- **N3 closed for every caller.** The 2 KiB outbound request-body preview is deleted from `forward_request_with_extra_outbound_headers` entirely — the `if url.contains("/responses")` block is gone from the diff with no replacement. Since every executor (REST, MCP, node direct-fallback, public/anonymous) funnels through that function, no caller retains it.
- **N4 closed — the redaction test is real.** `upstream_error_body_is_redacted_end_to_end` (`handlers/proxy.rs:7004`) stands up an actual `axum` server returning `422` with body `SENTINEL_PASSENGER_NAME` and header `x-request-id: upstream-redaction-1`, inserts a real credentialed catalog row, calls `execute_admin_proxy` end to end, and asserts the sentinel is absent from captured tracing while the request id and `response_size` are present. Reintroducing a `body = %preview` line beside `log_upstream_error` at `handlers/proxy.rs:2995` would fail it. Note the row it builds (`auth_method: "bearer"`, internal, public, real encrypted credential, `inject_delegation_token: false`) also exercises `authorize_master_credential_server_chosen`'s allow path for real — an unadvertised bonus.
- **N5 closed.** `log_upstream_error`'s `_response_body` parameter and the corresponding argument are removed (`handlers/proxy.rs:105-118`, `:2993-2999`); the function now cannot be handed a body at all.
- **N6 closed for the gate.** Every denial branch in both authorize functions logs with a distinct `reason` (see N12 for the one remaining unlogged outcome). The server-chosen variant logs at `error` and the caller-addressed one at `warn`, which is the right split.
- **No under-gating was introduced — I checked this specifically.** Enumerating all four sites that can produce a catalog credential: `:664` (server-chosen), `:787` (strict), `:926` (lenient), `:2055` (auto-provision). Every one either calls a gate function or produces `String::new()`. There is no code path in the new arrangement that yields a *non-empty* master credential without an `AuthorizedMasterCredential`, and the `else` branches produce an empty string rather than falling through to a raw decrypt. B5 is an availability regression, not a leak: when the predicate is wrong the credential is dropped, never injected unguarded.
- **`requires_user_credential` behavior is unchanged.** The connection-required check (`:739-743`), the missing-credential error text (`:781-784`), and the lenient `None → ("", false)` fallback (`:924`) all survive the restructure with identical semantics.
- **Denial error shapes unchanged where they were right.** `authorize_master_credential` still returns `NotFound("Service not found")` on every deny branch, so existence does not leak to a caller who named the row. The new `Internal` at `:227` is correct in kind for a server-chosen provisioning fault (it restores the convention documented at `:606-607` that cycle 1's N-note flagged) — it is just being reached by the wrong rows, which is B5.
- **Tests genuinely executed.** `cargo test -p nyxid -- master_credential_authorization_covers_visibility_and_consent assistant_shaped_server_target_without_credential_resolves upstream_error_body_is_redacted_end_to_end server_chosen_master_credential_requires_public_valid_row --nocapture` → `4 passed; 0 failed`, 1.12s. Two of the four abort without Mongo, so their passing confirms `:27018` was live.

---

## Could not verify

1. **The live `aevatar` row's `auth_method` and `inject_delegation_token` combination.** This is now the single question that decides whether B5 is theoretical or an outage, and whether the B4 rescoping was needed at all. Query in B6. Still requires a prod readback.
2. **Whether any live row combines a stored master credential with `inject_delegation_token: true` or `forward_access_token: true`.** Same query, same answer needed. The repo contains no seeded row with that shape (all 30 provider seeds are `auth_method: "none"` with `provider_config_id` set), so if any exist they were created through the admin UI and are invisible from here.
3. **The full verification gate.** Only the four targeted tests were run here. `cargo fmt`, `cargo clippy --workspace --all-targets -- -D warnings`, the boundary script, and the full suite are running separately. One thing to watch in the clippy leg that this commit did not change: `authorize_master_credential_server_chosen` still takes an unused `_db` and is `async` with no `.await` (`:216-218`) — inert under the default lint set.
4. **Whether the restructured resolvers changed behavior for any row shape I did not enumerate.** I compared the old and new credential blocks clause by clause for `requires_user_credential` true/false and `auth_method` none/non-none, which is the full input space of those branches. I did not run the broader proxy resolution suite; that is what the separate gate run covers.

---
---

# Cycle 3 — re-review of `f086a726` "fix(proxy): preserve master credentials with delegated identity"

+121 / −3 in `backend/src/services/proxy_service.rs` only, on top of `1b426418`. Three lines of production change (the predicate and one call to it), the rest tests.

Five targeted tests run and confirmed executed against Mongo on `:27018`: **5 passed, 0 failed, 0.65s** — `delegated_identity_does_not_suppress_master_credential_across_resolvers`, `assistant_shaped_server_target_without_credential_resolves`, `master_credential_authorization_covers_visibility_and_consent`, `upstream_error_body_is_redacted_end_to_end`, `server_chosen_master_credential_requires_public_valid_row`. Three of the five `panic!` rather than skip when Mongo is absent, so their passing proves the DB was live. The full gate is running separately.

---

## Verdict

**SHIP.**

B5 is closed correctly and at every call site, and the fix is verified by a test that could not pass against the old predicate. No blocking findings. Nothing from cycle 2 regressed. I looked specifically for new under-gating created by broadening the predicate and found none: broadening `master_credential_required` moves rows *into* the gated set, never out of it, and after this change the ungated branch in each resolver is provably unreachable.

The remaining items are two maintenance notes and three previously-flagged deferrals, none of which is a path to a wrong outcome. There is one deployment verification I still cannot do from here — unchanged from cycle 1 and not a code defect.

---

## Blocking findings

None.

---

## Non-blocking findings

### N14. The twin predicate in `unified_key_service.rs` was not updated, so the two copies are now written differently while meaning the same thing

**Consequence today:** none — both evaluate identically. **The hazard:** `proxy_service.rs:1832-1841` now reads `&& master_credential_required(service)` while its copy at `unified_key_service.rs:211-221` still spells out `&& service.auth_method != "none"`. The `unified_key_service` copy drives auto-provision *creation* and `reconcile_stale_auto_provisions`, which **deletes** users' auto-provisioned `UserService` rows when the predicate stops holding. If `master_credential_required` is ever narrowed again — which is exactly what cycle 2 did — proxy-time resolution and the reconcile sweep would silently disagree, and the sweep would delete rows the proxy still honors (or leave rows the proxy now refuses). Either extract one shared function or add a comment on both copies naming the other.

### N15. Two defensive branches are now dead code

`authorize_master_credential_server_chosen`'s `!master_credential_required(service)` → `AppError::Internal` (`proxy_service.rs:220-232`) is unreachable: its only caller is `proxy_service.rs:664`, which is preceded by the `auth_method == "none"` early return at `:649`. Same for the `else { String::new() }` arms at `:798` and `:930`, each preceded by the early returns at `:755` and `:875`. All three are fail-closed or fail-safe, so this is inert — worth noting only so nobody reads them as live behavior, and because they are the right shape to keep if the early returns are ever removed.

### N16. N1, N2 and N7 are untouched and still undocumented

This commit did not address them and did not record a deferral:

- **N1** — `!service.credential_encrypted.is_empty()` (`proxy_service.rs:251`) is a ciphertext-length test satisfied by `encrypt(b"")` (`provider_service.rs:3578`, `handlers/services.rs:884`), so it does not mean "this row holds a secret". Latent, no current wrong outcome.
- **N2** — a private row with no `developer_app_ids` is denied with no creator/admin escape hatch (`proxy_service.rs:169-177`). This is what the design specifies; it just is not written down anywhere a future operator would find it.
- **N7/N10** — `EffectiveActor::from_user_id` is private (`proxy_service.rs:89-95`), so `authorize_master_credential` has no callable form outside `proxy_service`. That is correct sealing for PR-A. It does mean `TRAVEL_BOOKING.md:221`'s plan for `/api/v1/resource-tokens/exchange` to reach the credential gate from a new handler module is not expressible as written — a PR-B design decision (add `EffectiveActor::for_auth_user`, or host the exchange inside `proxy_service`), not a PR-A defect.

### N17. `TRAVEL_BOOKING.md:114` still overclaims type-system enforcement

Unchanged from cycle 2's N8: the doc was not modified in this commit either (`git log -- docs/assistant/TRAVEL_BOOKING.md` still ends at `b786c8eb`). Line 193 is accurate. Line 114's "enforced by the type system: a single authorization function is the only constructor of a decryptable catalog credential" remains stronger than the code, because `EncryptionKeys::decrypt` is public and `handlers/services.rs:2033` still decrypts a catalog row outside the newtype. The accurate version of that sentence is in N8. Documentation accuracy, not a code defect — but it is the sentence a future reader will trust.

---

## Verified correct

### B5 closed — all four gate call sites agree, and the row shape is identical through every one

The predicate is now `service.auth_method != "none"` (`proxy_service.rs:143-147`), with a comment recording *why* identity propagation is additive. Enumerating every site that can produce a catalog master credential:

| Path | Early return | Gate | Behavior for bearer + `inject_delegation_token: true` + credential |
|---|---|---|---|
| server-chosen (assistant) | `:649` | `:664` `authorize_master_credential_server_chosen` | authorizes, injects credential |
| strict (`/proxy/{id}`, `/proxy/s/{slug}`, WS, `_nyxid_via`) | `:755` | `:787-793` `authorize_master_credential` | authorizes, injects credential |
| lenient (node-routed, MCP platform) | `:875` | `:926-932` `authorize_master_credential` | authorizes, injects credential |
| auto-provisioned `UserService` | — | `:2048` predicate → `:2054` `authorize_master_credential` | authorizes, injects credential |

The fourth row is the one that diverged in cycle 2. It is fixed indirectly but correctly: `is_public_internal_master_credential_service` (`:1835`) now delegates to `master_credential_required` instead of duplicating `auth_method != "none"`, so the guard that precedes the unguarded gate call at `:2054` and the gate's own `is_valid_master_credential_service` (`:246`) can no longer disagree about the delegation flag. Cycle 2's three divergent symptoms — silent empty bearer / 500 / 404 — collapse to one behavior.

This is not just my reading. `delegated_identity_does_not_suppress_master_credential_across_resolvers` (`:3485`) builds one row — `auth_method: "bearer"`, `inject_delegation_token: true`, real encrypted credential `"catalog-secret"`, public, internal, `provider_config_id: None` — and drives it through `resolve_admin_proxy_target`, `resolve_proxy_target`, `resolve_proxy_target_lenient`, and `resolve_proxy_target_from_user_service`, asserting `credential == "catalog-secret"` and `inject_delegation_token` on each. The auto-provision leg additionally asserts `auto_resolution.master_credential` (`:3581`), which is set only inside the `finish_resolution` auto-provision branch — so the test provably reached the fourth site rather than an easier one. That is exactly the cross-site agreement check, automated.

### The B5 regression test genuinely discriminates against the old predicate

`delegated_identity_does_not_suppress_master_credential_across_resolvers` asserts `master_credential_required(&service)` at `:3512` on a row with `inject_delegation_token = true`. Under cycle 2's predicate that expression evaluated to `true && !true && !false` = **false**, so the assertion fails outright — before any resolver runs. The `.expect("server-chosen resolver should retain the master credential")` is a second independent failure point, since cycle 2 returned `AppError::Internal` there. The test cannot pass against the old code.

I did not run the counterfactual by reverting the predicate locally, deliberately: a full gate run is in flight and mutating the tree mid-run would corrupt those results. The proof above is a direct read of the two predicates against the test's own assertion and is checkable without running anything.

### `assistant_shaped_server_target_without_credential_resolves` now reaches the gate

Cycle 2's version stopped at the `auth_method == "none"` early return and never touched the new code. It now has a second half (`:3469-3481`): it flips the stored row to `auth_method: "bearer"` in Mongo and asserts `resolve_admin_proxy_target` errors, which does reach `authorize_master_credential_server_chosen`. It also pins the intended semantics of the first half with `assert!(!master_credential_required(&service))` at `:3456`.

One honest caveat: neither of those two assertions *discriminates* between the old and new predicates (both evaluate the same way for `auth_method: "none"`, and the second half errors under both, just with different error kinds). This test is now a correct behavioral pin rather than the B5 regression test — `delegated_identity_does_not_suppress_master_credential_across_resolvers` is the one that does the discriminating work, and it does it properly.

### No new under-gating — broadening only added rows to the gated set

The direction holds, and it now holds more strongly than before. `master_credential_required` returning `true` for strictly more rows means strictly more rows route through authorization. Concretely, after this change the ungated `else { String::new() }` arms at `:798` and `:930` are unreachable (N15), so **every** row with `requires_user_credential == false` and `auth_method != "none"` goes through `authorize_master_credential` on the caller-addressed paths — there is no residual shape that skips it.

The census confirms it from the other direction: `service.credential_encrypted` / `catalog_service.credential_encrypted` appear in non-test code at exactly five places — `:210` and `:241` (inside the two authorize functions), and `:251`, `:1840` (predicates). Every producer of a catalog credential value is a gate function. Nothing else in `proxy_service.rs` or `handlers/proxy.rs` can name the ciphertext.

Broadening also cannot re-open the original exposure: it restores the pre-PR baseline on the `auth_method` axis only. The narrowing clauses that close the hole — `visibility == "public"` on the server-chosen path, consent for private rows, internal category, no `provider_config_id`, non-empty credential, active, http — are all untouched (`:245-252`).

### Nothing from cycle 2 regressed

Verified by direct read, not inference — the commit touches one file, but I checked each item rather than trusting the diffstat:

- `EffectiveActor` still has a private field and a private `from_user_id` (`:85-96`); no `Default`, no `From`, no public constructor. Sealed.
- The typed decrypt split is intact: `AuthorizedMasterCredential::decrypt` (`:108`), `decrypt_user_credential` (`:120`), `decrypt_master_credential_string` (`:131`). The single `if / else if / else` credential chain in both resolvers is unchanged, so the cycle-1 two-boolean desync stays gone.
- The 2 KiB outbound request-body preview is still absent — `rg "url.contains"` over `proxy_service.rs` returns nothing.
- `log_upstream_error` still takes no body parameter, and `upstream_error_body_is_redacted_end_to_end` still passes end to end against a stubbed 422 carrying `SENTINEL_PASSENGER_NAME` (`handlers/proxy.rs` untouched this cycle).
- All denial branches in both authorize functions still log a `reason` discriminator.

### Consent semantics unchanged and still correct

`master_credential_authorization_covers_visibility_and_consent` (`:3325`) is untouched and still passes: allow for public, allow for private-with-consent, deny for private-without-consent / expired / revoked / `None` app ids / empty app ids. Bidirectional, Mongo-backed, `panic!`s rather than skipping.

---

## Could not verify

1. **The live `aevatar` row's shape, and whether any live row with `auth_method != "none"` fails the narrowing clauses.** Unchanged from cycle 1 and cycle 2, and no longer about the delegation flag — B5's fix removed that axis entirely. What remains is the intended security narrowing: a server-chosen or caller-addressed row that carries a credential must be `public` + `internal` + `provider_config_id: null` + non-empty credential + active + http. This is a deployment check, not a code defect, and I would run it as a post-deploy readback rather than a merge gate now that the code is verified for every shape:

   ```js
   db.downstream_services.find(
     { requires_user_credential: false, auth_method: { $ne: "none" } },
     { slug:1, visibility:1, service_category:1, provider_config_id:1,
       is_active:1, credLen: { $binarySize: "$credential_encrypted" } }
   )
   ```

   Every row returned must satisfy all six clauses or it will 404. If `aevatar` is `auth_method: "none"` — the shape Sol's test models, and consistent with a delegation-token-only integration — it never reaches the gate and is unaffected.

2. **The full verification gate.** Only the five targeted tests were run here. `cargo fmt`, `cargo clippy --workspace --all-targets -- -D warnings`, `scripts/check-rci-backend-boundary.sh`, `cargo build`, and the full `nextest` run are yours. Two inert things clippy might notice, neither introduced by this commit: `authorize_master_credential_server_chosen` takes an unused `_db` and is `async` with no `.await` (`:216-218`), and N15's dead branches are dead by call-graph rather than by type, so `dead_code` will not fire on them.

---

## Certified

Concretely, what I am signing off on:

- Every path in the tree that decrypts a catalog row's master credential routes through `authorize_master_credential` or `authorize_master_credential_server_chosen`: strict, lenient, server-chosen, auto-provisioned, `_nyxid_via`, WebSocket, node-routed, LLM gateway, and the MCP executor (which resolves through the same functions rather than around them).
- No row shape injects a master credential without authorization. The ciphertext is namable only inside the gate functions and the predicates.
- Private credentialed rows require a valid, unexpired, unrevoked consent to an active OAuth client for one of the row's `developer_app_ids`; absent or empty `developer_app_ids` denies; DB errors deny; denials are `NotFound`-shaped so existence does not leak; every denial logs a `reason`.
- The server-chosen surface can never serve a private row, and no synthetic actor can be constructed outside `proxy_service` to pretend otherwise.
- A row combining a platform credential with `inject_delegation_token` or `forward_access_token` resolves identically through all four gate call sites, with the credential preserved.
- No response body, and no request body, is written to tracing from the proxy path — verified end to end with a sentinel through a real stubbed 422, in the buffered branch, with the streaming and node branches read and confirmed to log no bytes.
- The five tests above executed against a live Mongo and pass; the B5 regression test cannot pass against the pre-`f086a726` predicate.

---
---

# Cycle 4 — post-rebase re-census against `origin/main` (`62add35b`)

The branch was created from a stale `21297220` and has been rebased onto current `origin/main`. Confirmed: `travel-booking` is **0 behind, 10 ahead** of `62add35b`, linear history. Mainline changed the two files this PR touches by **1,464 insertions / 670 deletions** — the typed NyxIdChat migration, per-class LLM token metering, node-routed header work, and the direct-chat engine — so cycle 1's decrypt-site census was performed against a file that no longer exists.

I rebuilt the census from scratch against the current tree and then compared to Sol's. **They agree.** Below is my derivation, not an audit of theirs.

Five targeted tests run and confirmed executed: **5 passed, 0 failed, 1.72s** — see the environment note, which cost me one failed run and will affect your full-suite run too.

---

## Verdict

**SHIP.**

Mainline introduced no new read, decrypt, or plaintext-producer of a catalog master credential. The four gated branches remain the complete set. Every property certified in cycle 3 survived the rebase intact — I checked each one against the current file rather than inferring from the diffstat. No body logging was reintroduced. No blocking findings.

Two things worth your attention that are not defects: `4410e7c5` adds no test despite its `test(proxy):` title (it is a rebase compile fix, and the property is pinned by the existing cycle-2/3 tests), and mainline added a **second** platform row that now flows through the server-chosen gate, which extends the pre-merge readback from one slug to two.

---

## Blocking findings

None.

---

## Non-blocking findings

### N18. `4410e7c5` adds no test; its title says it does

**Consequence:** the commit log claims a verification artifact that is not in the diff, so a future reader looking for "the test that verifies the gate on current main" will not find one there.

The production half of `4410e7c5` is a **rebase compile fix**, not a test: four new `OauthClient` fields (`connection_webhook_url`, `connection_webhook_secret_encrypted`, `connection_webhook_key_id`, `connection_webhook_enabled`) added to the fixture in `master_credential_authorization_covers_visibility_and_consent`, and one new `None` argument for the `connection_expiry_notifier: Option<&ConnectionExpiryNotifier>` parameter mainline added to `resolve_proxy_target_from_user_service` (`proxy_service.rs:1128`). The other half is the census appended to `PR_A_VERIFICATION.md`.

That is the right work — the property is pinned by the *existing* cycle-2 and cycle-3 tests, which is stronger than a fresh shape assertion would have been, and keeping them compiling against the new base is exactly what was needed. The title should say `fix(test):` or `chore:` and the body should say the census is the deliverable. Answering the question directly: **it is not a shape assertion, because it is not an assertion at all** — the pinning is done by `delegated_identity_does_not_suppress_master_credential_across_resolvers` and `master_credential_authorization_covers_visibility_and_consent`, both of which I re-ran against the current base.

I also checked the new parameter is inert for this PR: `connection_expiry_notifier` threads through to `maybe_refresh_provider_backed_api_key` (`:2243`) and the agent-binding override path (`:2506`), both `UserApiKey` refresh concerns downstream of the master-credential branch at `:2173`. It cannot reach or bypass the gate.

### N19. Mainline added a second platform row that flows through the server-chosen gate

**Consequence:** the pre-merge readback I have been carrying since cycle 1 now covers two slugs, not one. This is a deployment check, not a code defect.

`execute_admin_proxy` — and therefore `resolve_admin_proxy_target` and the server-chosen gate — has a production caller from the direct Chrono-LLM surface at `handlers/assistant_direct.rs:170`. It targets slug `chrono-llm-public` (`services/assistant_direct.rs:6`) via `assistant_service::resolve_admin_service_by_slug`. That row is not seeded in code (no hits in `provider_service.rs`, `catalog_service.rs`, or `db.rs`), so it is admin-created per environment.

If `chrono-llm-public` carries `auth_method != "none"`, it must satisfy the same six clauses as any other server-chosen row: `public` + `internal` + `provider_config_id: null` + non-empty credential + active + http. Both test fixtures (`handlers/assistant_direct.rs:308-316`, `billing_integration_tests.rs:315-321`) build it from `dummy_service()` and leave `auth_method: "none"`, so in tests it takes the early return at `proxy_service.rs:736` and never reaches the gate — the same shape as the assistant row. Whether production matches is the readback below.

### N20. N14 unchanged — the twin predicate still diverges in form

`proxy_service.rs:1949-1959` delegates to `master_credential_required`; its copy at `unified_key_service.rs:190-200` still spells out `service.auth_method != "none"`. Identical today. The hazard is unchanged from cycle 3: the `unified_key_service` copy drives auto-provision creation and the reconcile sweep that *deletes* users' auto-provisioned rows, so a future edit to `master_credential_required` would silently desync provisioning from proxy-time resolution.

### N21. N1, N2, N7 and the `TRAVEL_BOOKING.md:114` wording remain as recorded

All four are tracked in `PR_A_VERIFICATION.md`'s deferred list and none is a path to a wrong outcome. N7 (the sealed `EffectiveActor` constructor blocking PR-B's `/resource-tokens/exchange` as designed) is the one that needs a decision before PR-B starts, not before this merges.

---

## Verified correct

### 1. Mainline added no new credential-producing path — independent census

Every non-model occurrence of `credential_encrypted` in `backend/src`, classified by the type it is read from. **Six** are reads of a `DownstreamService` (catalog) credential:

| Site | Kind |
|---|---|
| `proxy_service.rs:211` | inside `authorize_master_credential` — authorization constructor |
| `proxy_service.rs:242` | inside `authorize_master_credential_server_chosen` — authorization constructor |
| `proxy_service.rs:252` | `is_valid_master_credential_service` — ciphertext-presence check |
| `proxy_service.rs:1957` | `is_public_internal_master_credential_service` — ciphertext-presence check |
| `unified_key_service.rs:198` | twin predicate — ciphertext-presence check |
| `handlers/services.rs:2090` | admin/creator-gated OIDC client-secret endpoint, `auth_method == "oidc"` only |

Everything else resolves to a `UserApiKey` or `UserServiceConnection`: `credential_push_service.rs:649`, `connection_service.rs:158`, `user_api_key_service.rs:81`, `keys.rs:684`, `gcp_sa_service.rs:239`, `proxy_service.rs:864/866/873/1006/2660`. I re-checked `assistant_readiness_service.rs:533`, which reads `connection.credential_encrypted` — a `UserServiceConnection` presence test, not catalog.

**All 30 backend files mainline added** were checked individually. Exactly one contains the string `credential_encrypted` at all: `connection_expiry_service.rs:352`, a `credential_encrypted: None` struct initializer on a `UserServiceConnection`. Their decrypt calls are webhook secrets (`developer_webhook_service.rs:165,506,523`) and trigger envelopes (`trigger_service.rs:411,731,838,1102,1458`) — neither is a catalog credential. The direct-chat engine, the durable-operation grant service, connect links, triggers, and catalog identity reconciliation add **zero** catalog credential reads.

I also checked the metering work specifically, since #1393 touched credential classification: `final_credential_class` (`handlers/proxy.rs:3571-3601`) inspects `target.credential.is_empty()` and boolean flags only. It classifies; it never reads ciphertext.

### 2. The four gated branches are still the complete set of catalog-master plaintext producers

Derived from the decrypt census rather than assumed. Every decrypt in `proxy_service.rs`:

- `:110` `AuthorizedMasterCredential::decrypt` — requires the newtype; field and constructor private
- `:118` `decrypt_authorized_master_credential` — requires the newtype
- `:125` `decrypt_user_credential` — raw bytes, `UserServiceConnection`
- `:137` `decrypt_master_credential_string` — requires the newtype
- `:2667` `resolve_agent_credential_override` — `UserApiKey`

Master **plaintext** is therefore produced at exactly four sites, each immediately preceded by an authorize call: `:753` (server-chosen, gate at `:751`), `:881` (strict, gate at `:875`), `:1021` (lenient, gate at `:1014`), `:2180` (auto-provision, gate at `:2173`). No fifth.

Cross-checked from the consumer side: every non-test `ProxyTarget` construction that carries a non-empty credential is one of those four (`:759`, `:892`, `:1035`, `:2196`), two `UserApiKey` branches (`:2275`, `:2327`), or `llm_gateway.rs:852`, which moves an already-resolved `target.credential` into a base-URL-overridden copy. `public_proxy.rs:152` and `mcp_service.rs:6973` construct empty credentials — and `mcp_service.rs:6973` is inside `#[cfg(test)]` (mod opens at `:3956`).

Entry-point convergence re-confirmed post-rebase: REST UUID/slug, `_nyxid_via`, WS, and node fallback at `handlers/proxy.rs:647, 708, 869, 930, 1195, 1218`; server-chosen at `:1626`; both LLM gateway paths at `llm_gateway.rs:292, 328, 653, 726`; MCP at `mcp_service.rs:3084, 3226, 3234`; assistant surfaces at `handlers/assistant.rs:693` and `handlers/assistant_direct.rs:170`.

### 3. Nothing certified in cycle 3 was dropped or weakened by the rebase

Checked against the current file, item by item:

- **Sealed `EffectiveActor`** — private field, private `from_user_id`, no `Default`/`From` (`proxy_service.rs:86-96`). Intact.
- **Typed decrypt split** — `:110`, `:114`, `:121`, `:132` all present with the same signatures. Intact.
- **`master_credential_required` at every call site** — five production sites: `:221` (server-chosen gate), `:247` (`is_valid_master_credential_service`), `:874` (strict), `:1013` (lenient), `:1952` (`is_public_internal_master_credential_service`, which is what covers `finish_resolution`'s otherwise-unguarded call). Same five as cycle 3. The predicate body is still exactly `service.auth_method != "none"` (`:144-147`) with the explanatory comment.
- **Four gated branches with their early returns** — early returns at `:736`, `:842`, `:962`; gates at `:751`, `:875`, `:1014`, `:2173`. Same structure, same ordering.
- **`log_upstream_error` takes no body** — `handlers/proxy.rs:117-123`, four parameters: `service_id`, `status`, `response_size`, `upstream_request_id`. Intact.
- **End-to-end redaction test** — `upstream_error_body_is_redacted_end_to_end` at `handlers/proxy.rs:7544`, sentinel at `:7554`, negative assertion at `:7621`. Re-run and passing.

No hunk went missing in the conflict resolution.

### 4. No body logging reintroduced by mainline

Exactly two `from_utf8_lossy` occurrences remain in the two files. `handlers/proxy.rs:3189` feeds an SSE parse buffer consumed by `parse_sse_event` at `:3190` — parsed, never logged. `handlers/proxy.rs:7679` is the `echo_node_request` test helper inside `#[cfg(test)]` (mod opens at `:7434`), which echoes a request body back as JSON to the test.

The `/responses` request-body preview this PR deleted has not returned — `rg "url.contains"` over `proxy_service.rs` is empty. `response_body` in `handlers/proxy.rs` is used only for `.len()` (`:3448`, `:3465`), a JSON parse for usage extraction (`:3456`), and passthrough to the client (`:3483`). I also swept every `tracing::` macro in both files for a field carrying a credential, body, payload, or preview value: the only matches are message strings and field *names* in the gate's denial logs, never a value.

### 5. Consent, gate, and redaction behavior verified by execution on the current base

`cargo test -p nyxid -- delegated_identity_does_not_suppress_master_credential_across_resolvers assistant_shaped_server_target_without_credential_resolves master_credential_authorization_covers_visibility_and_consent upstream_error_body_is_redacted_end_to_end server_chosen_master_credential_requires_public_valid_row` → **5 passed, 0 failed, 1.72s**, against Mongo `rs0` on `:27018`. Three of the five `panic!` rather than skip when the DB is unreachable, so their passing proves the run was DB-backed.

---

## Environment note — this will affect your full-suite run

My first run of these tests failed four of five with `MongoDB is required for ...`. **It is not the topology, and it is not a code defect.** The replica set is healthy: `rs.conf()` reports `_id: rs0` with member `127.0.0.1:27018`, `isWritablePrimary: true`, and an unauthenticated connection reads and writes fine.

The cause is **credentials**. The test harness's first probe candidate is

```
mongodb://nyxid:nyxid_dev_password@127.0.0.1:27018/...?authSource=admin&directConnection=true
```

(`backend/src/test_utils.rs:181-192`), but the restored server runs with **no authentication** — `mongod --port 27018 --dbpath /tmp/nyxid-rs0 --replSet rs0 --bind_ip 127.0.0.1`, no `--auth`. SCRAM against a server with no such user returns `Authentication failed`, the probe rejects the candidate, falls through to `127.0.0.1:27017` (no listener), and returns `None` — at which point the strict tests panic and the older lenient ones silently skip.

Two ways out:

```bash
# either point the harness at the running server explicitly
export NYXID_TEST_DATABASE_URL="mongodb://127.0.0.1:27018/nyxid_test_probe?directConnection=true"

# or create the user the harness expects
mongosh "mongodb://127.0.0.1:27018/?directConnection=true" --eval \
  'db.getSiblingDB("admin").createUser({user:"nyxid",pwd:"nyxid_dev_password",roles:["root"]})'
```

I used the first. Note the harness *panics* when `NYXID_TEST_DATABASE_URL` is set but unreachable (`test_utils.rs:168-172`), so a typo there fails loudly rather than silently skipping — which is what you want.

---

## Could not verify

**The live shapes of `aevatar` and `chrono-llm-public`.** Unchanged in kind from cycles 1–3, now covering two slugs because of N19. Any row that reaches a gate must satisfy all six clauses or it will 404. Rows with `auth_method: "none"` never reach a gate and are unaffected.

```js
db.downstream_services.find(
  { requires_user_credential: false, auth_method: { $ne: "none" } },
  { slug:1, visibility:1, service_category:1, provider_config_id:1,
    is_active:1, service_type:1, credLen: { $binarySize: "$credential_encrypted" } }
)
```

Run it against production. Every returned row must be `visibility: "public"`, `service_category: "internal"`, `provider_config_id: null`, `is_active: true`, `service_type: "http"`, `credLen > 0`. Pay particular attention to `aevatar` and `chrono-llm-public`. I would run this as a post-deploy readback rather than a merge gate: the code is verified correct for every shape, and this only tells you whether any deployed row is misconfigured relative to the new — intended — narrowing.

The full CI suite is yours; only the five targeted tests were run here.

---

## Certified (cycle 4, against `origin/main` `62add35b`)

- The census was rebuilt independently against the current tree and matches: six catalog `credential_encrypted` reads, four gated plaintext producers, one newtype-guarded raw master decrypt.
- Mainline's 1,464 inserted lines across these two files, and all 30 files it added, introduce **no** new read, decrypt, or producer of a catalog master credential.
- The four gated branches remain the complete set; no fifth producer exists, verified from both the decrypt side and the `ProxyTarget` construction side.
- Every cycle-3 property survived the rebase: sealed `EffectiveActor`, typed decrypt split, `master_credential_required` at all five sites with its body unchanged, four gates behind their early returns, body-free `log_upstream_error`, end-to-end sentinel test.
- No response-body or request-body preview exists in either proxy file, and no `tracing` call in either file carries a credential, body, or payload value.
- The five gate and redaction tests execute and pass against the current base on a live replica set.
