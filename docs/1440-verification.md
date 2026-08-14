# #1440 Verification

Recorded from real runs on branch `feat/1440-impl`. Commands and output are reproduced as executed.

## Environment

The backend suite needs a MongoDB **replica set** (transactions). Docker Desktop failed to stay up on this
machine, so verification ran against a local throwaway single-node replica set rather than `docker compose`:

```
mongod --dbpath /tmp/nyxid-test-rs --port 27019 --replSet rs0 --fork --logpath /tmp/nyxid-test-rs/mongod.log
mongosh --port 27019 --eval 'rs.initiate()'
export NYXID_TEST_DATABASE_URL="mongodb://localhost:27019/?replicaSet=rs0"
```

`db.hello().isWritablePrimary` returned `true` before the suite was started.

## Targeted suites

```
cargo test -p nyxid --bin nyxid-server handlers::delegation
test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 5264 filtered out

cargo test -p nyxid --bin nyxid-server handlers::exact_service_approvals
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 5270 filtered out
```

Both exited 0. Note the crate is a binary-only package (`-p nyxid --bin nyxid-server`); `--lib` fails with
`no library targets found`.

### What the delegation tests actually assert

`get_operation_catalog` requires a live database, so the authority decision was extracted into two pure
functions — `require_catalog_authority` (pre-DB eligibility, returns the token `jti`) and
`grant_matches_token` (the bound-field comparison). Handler behavior is unchanged; the rejection paths are
now assertable without Mongo.

| Test | Asserts |
|---|---|
| `every_non_delegated_auth_method_is_rejected_before_any_grant_lookup` | All five non-delegated `AuthMethod` variants → `Forbidden`; `Delegated` proceeds |
| `delegated_token_without_catalog_scope_is_rejected` | Proxy scope alone → `Forbidden` |
| `delegated_catalog_token_without_jti_cannot_bind_to_a_grant` | Missing `jti` → `Unauthorized` |
| `eligible_delegated_token_yields_its_own_jti` | Happy path returns the caller's own `jti` |
| `grant_agreeing_on_every_bound_field_is_accepted` | Fully matching grant accepted |
| `drift_in_any_bound_authority_field_fails_closed` | **Each of the 9 bound fields** mutated independently → rejected |
| `delegated_token_missing_client_identity_never_matches_a_grant` | Missing `acting_client_id` or `oauth_client_id` → rejected |
| `operation_catalog_response_is_typed_and_secret_free` | Serialized response contains no credential/token/secret keys |

## Full backend suite

```
cargo test -p nyxid --bin nyxid-server
test result: FAILED. 5273 passed; 1 failed; 0 ignored; 0 measured; finished in 335.36s
```

The single failure is **unrelated to this branch** and is a load-sensitive timing test:

```
handlers::devices::tests::approve_handler_returns_before_slow_notification_dispatch_finishes
panicked at backend/src/handlers/devices.rs:772:10:
approve should return without waiting for notification task: Elapsed(())
```

Classified as flake on two independent grounds:

1. `backend/src/handlers/devices.rs` is **not modified by this branch** (`git diff --name-only` has no match).
2. Re-run in isolation passes:
   ```
   cargo test -p nyxid --bin nyxid-server handlers::devices::tests::approve_handler_returns_before_slow_notification_dispatch_finishes
   test result: ok. 1 passed; 0 failed ... finished in 6.32s   (exit 0)
   ```

It asserts a wall-clock deadline and ran concurrently with a cargo build and a local `mongod`.

## Reachability check

`/api/v1/delegation/operation-catalog` is reachable by a token carrying only `mcp:catalog:read`:
`delegated_request_allowed` returns early via `is_delegated_native_path`, which matches first segment
`delegation`. The endpoint does **not** require `account:read` and is correctly absent from
`delegated_read_denied_path` (it delivers no secrets and is not execution-shaped).

The middleware already calls `catalog_delegation_service::validate_live_grant` for catalog-scoped delegated
tokens (`mw/auth.rs`). The handler's own grant lookup is an additional bind of all nine authority fields, not
a duplicate of that liveness check.

## Not covered

Stated explicitly rather than implied by silence:

- **No end-to-end HTTP test of the handler.** Driving `get_operation_catalog` needs a constructed `AppState`;
  the authority logic is covered by the pure-function matrix above, the response shape by the serialization
  test, but the wiring between them is covered only by CI and by review.
- **No adversarial review of the final diff.** Deliberately skipped at the owner's direction — a plan-stage
  adversarial review was already completed (`docs/1440-adversarial-review.md`).
- **No mutation-spy test proving the route performs zero writes.** The handler contains no write call, but
  that is asserted by reading, not by a test.
- `load_operation_catalog` may perform a hardened, cached, DNS-pinned outbound OpenAPI fetch, so the route is
  not a pure database read. See the plan's scope note.
