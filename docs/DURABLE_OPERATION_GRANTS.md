# Durable Operation Grants

Durable operation grants authorize unattended scheduled writes without changing
the interactive `ApprovalGrant` contract. They follow least privilege and
complete mediation: authority is bound to one scheduled API key, one
`UserService`, one active published endpoint contract, bounded request values,
a finite lifetime, and finite usage quotas. NyxID revalidates all of those
properties at the proxy terminal on every invocation.

## Preview and provision

Create a JSON scope-plan request with exact `selected_service_ids`, a finite
`key_expires_at`, and one or more `selected_operations`. Each operation needs an
explicit `valid_from` so preview and confirmation are deterministic. Phase 1
accepts only endpoints explicitly classified as write with method `POST`,
`PUT`, or `PATCH`. Every path variable and required parameter must have an
`exact` or finite `one_of` constraint. JSON bodies must constrain the complete
body with the empty JSON Pointer or constrain every leaf; additional fields are
not supported.

```bash
nyxid api-key durable-plan --file durable-plan.json --output json
```

Review the returned endpoint IDs, methods, normalized paths, contract digests,
constraints, expiry, quotas, replay policies, exact service/node grants, and
owner. To provision, create a key request containing the unchanged
`selected_operations`, the returned `allowed_service_ids` and
`allowed_node_ids`, `allow_all_services: false`, `allow_all_nodes: false`, the
same `expires_at`, and the returned `normalized_grant_digest` as
`scope_plan_digest`.

```bash
nyxid api-key durable-create --file durable-create.json --yes --output json
```

NyxID recomputes the plan before mutation. Authorization, route, endpoint, or
contract drift returns a stale-plan conflict. Provisioning creates the
`scheduled_invocation` key behind a write-denied activation fence, stores all
grants, and only then enables scheduled writes. The raw key is returned once;
grant receipts contain no credential.

## Invoke

Use the scheduled key with its exact `UserService` route and provide a stable,
caller-generated operation ID. Reusing `(grant_id, operation_id)` never starts
a second downstream request.

```bash
NYXID_API_KEY='nyxid_ag_...' nyxid proxy request SERVICE_SLUG /bounded/path \
  --via-service USER_SERVICE_ID \
  --method POST \
  --data '{"exact":"authorized value"}' \
  -H 'Content-Type: application/json' \
  -H 'X-NyxID-Durable-Grant-Id: GRANT_ID' \
  -H 'X-NyxID-Operation-Id: SCHEDULE_RUN_AND_CALL_SITE_ID'
```

NyxID strips both authorization headers before forwarding. A caller-supplied
`Idempotency-Key` is also stripped. When the selected replay policy is
`downstream_idempotency_key` and the current endpoint metadata explicitly
supports it, NyxID forwards `Idempotency-Key` with the operation ID as its
value.

This is an at-most-once dispatch contract, not a universal exactly-once claim.
After a possible dispatch, transport failure is recorded as
`durable_operation_outcome_uncertain` and NyxID does not fail over to another
node. Do not retry a non-replayable write with a new operation ID. Reusing the
same ID returns the stored uncertain classification rather than dispatching.

## Not a code-execution credential

The bearer from a `scheduled_invocation` key is never forwarded downstream.
Its durable grant authorizes one exact published write body: request
constraints use `Exact` or `OneOf`, and an empty JSON Pointer binds the whole
body. That contract cannot authorize a multi-request `/executions` lifecycle
whose later calls depend on runtime-created identifiers and outputs.

Unattended code execution instead uses a `purpose = "general"` agent key with
an exact service allowlist plus downstream admission controls, or a refreshable
service delegation token. Those credentials cover the runtime session while
the downstream still enforces which execution operations are admissible; they
do not turn a durable one-write grant into a general bearer.

## Manage and renew

```bash
nyxid api-key durable-grants KEY_ID --include-revoked --output json
nyxid api-key durable-revoke KEY_ID GRANT_ID --yes --output json
nyxid api-key durable-reauthorize KEY_ID --file reauthorize.json --yes --output json
```

Revocation, expiry, total quota, window quota, endpoint deactivation, contract
drift, wrong key/grant identity, and constraint mismatch fail before downstream
dispatch. Reauthorization requires a freshly previewed v2 digest, inserts the
replacement grants only after revoking prior active grants. Only personal owners
and organization admins may list or mutate grant receipts.

Stable durable error codes are `9008` through `9016`: missing, mismatch,
expired, revoked, contract drift, quota exhausted, duplicate operation,
conflicting operation reuse, and outcome uncertain, respectively.
