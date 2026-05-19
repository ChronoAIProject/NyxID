# Organizations (Shared Credentials)

## Table of contents

- [Mental model](#mental-model)
- [Creating and managing an org](#creating-and-managing-an-org)
- [Sharing a service with the org](#sharing-a-service-with-the-org)
- [Disconnecting a provider token](#disconnecting-a-provider-token)
- [Inviting members](#inviting-members)
- [Managing members](#managing-members)
- [Managing role-level scopes (per-role defaults)](#managing-role-level-scopes-per-role-defaults)
- [Multi-org tiebreaker: `primary_org_id`](#multi-org-tiebreaker-primary_org_id)
- [Working with org services in agents](#working-with-org-services-in-agents)
- [Org-level approval policies](#org-level-approval-policies)
- [Related error codes](#related-error-codes)
- [CLI profiles](#cli-profiles)

NyxID supports **organizations** for sharing a single set of credentials across multiple users. The classic example is a family Home Assistant or a company OpenAI key: one credential record, many people calling it through their own NyxID accounts. The proxy automatically falls back to org credentials when a personal one is missing for the requested service, with full per-member audit attribution.

## Mental model

- An **org is just a special user** (`user_type: "org"`) that cannot log in directly. It owns its own services, endpoints, API keys, nodes, etc., the same way a person does.
- **Membership** lives in `org_memberships`. Each member has a **role** (`admin` / `member` / `viewer`) and a `scope_source` — either `inherit` (follow the role's default) or `override` (use the membership row's own list). Admins manage everything; members can use org services through the proxy; viewers can see them in `nyxid service list` but cannot proxy through them.
- **Role-level defaults** live in `org_role_scopes` (one row per `(org_user_id, role)`). Admins pin each role's allowed services via `nyxid org role-scope set`; new members default to `scope_source = inherit` and pick up changes immediately. `override` members are unaffected by role edits until reset back to `inherit`. Missing role-scope rows mean "full access" — nothing is restricted until an admin configures one.
- **Resolution priority:** when a proxy request comes in, NyxID first looks for a personal `UserService` matching the slug. Only if that misses does it walk the user's active memberships (in `primary_org_id` order, then earliest-joined) and try the org's services. Personal credentials always win.
- **`credential_source` on `nyxid service list`**: every service in the response is tagged with `{ "type": "personal" }` or `{ "type": "org", "org_id": ..., "org_name": ..., "role": ..., "allowed": ... }`. Use this to tell the user which credentials are theirs vs. shared.

Wherever a command takes `--org <ID|SLUG|NAME>`, you can pass the org UUID, slug, or display name. UUIDs and slugs resolve in one round trip; display names trigger a one-time `GET /orgs` lookup and error with candidate rows if multiple orgs share the same name.

## Creating and managing an org

```bash
# Create an org. You become the first admin.
nyxid org create --display-name "Chrono AI"

# List all orgs you belong to. Output includes the org slug.
nyxid org list

# Show details (member count, your role, slug)
nyxid org show <ORG_ID>

# Update metadata (admin only). Pass --avatar-url "" to clear.
# Slugs are auto-generated on create and can be edited later.
nyxid org update <ORG_ID> --display-name "New Name" --slug chrono-ai-labs

# Delete (admin only). Refuses if the org still owns any shared services,
# endpoints, API keys, NyxID API keys, nodes, or non-revoked provider tokens --
# transfer or delete those first. Deleting an org service auto-cleans the
# linked provider token when no other org service uses it; the orphan-token
# escape hatch for older orgs is `nyxid provider disconnect --org` (below).
nyxid org delete <ORG_ID> --yes
```

## Sharing a service with the org

An org admin creates a shared service by passing `--org <ID|SLUG|NAME>` to `nyxid service add`. The resulting `UserService`, `UserEndpoint`, and `UserApiKey` rows are written with `user_id = <org_user_id>`, so every member of the org immediately sees the service in their `nyxid service list` (tagged with `credential_source.type = "org"`) and can proxy through it using their own NyxID account.

```bash
# Shared OpenAI key for the whole org (API key credential)
nyxid service add llm-openai --org chrono-ai

# OAuth flow targeted at the org. The browser opens under YOUR login, you
# grant access to the org's copy of the provider, and the resulting token
# is stored under the org's user_id so every member can proxy through it.
nyxid service add api-google --oauth --org chrono-ai

# Device-code flow targeted at the org
nyxid service add llm-anthropic --device-code --org chrono-ai

# Custom endpoint targeted at the org
nyxid service add --custom --org chrono-ai \
  --label "Shared Home Assistant" \
  --endpoint-url https://ha.home.local:8123 \
  --auth-method bearer

# Node-routed shared service. The credential lives on the admin's
# personal node (encrypted at rest there) but the org service points
# at it. Every org member's proxy calls flow: NyxID -> admin's node ->
# downstream API. The admin must have write access to the node (which
# they do because it's their personal node); the node itself does not
# need to be re-registered under the org.
nyxid service add llm-openai --org chrono-ai --via-node my-laptop-node
# Then on the node:
nyxid node credentials setup --service llm-openai

# Shared SSH bastion for the whole org (cert-auth via a node the org owns)
nyxid service add-ssh --label prod-bastion --host bastion.acme.internal \
  --via-node acme-shared-node --cert-auth --principals ubuntu \
  --org acme-corp
```

The backend enforces that the caller is an admin of the target org before writing the row (returns `8103 org_role_insufficient` otherwise). Creating or updating an org-owned service respects the admin's **effective** scope (per-member override if set, otherwise the role's default) just like the proxy path — a scoped admin cannot reach a service outside their effective allow-list.

Org-owned SSH services inherit the org's approval policy via `authorize_ssh_access`, so SSH access requests pass through the same per-org approval gate as HTTP proxy calls.

> **How org-OAuth works under the hood.** The CLI resolves `--org <ID|SLUG|NAME>` to the org UUID, creates a placeholder `UserApiKey` under the org's user_id (`POST /keys` with `target_org_id`), then initiates the OAuth / device-code flow with `target_org_id=X` on the query string. The backend stores the resulting `UserProviderToken` with `user_id = org`, and the sync routine matches it to the placeholder because both share the same user_id. The admin's personal scope is untouched -- the grant lives entirely under the org. If you prefer a dedicated identity for the org's OAuth grants (so personal account compromise does not leak the org credential), create a dedicated service account and use its token for the initial `nyxid login` before running `nyxid service add ... --oauth --org <ID|SLUG|NAME>`.

> Viewer-role members still see org services in the list (tagged `credential_source.allowed = false`) but cannot click into their detail page or proxy through them. Scoped members only see services within their `allowed_service_ids` scope -- services outside the scope are hidden entirely, not just disabled.

The frontend `/keys` page groups personal vs. each org section, with viewer-role and out-of-scope items rendered read-only. The frontend `/providers` page exposes the same org scope selector for org admins, so an admin can list and disconnect tokens owned by any org they administer.

Orgs can also own credential nodes. See [`nodes.md`](nodes.md#org-owned-nodes) for org-scoped node registration, admin management, member proxy access, and audit attribution.

## Disconnecting a provider token

Most users never need this directly: deleting the last `UserService` backed by a provider auto-soft-revokes the linked `UserProviderToken` (node-managed services do not count toward "in use"). The explicit command is the escape hatch for orphan tokens left behind by older releases or for unusual cleanup flows.

```bash
# Disconnect a personal provider token
nyxid provider disconnect <PROVIDER_ID>

# Disconnect an org-owned provider token (admin required)
nyxid provider disconnect <PROVIDER_ID> --org <ID|SLUG|NAME>
```

`<PROVIDER_ID>` is the provider config UUID, not the slug -- get it from `nyxid catalog list --output json` or from the token row in the `/providers` page. The command revokes the token (any value of `status != "revoked"` flips to `"revoked"`) and syncs the change into the matching `UserApiKey` rows so dependent services flip back to `pending_auth` until they are reconnected. Once the org's last non-revoked provider token is gone, `nyxid org delete` succeeds.

## Inviting members

```bash
# Issue a one-time invite (admin only). Default role is member, default TTL 24h.
nyxid org invite create <ORG_ID> --role member
nyxid org invite create <ORG_ID> --role viewer --ttl-hours 168

# Restrict the invitee to specific UserService IDs (comma-separated, implies override)
nyxid org invite create <ORG_ID> --role member --allowed-service-ids "<svc1>,<svc2>"

# Force inherit mode (ignores any --allowed-service-ids you also pass)
nyxid org invite create <ORG_ID> --role member --scope-source inherit

# The output includes a join link AND a bare nonce. Share whichever is convenient:
#   Join link: https://<frontend>/orgs/join/ORGINV-...
#   CLI:       nyxid org join ORGINV-...

# Recipient redeems while signed in
nyxid org join ORGINV-ABCDEF12345678
nyxid org join "https://nyx.example.com/orgs/join/ORGINV-..."   # full URL also works

# List or cancel pending invites
nyxid org invite list <ORG_ID>
nyxid org invite cancel <ORG_ID> <INVITE_ID> --yes
```

Direct add (without an invite) is also available for tooling but not recommended:

```bash
nyxid org member add <ORG_ID> --user-id <USER_ID> --role member
```

## Managing members

```bash
# List members of an org (any member can read). Shows each member's role,
# scope mode (inherit/override), and effective allowed services.
nyxid org member list <ORG_ID>

# Change a member's role (admin only)
nyxid org member update <ORG_ID> <MEMBER_USER_ID> --role admin

# Flip a member back to the role's default scope
nyxid org member update <ORG_ID> <MEMBER_USER_ID> --scope-source inherit

# Set a per-member override
nyxid org member update <ORG_ID> <MEMBER_USER_ID> \
  --scope-source override --allowed-service-ids "<svc1>,<svc2>"

# Shortcut: passing --allowed-service-ids without --scope-source implies override
nyxid org member update <ORG_ID> <MEMBER_USER_ID> --allowed-service-ids "<svc1>,<svc2>"
nyxid org member update <ORG_ID> <MEMBER_USER_ID> --allowed-service-ids ""    # clear to full-access override

# Remove a member (admin only). Re-adding later reactivates the same membership row.
nyxid org member remove <ORG_ID> <MEMBER_USER_ID> --yes
```

## Managing role-level scopes (per-role defaults)

Admins can pin a default allowed-services list for each role. New members (and
any member with `scope_source = inherit`) automatically pick up the role's
scope. Per-member overrides win and are unaffected by role edits — so
tightening a role's scope never silently narrows an explicitly-overridden
member.

```bash
# Show defaults for admin / member / viewer (admin only)
nyxid org role-scope list <ORG_ID>

# Restrict the `member` role to two specific services
nyxid org role-scope set <ORG_ID> --role member \
  --allowed-service-ids "<svc1>,<svc2>"

# Grant the `admin` role full access (the default, but pins an explicit row)
nyxid org role-scope set <ORG_ID> --role admin --full-access

# Remove the row entirely — role reverts to the default (full access)
nyxid org role-scope clear <ORG_ID> --role viewer
```

Deleting an org-owned service automatically prunes its ID from every role
scope and every membership override, so stale IDs never linger.

## Multi-org tiebreaker: `primary_org_id`

When a user belongs to multiple orgs that all happen to share the same service slug, the proxy needs a deterministic winner. The default is the earliest-joined org. To override:

```bash
nyxid org set-primary --org-id <ORG_ID>      # set
nyxid org set-primary --clear                # unset (revert to earliest-joined)
```

## Working with org services in agents

For an AI agent making proxy requests, **nothing changes by default**. The agent calls `nyxid proxy request <slug> ...` exactly as before. NyxID looks up the credential -- personal first, then org -- and injects it. The audit log records `routed_via: "personal"` or `routed_via: "org"` (with `org_user_id` and `member_user_id`) so the org admin can see who used what.

If the user has both a personal and an org credential for the same slug and wants to explicitly choose which one the proxy uses, pass `--via-service <USER_SERVICE_ID>`:

```bash
# List services to see both personal and org entries with their IDs
nyxid service list --output json

# Use the org credential explicitly (bypasses personal-first auto-resolution)
nyxid proxy request llm-openai /chat/completions -m POST \
  --via-service <ORG_USER_SERVICE_ID> \
  -d '{"model":"gpt-4","messages":[{"role":"user","content":"Hello"}]}'
```

The `?_nyxid_via=` param is stripped before forwarding to the downstream service, so the downstream never sees NyxID routing metadata.

When listing services for the user, **always print the `credential_source` field** so the user can tell which credentials are theirs and which are shared. Viewer-role items have `credential_source.allowed = false`; do not attempt to proxy through them -- the request will return `8103 org_role_insufficient`.

## Org-level approval policies

An org admin can require approval whenever any member uses a specific org-owned service. Unlike personal per-service configs, **the org policy is dominant**: it overrides the member's personal approval gate for that service.

```bash
# Set an org policy: every member must get an admin's approval before
# their proxy call through this org service goes through. Per-request mode.
nyxid approval set-config <SERVICE_ID> --org <ID|SLUG|NAME> --require-approval true

# Same but use time-based grant mode (approval creates a reusable grant)
nyxid approval set-config <SERVICE_ID> --org <ID|SLUG|NAME> --require-approval true --approval-mode grant

# List current org-level policies on org-owned services
nyxid approval service-configs --org <ID|SLUG|NAME> --output json

# List approval requests filed against org services (admin-only view)
nyxid approval list --org <ID|SLUG|NAME> --output json

# List active grants created from org-policy approvals (grant mode only).
# These live under the org's user_id, so this is the only way for admins
# to see / revoke them.
nyxid approval grants --org <ID|SLUG|NAME> --output json
nyxid approval revoke-grant <GRANT_ID> --org <ID|SLUG|NAME> --yes
```

Frontend: the Org detail page has an **Approvals** tab for admins to manage these policies via a UI.

**Cascade semantics** (`approval_service::resolve_org_aware_approval`):

1. If the service being called is **org-owned** and the org has a per-service config row, the org's config wins absolutely. The `notify_user_ids` fan-out goes to every admin of the owning org, and **any** of them can decide.
2. Otherwise, fall back to the **actor's** per-service config for that service.
3. Otherwise, fall back to the actor's **global** approval toggle (`nyxid approval enable/disable`).

When a request is decided by an org admin, the decision is accepted regardless of which admin was the first to receive the notification -- the decide endpoint cross-checks current org admin status as defense-in-depth against admin set changes since the request was created. Grants created by approved org-policy requests live under the **org's** `user_id`, so the next member's call reuses the same grant instead of triggering a second approval.

Audit entries for org-policy-gated decisions include `routed_via: "org"`, `org_user_id`, `member_user_id`, and `policy_owner_user_id: "<org_user_id>"` so it is unambiguous whose policy caused the gate.

## Related error codes

- `1403 org_cannot_authenticate` -- attempted to log in as an org user (orgs cannot log in directly)
- `8100 org_query_timeout` -- the org-fallback membership query exceeded its 500ms wall-clock budget. The proxy returns 503; usually indicates MongoDB is degraded.
- `8101 org_not_found`
- `8102 org_membership_required` -- you tried to access an org you do not belong to
- `8103 org_role_insufficient` -- viewer tried to proxy, or non-admin tried to manage
- `8104 org_invite_invalid` -- unknown nonce or already-redeemed
- `8105 org_invite_expired` -- invite TTL elapsed; ask the admin for a new one
- `8106 org_approval_no_admin` -- the org policy on this service requires approval but the org has no active admins; an admin must be added before any member can use the service. Returned as 503 to make the degraded state obvious.
- `8107 org_slug_taken` -- attempted to update an org slug to one already used by another org.

## CLI profiles

For running multiple identities on one machine, the CLI supports `--profile`:

```bash
nyxid login --base-url https://nyx-api.chrono-ai.fun --profile coding-agent
nyxid proxy request llm-openai /chat/completions --profile coding-agent -m POST -d '...'
NYXID_PROFILE=coding-agent nyxid proxy request ...  # or via env var
```

Profiles store tokens under `~/.nyxid/profiles/{name}/`. Without `--profile`, the default `~/.nyxid/` path is used.
