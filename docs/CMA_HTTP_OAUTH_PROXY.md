# CMA through the user OAuth HTTP proxy

A third-party web application requests `openid offline_access proxy` and the
single CMA proxy resource through its existing confidential OAuth client. It
keeps OAuth tokens encrypted on its backend and calls the fixed catalog proxy
route, selecting the exact connection with `_nyxid_via`. NyxID checks the live
service grant and connection access before signing the identity assertion.

Legacy credential-free automatic connections follow the current catalog
destination. The fallback requires a provider-free, non-node connection with no
attached credential and matching endpoint ownership and catalog identity.
Explicit custom destinations retain their routing.

A human OAuth caller may use an authorized organization connection. CMA still
receives the human actor's identity, separate from the connection owner. This
does not create organization-owned CMA Sandboxes or grant another person's
resources. Agent-key, guest and service-account eligibility is unchanged.

Binary `POST /api/v1/agents/{id}/attachments?name=<filename>` uses existing
owned-parent admission and accepts only one safe filename parameter. The exact
resolved destination, path, query, original bytes and Idempotency-Key remain
bound through the final send. Other mutation query strings stay prohibited.
CMA retains responsibility for renewable execution custody.

## Recover a confirmed creation

If creation completed but the application lost its response, replaying the
original key and body returns HTTP 409 with existing `error=conflict`, numeric
code 1004 and additive metadata:

```json
{
  "error": "conflict",
  "error_code": 1004,
  "message": "Conflict: cma_creation_already_completed: query the original resource",
  "details": {
    "reason": "cma_creation_already_completed",
    "operation_id": "op_original",
    "agent_id": "agt_original",
    "sandbox_id": "sb_original"
  }
}
```

Identifiers can be null when the creation kind has no corresponding receipt.
The original owner, service, method, path, body hash, key and deployment must
match. Changed requests cannot obtain the receipt. No creation is resent. The
application observes the actual lifecycle or resource to learn its current
status; the receipt does not claim the remote task succeeded.

The existing personal feature gate, Sandbox account slot, NyxID-created parent
requirement, capacity checks and retirement fences are preserved. Workers
created outside NyxID do not become mutable through repeated OAuth consent.

## Release verification

Run `assistant_cma_admission`,
`credential_free_auto_connection_resolves_current_catalog_destination` and
`service_dispatch_registry` regressions against a real MongoDB replica set.
Probe deployed personal and organization HTTP connections before enabling the
web application. Then validate creation, upload, two Responses turns and
lost-receipt recovery. Mock provider tests and health checks alone do not
establish CMA execution compatibility.
