# Service configuration and vendor retirement

Administrators configure a catalog service's shared credential, platform-key
availability, audience, inference metadata, billing lanes, and endpoint policy in
the shared form used by Add Service and Manage/Edit Service. New shared keys
default to disabled with a restricted, empty audience. Blank credential input
preserves stored material; supplying a nonblank value explicitly rotates it.
Responses expose only whether a credential is configured. This read-only status
is inspected only for authorized administrators and uses the existing encryption
API, so encrypted-empty legacy values report false. Other callers receive null
without decrypting stored material.
Unreadable stored material reports null (shown as “stored but unavailable”); the
editor remains available for explicit rotation. Status reads never rewrite keys
or change private/public legacy access.

The endpoint policy applies to **every binding of the service**, including BYOK,
platform keys, and scoped agents. Rules are exact HTTP methods plus absolute path
templates with single-segment `{parameter}` placeholders. Empty rules deny all;
omission preserves the current policy, and explicit null removes the policy.
The existing optional `PLATFORM_REQUIRE_OPERATION_POLICY` gate still applies to
master-key execution. This policy does not validate request bodies or ownership
of downstream resource IDs.

Billing continues through the existing lane and legacy price services. Omission
preserves lanes, null clears them, and Lago synchronization retains the complete
plan charge array and IDs. The last lane's removal returns the service to legacy
billing. Shared credential rotation does not alter personal credentials.

## Providers and services

Creating a provider opens its detail page with a Linked services section. A
provider may back multiple services. Select an existing eligible service to link,
or create a linked service with the same Add Service form. Configure service
opens that catalog record's existing editor; provider OAuth credentials and
channel Platform Credentials remain separate.

The admin-only `GET /api/v1/providers/{provider_id}/services` inventories canonical
links and legacy provider requirements. `PUT` on
`/api/v1/providers/{provider_id}/services/{service_id}` links an existing record;
service creation accepts `provider_config_id`. Creation and linking are atomic.
Links to another provider, incompatible requirements, SSH/OIDC services, and
enabled platform keys on gateway-URL providers are rejected.

Direct-auth services (including `api-telnyx`) store the canonical provider ID and
their injection method on the service. Linking does not create a
ServiceProviderRequirement: those credentials live on UserApiKey, while a
requirement would demand UserProviderToken. Existing requirement-based services
retain their provider and injection semantics. Secondary requirements remain
editable; the primary requirement cannot be removed while canonically linked.
Linking never silently changes an
unrelated requirement. Requirement mutations and linking serialize through the
service row to prevent conflicting concurrent writes.

## Retirement

Platform Operations is removed, including its user/admin routes, named MCP
operations, feature flag, vendor templates and execution services. Its old
collections are no longer read or written. Historical audit and usage data are
retained. Error codes 11800 and 11801 remain reserved.

Before serving traffic, startup inventories downstream services with both
`service_category = internal` and a `platform-` slug, plus already retired rows.
It sets their category to the durable `retired_platform_vendor` tombstone,
disables them and their existing legacy/unified bindings, disables shared-key
access, and installs a deny-all operation policy. Running this migration again
has the same result. Credentials remain encrypted in their original records;
no key is copied or overwritten. Ordinary connection services with a
`platform-` prefix are unaffected. The historical `internal` plus `platform-`
namespace remains reserved because an old writer's vendor and a new internal
service using that identity cannot be safely distinguished.

The small retirement module remains because older replicas can write legacy
vendor rows after the startup sweep. Discovery, provisioning, catalog execution,
explicit user-service execution, anonymous endpoints, and master-key authorization reject both the
tombstone and the historical internal/prefix shape on every request. Changing a
retired row's slug or re-enabling it does not bypass the tombstone. This is an
access guard for old credential stores, not a retained operation feature. Checks
use catalog identities; a user's platform-prefixed alias for an ordinary service
does not match the historical vendor predicate.

Upgrade all execution replicas before considering retirement complete: an old
binary still has its own removed routes and cannot enforce the new tombstone
guard on newly written rows. The migration prevents existing vendors executing
on old replicas by disabling their catalog records; updated replicas reject new
old-writer records immediately. No live database mutation is required from an
operator tool. There is no credential-transfer operation. Configure a new or
unconfigured ordinary service explicitly through its write-only credential field
after reviewing its audience and policy; retirement never fills or replaces a
destination credential.
