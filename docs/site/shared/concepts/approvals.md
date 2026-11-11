---
title: Approvals (human-in-the-loop)
description: How NyxID can require human sign-off before forwarding an agent's proxy request, the difference between per-request and grant modes, and where approvals fit in the proxy pipeline.
---

The approvals system adds a human checkpoint to the proxy. When a service is configured to require approval, any programmatic request to that service — from an AI agent, a service account, or a delegated token — is held until the resource owner explicitly approves or rejects it. The downstream call is only made if the user approves.

## Why approvals exist

Agents are useful precisely because they act autonomously. But autonomous action on sensitive services — a production API, a financial data provider, a credential that unlocks write access — carries risk. The approvals system is a circuit breaker: it lets users delegate proxy access to agents while retaining the ability to review and allow or deny specific calls before they happen.

The key property is that approvals are **blocking and synchronous from the agent's perspective**. The agent sends a proxy request; if approval is required, the HTTP connection stays open while NyxID waits for the user's decision. If the user approves, the downstream response is returned directly — no retry, no callback. If the user rejects or the timeout expires, the agent receives a `403`.

## What triggers approval

Approval is triggered for all **programmatic authentication methods**: API keys, delegated tokens, and service accounts. Direct browser sessions (session-cookie auth) bypass the approval system — approvals are for non-interactive callers.

New users require approval for programmatic requests by default, even before opening notification settings. Newly created settings retain that default. Existing explicit false preferences and legacy rows missing the field retain their previous false value; they are not rewritten. The global default exempts auto-connected, credential-free services. Explicit per-service `ServiceApprovalConfig` and org policies retain their precedence, including explicit auto-allow/deny and existing grant behavior. Users can manage these policies through the CLI or web console.

Required approval remains enforced when notifications are unavailable. Deploy this behavior to **all serving backend replicas and drain old binaries** before relying on it. Readiness observations cannot discover a hidden old replica; see the repository [security rollout contract](https://github.com/ChronoAIProject/NyxID/blob/main/docs/SECURITY_ROLLOUT_READINESS.md).

## Two approval modes

### Per-request mode (default)

Every gated proxy call creates a fresh pending approval request and attempts configured notifications. No grants are created. The approver sees a human-readable action description that describes what the specific request will do:

```
POST /v1/chat/completions
model: gpt-4o
3 messages
```

This mode gives the highest level of visibility. The approver knows exactly what operation is about to be executed. The trade-off is that every individual call requires a decision.

### Grant mode

When a service is configured with `approval_mode: grant`, approving a request creates a reusable time-based grant (default TTL: 30 days). Subsequent requests from the same agent to the same service reuse the existing grant without prompting again, until the grant expires or is revoked.

Grant mode is suitable for workflows where the approver wants to authorize a class of access ("this agent may call OpenAI for the next month") rather than individual operations.

```bash
# Enable approvals in per-request mode (default)
nyxid approval set-config <SERVICE_ID> --require-approval true

# Enable approvals in grant mode
nyxid approval set-config <SERVICE_ID> --require-approval true --approval-mode grant
```

:::note
Grant mode is the older behavior. Per-request mode is the default for newly-enabled approval configurations. Use grant mode when per-request approval is too granular for the workflow.
:::

## Notification channels

When an approval request is created, NyxID notifies the resource owner through configured notification channels. Supported channels:

- **Telegram** — the bot sends a message with Approve and Reject inline keyboard buttons. The decision is processed via the webhook or long-polling connection.
- **Mobile push** — the iOS/Android authenticator app receives a push notification.
- **Web console** — the durable pending request appears in [Approval History](/approvals/history) and can be decided there. Use **Refresh** to discover a request created after opening the page, including when the list is initially empty.

Telegram and push are optional delivery routes on a server that explicitly advertises required web approval. A registered device or chat alone does not establish initialized server credentials or successful delivery. An unavailable provider, disconnected channel, invalid token, or failed send leaves the request pending for a web decision; expiry does not run the operation. Older servers without this capability may suspend protection, so older/unknown-server status must be taken literally.

All channels share the same approval record. The first response wins; if the user approves via Telegram, subsequent mobile notifications are updated to reflect the decision.

## Approval flow

```
Agent ──POST /proxy/s/openai/…──> NyxID
                                     │
                               approve required?
                                     │ yes
                               existing grant?
                                     │ no
                               create approval request
                               notify user (Telegram / mobile / web)
                                     │
                               hold HTTP connection
                               poll DB every 1 second
                                     │
                         User approves (Telegram button)
                                     │
                               create grant (grant mode only)
                               continue proxy pipeline
                                     │
Agent ◄──downstream response────────┘
```

Pending requests auto-expire after the configured timeout (300 seconds for newly created settings). Existing timeouts remain unchanged; legacy rows with no timeout still read as 30 seconds. Keep Approval History open and refresh before the deadline when using web-only decisions. A background sweep runs every 5 seconds to mark timed-out requests as expired and update any Telegram messages.

## Org approval policies

When a service is org-owned, the org can set a service-level approval policy that applies to every member. The policy is dominant: even if a member has turned off personal approvals, the org policy still gates their requests. When an org policy fires, NyxID fans out notifications to all org admins — any of them can approve on behalf of the org.

If an org has no admins (which should not happen in normal operation but can occur if the last admin is removed), NyxID refuses the request with `503` rather than allowing self-approval by a member.

In grant mode under an org policy, the grant is owned by the org, not the individual member. The first approval covers subsequent requests from any member of the same org to that service.

## What approval cannot do

Approvals operate at the proxy boundary — they gate whether the HTTP request is forwarded. They do not:

- Inspect or modify response bodies.
- Gate requests made directly to the downstream API (outside NyxID's proxy).
- Apply to requests made with a session cookie (browser sessions bypass approvals by design).
- Persist request bodies beyond what is needed to build the action description.

## Related guides

- [Approvals for agents](/docs/ai/guides/approvals-for-agents)
- [Approvals in the web console](/docs/web/guides/approvals)
- [The proxy](/docs/shared/concepts/the-proxy)
- [Agent isolation](/docs/shared/concepts/agent-isolation)
- [Organizations](/docs/shared/concepts/organizations)
