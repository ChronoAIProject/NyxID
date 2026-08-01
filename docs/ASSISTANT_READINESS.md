# Assistant Readiness Contract

Related delivery: [nyxid-chat#5](https://github.com/eanz17/nyxid-chat/issues/5).

`GET /api/v1/assistant/readiness` is an authenticated, human-session read model.
It reports whether the verified `AuthUser` can use the closed assistant capability
registry; request parameters never select a user, organization, service, or scope.
The response revision is `nyxid-assistant-readiness.v1`.

The v1 registry contains one optional capability:

| `capabilityId` | `label` | `required` | `requestedScopes` |
|---|---|---:|---|
| `api-github` | GitHub | `false` | `repo` |

The producer derives connection and OAuth grant evidence independently. Personal
services take precedence over legacy personal state, which takes precedence over
an accessible organization service. The final execution check uses the canonical
MCP callability projection, so `available` requires all of the following:

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
`tests/fixtures/assistant/readiness-v1.json`. `reasonCode` is a closed diagnostic
code and is `null` only when `status = available`.

`managementUrl` is built from `FRONTEND_URL` with its path replaced by `/keys`.
It is returned only when the configured URL is HTTPS, host-bearing, and contains
no userinfo; otherwise it is `null`. Query strings and fragments from configuration
are discarded.

Operation approvals are not a v1 readiness authority. The route accepts only a
human session, whose proxy execution does not require an `ApprovalGrant`, and the
registry does not identify an operation. Consequently there is no applicable
approval request or decision identity to publish. Adding approval correlation in
a later revision requires an operation-bound authority and may expose only its
safe opaque identity, never an approval payload.
