---
title: nyxid org
description: Reference for nyxid org — create organizations, manage members and invites, and control role-level service scopes.
---

`nyxid org` manages organizations — shared owners for services, keys, and nodes. Writes require **admin** role on the org; reads require membership. Wherever a command takes `--org` it accepts a UUID, slug, or display name. For the procedure see [Manage organizations](/docs/cli/guides/organizations); for the model see [Organizations](/docs/shared/concepts/organizations).

:::note
Every subcommand accepts the common flags `--base-url`, `--access-token` / `--access-token-env`, `--profile`, and `--output table|json`. See [Authenticate](/docs/cli/getting-started/authenticate).
:::

## org create

```bash
nyxid org create --display-name <name> [--contact-email <e>] [--avatar-url <url>]
```

Create an org; you become its first admin.

## org list / show

```bash
nyxid org list                 # orgs you belong to (with slugs)
nyxid org show <id>            # one org's detail (member only)
```

## org update

```bash
nyxid org update <id> [--display-name <n>] [--slug <s>] [--avatar-url <url>]
```

Update org metadata (admin). Pass `--avatar-url ""` to clear.

## org delete

```bash
nyxid org delete <id> [--yes]
```

Delete an org (admin). Refused while the org still owns shared resources.

## org join

```bash
nyxid org join <nonce-or-url>
```

Redeem an invite nonce or full join URL from your own account.

## org set-primary

```bash
nyxid org set-primary [--org-id <id> | --clear]
```

Set or clear your primary org — the tiebreaker when a service slug exists in more than one scope.

## org member

Manage members of an org:

- **`member list <org>`** — list members.
- **`member add <org> --user-id <id> [--role member] [--scope-source inherit|override] [--allowed-service-ids <ids>]`** — add directly (prefer `org invite create`).
- **`member update <org> <member-id> [--role <r>] [--scope-source <s>] [--allowed-service-ids <ids>]`** — change role or service scope.
- **`member remove <org> <member-id> [--yes]`** — remove a member.

## org invite

One-time invites (admin):

- **`invite create <org> [--role member] [--scope-source <s>] [--allowed-service-ids <ids>] [--ttl-hours 24]`** — issue an invite.
- **`invite list <org>`** — outstanding invites.
- **`invite cancel <org> <invite-id> [--yes]`** — cancel a pending invite.

## org role-scope

Default service scope per role (admin):

- **`role-scope list <org>`** — show every role's default scope.
- **`role-scope set <org> --role <r> [--allowed-service-ids <ids> | --full-access]`** — set the default. Members in `inherit` mode pick it up immediately.
- **`role-scope clear <org> --role <r>`** — delete the row, reverting the role to full access.
