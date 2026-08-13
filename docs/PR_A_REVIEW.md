# PR-A adversarial review — `fcc93df7` "fix(proxy): gate catalog credentials and redact upstream bodies"

Reviewed against `docs/TRAVEL_BOOKING.md` §A.1 (contract) and Part I §9. Branch `travel-booking`, worktree `golden-badger`. 261 insertions across `backend/src/services/proxy_service.rs` and `backend/src/handlers/proxy.rs`.

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
