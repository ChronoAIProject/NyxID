---
title: Sign up & sign in
description: Create a NyxID account with an invite code, sign in via SSO or email, and orient yourself in the dashboard.
---

NyxID is available as a hosted service at `https://nyx.chrono-ai.fun` (no setup required) or self-hosted via Docker. This page covers the hosted path. For self-host, bring up the stack first — see the self-host setup guide — then come back here; the sign-in and dashboard sections apply identically.

## Create an account

Early access to the hosted instance requires an invite code.

1. Open **[nyx.chrono-ai.fun/register](https://nyx.chrono-ai.fun/register)** in a new tab.
2. Enter invite code: `NYX-FGNY85AF`
3. Sign in with **Google**, **GitHub**, or **Apple** — or create an account with an email address and password.

:::note
Social sign-in (Google / GitHub / Apple) is the fastest path. Password-based registration requires email verification before you can proceed.
:::

:::tip
If you're on a self-hosted instance that was started with `AUTO_VERIFY_EMAIL=true` (the recommended dev default), email verification is skipped automatically.
:::

## Sign in

Return visits: open **[nyx.chrono-ai.fun](https://nyx.chrono-ai.fun)** and use the same sign-in method you registered with. NyxID keeps your session alive with a rotating refresh token (7-day TTL). When the session expires you are redirected back to the login page.

### Forgot password

On the login page, click **Forgot password**. NyxID sends a reset link to your registered email address. The link is single-use and expires after 1 hour.

## The dashboard at a glance

After sign-in you land on the dashboard. The left sidebar groups all features by area:

| Sidebar item | What you do there |
|---|---|
| **AI Services** | Add external API credentials (OpenAI key, GitHub PAT, etc.) and create Agent Keys for your tools. This is your primary workspace. |
| **Approvals** | Review pending approval requests and active grants for gated services. |
| **Organizations** | Create or join an org to share credentials with your team. |
| **Notifications** | Link Telegram and configure push notification preferences for approvals. |
| **Settings** | Profile, email, password, MFA, and active sessions. |
| **Developer** | Register OAuth clients (developer apps) to add "Sign in with NyxID" to your own apps. |
| **Admin** | (Admins only) User management, audit log, service catalog, OAuth clients. |

The **AI Services** page is the hub you will use most. It has two tabs:

- **External Services** — your external API credentials stored and encrypted by NyxID. Each entry is a `UserService` that the proxy resolves by slug.
- **Agent Keys** — scoped `nyx_...` keys you hand to your AI tools. These are NyxID-native keys; they never contain the underlying external secret.

## Two kinds of key — do not mix them up

| Term | What it is | Looks like |
|---|---|---|
| **External service credential** | A real third-party API key (e.g. OpenAI `sk-...`). NyxID stores this encrypted and injects it into proxied requests. | `sk-proj-...`, `ghp_...` |
| **NyxID Agent Key** | A scoped key your tools use to call NyxID. NyxID resolves the underlying credential server-side. | `nyx_...` or `nyxid_ag_...` |

Your AI tool (Claude Code, Cursor, n8n) authenticates to NyxID with a `nyx_...` key. It never sees the external credential.

## Next steps

Once your account is set up, follow [Your first connection](/docs/web/getting-started/first-connection) to add an external service and make your first proxied API call.
