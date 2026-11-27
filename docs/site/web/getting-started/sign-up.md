---
title: Sign up & sign in
description: Use the registration methods offered by your NyxID instance, sign in, and find Assistant and Studio.
---

Use the NyxID instance where you plan to work. An invitation and account belong to that instance. On a self-hosted installation, open your workspace's Studio URL before following these steps.

## Create an account

1. Select **Sign up** on the sign-in page of that instance or visit its `/register` path. [Open registration on this instance](/register)
2. If the registration form asks for an invite code, enter the code supplied by your workspace before selecting how to register.
3. Choose a sign-in provider offered on the registration form and authorize your account. If the form offers email and password, you can also use that option and follow the email verification prompt when it appears.

If social sign-in tells you to register with an invite first, return to this instance's **Create your account** page. An instance can require an invitation while offering only social registration. Enter your invitation there and continue with one of its provider buttons. If you already have a NyxID account and want to link a social login, choose the provider account whose verified email matches the email on your NyxID account.

## Sign in

Return to this instance's sign-in page and use the same account and provider, or your password if you registered with email. [Open sign-in](/login)

### Forgot password

If your instance offers password sign-in, click **Forgot password** and follow the reset link sent to your registered email address. For a provider login, use that provider's account recovery.

## Assistant and Studio at a glance

After sign-in you land in **Assistant**. If you started from a protected page or an OAuth/consent flow, sign-in resumes that requested destination instead. An explicit visit to `/dashboard` returns you to **Studio**.

Use **Open Studio** in the Assistant account menu to manage services and developer settings. Studio's left sidebar groups these features by area:

| Sidebar item | What you do there |
|---|---|
| **AI Services** | Add external API credentials (OpenAI key, GitHub PAT, etc.) and create Agent Keys for your tools. |
| **Approvals** | Review pending approval requests and active grants for gated services. |
| **Organizations** | Create or join an org to share credentials with your team. |
| **Notifications** | Link Telegram and configure push notification preferences for approvals. |
| **Settings** | Profile, email, password, MFA, and active sessions. |
| **Developer** | Register OAuth clients (developer apps) to add "Sign in with NyxID" to your own apps. |
| **Admin** | (Admins only) User management, audit log, service catalog, OAuth clients. |

In Studio, the **AI Services** page manages connections and keys. For this guide, use these tabs:

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
