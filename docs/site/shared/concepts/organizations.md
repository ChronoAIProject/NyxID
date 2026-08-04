---
title: Organizations
description: How NyxID organizations let multiple users share a single set of credentials, services, and API keys under one owner without duplicating secrets.
---

Organizations let a group of people share a single set of connected services, external credentials, and API keys under one logical owner. The canonical examples are a development team sharing a company OpenAI API key, a family sharing access to a Home Assistant instance, or a team whose agents all post through a single Lark bot.

Without organizations, every person who needs access to a shared service must hold their own copy of the credential. Copies multiply, rotation becomes a coordination problem, and audit logs mix personal and team traffic.

## The design: an org is a user

NyxID takes the same approach as GitHub: **an organization is a special kind of user**. There is no separate org model. An org is a row in the `users` collection with `user_type: "org"` and no login credentials. Every resource model that is owned by a `user_id` — services, API keys, credentials, approval configs — works for orgs immediately because the org's `user_id` is a valid owner.

The only new model is `OrgMembership`, which connects person users to org users with a role and optional service scope.

Because the org is a user, every existing API that filters by `user_id` returns org-owned resources when given the org's user ID. Org admins use the `--org` flag on existing CLI commands to operate on behalf of the org:

```bash
nyxid service add llm-openai --org <ORG_ID> --credential-env OPENAI_KEY
nyxid api-key create --name "team-coding-agent" --org <ORG_ID> --platform claude-code
```

## Membership roles

Three roles control what a member can do:

| Role | Manage org | Use services via proxy | See services in `/keys` |
|------|-----------|----------------------|------------------------|
| Admin | Yes | Yes | Yes |
| Member | No | Yes | Yes |
| Viewer | No | No | Yes (read-only) |

Admins can invite new members, change roles, manage org-owned resources, and delete the org. Members can make proxy calls using org-owned credentials. Viewers can see what services exist but cannot call them.

## Service scope

Beyond roles, each membership carries a service scope: which `UserService` records the member is allowed to use. Scope has two layers:

- **Role defaults** (`OrgRoleScope`) — an org admin sets a default allowed-service list for each role. All members with `scope_source: Inherit` (the default for new memberships) follow their role's current scope. Changes to the role scope take effect immediately for all inheriting members.
- **Per-membership overrides** — a specific member can be granted a different allowed-service list that overrides their role default. The override is frozen at the time it is saved; subsequent changes to the role scope do not affect that member until the override is cleared.

A member whose effective scope excludes a service gets `403` on proxy calls to that service, even if they have the Member role.

## Credential resolution cascade

When a proxy request arrives and the user has no personal service for the requested slug, NyxID walks the user's org memberships:

1. Personal `UserService` — always wins if present.
2. Legacy personal connection — wins for users not yet on the streamlined model.
3. Org memberships, ordered by `primary_org_id` preference then earliest-joined.

For each org in order, NyxID checks whether the org has a matching service, whether the member's role permits proxy access, and whether the member's effective scope includes that service. The first match is used.

If the same slug exists on multiple orgs the user belongs to, the tiebreaker is `primary_org_id` (set by the user with `nyxid org set-primary`) then earliest membership creation date. Users can also explicitly choose a specific service with the `?_nyxid_via=<user_service_id>` query parameter.

The org membership query has a 500-millisecond wall-clock timeout to prevent proxy latency from blowing up if the database is degraded. Users with a personal service never hit the org path.

## Org-owned resources

Every resource that already keys off `user_id` becomes org-owned by setting `user_id` to the org's user ID:

- `UserService`, `UserEndpoint`, `UserApiKey` — shared credentials and routing configs
- `ApiKey` (NyxID agent identity) — team-scoped agent keys
- `ServiceAccount` — machine-to-machine identities for the org
- `OauthClient` — developer apps registered on behalf of the org
- Approval policies and grants — org-level access policies

## Org-level approval policies

Orgs can configure service-level approval policies that apply to every member. If an org sets `require_approval: true` on a service, that policy is dominant — it overrides any personal approval setting the member may have. Approval notifications are fanned out to all org admins; the first admin to respond decides.

In grant mode, the created grant is owned by the org, not the individual member. Any member's subsequent request to the same service reuses the org-level grant until it expires.

## Org cannot log in

Org users have no password, no MFA, and cannot authenticate directly. On public auth paths (login, forgot password), an org-owner email is treated exactly like an unknown email — no distinct error is surfaced — to prevent enumeration. On authenticated paths (token refresh, email verification), NyxID returns `403 OrgCannotAuthenticate` because the caller has already proven possession of a token and the org-vs-person distinction can be revealed.

This means all access to org resources goes through person users who are org members. There is no org-level session.

## Related guides

- [Organizations in the web console](/docs/web/guides/organizations)
- [Approvals (human-in-the-loop)](/docs/shared/concepts/approvals)
- [The proxy](/docs/shared/concepts/the-proxy)
- [Agent isolation](/docs/shared/concepts/agent-isolation)
