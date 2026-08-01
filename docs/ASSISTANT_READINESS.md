# Assistant Readiness Contract

Related delivery: [nyxid-chat#5](https://github.com/eanz17/nyxid-chat/issues/5).

`GET /api/v1/assistant/readiness` is an authenticated, human-session read model.
It reports whether the verified `AuthUser` can use the closed assistant capability
registry; request parameters never select a user, organization, service, or scope.
The current response revision is `nyxid-assistant-readiness.v2`.

The v2 registry contains two required core capabilities and one optional
connector. Public capability identities remain stable even though core evidence
is backed by exact internal catalog slugs:

| `capabilityId` | backing slug | evidence | `required` | `requestedScopes` |
|---|---|---|---:|---|
| `model` | `chrono-llm-public` | platform configuration | `true` | none |
| `runtime` | `aevatar` | platform configuration | `true` | none |
| `api-github` | `api-github` | user connector | `false` | `repo` |

Core evidence is configuration-only and never performs a network probe or
manufactures an operation approval. An active HTTP public/internal platform row
that does not require a user credential and has an eligible master-credential
configuration is `connected`, `not_required`, and `available`. An absent or
inactive row is `missing`; a structurally unusable row is `cannot_use`; a
database/evidence failure is `cannot_check`.

For connectors, the producer derives connection and OAuth grant evidence
independently. Personal services take precedence over legacy personal state,
which takes precedence over an accessible organization service. The final
execution check uses the canonical MCP callability projection, so `available`
requires all of the following:

- `connectionState = connected`
- `grantState = granted | not_required`
- organization access is allowed
- the selected service is executable

Any absent catalog, credential, scope, or execution evidence yields
`cannot_check`; it is never converted into `missing`. A positively absent
connection yields `missing`. Expired, revoked, partial, denied, and otherwise
known unusable evidence yields `cannot_use`.

`status`, `connectionState`, and `grantState` are closed enums documented by the
checked-in consumer fixture at
`tests/fixtures/assistant/readiness-v2.json`. The immutable v1 fixture remains
available for consumers that explicitly test that historical revision.
`reasonCode` is a closed diagnostic code and is `null` only when
`status = available`.

`managementUrl` is built from `FRONTEND_URL` with its path replaced by `/keys`.
It is returned only when the configured URL is HTTPS, host-bearing, and contains
no userinfo; otherwise it is `null`. Query strings and fragments from configuration
are discarded.

Operation approvals are not a readiness authority. The route accepts only a
human session, whose proxy execution does not require an `ApprovalGrant`, and
the registry does not identify an operation. Consequently there is no applicable
approval request or decision identity to publish. Adding approval correlation in
a later revision requires an operation-bound authority and may expose only its
safe opaque identity, never an approval payload.
