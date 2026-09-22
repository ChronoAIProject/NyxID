# Rollup: 2026-09-22 ctkm-1

This rollup starts from `main` commit
`9218f3894c4a2770a28a8f0e6935d01599ba5b22` and collects the Google Drive
editor-operation follow-up in [PR #1633](https://github.com/ChronoAIProject/NyxID/pull/1633).
The earlier [PR #1572](https://github.com/ChronoAIProject/NyxID/pull/1572)
is already part of that base.

The rollup was subsequently updated to current `main` commit
`6f633320c636963e7baa1aad7921814ca1d819cd` on 2026-09-22, preserving its
existing constituent PRs and rebuilding the combined CLI wizard bundle.

## Problem and resulting behavior

Google Drive exposed only nine file operations. Even a connection with full
Drive permission could not call the native Docs, Sheets, or Slides APIs through
the Drive service. Workspace published those editor operations, but execution
still required its catalog destination activation.

PR #1633 makes Drive own the editor bundle and composes Workspace from it:

| Catalog service | Operations after deployment and activation |
| --- | --- |
| Google Drive | 22: nine file operations, three Docs, seven Sheets, three Slides |
| Google Workspace | 38: the same 22 Drive/editor operations, 13 Calendar, three Gmail |

Both services select the exact Google editor origin over REST, typed MCP,
generic MCP, and exact-approval execution. Full Drive OAuth scope already
authorizes the 13 editor operations, subject to file permissions. Narrower
`drive.file` and read-only grants retain their limits. Separate editor services
remain available.

## Migration and deployment

The existing `GOOGLE_WORKSPACE_MULTI_ORIGIN_ENABLED` flag activates both Drive
and Workspace. Startup updates only the known seeded policy with an absent or
empty destination map, preserves administrator changes, and adds 13 endpoints
per service. Existing endpoint IDs, contracts, and generations are preserved.
The older Workspace migration that added Gmail retains its original policy
precondition.

Before activation, editor calls return HTTP 503/code 12300. Deploy updated
backend readers and upgrade participating nodes before enabling the flag;
the corresponding Google APIs must also be enabled in the OAuth client's Cloud
project. Merging this rollup does not itself activate production. See
[Google Workspace OAuth](../GOOGLE_WORKSPACE_OAUTH.md) for the activation order
and pending-approval behavior.

## Verification and history

Fable reviewed the design and implementation with no blocking findings. All
110 selected local tests passed on `6c39908c`, including catalog migration,
preserved endpoint contracts, Google OAuth boundaries, all three editor hosts
through both services, forwarded JSON bodies, node routing, and exact approvals.
[CI on that implementation](https://github.com/ChronoAIProject/NyxID/actions/runs/35694580318)
passed; PR #1633 also records the final checks for its rollup destination.

Each constituent PR is squash-merged into this rollup. A squash adds a new commit
containing the combined changes and its source PR reference. It does not rewrite
the inherited `main` commits. The source PR retains its individual commits and
review discussion. A later squash merge of this rollup into `main` likewise adds
a new commit; its message and this document retain the constituent PR details.

## Ownership transfer from asset settings

This follow-up to #1615 and #1616 makes transfers available in the asset's
existing management flow. Channel bot detail pages contain an ownership card;
connected-service detail pages show the catalog ownership card under
**Advanced**. The cards and retained **Admin → Ownership transfers** inventory
share one review dialog and searchable destination picker. No separate service
settings page is introduced.

Asset owners and active organization admins with unrestricted management can
transfer their assets without mobile approval. General Agent Keys with live
`write` or `admin` scope and `allow_all_services=true` act under the same owner
authority; scoped execution keys do not gain asset management rights. Commit
revalidates and fences the actor, key, and organization membership in its
transaction. Platform admins retain the administrative inventory and override.

A dedicated X channel-onboarding OAuth credential moves atomically with its
bot when it belongs to the source owner and has no other consumers. Tokens stay
on the same encrypted row; the callback handle rotates. Shared, pending,
mismatched, and in-flight dependencies block transfer. Selected X DM, mention,
and reply events are preserved, and required OAuth scopes follow those events.
Old routes are retired, conversation history stays with the source owner, and
audit records and idempotent receipts retain the actor and both owners.

Service transfers apply to custom catalog definitions. Connected
`UserService`/endpoint/credential bundles retain their owners. Aurinko, managed
Telegram, and OIDC client handover remain unsupported. See
[Ownership transfers](../ADMIN_OWNERSHIP_TRANSFERS.md) for supported adapters,
authorization rules, effects, and blockers.

Regression coverage includes owner and agent authorization, live revocation,
destination search boundaries, transaction races, destination OAuth refresh,
stale callback rejection, X public-event scopes, and desktop/mobile transfers
from the existing asset pages.
