---
title: Set up approvals
description: Require human approval before AI agents can call sensitive services, delivered via Telegram or mobile push notification.
---

NyxID's approval system adds a human-in-the-loop gate on any service. When approval is enabled, an agent's proxy request is blocked until a designated approver confirms or denies it. Approvers are notified via Telegram or the NyxID mobile app (iOS/Android).

For the underlying design, see [Approvals](/docs/shared/concepts/approvals).

## Approval modes

Two modes are available per service:

| Mode | Behavior |
|---|---|
| **Per-request** (default) | Every proxy call creates a fresh approval request. No grants are stored. The approver sees a human-readable description of exactly what will be executed (e.g. `POST /v1/chat/completions (model: gpt-4o, 3 messages)`). |
| **Grant** | The first approval creates a time-based grant. Subsequent requests within the grant window pass automatically. Grant mode is opt-in — use it for services your team trusts but still wants audited on first access. |

Per-request mode is the secure default. Use grant mode when the approval friction of every individual call is too high for the workflow.

## Enable approval on a service

1. Go to **AI Services → External Services** and open the service's detail page.
2. Scroll to **Approval Settings**.
3. Toggle **Require approval** on.
4. Choose the mode: **Per request** (default) or **Grant**.
5. Click **Save**.

From this point, any proxy request to that service by any of your agents will be blocked with `HTTP 403` and an `action_description` describing the pending operation, until an approver acts on it.

## Configure notification channels

Approval requests are delivered wherever you configure. Set up at least one channel before enabling approvals.

### Telegram

1. Go to **Settings → Notifications**.
2. Click **Link Telegram account**.
3. NyxID sends a deep-link — open it in Telegram to complete the link.
4. Under **Approval notifications**, enable **Telegram**.

When an approval request arrives, NyxID sends a Telegram message with the `action_description` and **Approve** / **Deny** buttons. Tap to decide.

:::tip
Start a conversation with `@NyxIDBot` before linking if NyxID cannot initiate the first message. Bot accounts must receive a message before they can contact a user.
:::

### Mobile push (iOS / Android)

1. Install the NyxID mobile app and sign in with the same account.
2. Accept the push notification permission prompt.
3. Go to **Settings → Notifications**, enable **Push notifications**.

Push notifications show the `action_description`. Tap the notification to open the approval in the app and decide.

### Email

1. Go to **Settings → Notifications**.
2. Enable **Email notifications for approvals**.

Email approval requires you to click a link in the email and confirm in the web console. It is slower than Telegram or push for time-sensitive requests.

## Approve or deny a request

Regardless of delivery channel, you can always act on pending requests from the web console:

1. Go to **Approvals** in the left sidebar.
2. The **Pending** tab shows all outstanding requests with their `action_description`.
3. Click **Approve** or **Deny** on a row.

Approved requests unblock the waiting agent call (the proxy retries automatically). Denied requests return `HTTP 403` with `error_code: 7001` to the agent.

## View approval history and grants

- **Approvals → History**: all past requests with outcome, timestamp, and the `action_description` that was shown to the approver.
- **Approvals → Grants**: active grants (grant mode only). Click **Revoke** to cancel a grant before it expires.

## Organization approval policies

When services are owned by an org, the org admin sets the approval policy. All org admins receive the notification fan-out — any one of them can decide. See [Organizations](/docs/web/guides/organizations) for how org-level approval policies work.

## What the agent sees

When approval is required and none has been given, the proxy responds:

```json
HTTP 403
{
  "error": "approval_required",
  "error_code": 7000,
  "request_id": "<uuid>",
  "action_description": "POST /v1/chat/completions (model: gpt-4o, 2 messages)",
  "approve_url": "https://nyx.chrono-ai.fun/approvals/<request_id>"
}
```

After denial or timeout:

```json
HTTP 403
{
  "error": "approval_failed",
  "error_code": 7001,
  "request_id": "<uuid>"
}
```

Agents built for NyxID check these codes and surface them to the user. The `approve_url` links directly to the console review page.
