# Rollup Health 1093 Root-Cause Evidence

This note records the durable source-level evidence for issue #1093. It follows
the repository's rollup-health practice: keep the repair scoped to the failing
check evidence, bind the diagnosis to the rollup head, and document why the code
change is the smallest root-cause fix.

## Rollup Signal

- Rollup PR: #1090
- Rollup head: `f9cd3102eee6082706f97cf1c8c29dc30d41c75e`
- Failed checks reported by the watchdog: `Backend Test`, `Coverage (Backend)`,
  and `CI Pipeline`
- Backend job command in `.github/workflows/ci.yml`: `cargo nextest run -p nyxid --profile ci`

The retained watchdog snapshot at
`/tmp/fkst-github-devloop-rollup-health-ChronoAIProject-NyxID-pr-1090-f9cd3102eee6082706f97cf1c8c29dc30d41c75e.json`
binds the failed `Backend Test` status to rollup head
`f9cd3102eee6082706f97cf1c8c29dc30d41c75e`; it records
`Backend Test: COMPLETED/FAILURE; Coverage (Backend): COMPLETED/FAILURE; CI Pipeline: COMPLETED/FAILURE`.
No GitHub state was fetched or edited during this repair round.

## Failing-Run Evidence

To bind the source-level diagnosis to the failing backend gate, the exact
rollup head was checked out in a detached worktree and the backend job's
`nextest` command was run with the narrowed failing-test filter:

```text
cargo nextest run -p nyxid --profile ci handlers::oauth::tests::authorize_inner_threads_stored_service_consent_into_code
```

On `f9cd3102eee6082706f97cf1c8c29dc30d41c75e`, with MongoDB 8.0 reachable on
the CI-style `127.0.0.1:27017` port, that command produced:

```text
FAIL nyxid::bin/nyxid-server handlers::oauth::tests::authorize_inner_threads_stored_service_consent_into_code
thread 'handlers::oauth::tests::authorize_inner_threads_stored_service_consent_into_code' panicked at backend/src/handlers/oauth.rs:2826:9:
assertion failed: !stored.allow_all_services
Summary: 1 test run: 0 passed, 1 failed, 4505 skipped
```

The fixed PR head passes the same filtered backend command:

```text
PASS nyxid::bin/nyxid-server handlers::oauth::tests::authorize_inner_threads_stored_service_consent_into_code
Summary: 1 test run: 1 passed, 4505 skipped
```

## Root Cause

The failing backend surface is the existing regression test
`handlers::oauth::tests::authorize_inner_threads_stored_service_consent_into_code`
in `backend/src/handlers/oauth.rs`. The test grants restricted OAuth consent
with `allowed_service_ids = ["svc-allowed"]`, issues an authorization code, and
asserts that the stored `AuthorizationCode` has:

- `allow_all_services == false`
- `allowed_service_ids == ["svc-allowed"]`

At the rollup head, `issue_authorization_code` computed `service_restricted =
true` whenever requested resources or stored consent narrowed the grant, but it
passed that value directly to `oauth_service::create_authorization_code` as the
stored `allow_all_services` argument. That inverted the persisted contract:
restricted grants were stored as allow-all grants.

The one-line OAuth fix passes `!service_restricted`, so the stored
`allow_all_services` value matches the authorization-code model and the audit
event's `allow_all_services` field.

## Smallest Fix Boundary

The repair intentionally stays inside `backend/src/handlers/oauth.rs` because
the failing contract is local to authorization-code issuance. No endpoint,
schema, limit, token format, or CI configuration is changed.

## Recurrence Waiver

#1093 is handled as a one-off backend root-cause fix rather than a class-level
rollup-health change because the failing signal maps to a concrete existing
backend regression test and a single local field-contract inversion. Prior
same-class rollup-health issues remain covered by the rollup-health runbook in
`CONTRIBUTING.md`; this PR does not need a broader guardrail to restore the
named `Backend Test` failure.
