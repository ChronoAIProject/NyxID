---
title: Approvals for agents
description: Require human sign-off before an agent's proxy call is forwarded — configure approval mode, notification channels, and how agents handle the approval flow.
---

NyxID can gate any proxied service call behind a human approval. When a service has approval enabled, an agent's request is held until an authorized user approves or denies it. Approval notifications are delivered via Telegram, mobile push notification, or the web console.

For the conceptual background, see [Approvals](/docs/shared/concepts/approvals).

## Approval modes

Two modes are available:

| Mode | Behavior | Default? |
|---|---|---|
| `per_request` | Every proxy call requires fresh approval. No reusable grants. | Yes |
| `grant` | First approval creates a time-based grant. Subsequent requests within the grant period pass automatically. | No — opt in |

`per_request` is the default. The approver sees the `action_description` of the exact request (e.g., `POST /v1/chat/completions (model: gpt-4o, 3 messages)`) before approving.

## Enable approval on a service

```bash
# Per-request mode (default)
nyxid approval set-config <SERVICE_ID> --require-approval true

# Grant mode
nyxid approval set-config <SERVICE_ID> --require-approval true --approval-mode grant
```

Via API:

```bash
# Per-request
curl -X PUT https://nyx-api.chrono-ai.fun/api/v1/approvals/service-configs/<SERVICE_ID> \
  -H "Authorization: Bearer $NYXID_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"approval_required": true}'

# Grant mode
curl -X PUT https://nyx-api.chrono-ai.fun/api/v1/approvals/service-configs/<SERVICE_ID> \
  -H "Authorization: Bearer $NYXID_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"approval_required": true, "approval_mode": "grant"}'
```

## What the agent receives

When an agent's request hits an approval-gated service, NyxID holds the HTTP connection open until the user decides. If the user approves, NyxID sends the original request and the agent receives the downstream response directly; the agent does not retry.

Only an eligible request that opts into async approval with the `X-NyxID-Approval-Mode: async` header (or an assistant call that needs an execution-permission review) gets an immediate `403` with `error_code: 7000` instead:

```json
{
  "error": "approval_required",
  "error_code": 7000,
  "request_id": "b1c2d3e4-...",
  "message": "Approval required. Review the pending request in Approval History; configured notifications are attempted."
}
```

The `request_id` is used to poll for status (async callers, with the same credential) and to display to the user. After approval NyxID sends or resumes the request itself.

If the approval is rejected, expires, or times out, the agent receives `error_code: 7001`:

```json
{
  "error": "approval_failed",
  "error_code": 7001,
  "request_id": "b1c2d3e4-...",
  "approve_url": "https://nyx.chrono-ai.fun/assistant/approvals",
  "message": "Approval failed: Approval request was rejected. Review the approval request and its history at https://nyx.chrono-ai.fun/assistant/approvals",
  "details": { "code": "approval_denied", "status": "rejected", "request_id": "b1c2d3e4-..." }
}
```

`details` also carries `error`, `message`, `instructions` and `review_url`. The request was not sent.

## Agent-side handling

A well-behaved agent should:

1. Keep the HTTP connection open long enough for a held request; the response arrives after the decision.
2. Detect `error_code: 7000` and inform the user that approval is needed.
3. Ask the user to approve. An async caller may poll for the decision with the same credential; an assistant key cannot poll its own reviews. Never re-send the original request: an approved request is sent by NyxID, so a re-sent write can run twice.
4. On `error_code: 7001`, show the `approve_url` and suggest setting up notifications if none are configured.

### Polling for approval status

```bash
curl https://nyx-api.chrono-ai.fun/api/v1/approvals/requests/<REQUEST_ID>/status \
  -H "Authorization: Bearer $NYXID_API_KEY"
```

Response:

```json
{
  "status": "pending",
  "expires_at": "2026-05-20T16:00:00Z"
}
```

Status values: `pending`, `approved`, `rejected`, `expired`. For an async request, the response also has an `execution` field reporting whether NyxID sent the approved request and its bounded result. Poll with the same credential that made the request.

Poll at a reasonable interval (5–10 seconds). `status` leaves `pending` when the user decides, but an approved async request is only queued at that moment (`execution.state` is `queued`). If you need the outcome, keep polling until `execution.state` reaches a final state such as `completed`, `not_sent` or `unknown`, and check `execution.result_available` before reading `execution.result`. Do not re-send the original request; NyxID sends it.

## Set up notification channels

Without a notification channel, the user only sees pending approvals in the web console. Setting up Telegram or mobile push delivers the request immediately.

### Telegram

```bash
nyxid notification telegram-link
nyxid notification update --approval-telegram true
```

Follow the printed instructions to link your Telegram account. Approval requests arrive as interactive Telegram messages with approve / deny buttons.

### Mobile push (NyxID mobile app)

Install the NyxID app on iOS or Android, sign in, and enable push notifications. Approval requests arrive as push notifications with inline approve / deny actions.

### Web console

Pending approvals are always visible at `https://nyx.chrono-ai.fun/approvals/history` regardless of notification configuration.

## Approving and denying requests

**Web console:** Navigate to **Approvals → History**, find the pending request, and click **Approve** or **Deny**.

**CLI:**

```bash
nyxid approval approve <REQUEST_ID>
nyxid approval deny <REQUEST_ID> --reason "Not authorized for production data"
```

**API:**

```bash
curl -X POST https://nyx-api.chrono-ai.fun/api/v1/approvals/requests/<REQUEST_ID>/decide \
  -H "Authorization: Bearer $NYXID_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"approved": true}'
```

## Managing grants (grant mode only)

In `grant` mode, approved requests create time-based grants. Subsequent requests within the grant period pass automatically.

```bash
nyxid approval grants                      # list active grants
nyxid approval revoke-grant <GRANT_ID>     # revoke a grant early
```

API:

```bash
GET    /api/v1/approvals/grants
DELETE /api/v1/approvals/grants/<GRANT_ID>
```

:::note
`per_request` mode never creates grants. If you see no grants after an approval, confirm the service is configured with `approval_mode: grant`.
:::

## View per-service configs

```bash
nyxid approval service-configs   # lists all services with approval enabled, including approval_mode
```

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| User never receives the approval request | No notification channel configured | Run `nyxid notification telegram-link` or install the mobile app |
| Approval times out before user sees it | Default expiry is short | Increase `APPROVAL_EXPIRY_INTERVAL_SECS` on the server, or set up real-time notifications |
| Agent receives `7001` immediately | Service returned `approval_failed` without a pending phase | The previous approval request for this `(user, service, requester)` tuple was already rejected. The agent should show `approve_url` |
| `per_request` service creates grants | Service was reconfigured to `grant` mode | Run `nyxid approval service-configs` to confirm the current mode |
| Agent loops infinitely on `7000` | Polling without timeout | Add a polling limit and show the user the `request_id` and a link to the web console |
