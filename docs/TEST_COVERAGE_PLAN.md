# Test Coverage Plan — CLI + FE Wizard (W21)

Plan for the W21 test-coverage epic: GitHub issues **#782, #783, #784** (CLI commands),
**#787** (FE CLI wizard), and **#785** (CI coverage gates). FE web flows (#786) and the
BE hot-path (#788) are tracked separately.

Languages: CLI = **Rust** (`cargo nextest`), FE wizard = **TypeScript/React** (`vitest` +
`@testing-library/react`). Note: the `react-native-testing` skill does **not** apply to the
web wizard (#787 is React web, not React Native); the relevant new skill is `rust-testing`.

## Guiding principles ("best practice + within designed limits")

- **No production behavior changes.** Tests use seams the code already exposes.
- **CLI seam:** commands build their client via `ApiClient::from_auth(&auth)`, and
  `AuthArgs.base_url` (public) + `access_token` (public, overrides saved token) let a test
  point any command at a mock server with zero refactor. `ApiClient::new(base_url, token)`
  is also public for lower-level tests.
- **No trait abstraction over `ApiClient`.** We mock at the HTTP boundary (`wiremock`),
  which matches the existing architecture, instead of wrapping the client in a trait +
  `mockall` (that would be an architectural change, outside designed limits).
- **Applying the `rust-testing` skill:** integration-at-the-boundary over heavy mocking;
  `#[cfg(test)]` modules; `rstest`/`proptest` where parameterized/property tests help;
  no `sleep()` in tests (inject time); `cargo-llvm-cov` for the gate.
- **FE:** extend the existing idiom in `auth-flows.test.tsx` — pure business logic
  extracted into dependency-injected functions (see `auth-flow-polling.ts`), components
  mocked via `vi.mock("@/lib/api-client")`. No new test infra, no component behavior changes.
- The **only** code change permitted is extracting a small *behavior-preserving* pure
  helper when logic is otherwise unreachable (the move `service.rs` already made).

## Tooling

- **Added:** `wiremock = "0.6"` to `cli/Cargo.toml` `[dev-dependencies]` (test-only; vetted —
  LukeMathWalker, 52M downloads, MIT/Apache, mainstream deps already in the graph).
- **Shared helper:** `cli/src/test_support.rs` → `mock_auth(uri)` / `mock_auth_with_output(uri, fmt)`
  build an `AuthArgs` pointed at a `MockServer` with a token override (no `$HOME`/token-file
  dance needed, because `resolve_access_token` returns the explicit token first).
- Tests are **inline `#[cfg(test)] mod tests`** per command file (binary crate has no `lib.rs`,
  so external `tests/` can't reach crate internals).

### Reusable CLI pattern

```rust
let server = MockServer::start().await;
Mock::given(method("POST")).and(path("/api/v1/...")).and(body_json(json!({...})))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!({...})))
    .expect(1).mount(&server).await;
run(SomeCommand::Variant { /* fields */, auth: mock_auth(server.uri()) })
    .await
    .expect("ok");
// MockServer verifies `.expect(1)` on drop.
```

For commands with a wizard gate (`api-key create`/`rotate`), pass `terminal: true` to force
the scripted/headless path (byte-identical to pre-wizard CI behavior).

## Status (live tasks tracked in the harness task list)

| # | Task | Ticket | Status |
|---|------|--------|--------|
| 1 | Add & verify `wiremock` dev-dependency | infra | ✅ done |
| 2 | Shared CLI test helper (`mock_auth`) | infra | ✅ done |
| 3 | `endpoint.rs` command tests (reference) | #784 | ✅ done (5 tests) |
| 4 | `external_key.rs` + `catalog.rs` tests | #784 | ✅ done (11 tests) |
| 5 | `api_key.rs` command + binding tests | #784 | ✅ done (12 tests) |
| 6 | `service.rs` command-level tests | #784 | ⏳ next |
| 7 | `auth_flows.rs` + `mfa.rs` tests | #783 | pending |
| 8 | `oauth.rs` + `pairing.rs` tests | #783 | pending |
| 9 | `proxy.rs` + `mcp.rs` tests | #782 | pending |
| 10 | `approval.rs` + `notification.rs` tests | #782 | pending |
| 11 | `channel_bot.rs` + `channel_event.rs` tests | #782 | pending |
| 12 | FE wizard pure-function tests | #787 | pending |
| 13 | FE wizard component tests | #787 | pending |
| 14 | FE cli-pair pages + cli-auth page | #787 | pending |
| 15 | CI coverage gates | #785 | pending (last) |

**28 tests passing so far, zero production-code changes.**

## Per-ticket test cases

### #784 — key & service management (CLI)
- `endpoint`: list (json/table/empty), update body shape, delete (`--yes`), 5xx surfaces as error.
- `external_key`: list, rotate (credential body), empty-credential rejection, delete.
- `catalog`: list (default + `--all` query param), show `<slug>`, endpoints `<slug>`; pure
  `truncate_line`.
- `api_key`: create (+`--platform`, scripted via `terminal:true`), list, show, rotate, delete,
  update (only-changed-fields body), `bind` (3-request auto-resolve flow); ambiguous-name
  refusal in `find_key_by_name`; pure `array_from_response`.
- `service` (next): `add` (HTTP + SSH + `--org` ownership), list, show, update, delete —
  extend the existing 44 pure-helper tests with command-level I/O paths.

### #783 — auth & onboarding (CLI)
- `login` happy + bad-creds; MFA challenge TOTP success/failure/rate-limit.
- `mfa setup/confirm/disable`.
- `oauth` device-code: pending → approved → token saved (uses `HomeGuard` + `save_tokens`).
- `pairing` reserve → claim → consume + expired-code.

### #782 — runtime & integration (CLI)
- `proxy` HTTP (extend existing `proxy_request` tests in `api.rs`) + WS path.
- `mcp` session lifecycle.
- `approval` request → approve/deny/expire.
- `notification` register/list/remove device token.
- `channel-bot register` (telegram/lark/discord) + `update` (verification-token/encrypt-key).
- `channel-event` ingress + dedup.

### #787 — FE CLI wizard
- Pure units: `reserve-action.ts` (success/expired/already-consumed), `client.ts` (network
  failure/token expiry/terminal status), following the `auth-flow-polling.ts` DI idiom.
- Components (extend `auth-flows.test.tsx` pattern): `shell`, `step-label`, `wizard-footer`,
  `catalog-grid` (filter→select→propagate), `name-input`, `scope-picker` +
  `access-scope-card`, `confirm-panels` (submit happy + validation error),
  `disconnect-banner`, `display-once-panel`, and `upstream-error-banner` (new — folded into
  scope; predates the ticket's file list).
- Pages: `cli-pair/index.tsx`, `cli-pair/display-once.tsx` (shows secret once, blocks
  re-display), `pages/cli-auth.tsx` (helpers test already exists).

### #785 — CI coverage gates (do last)
- `cargo llvm-cov` for `nyxid` + `nyxid-cli`; `vitest run --coverage` (install
  `@vitest/coverage-v8`, add `test:coverage` script + `vite.config.ts` coverage block).
- PR comment with per-component line-% delta; initial thresholds (BE ≥40 / CLI ≥30 / FE ≥15)
  with a ratchet plan.

## Sequencing
Setup (1–2) → #784 (3–6) → #783 (7–8) → #782 (9–11) → #787 (12–14) → #785 gates (15).
