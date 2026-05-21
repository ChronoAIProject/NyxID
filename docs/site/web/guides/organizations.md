---
title: Share credentials across an organization
description: Create a NyxID organization so a team can share external API credentials, Agent Keys, and approval policies under a single logical owner.
---

NyxID Organizations let multiple people share a set of services, Agent Keys, developer apps, and approval policies under a single logical owner. Common examples:

- A team that buys one OpenAI API key for everyone
- A company whose agents all call the same internal API
- A shared bot that multiple developers manage

For the full design and all edge cases, see [Organizations](/docs/shared/concepts/organizations).

## How it works

An org is not a separate model — it is a special `user_type: "org"` user that cannot log in directly. Every resource that has a `user_id` (services, API keys, developer apps, approval grants) is shared across the org by having that `user_id` point to the org. Members access org resources by virtue of their membership role. Personal resources are unaffected.

## Create an org

1. Go to **Organizations** in the left sidebar.
2. Click **Create organization**.
3. Enter a display name.
4. Click **Create**.

You become the first Admin. The org now has its own `user_id` internally; resources you add with this org as owner are shared with all members.

## Roles

| Role | Manage org / members | Proxy through org services | See org services |
|---|---|---|---|
| **Admin** | Yes | Yes | Yes |
| **Member** | No | Yes | Yes |
| **Viewer** | No | No | Yes (read-only) |

## Invite members

1. Open the org from **Organizations**, go to the **Invites** tab.
2. Click **Create invite link**.
3. Choose the role the invitee will receive.
4. Copy the invite link and share it.

The invitee opens the link while signed into NyxID and clicks **Join**. Their membership is created immediately.

:::note
Invite links expire after the TTL you set (default 24 hours, max 30 days). Create a new link if the old one expires.
:::

## Add a member directly (admin path)

If you know the invitee's NyxID user ID:

1. Go to the org's **Members** tab.
2. Click **Add member**, enter the user ID, and choose a role.

## Manage member scope

By default, new members inherit the role's default scope (full access to all org services). You can restrict a member to specific services:

1. On the **Members** tab, click a member's row.
2. Change **Scope mode** to **Custom**.
3. Select the services this member can access.

To define defaults for a role across all members on the **Inherit** scope, go to the org's **Role permissions** tab and restrict the role.

:::tip
Members whose scope mode is **Inherit** pick up role-scope changes immediately. Members set to **Custom** are frozen at their explicit list until an admin resets them.
:::

## Add org-owned services and keys

Any resource that can be created personally can also be created under an org. From the **AI Services** page:

1. Click **Add Service**.
2. In the dialog, expand **Owner** and select the org.
3. Complete the rest of the setup as normal.

The service is now owned by the org. All members with proxy permission and the right scope can call it. The `credential_source` field in API responses marks it as `"org"` with the org name so the frontend groups personal and org resources separately.

## Org-level approval policies

Admins can require approval on org-owned services so any proxy call from any member triggers a notification to all admins. Set this from the org's **Approvals** tab or from the service detail page.

When `from_org_policy = true`, **all** org admins receive the notification — any one of them can decide. See [Set up approvals](/docs/web/guides/approvals) for the full workflow.

## Multi-org disambiguation

If you belong to multiple orgs that expose the same service slug, NyxID picks a winner automatically:

1. Your personal `UserService` (always wins if it exists)
2. Your primary org (set in **Settings → Primary org**)
3. Your earliest-joined org with that service

To override on a per-call basis, append `?_nyxid_via=<UserService.id>` to the proxy URL. The service ID is visible on the service detail page.

## Remove a member

On the **Members** tab, click the member's row and select **Remove**. This revokes the membership immediately. Personal resources owned by that user are unaffected; org resources remain under the org.

:::warning
You cannot remove the last active admin. The org must always have at least one admin to remain manageable. To dissolve the org, delete it instead (after clearing all org-owned resources).
:::

## Delete an org

On the org's **Settings** tab, click **Delete organization**. NyxID refuses if the org still owns active services, Agent Keys, developer apps, bots, or approval grants. Delete those resources first, then retry.
