# Ownership transfers

Owners can transfer their own custom catalog services and supported channel bots.
For organization assets, active organization admins with unrestricted management
access can transfer them. Platform admins retain cross-owner transfer access and
the **Admin → Ownership transfers** inventory.

The **Ownership transfer** card lives on channel bot detail pages, the existing
**Advanced** tab of a connected service, and admin catalog service detail pages.
Agent Keys with `write` or `admin` scope can use the same API
under their owner's authority. Keys must be active, unexpired, general-purpose,
and have `allow_all_services=true`: the existing limited service allowlist names
connected `UserService` rows, not catalog definitions or channel bots. Proxy-only
keys, scheduled-invocation keys, and keys limited to connected services cannot
transfer these assets. A platform admin's Agent Key does not inherit cross-owner
platform authority.

There is no mobile approval or recipient acceptance step. The web flow uses a
preview and explicit confirmation; agents submit that same preview version.

For services, this feature transfers the definitions managed under Admin →
Services. It does not transfer users' connected `UserService` / `UserEndpoint` /
`UserApiKey` bundles, nodes, agent identities, or external provider accounts.
For eligible X bots, the bot's dedicated OAuth credential moves with the bot as
described below.

## Review and commit

1. Open **Channel Bots → bot** (`/channel-bots/{id}`), or **AI Services → service
   → Advanced** (`/keys/{id}`). Admin catalog service detail pages also contain
   the card. Choose **Transfer ownership** to open the review dialog in place.
   The server determines whether the actor has transfer authority over the bot
   or catalog definition, independently of connected-service access.
2. Select **Person** or **Organization** as the destination type, then choose
   the destination account in the searchable owner dropdown. Search, results,
   and pagination are part of the same picker. Inactive accounts and the current
   owner cannot be selected. Owners see themselves, their organizations, and
   members of the owning organization. An exact email or account ID can find
   another destination without exposing the full platform directory. Platform
   admins retain global destination search.
3. Review the authoritative current owner, destination, effects, and blockers.
4. Confirm. The backend rechecks live ownership, organization membership or
   platform authority, Agent Key permissions, destination, dependencies, and bot
   capacity before committing. Actor, key and membership writes participate in
   the ownership transaction so revocation cannot be bypassed by an old preview.

After a bot transfer, the UI returns to Channel Bots because the actor may no
longer have access to the bot. The destination creates its own routes. After a
catalog transfer, the connected-service page remains open and refreshes transfer
authority; the connected service and its credentials remain in place. The admin
inventory remains available and reuses the same review dialog and searchable
destination picker.

A stale preview returns 409 and requires another review. MongoDB transactions
commit the resource change, route retirement, and an `ownership_transfers` receipt
together. The request UUID is the receipt ID. Receipts bind the acting user and
Agent Key identity (if present). Retrying the same request with that identity returns
the committed result; reusing its UUID with different inputs is rejected. The UI
retains this UUID after an uncertain network or server failure.

The preview version includes the complete stored resource, the destination, and
the number of routes to retire. For X OAuth transfers it also binds the backing
credential and its dependency state. A routine bot update, such as webhook
activation, reconnection, credential refresh, or route creation, can therefore
make a preview stale even when ownership has not changed. Review the refreshed
preview before confirming again. This conservative review check is separate
from the ownership generation used to fence channel delivery.

The chained audit event retains the name `admin_ownership_transferred` for
compatibility. It includes the actor and Agent Key identity when applicable,
transfer ID, resource kind and ID, both owners, and retired-route count. It uses
the existing audit append service. Audit delivery is awaited before returning
success. If it fails after the ownership transaction committed, retrying the same
request replays the receipt and retries the audit append. Replays can produce
multiple audit entries for the same transfer ID, but do not repeat the transfer.
The transactional receipt remains available even if the process exits before
the audit append.

## Asset API

The asset routes accept human sessions and eligible Agent Keys:

- `GET /api/v1/ownership/{kind}/{id}/authorization` — current transfer capability.
- `GET /api/v1/ownership/{kind}/{id}/destinations?user_type=person&search=...&offset=0` — compact destination search, 20 results per page.
- `POST /api/v1/ownership/{kind}/{id}/preview` — body `{ "new_owner_user_id": "UUID" }`.
- `POST /api/v1/ownership/{kind}/{id}/transfer` — body `{ "new_owner_user_id": "UUID", "expected_version": "preview version", "request_id": "UUID v4" }`.

`kind` is `service` or `channel_bot`. The existing `/admin/ownership` inventory
and preview/transfer aliases remain for compatibility; the admin inventory is
still platform-admin-only. Delegated tokens and service accounts cannot commit
transfers. All routes retain the same dependency blockers and atomic commit.

## Catalog services

`DownstreamService.owner_user_id` is the current owner after transfer. Absent or
null values fall back to `created_by` for compatibility. `created_by` remains the
original creator. Private-catalog checks use current ownership; eligible members
of the destination organization can discover its transferred private entries.
Existing independently authorized connections retain their catalog access.

Organization discovery through an explicit `owner_user_id` is additive. Legacy
private entries without that field retain their existing discovery rules; the
creator fallback does not newly expose those entries to the creator organization's
members. Readable organization memberships, including viewers, can discover a
transferred private entry without gaining permission to execute it.

Transferred catalog services remain managed by NyxID platform admins. Assignment
to a person or organization does not grant that recipient permission to edit
shared destinations, endpoints, credentials, anonymous rules, or other shared
catalog configuration. This prevents an ownership reassignment from handing over
control of traffic belonging to other consumers. Platform-protected curation
grants retain their existing platform-issued authority.

This admin-only management rule also applies after transferring a definition back
to its original creator. A transfer keeps `owner_user_id` explicit; it does not
restore the legacy creator-edit permission.

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
An ownership generation detects transfers away and back without treating routine
webhook status or polling updates as ownership changes.
Route creation and ownership transfer serialize on the bot document in MongoDB
transactions, so a concurrent request cannot leave a new previous-owner route
active after the move.

Previously dispatched external requests may finish. The operation does not undo
messages already delivered to an agent or provider, copy downstream conversation
content, or transfer historical message/attachment access. There is no durable
message queue for the interval before the destination configures its routes;
messages without a route follow the existing channel's ingress behavior.

A live ownership check can stop an inbound webhook batch partway through a
handover. Earlier messages remain stored, and the platform may retry the batch
against the bot's current owner and routes. Existing adapter-specific deduplication
and no-route handling still apply; there is no exactly-once delivery guarantee
across a handover.

X bots created through managed OAuth onboarding can also be transferred. The
transfer atomically reassigns the bot and its exact `UserApiKey` row when that
credential is active, belongs to the bot's current owner, uses the platform
OAuth application, has `source: "channel_onboarding"`, and is dedicated to that
bot. The encrypted access and refresh tokens stay on the same row; the transfer
does not duplicate credentials or copy a refresh token to another connection.
Required OAuth scopes follow the bot's selected X events, including public
posting permission when mentions or replies are enabled. Event selections stay
with the bot.
The internal OAuth callback handle changes so a stale authorization callback
cannot overwrite the transferred connection. The bot's credential-row ID stays
the same.

The preview reports specific blockers for shared credentials, pending or invalid
connections, mismatched ownership or provider configuration, and OAuth work in
progress. References from other bots, including inactive bots, connected
services, and agent credential bindings prevent the credential from moving.
Commit rechecks these dependencies transactionally. A running channel poll or
OAuth refresh must finish before the transfer can proceed. Destination limits
use the same transactional capacity fence as bot registration.

Managed Telegram (`telegram-new`), Aurinko email, and other unsupported adapters
remain blocked. Aurinko has separate owner-bound mailbox subscription, batch,
receipt, and send records; transferring its credential alone is insufficient.
These adapters require their own registration or subscription handover.

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
X regressions cover organization-to-person handover, destination token refresh,
source-owner denial, stale callback rejection, shared and inactive consumers,
pending authorization and refresh leases, stale-owner reference creation, and
receipt replay without repeating the credential handover.
