# NyxID specs: where to look

This is the map of NyxID's specs. The same map in machine-readable form is
[`spec-index.yaml`](spec-index.yaml); agents match the code path they are editing
against its `governs` patterns and use each spec's `covers` line to choose what to
read. If several areas match, start with the most specific pattern. Agent rules for keeping specs current are in
[`AGENTS.md`](../AGENTS.md) ("Specs: keep them current as you work").

## Find the spec for an area

| Area | Governing spec | Main code |
|---|---|---|
| NyxBot (NyxAgent engine, agents, groups) | [chat/08-nyxagent-engine.md](chat/08-nyxagent-engine.md) (engine contract); [chat/09-nyxbot-orchestrator.md](chat/09-nyxbot-orchestrator.md) (team and channel design); [chat/12-shared-agents-and-groups.md](chat/12-shared-agents-and-groups.md); [NYXBOT_SERVICE_EXECUTION.md](NYXBOT_SERVICE_EXECUTION.md); [NYXBOT_FIRST_RUN.md](NYXBOT_FIRST_RUN.md) | `backend/src/services/assistant_*`, `backend/src/handlers/assistant_*`, `frontend/src/components/assistant/` |
| Channel bots | [CHANNEL_BOT_RELAY.md](CHANNEL_BOT_RELAY.md); [CHANNEL_EVENT_GATEWAY.md](CHANNEL_EVENT_GATEWAY.md); [CHANNEL_COMPLETE_REPLIES.md](CHANNEL_COMPLETE_REPLIES.md); [CHANNEL_ACTIVITY.md](CHANNEL_ACTIVITY.md); [CHANNEL_THREAD_FOLLOW.md](CHANNEL_THREAD_FOLLOW.md); [CHANNEL_THREAD_FOLLOW_GATEWAY_CONTRACT.md](CHANNEL_THREAD_FOLLOW_GATEWAY_CONTRACT.md); [TELEGRAM_NEW.md](TELEGRAM_NEW.md); [PLATFORM_BOTFATHER.md](PLATFORM_BOTFATHER.md); [AURINKO_INTEGRATION.md](AURINKO_INTEGRATION.md) | `backend/src/services/channel_*`, `backend/src/services/channel_adapters/`, `backend/src/services/nyxbot_gateway_*`, `backend/src/services/telegram_*`, `backend/src/services/whatsapp_native*` |
| Automations | [NYXBOT_SCHEDULES.md](NYXBOT_SCHEDULES.md); [API.md](API.md) ("Inbound Triggers") | `backend/src/services/trigger_*`, `backend/src/handlers/trigger*`, `backend/src/handlers/assistant_schedules.rs` |
| Notifications | [chat/12-notifications.md](chat/12-notifications.md) (NyxBot feed); [MOBILE_APP_INTEGRATION.md](MOBILE_APP_INTEGRATION.md) (push); [API.md](API.md) (settings, devices) | `backend/src/services/assistant_notification_service.rs`, `backend/src/services/notification_service*`, `backend/src/handlers/notifications.rs`, `backend/src/handlers/device_tokens.rs` |
| Approvals | [EXECUTION_PERMISSIONS.md](EXECUTION_PERMISSIONS.md); [GRANULAR_APPROVALS_DESIGN.md](GRANULAR_APPROVALS_DESIGN.md) (design); [API.md](API.md) (Transaction Approval, approval routes); [MOBILE_APP_INTEGRATION.md](MOBILE_APP_INTEGRATION.md) (approval pushes); guides under `site/` | `backend/src/services/approval_*`, `backend/src/handlers/approvals.rs`, `backend/src/services/execution_permission_*` |
| Sandboxes | [chat/12-cma-sessions.md](chat/12-cma-sessions.md); [CMA_BILLING.md](CMA_BILLING.md); [REMOTE_NYX_MACHINE_FINAL_READ_CONTRACTS.md](REMOTE_NYX_MACHINE_FINAL_READ_CONTRACTS.md); [REMOTE_MACHINE_DELETE_RECOVERY.md](REMOTE_MACHINE_DELETE_RECOVERY.md); [chat/10-uploads.md](chat/10-uploads.md) (generated files) | `backend/src/services/assistant_cma_*`, `backend/src/services/remote_machine_*`, `backend/src/services/billing/sandbox_*`, `backend/src/services/generated_artifacts/` |

"Sandbox" means three different things: CMA sandboxes (an external cloud
environment NyxID manages one slot of per account), the Kubernetes Agent Sandbox
object under a hosted remote machine, and the `chrono-sandbox` execution
service behind generated files.

## Shared registries

| Fact | Code | Spec |
|---|---|---|
| Routes | `backend/src/routes.rs` | [API.md](API.md) |
| Environment variables and defaults | `backend/src/config.rs` | [ENV.md](ENV.md) |
| Error codes | `backend/src/errors/mod.rs` | [AGENTS.md](../AGENTS.md), Critical Rule 3 |
| Collections | `backend/src/db.rs`, `backend/src/services/personal_data_registry.rs` | [security/PERSONAL_ERASURE_INVENTORY.md](security/PERSONAL_ERASURE_INVENTORY.md) |

## Areas not yet indexed

Start from these docs and the Critical Rules in [AGENTS.md](../AGENTS.md):

- Auth and login: [ADR-015-auth-device-login.md](ADR-015-auth-device-login.md), [DEVICE_LOGIN_PROTOCOL.md](DEVICE_LOGIN_PROTOCOL.md)
- Billing: [BILLING_EXACT_ACCOUNTING.md](BILLING_EXACT_ACCOUNTING.md), [BILLING_OPERATIONS.md](BILLING_OPERATIONS.md), [PLATFORM_KEYS_AND_INFERENCE.md](PLATFORM_KEYS_AND_INFERENCE.md)
- Catalog and services: [API_DISCOVERY.md](API_DISCOVERY.md), [AI_SERVICES_ARCHITECTURE.md](AI_SERVICES_ARCHITECTURE.md), [CATALOG_MANAGER.md](CATALOG_MANAGER.md), [TOOLS.md](TOOLS.md)
- Agent keys and isolation: [AGENT_ISOLATION.md](AGENT_ISOLATION.md), [AGENT_OPERATION_SCOPES.md](AGENT_OPERATION_SCOPES.md), [SERVICE_ACCOUNTS.md](SERVICE_ACCOUNTS.md)
- Nodes and machines: [NODE_PROXY_ARCHITECTURE.md](NODE_PROXY_ARCHITECTURE.md), [MACHINE_NODES.md](MACHINE_NODES.md), [NYXID_NODE.md](NYXID_NODE.md), [SSH_NODE_KEY_AUTH.md](SSH_NODE_KEY_AUTH.md)
- OAuth and security rollout: [OIDC.md](OIDC.md), [security/OAUTH_AUTHORITY_ROLLOUT.md](security/OAUTH_AUTHORITY_ROLLOUT.md), [SECURITY_ROLLOUT_READINESS.md](SECURITY_ROLLOUT_READINESS.md), [EXECUTION_PERMISSIONS.md](EXECUTION_PERMISSIONS.md)
- Plugins: [PLUGINS.md](PLUGINS.md)
- Deployment: [DEPLOYMENT.md](DEPLOYMENT.md), [ENV.md](ENV.md)

## Docs that code reads directly

Some docs are shipped to models at runtime, read by tests, or built into the
frontend. They are listed under `input_of` in [spec-index.yaml](spec-index.yaml).
Do not edit runtime docs unless the task asks for it, run the named test when
you edit a test-read doc, and edit only existing pages under `site/`.

## Kinds of documents

| Kind | Purpose | How it changes |
|---|---|---|
| Reference | Current behaviour | Edited in place in the same PR as the code |
| Design | Why a design was chosen | Dated; not rewritten later |
| Runbook | Rollout, readiness, drain, rollback, operator steps | Kept with the feature it operates |
| Guide | Tasks for users and operators (`site/`) | Edited with the product |

Plans, reviews and verification logs are working records. When one is
superseded it carries a status line pointing to the current spec.

Specs are checked against the code nightly; see [SPEC_MAINTENANCE.md](SPEC_MAINTENANCE.md).
