# Administrative ownership transfers

NyxID platform admins can reassign custom catalog services and supported channel
bots between active people and organizations from **Admin → Ownership transfers**
(`/admin/ownership`). Organization admins, resource owners, and platform operators
cannot perform transfers without the NyxID platform-admin role. A personal
destination is an explicitly selected person, not the administrator making the
request.

This feature transfers the definitions managed under Admin → Services. It does
not transfer users' connected `UserService` / `UserEndpoint` / `UserApiKey`
bundles, nodes, agent identities, or external provider accounts.

## Review and commit

1. Choose Catalog services or Channel bots and find the resource by name, slug,
   or ID. The inventory is available only to platform admins and excludes secrets.
2. Select a destination person or organization. Inactive accounts and the current
   owner cannot be selected.
3. Review the authoritative current owner, destination, effects, and blockers.
4. Confirm. The backend rechecks the live platform-admin role, destination,
   resource state, dependencies, and bot capacity before committing.

A stale preview returns 409 and requires another review. MongoDB transactions
commit the resource change, route retirement, and an `ownership_transfers` receipt
together. The request UUID is the receipt ID. Retrying the same request returns
the committed result; reusing its UUID with different inputs is rejected. The UI
retains this UUID after an uncertain network or server failure.

The chained audit event `admin_ownership_transferred` includes the acting admin,
transfer ID, resource kind and ID, both owners, and retired-route count. It uses
the existing audit append service. Audit delivery is awaited before returning
success. If it fails after the ownership transaction committed, retrying the same
request replays the receipt and retries the audit append. Replays can produce
multiple audit entries for the same transfer ID, but do not repeat the transfer.
The transactional receipt remains available even if the process exits before
the audit append.

## Catalog services

`DownstreamService.owner_user_id` is the current owner after transfer. Absent or
null values fall back to `created_by` for compatibility. `created_by` remains the
original creator. Private-catalog checks use current ownership; eligible members
of the destination organization can discover its transferred private entries.
Existing independently authorized connections retain their catalog access.

Transferred catalog services remain managed by NyxID platform admins. Assignment
to a person or organization does not grant that recipient permission to edit
shared destinations, endpoints, credentials, anonymous rules, or other shared
catalog configuration. This prevents an ownership reassignment from handing over
control of traffic belonging to other consumers. Platform-protected curation
grants retain their existing platform-issued authority.

IDs, slugs, endpoints, provider requirements, existing user connections,
credentials, visibility, platform-key grants, prices, approvals, billing history,
and execution configuration are preserved. The current owner's org deletion
checks follow `owner_user_id`; deleting the creator's organization must not
remove a transferred definition.

The first version blocks seeded platform entries, retired platform services,
SSH backing catalog records, and services linked to OIDC clients. Those cases
require infrastructure or OAuth-client handover beyond catalog reassignment.
Blockers appear in the preview and are enforced again on commit.

## Channel bots

Embedded-credential Telegram, Discord, Lark, Feishu, Slack, and WhatsApp bots can
be transferred. The encrypted bot credentials and webhook configuration stay on
the same bot; only its owner changes. This does not revoke token copies or change
the account's ownership at the external platform. Rotate credentials at the
provider when a handover requires that separation.

Every existing route, including disabled routes, is permanently retired by
setting `retired_by_transfer`. Its owner, assigned agent, conversation identifiers,
and historical message metadata are retained. Retired routes cannot be
reactivated through the conversation API, even after a later transfer back to
the original owner. Reply authorization rejects inactive/retired conversations.
The destination creates fresh routes with destination-owned agent keys. Inbound
resolution matches both bot and owner and checks the live bot before callbacks;
outbound delivery checks the current bot before provider work.
Route creation and ownership transfer serialize on the bot document in MongoDB
transactions, so a concurrent request cannot leave a new previous-owner route
active after the move.

Previously dispatched external requests may finish. The operation does not undo
messages already delivered to an agent or provider, copy downstream conversation
content, or transfer historical message/attachment access. There is no durable
message queue for the interval before the destination configures its routes;
messages without a route follow the existing channel's ingress behavior.

Connection-backed OAuth bots, managed Telegram (`telegram-new`), Aurinko email,
and other unsupported adapters are blocked because their connection,
registration, polling, or subscription state is owner-bound. An active polling
lease also blocks transfer. Destination limits use the same transactional
capacity fence as bot registration.

## API

All endpoints require a live NyxID platform admin. `kind` is `service` or
`channel_bot`. They are included in the generated OpenAPI document.

| Method | Path | Purpose |
| --- | --- | --- |
| GET | `/api/v1/admin/ownership/{kind}` | List up to 50 active resources; optional `owner_user_id`, `search`, and `offset`. Follow `next_offset`. |
| POST | `/api/v1/admin/ownership/{kind}/{id}/preview` | Read-only preview with destination, effects, blockers, and version. |
| POST | `/api/v1/admin/ownership/{kind}/{id}/transfer` | Commit or replay the reviewed transfer. |

Preview body:

```json
{"new_owner_user_id":"<destination UUID>"}
```

Transfer body:

```json
{
  "new_owner_user_id":"<same destination UUID>",
  "expected_version":"<version returned by preview>",
  "request_id":"<fresh UUID v4; reuse for retries>"
}
```

Unknown request fields are rejected. Same-owner, blocked, and stale transfers
never partially move a resource. No force/bypass option exists.

## Deployment and verification

Transactions require the existing MongoDB replica-set deployment. Upgrade all
API replicas before performing the first transfer: old binaries do not
understand catalog owner overrides or retired channel routes.

Regression coverage exercises live admin revocation, org-admin/operator denial,
transaction conflicts, invalid/inactive destinations, idempotent retries,
preserved catalog configuration and connections, source-access removal,
destination-org discovery and revocation, bot route retirement and transfer-back,
dependency blockers, response redaction, audit delivery, and UI confirmation,
cancellation, conflict recovery, and uncertain-response retry behavior.
Additional transaction tests cover concurrent registration at the destination's
bot limit, route creation during transfer, and organization-deletion cleanup.
