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

## Automatic Google activation and corrected contracts

[PR #1643](https://github.com/ChronoAIProject/NyxID/pull/1643) builds on #1633.
The temporary `GOOGLE_WORKSPACE_MULTI_ORIGIN_ENABLED` flag previously left
editor routing inactive while the hosted specs advertised those operations.
This follow-up removes the flag and runs recognized-default migration and
endpoint synchronization automatically during normal startup. Fresh and legacy
catalogs receive Drive22/Workspace38 without a separate environment setting.

Workspace is composed from the same Drive spec, then adds Calendar and Gmail.
All 22 shared operations have identical definitions, including native editor
origins and the corrected upload contracts. The existing upload POST/PATCH
routes now document raw media and `multipart/related`, with complete HTML import
and replacement examples for Google Docs. Generated MCP uploads preserve their
media/base64 contract; unsupported multipart MCP requests fail before approval
or execution. Single-file field selectors and PATCH parent-move guidance are
also corrected.

The six corrected operations in each catalog retain their IDs and advance their
generations once. Approvals and durable grants bound to those changed contracts
require normal re-approval. Unchanged contracts retain their generations and
grants. Startup preserves administrator-customized policies and destination
maps, disabled endpoints, existing connections and credentials. Concurrent
startup reconciles the same definition without repeated generation changes.
No OAuth scopes, Google routes, billing, or pricing changes are added.

## Migration and deployment

Historical #1633 deployments used the flag to order compatible backend readers
and nodes before activation. With #1643, activation is automatic. Before rollout,
verify every serving backend and participating/failover node supports destination
routing and the required HTTP signature version. Inspect persisted policies,
destination maps and endpoint rows after startup. Customized configurations are
retained and diagnostic logs identify blocked migrations. See
[Google Workspace OAuth](../GOOGLE_WORKSPACE_OAUTH.md) for rollout, rollback,
and approval details.

Google API enablement is a separate prerequisite. A Google 403 with
`SERVICE_DISABLED` requires enabling the named API in the consumer Cloud project;
it is distinct from NyxID's 503/code12300 activation error. Recreating services
or OAuth connections does not resolve API enablement. Production acceptance must
verify Docs create/edit/read-back, Sheets formulas and calculated results, and
Drive multipart import/update through the exact existing connections. Merging
this rollup does not establish deployment or live execution success.

The automatic-activation implementation passed 260 selected backend tests and
23 CLI security tests before rollup integration, plus format, all-target Clippy,
an optimized gcp-kms backend build, and build-input guards. All 38 unique Google
operations passed the independent structural Discovery audit. The shared-spec
regression compares every Drive path definition with Workspace. PR #1643 records
final integration CI and its squash provenance; the rollup PR records the
combined branch's checks.

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

[PR #1641](https://github.com/ChronoAIProject/NyxID/pull/1641), a follow-up to
#1615 and #1616, makes transfers available in the asset's
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

## Standalone channel bot onboarding

Channel setup previously required finding the Add Bot dialog in the dashboard.
This change adds shareable, authenticated setup links at
`/channel-bots/connect` and a dedicated connection page at
`/channel-bots/connect/{platform}`. Each page identifies NyxID and the selected
platform, centers the form with an animated dotted connection, and offers one
primary action. The ownership picker is hidden when personal is the only scope.

The shared form reads fields, validation, secret flags, instructions, and managed
flow selection from the platform catalog. It supports Telegram tokens, Telegram
creation, Discord, Lark, Feishu, Slack, WhatsApp, X, and Aurinko. New catalog
platforms using credential forms or an existing managed protocol do not need a
new page; a new provider authorization protocol still needs its own frontend
flow implementation.

Links can prefill the bot name, organization, and catalog-declared credential
fields. Numeric identifiers and credential strings retain their exact values
through the sign-in return. Secret parameters are removed from the visible URL
after consumption, but can still appear in upstream logs, shared messages, and
login history; broadly shared links should contain only non-secret identifiers.
Prefill never submits the form automatically. Provider consent and any external
webhook configuration remain explicit steps.

Completion shows callback URLs and setup instructions even when the platform
does not issue a one-time secret. Slack and Discord now provide those setup
instructions. X authorization supports retry after a closed popup while keeping
the original completion listener active for browsers with COOP isolation.

The [user guide](../site/web/guides/channel-bots.md) documents every current
platform, query parameters, and setup steps. The
[channel relay architecture](../CHANNEL_BOT_RELAY.md#standalone-onboarding-pages)
documents the extension contract and verification workflow. Fable independently
reviewed the implementation twice and found no remaining blocking issues after
the fixes; its final focused checks passed 95 unit tests and 29 browser scenarios.
Provider browser tests use fixtures and do not create real external bots.
