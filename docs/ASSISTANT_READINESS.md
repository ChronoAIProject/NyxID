# Assistant Readiness Contract

Related delivery: [NyxID#1353](https://github.com/ChronoAIProject/NyxID/issues/1353)
and [nyxid-chat#5](https://github.com/eanz17/nyxid-chat/issues/5).

`GET /api/v1/assistant/readiness` is an authenticated, human-session read model.
It reports whether the verified `AuthUser` can use the closed assistant capability
registry; request parameters never select a user, organization, service, or scope.
The response revision is `nyxid-assistant-readiness.v2`. The revision is a
hand-maintained constant, not the output of a revision algorithm.

The v2 registry contains one optional connector and two required platform
capabilities:

| `capabilityId` | Evidence source | `label` | `required` | `requestedScopes` | Management path |
|---|---|---|---:|---|---|
| `api-github` | User service catalog slug `api-github` | GitHub | `false` | `repo` | `/keys` |
| `model` | Platform callback slug `chrono-llm-public` | Model | `true` | none | `/keys` |
| `runtime` | Admin service slug `aevatar` | Runtime | `true` | none | none |

The public capability identity is independent of its backing catalog slug.
Consumers see the stable ids `model` and `runtime`; backing slugs are producer
configuration and are never exposed as substitute capability ids. The existing
`api-github` public id is unchanged.

## State semantics

`connected` has one normative meaning for every capability: credential material
is stored, or the selected path requires no credential. It does not assert that
the material decrypts, decodes, or is accepted by an upstream. Readiness never
decrypts credentials. This is the same storage-presence meaning used by the
existing GitHub evidence.

`grantState = not_required` is literal for the empty-scope platform profiles.
`executable` is derived from named execution-path predicates. A positively
observed configuration failure produces `cannot_use`; missing or unclassifiable
authority produces `cannot_check`. A positively absent or explicitly disconnected
user route produces `missing`. Expired, revoked, partial, denied, and otherwise
known unusable evidence produces `cannot_use`.

`status`, `connectionState`, `grantState`, and `reasonCode` are closed enums.
`reasonCode` is `null` exactly when `status = available`.

For the two platform profiles, the evidence dimensions map to execution
authorities as follows:

| Dimension | `model` | `runtime` |
|---|---|---|
| Connection | Stored selected-route credential, stored platform master credential, or `none` auth | Stored admin-service master credential or `none` auth |
| Grant | `not_required`; no requested user scopes | `not_required`; no requested user scopes |
| Execution | Callback catalog configuration plus any selected-route integrity checks | Admin resolver configuration plus the identity/delegation auth chain |
| Access | Classified personal/platform paths are allowed; org, pool, and node authorities fail closed | The server-selected admin path has no caller ACL |

Accordingly, `available` means the NyxID preconditions for a turn hold. It does
not predict which capabilities Aevatar will invoke, just as GitHub readiness does
not predict whether Aevatar will call a GitHub tool.

## GitHub evidence

The producer derives connection and OAuth grant evidence independently. Personal
services take precedence over legacy personal state, which takes precedence over
an accessible organization service. The final execution check uses the canonical
MCP callability projection.

An exact catalog-slug match wins within the selected personal or organization
tier. When no exact match exists and multiple catalog-linked aliases survive in
the winning tier, the evidence is ambiguous and reports `cannot_check` with
`capability_evidence_unavailable`. This preserves the common exact-slug plus
custom-alias setup while preventing readiness from choosing an arbitrary alias.

## Runtime evidence

`runtime` reads the active `aevatar` admin catalog row without consulting caller
credentials or grants. `available` attests all of the following NyxID-side facts:

- the row is an active HTTP service;
- it is not a provider and does not require a user credential;
- it needs no credential or has stored master credential material; and
- it has an auth-delivery chain Aevatar can accept.

The auth-delivery chain is configured when either `forward_access_token` is true,
or identity propagation is `jwt`/`both` with a non-empty identity JWT audience
and delegation-token injection enabled. This check covers the configuration
class that caused the 2026-07-18 production chat outage.

Delegation-token scope is deliberately not attested: the assistant path repairs
an empty or LLM-only scope to `proxy`. The exact audience value is also outside
NyxID readiness authority; the producer verifies that it is non-empty, while the
deployment owns its correct value. Upstream Aevatar liveness is not attested.

## Model evidence

`model` models the bare `/proxy/s/chrono-llm-public` callback route. The optional
`?_nyxid_via=` route override is outside this contract. The catalog target must be
an active, non-provider HTTP service.

For a personal exact-slug route, readiness checks endpoint existence and the
actual stored-key states. Auto-provisioned routes additionally reuse the proxy
resolver's complete eligibility and auth-snapshot check; a drifted route or
missing endpoint is `cannot_use`. In a BYOK deployment, only the personal exact
slug or the legacy personal connection can establish readiness. A disconnected
legacy row is `missing`. Without a personal route, a public internal master
credential (or `none` auth) is the platform backstop.

This revision cannot classify every production route without duplicating the
side-effectful proxy selector. It deliberately reports `cannot_check` with
`capability_evidence_unavailable` when any of these authorities is present:

- organization-owned matching services, including inactive aliases and
  catalog-id aliases;
- organization legacy connections or non-revoked provider tokens;
- personal or organization service pools; or
- a node-pinned personal exact-slug route.

The guard is conservative: it includes all active organization memberships and
does not claim that a guarded route is unavailable. [NyxID#1386](https://github.com/ChronoAIProject/NyxID/issues/1386)
will extract a side-effect-free production route selector so readiness can
classify these populations without maintaining a second resolver.

Model readiness attests only NyxID's side of the callback path. It does not attest
`chrono-llm-public` upstream liveness or whether Aevatar invokes the callback on a
particular chat turn.

## Fixtures

The directly importable canonical endpoint response is checked in at
`tests/fixtures/assistant/readiness-v2.json`; this is the file consumers pin. Its
sibling `tests/fixtures/assistant/readiness-v2-matrix.json` is supplementary and
contains 22 named, complete endpoint responses. The matrix collectively covers
every closed status, connection state, and grant state without publishing a
`missing` runtime state.

Every fixture row is evaluated by the real readiness classifier and serialized
through the handler response path. Contract tests also require three registry-
ordered rows per response, safe management URLs, and secret-free shapes.

`managementUrl` is built from `FRONTEND_URL` when a profile has a repair path. It
is returned only when the configured URL is HTTPS, host-bearing, and contains no
userinfo; otherwise it is `null`. Configuration query strings and fragments are
discarded. Runtime has no user-side repair path and always publishes `null`.

Operation approvals are not a v2 readiness authority. The route accepts only a
human session, whose proxy execution does not require an `ApprovalGrant`, and the
registry does not identify an operation. Consequently there is no applicable
approval request or decision identity to publish. Adding approval correlation in
a later revision requires an operation-bound authority and may expose only its
safe opaque identity, never an approval payload.
