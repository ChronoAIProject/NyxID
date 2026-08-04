---
title: Manage organizations
description: Create an org from the terminal, invite members, share services across the team, and control which services each role can reach.
---

An organization is a shared owner for services, keys, and nodes. Resources created under an org are visible to every member, but each member still proxies through them with their **own** NyxID account and audit trail. This guide is the CLI procedure; for the model — roles, scoping, primary-org resolution — see [Organizations](/docs/shared/concepts/organizations).

Most write operations require **admin** role on the org; reads require membership. Wherever a command takes `--org`, you can pass a **UUID, slug, or display name** — the CLI resolves it (display-name lookup errors with candidates if ambiguous).

## Create an org

```bash
nyxid org create --display-name "Chrono AI"
nyxid org list                 # orgs you belong to, with their slugs
```

You become the first admin. `nyxid org list` shows each org's auto-generated `slug`, which you can use in place of the UUID everywhere below.

## Invite members

Prefer one-time invites over adding by raw user ID:

```bash
nyxid org invite create <org> --role member --ttl-hours 48
```

This prints an invite nonce / URL. The recipient redeems it from their own account:

```bash
nyxid org join <nonce-or-url>
```

Roles are `admin`, `member`, `viewer`. List or cancel outstanding invites with `nyxid org invite list <org>` and `nyxid org invite cancel <org> <invite-id>`.

## Share a service across the org

Any service or key command that accepts `--org` creates an org-owned resource (you must be an org admin):

```bash
export OPENAI_API_KEY=sk-...
nyxid service add llm-openai --credential-env OPENAI_API_KEY --org "Chrono AI"
```

Now every member sees that service in their own `nyxid service list` and can proxy through it. The same `--org` flag works on `service add-ssh`, `api-key create`, `node register-token`, and `channel-bot register`. See [Connect an AI service](/docs/cli/guides/connect-a-service) for the full add flow.

## Scope what each role can reach

By default every role has full access to org services. Set a per-role default scope (members in `inherit` mode pick it up immediately):

```bash
nyxid org role-scope list <org>
nyxid org role-scope set <org> --role member --allowed-service-ids "<svc-1>,<svc-2>"
nyxid org role-scope set <org> --role member --full-access      # remove the restriction
```

Override the scope for one member regardless of their role default:

```bash
nyxid org member update <org> <member-id> --scope-source override --allowed-service-ids "<svc-1>"
nyxid org member update <org> <member-id> --scope-source inherit   # back to the role default
```

## Primary org

When the same service slug exists in more than one of your scopes, NyxID uses your **primary org** as the proxy tiebreaker:

```bash
nyxid org set-primary --org-id <org>
nyxid org set-primary --clear
```

(You can always force a specific instance per call with `nyxid proxy request --via-service <user-service-id>`.)

## Manage members & the org

```bash
nyxid org member list <org>
nyxid org member update <org> <member-id> --role admin
nyxid org member remove <org> <member-id> --yes
nyxid org update <org> --display-name "New Name" --slug new-slug
nyxid org delete <org> --yes        # refused while the org still owns shared resources
```

:::note
Removing an org admin does not retroactively cancel registration tokens they already minted. If you remove an admin, also delete any pending node registration tokens for that owner.
:::

## Next

- [`org` command reference](/docs/cli/reference/org) — every subcommand and flag.
- [Organizations](/docs/shared/concepts/organizations) — roles, scoping, and proxy resolution in depth.
