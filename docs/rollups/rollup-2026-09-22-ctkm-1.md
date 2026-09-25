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

Merge [PR #1637](https://github.com/ChronoAIProject/NyxID/pull/1637) into `main`
using **Create a merge commit**. This preserves the existing rollup commit SHAs,
authors, source PR references, and the merge from `main`. Squash merging the
rollup would combine these commits again; rebase merging would rewrite their
SHAs. Neither method meets this rollup's history-preservation requirement.

The eight constituent PRs were already squash-merged into the rollup, producing
one attributed commit per PR. Those squashes did not rewrite inherited `main`
history. Their 23 original development commits remain available on the source
PR pages and through `refs/pull/<number>/head`; they are not ancestors of the
rollup. The final merge preserves the landed squashes and integration commits.

| Source PR | Reviewed source head | Landed squash | Purpose |
| --- | --- | --- | --- |
| [#1633](https://github.com/ChronoAIProject/NyxID/pull/1633) | `a1e12d2e1672a5658d30e1cc25f677cb33e31a5b` | `8e83665bf26a6d28daabcb4579fe57df0ef24181` | Expose native Docs, Sheets, and Slides through Drive and Workspace. |
| [#1632](https://github.com/ChronoAIProject/NyxID/pull/1632) | `95c1c8c05985aa956b7db1444ba988123fb8ef9a` | `6dbe30318f86d0a381ebf86938e1fa429db4091f` | Add Supabase Data API connections with each owner's project URL and API key. |
| [#1638](https://github.com/ChronoAIProject/NyxID/pull/1638) | `3df3784deffab0bc495c32203b3c0cddb86bde7e` | `e9ba77258a7b4243f857172461c5a38f02c5e644` | Add combined, personal, and organization channel-bot listings. |
| [#1640](https://github.com/ChronoAIProject/NyxID/pull/1640) | `3a00320badf46055d7c236ea0a670b0e4d171288` | `25d56aec2fe125eba1a1ad45017805d066a4e762` | Receive X mentions and direct replies and send authorized public replies. |
| [#1639](https://github.com/ChronoAIProject/NyxID/pull/1639) | `9b2b19c38b14a56072a73f2e987bfea9e1307229` | `df2a3863e17d98d3c15626853c4d189ba40e43df` | Allow explicitly scoped and granted service-account connection metadata reads. |
| [#1642](https://github.com/ChronoAIProject/NyxID/pull/1642) | `374644195214139f2d344d997eeba9a3430ff4b1` | `0804da17575b9c83b24f076e29340714bf303cb1` | Add standalone channel-bot onboarding pages. |
| [#1641](https://github.com/ChronoAIProject/NyxID/pull/1641) | `99f778be5de3673902f8f05c9488dd8bc8da6cc3` | `06149b45bc6446e729cbe369ede3d1bed77424c8` | Embed ownership transfers in asset settings and support dedicated X OAuth handover. |
| [#1643](https://github.com/ChronoAIProject/NyxID/pull/1643) | `cd3d194e53f9dafe8d44c9050476cffb522fab80` | `4caf187283fdedf323ab8ce6a475fb7ce2ae9656` | Activate Google routing automatically and correct shared Drive/Workspace contracts. |

The 2026-09-23 attribution audit matched all eight source and squash SHAs against
GitHub. For each PR, replaying its source onto the actual squash parent reproduces
the exact landed tree; Fable independently confirmed matching stable patch IDs.
All 23 source commits and eight squashes resolve to GitHub author `ctkm-aelf`.
There are no source coauthor trailers omitted by the squashes.

The integration commits have separate purposes:

- `df687e10925f1a0f3282cef213d3c06ca50685f0` merges `main` at `6f633320`
  into the rollup, preserving both parents and resolving the generated wizard
  bundle overlap. Its regenerated bundle and source hash passed freshness checks.
- `730d1c146848914c58575eaadb956386f934308d` is an empty CI refresh commit
  associated with #1637. Its tree is identical to the #1643 squash; it records
  recovery from GitHub's stale PR reference without changing code.
- The documentation correction for #1637 updates this history record and the
  merge instructions. It changes only this file and preserves all ten preceding
  rollup commits.

Main's earlier #1634 squash already included the content of #1633, #1632, and
#1638. Consequently, `git blame` may attribute those existing lines to #1634.
The final merge preserves the original attributed rollup squashes as ancestors
of `main`; this table records their provenance without rewriting main's history.

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

## Compatibility corrections after combined review

The combined review of rollup head `730d1c14` against main `6f633320`
identified two corrections before integration. Latest `origin/main` was fetched
and merged into the corrective branch before implementation; the rollup already
contained that commit, so no main merge changes were necessary.

An existing X DM-only bot with billing disabled could stop receiving after a
transient Verify/Reconnect failure while reading subscriptions. Setup now tracks
whether a subscription change has been attempted. Failures before that point
preserve an already registered DM-only bot, including an unavailable setup lease
or app-token error. Credential revocation still stops delivery; billing/public
channels and uncertain subscription changes retain their failure handling.

Organization role-scope writes now share the organization revision with the
ownership-transfer transaction. A first restriction of an inherited default
scope therefore conflicts with an older transfer snapshot, just as a change to
an existing scope row does. Clearing a scope retains the existing implicit-default
representation. Credential-reference owner changes return HTTP 409 instead of a
server error, and Google activation/contract and X admission documentation match
the final behavior.

Regression coverage exercises the real webhook setup service and org scope writer,
including pre-effect errors, partial provider effects, scope restriction after a
transaction snapshot, denied stale transfers, and restored default scopes. The
corrective PR records final validation and Fable's independent review. Exact-email
destination lookup and recipient acceptance remain a separate product-policy
question; this correction introduces no mobile approval requirement.
