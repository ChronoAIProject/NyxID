# NyxID Chatbot — 3rd Party AI Service Integration Spec

> **Version:** 1.0
> **Date:** 2026-03-31
> **Audience:** 3rd party AI/NLP service implementor
> **Status:** Draft

---

## Table of Contents

1. [Overview](#1-overview)
2. [Transport Protocol](#2-transport-protocol)
3. [Request Payload](#3-request-payload)
4. [Response Payload](#4-response-payload)
5. [FAQ Knowledge Base](#5-faq-knowledge-base)
6. [Action Registry](#6-action-registry)
7. [Role-Based Access Control](#7-role-based-access-control)
8. [Data Security Policy](#8-data-security-policy)
9. [Error Handling](#9-error-handling)
10. [Example Flows](#10-example-flows)

---

## 1. Overview

### What is NyxID?

NyxID is a secure API proxy gateway. Users store their external API credentials (OpenAI, Anthropic, GitHub, etc.) once, and NyxID injects them into requests automatically — keys never leave the server. NyxID provides authentication, SSO (OpenID Connect / OAuth 2.0), MFA, API key management, transaction approvals, on-premise credential nodes, an LLM gateway, and MCP tool exposure for AI clients.

### What is the chatbot?

NyxID includes an in-app chat assistant available on both the React web dashboard and React Native mobile app. The chatbot answers product questions (FAQ) and performs platform actions on behalf of the user (create API keys, add services, approve requests, etc.).

### Your role

The 3rd party AI service handles **NLP only**:

- **Intent classification** — determine what the user wants (FAQ topic, platform action, chitchat, or unknown)
- **Natural language response** — generate a conversational reply
- **Parameter extraction** — for action intents, extract structured parameters from the user's message

### NyxID retains

- All action execution (API calls, database writes)
- Secret/credential handling
- Confirmation flows (user must confirm destructive actions)
- Authentication and authorization
- Client-side conversation state management

### Architecture principles

- **Stateless** — no server-side chat history. Conversation context is round-tripped by the client and forwarded to you in each request.
- **One-way** — NyxID calls your service. You never contact NyxID APIs directly.
- **No secrets in transit** — secret parameters (API keys, OAuth credentials) are never sent to you and must never be extracted from user messages.

---

## 2. Transport Protocol

### 2.1 Sync POST (Minimum Viable)

NyxID sends a JSON POST request and expects a JSON response.

```
POST /classify
Authorization: Bearer <your-api-token>
Content-Type: application/json

{request payload}
```

**Timeout:** 5 seconds. If your service does not respond within 5s, NyxID falls back to keyword-based classification.

**Response:** `Content-Type: application/json` with the response payload defined in [Section 4](#4-response-payload).

### 2.2 SSE Streaming (Optional Enhancement)

For progressive display of the `reply` field, your service may optionally support Server-Sent Events:

```
POST /classify/stream
Authorization: Bearer <your-api-token>
Content-Type: application/json
Accept: text/event-stream

{request payload}
```

**Event format:**

```
event: delta
data: {"reply": "partial text..."}

event: done
data: {"intent": "...", "intent_type": "...", "reply": "full text", "params": {...}, "context_summary": "..."}
```

- Stream `delta` events with progressive `reply` text for real-time display
- The `done` event contains the complete structured response (all fields)
- Structured fields (`intent`, `intent_type`, `params`) must appear in the `done` event

### 2.3 Authentication

All requests include a `Bearer` token in the `Authorization` header. The token is a shared secret configured on both sides.

### 2.4 Error Response Format

On error, return a JSON body with the following structure:

```json
{
  "error": {
    "code": "string",
    "message": "string"
  }
}
```

Error codes should be descriptive strings (e.g. `"invalid_request"`, `"rate_limited"`, `"internal_error"`).

---

## 3. Request Payload

NyxID sends the following JSON per chat message:

```json
{
  "message": "string",
  "context": [
    { "role": "user", "content": "string" },
    { "role": "assistant", "content": "string" }
  ],
  "is_admin": false,
  "pending_action": {
    "action": "string",
    "collected_params": {},
    "missing_params": ["string"],
    "awaiting_confirmation": false
  },
  "context_summary": "string | null"
}
```

### Field Reference

| Field | Type | Required | Description |
|---|---|---|---|
| `message` | string | Yes | The user's chat message. May be empty string when the user submitted secrets via a secure UI (treat as a continuation of the pending action). |
| `context` | array | Yes | Last 5 message pairs (max 10 entries), oldest first. Each entry has `role` (`"user"` or `"assistant"`) and `content` (string). Empty array on first message. |
| `is_admin` | boolean | Yes | Whether the user has admin privileges. Controls which actions are valid (see [Section 7](#7-role-based-access-control)). |
| `pending_action` | object or null | No | Present when a multi-turn action is in progress. `null` when no action is pending. |
| `pending_action.action` | string | — | The action key (from [Section 6](#6-action-registry)). |
| `pending_action.collected_params` | object | — | Parameters already collected. Keys are parameter names, values are strings/numbers. **Never contains secret parameters.** |
| `pending_action.missing_params` | array | — | Parameter names still needed. |
| `pending_action.awaiting_confirmation` | boolean | — | `true` when all params are collected and the user needs to confirm. |
| `context_summary` | string or null | No | One-line natural language summary of the current conversation state, echoed back from your previous response. Helps with continuity across turns. |

### Important Notes

- **`secret_input` is NOT sent.** When users provide credentials (API keys, OAuth secrets), they enter them through a secure UI popup on NyxID's side. The secret values never reach your service. When all non-secret params are collected but secret params remain, NyxID will handle the secure collection — you just need to indicate which non-secret params are still missing.
- **Empty `message`:** When `message` is empty and `pending_action` is present, it means the user just submitted secrets via the secure UI. Treat this as a continuation — the pending action should advance (typically to the confirmation step).
- **Context window:** `context` contains at most 10 entries (5 user + 5 assistant messages). The client trims older messages. Context resets on page refresh.

---

## 4. Response Payload

Your service must return:

```json
{
  "intent": "string",
  "intent_type": "string",
  "reply": "string",
  "context_summary": "string | null",
  "params": {}
}
```

### Field Reference

| Field | Type | Required | Description |
|---|---|---|---|
| `intent` | string | Yes | One of: the 15 FAQ keys ([Section 5](#5-faq-knowledge-base)), the 20 action keys ([Section 6](#6-action-registry)), `"chitchat"`, or `"unknown"`. |
| `intent_type` | string | Yes | Category bucket: `"faq"`, `"action"`, `"chitchat"`, `"unknown"`, or `"continue"`. |
| `reply` | string | Yes | Natural language text shown to the user. Max ~500 characters recommended. Should be conversational and helpful. |
| `context_summary` | string or null | No | One-line state summary echoed back in the next request's `context_summary` field. Useful for multi-turn actions (e.g. `"Creating API key 'prod-key'. Waiting for: expiry."`). Set to `null` when no ongoing state. |
| `params` | object | No | Only for `intent_type: "action"` or `"continue"`. Flat JSON of extracted parameters. Keys must match the parameter names defined in the action registry. Omit or set to `{}` for FAQ/chitchat/unknown. |

### Intent and Intent Type Mapping

| intent_type | Valid intent values |
|---|---|
| `faq` | `what_is_nyxid`, `authentication`, `credential_broker`, `llm_gateway`, `transaction_approval`, `mcp_integration`, `credential_nodes`, `api_keys`, `proxy`, `security`, `oauth_oidc`, `mfa`, `setup`, `mobile_app`, `use_cases` |
| `action` | `get_profile`, `list_api_keys`, `list_services`, `list_catalog`, `list_nodes`, `list_approvals`, `check_llm_status`, `list_endpoints`, `list_external_keys`, `create_api_key`, `rotate_api_key`, `delete_api_key`, `add_service`, `delete_service`, `route_service`, `set_service_credentials`, `approve_request`, `deny_request`, `list_users`, `list_service_accounts` |
| `continue` | Same action key as the `pending_action.action` from the request |
| `chitchat` | `chitchat` |
| `unknown` | `unknown` |

### `params` Rules

- **Only include parameters you can confidently extract** from the user's message. Omit uncertain values.
- **Never extract secret parameters** (`credential`, `client_id`, `client_secret`). See [Section 8.3](#83-secret-parameters).
- For `continue` intent type, include only the **newly extracted** parameters from the current message. NyxID merges them with `collected_params`.
- Parameter names must exactly match the names in the action registry.

---

## 5. FAQ Knowledge Base

Your service must be able to classify and respond to questions about these 15 topics. The reference answers below define the canonical, accurate information about each topic. When responding, **rephrase naturally** based on the user's specific question — do not return the reference answer verbatim. Do not invent features that are not mentioned in the reference answer.

---

### 5.1 `what_is_nyxid` — What is NyxID?

**Keywords:** what, nyxid, about, overview, explain, introduction, platform

**Reference answer:**
> NyxID is a proxy gateway that lets you access any API through it safely and in a controlled manner. You store your API credentials once, and NyxID injects them into requests automatically — your keys never leave the server. To support this, NyxID provides authentication, SSO (OpenID Connect/OAuth 2.0), MFA, API key management, transaction approvals, on-premise credential nodes, an LLM gateway, and MCP tool exposure for AI clients like Cursor and Claude Code.

**Related topics:** `proxy`, `credential_broker`, `use_cases`

---

### 5.2 `authentication` — Authentication & Login

**Keywords:** login, sign in, register, auth, password, social, google, github, session, JWT

**Reference answer:**
> NyxID supports email/password registration, plus social login via Google and GitHub. You can also log in from the mobile app or the nyxid CLI. Once logged in, you can enable MFA for extra security.

**Related topics:** `mfa`, `oauth_oidc`, `security`

---

### 5.3 `credential_broker` — Credential Broker / Vault

**Keywords:** credential, broker, vault, api key, secret, inject, store, encrypt

**Reference answer:**
> The Credential Broker stores your external API credentials securely, encrypted at rest. When you proxy a request to a downstream service, NyxID automatically injects your stored credential into the request. Your API keys never leave the server. You can connect external providers like OpenAI, Anthropic, and Google AI through catalog templates or custom endpoints.

**Related topics:** `proxy`, `security`, `api_keys`

---

### 5.4 `llm_gateway` — LLM Gateway

**Keywords:** llm, openai, anthropic, claude, gemini, gateway, model, ai proxy, chatgpt

**Reference answer:**
> NyxID includes an LLM gateway with provider-specific proxy and an OpenAI-compatible gateway that routes by model name and translates API formats. Supported providers include OpenAI, Anthropic, Google AI, Mistral, Cohere, and DeepSeek. The gateway automatically translates OpenAI-format requests to Anthropic (Claude) format. Access the provider proxy at /api/v1/llm/{provider}/v1/{path} or the unified gateway at /api/v1/llm/gateway/v1/{path}.

**Related topics:** `proxy`, `credential_broker`, `use_cases`

---

### 5.5 `transaction_approval` — Transaction Approvals

**Keywords:** approval, approve, deny, transaction, permission, grant, request, authorize

**Reference answer:**
> Approvals require users to manually approve sensitive proxy, LLM gateway, and SSH requests. NyxID supports two modes per service: per-request approval (every request needs fresh approval) and grant-based approval (once approved, access is granted for a configurable period of 1-365 days). Approval notifications are delivered via the web dashboard, Telegram bot, or mobile push notifications on iOS and Android.

**Related topics:** `security`, `mobile_app`, `proxy`

---

### 5.6 `mcp_integration` — MCP (Model Context Protocol) Integration

**Keywords:** mcp, model context protocol, cursor, claude code, codex, ai tool, tool, ai setup

**Reference answer:**
> Yes. NyxID exposes an MCP endpoint that aggregates all your connected service APIs into a single tool list for AI clients like Cursor, Claude Code, and Codex. If the web dashboard feels overwhelming, you can manage NyxID entirely through MCP, the nyxid CLI, or even the mobile app — most features are available across all interfaces. Configure MCP once in your AI client settings, authenticate via OAuth, and all your connected services become available as tools.

**Related topics:** `use_cases`, `llm_gateway`, `proxy`

---

### 5.7 `credential_nodes` — Credential Nodes

**Keywords:** node, agent, on-premise, local, self-hosted, websocket, daemon

**Reference answer:**
> A credential node is a lightweight agent you run on your own infrastructure via the nyxid node CLI. It connects to NyxID, holds credentials locally, and proxies requests without your secrets ever leaving your machine. Nodes support streaming, multi-node failover with priority routing, and can run as a background service on macOS or Linux.

**Related topics:** `credential_broker`, `security`, `proxy`

---

### 5.8 `api_keys` — API Key Management

**Keywords:** api key, key, token, create key, rotate, revoke, scope, programmatic

**Reference answer:**
> NyxID API keys allow programmatic access to the platform. Each key has optional scopes (read, write, proxy), expiration dates, and last-used tracking. You can restrict which external services and credential nodes a key can access. Create, list, rotate, and revoke keys from the dashboard, CLI, or this chatbot.

**Related topics:** `authentication`, `security`, `credential_broker`

---

### 5.9 `proxy` — API Proxy

**Keywords:** proxy, forward, request, downstream, slug, endpoint, route, passthrough

**Reference answer:**
> NyxID's proxy intercepts your HTTP requests to downstream services, looks up your stored credential, injects it automatically, and forwards the request. You can access services via friendly slug-based URLs or by service ID. The proxy supports streaming responses and includes built-in security protections.

**Related topics:** `credential_broker`, `llm_gateway`, `credential_nodes`

---

### 5.10 `security` — Security & Encryption

**Keywords:** security, encryption, encrypted, safe, secure, aes, password, hash, protection

**Reference answer:**
> Yes. All sensitive data including credentials, tokens, and MFA secrets are encrypted at rest. Passwords are securely hashed and never stored in plaintext. Encryption keys can be rotated with zero downtime. Additional protections include rate limiting, security headers, PKCE for OAuth flows, and SSRF protection on the proxy.

**Related topics:** `credential_broker`, `authentication`, `mfa`

---

### 5.11 `oauth_oidc` — OAuth / OIDC / SSO

**Keywords:** oauth, oidc, openid, sso, single sign-on, authorization, pkce, token, identity provider

**Reference answer:**
> Yes. NyxID is a full OpenID Connect provider. Register OAuth clients to add 'Sign in with NyxID' to your apps. It issues ID tokens and access tokens, exposes a UserInfo endpoint, and supports token introspection and revocation. Service accounts can authenticate via Client Credentials Grant for server-to-server access.

**Related topics:** `authentication`, `security`, `use_cases`

---

### 5.12 `mfa` — Multi-Factor Authentication

**Keywords:** mfa, 2fa, two-factor, totp, authenticator, google authenticator, authy, recovery

**Reference answer:**
> NyxID supports TOTP-based multi-factor authentication compatible with Google Authenticator, Authy, and 1Password. Set up MFA from your account settings — you'll get a QR code to scan with your authenticator app. Recovery codes are generated for account recovery if you lose your device.

**Related topics:** `authentication`, `security`

---

### 5.13 `setup` — Getting Started / Setup

**Keywords:** setup, install, getting started, start, configure, docker, deploy, quick start, environment

**Reference answer:**
> To get started: (1) Run Docker Compose to start MongoDB and Mailpit. (2) Generate RSA keys for JWT signing. (3) Set environment variables (DATABASE_URL, ENCRYPTION_KEY, and optionally SMTP, Telegram, push notification settings). (4) Start the backend with cargo run and frontend with npm run dev. (5) Create an account and log in. (6) Add services from the catalog or CLI. Optionally install the nyxid CLI for command-line management.

**Related topics:** `what_is_nyxid`, `use_cases`

---

### 5.14 `mobile_app` — Mobile App

**Keywords:** mobile, app, ios, android, phone, react native, expo, push notification

**Reference answer:**
> NyxID includes a React Native mobile app for iOS and Android built with Expo. The app lets you view and approve transaction approval requests, manage active grants, and receive push notifications via APNs (iOS) and FCM (Android). It supports deep linking for approval challenges and secure token storage via the OS keychain.

**Related topics:** `transaction_approval`, `what_is_nyxid`

---

### 5.15 `use_cases` — Use Cases

**Keywords:** use case, what for, example, scenario, purpose, why, benefit

**Reference answer:**
> NyxID is primarily a secure API proxy gateway. Common use cases: (1) API credential brokering — proxy requests to any API with automatic credential injection. (2) LLM gateway — unified proxy to OpenAI, Anthropic, Google AI, and other providers with format translation. (3) MCP AI tool exposure — let Cursor, Claude Code, and Codex call your APIs as tools. (4) On-premise credential management — run nodes to keep secrets on your infrastructure. (5) Identity federation — add 'Sign in with NyxID' to your apps via OIDC. (6) SSH bridging — manage SSH access with short-lived certificates and approval workflows.

**Related topics:** `what_is_nyxid`, `credential_broker`, `llm_gateway`, `mcp_integration`

---

## 6. Action Registry

Your service must classify action intents and extract parameters. NyxID handles all execution — you just identify what the user wants to do and extract the relevant parameters.

### How Actions Work

1. User says something like "create an API key called prod-key"
2. Your service classifies intent as `create_api_key` and extracts `params: { "name": "prod-key" }`
3. NyxID checks if all required params are present — if not, it asks the user for missing ones
4. For write actions, NyxID asks the user for confirmation before executing
5. NyxID executes the action via its internal API

### Parameter Extraction Guidelines

- Extract only parameters you are confident about from the user's message
- Use the parameter names exactly as defined below
- For `continue` intents, extract only the **new** parameters from the current turn
- **Never extract parameters marked as secret** (see [Section 8.3](#83-secret-parameters))

---

### 6.1 Read Actions — No Confirmation Required (9 actions)

These actions retrieve data and do not modify anything. No confirmation step is needed.

#### `get_profile`

| | |
|---|---|
| **Description** | Show your account info |
| **Example** | "show my profile" |
| **Parameters** | None |

#### `list_api_keys`

| | |
|---|---|
| **Description** | List your NyxID API keys |
| **Example** | "show my API keys" |
| **Parameters** | None |

#### `list_services`

| | |
|---|---|
| **Description** | List your configured services |
| **Example** | "what services do I have?" |
| **Parameters** | None |

#### `list_catalog`

| | |
|---|---|
| **Description** | Browse available services from the catalog |
| **Example** | "show the service catalog" |
| **Parameters** | None |

#### `list_nodes`

| | |
|---|---|
| **Description** | List your credential nodes |
| **Example** | "show my nodes" |
| **Parameters** | None |

#### `list_approvals`

| | |
|---|---|
| **Description** | List pending approval requests |
| **Example** | "show pending approvals" |
| **Parameters** | None |

#### `check_llm_status`

| | |
|---|---|
| **Description** | Check LLM provider status |
| **Example** | "is OpenAI up?" |
| **Parameters** | None |

#### `list_endpoints`

| | |
|---|---|
| **Description** | List your endpoints |
| **Example** | "show my endpoints" |
| **Parameters** | None |

#### `list_external_keys`

| | |
|---|---|
| **Description** | List your external API keys and credentials |
| **Example** | "show my external credentials" |
| **Parameters** | None |

---

### 6.2 Write Actions — Confirmation Required (7 actions)

These actions create, modify, or delete resources. NyxID will ask the user for confirmation before executing.

#### `create_api_key`

| | |
|---|---|
| **Description** | Create a new NyxID API key |
| **Example** | "create an API key called prod-key" |
| **Confirmation message** | "Create API key '{name}'" |

**Parameters:**

| Name | Type | Required | Secret | Description |
|---|---|---|---|---|
| `name` | string | Yes | No | Name for the key |
| `scopes` | string | No | No | Space-separated scopes (e.g. `"read write proxy"`) |
| `expires_in_days` | number | No | No | Expiry in days (`0` = no expiry) |

#### `rotate_api_key`

| | |
|---|---|
| **Description** | Rotate an API key (old key stops working immediately) |
| **Example** | "rotate my API key abc123" |
| **Confirmation message** | "Rotate API key {key_id}. The old key will stop working immediately." |

**Parameters:**

| Name | Type | Required | Secret | Description |
|---|---|---|---|---|
| `key_id` | string | Yes | No | ID of the API key to rotate |

#### `delete_api_key`

| | |
|---|---|
| **Description** | Delete an API key |
| **Example** | "delete API key abc123" |
| **Confirmation message** | "Delete API key {key_id}. This cannot be undone." |

**Parameters:**

| Name | Type | Required | Secret | Description |
|---|---|---|---|---|
| `key_id` | string | Yes | No | ID of the API key to delete |

#### `add_service`

| | |
|---|---|
| **Description** | Add an AI service from the catalog |
| **Example** | "add openai service" |
| **Confirmation message** | "Add {service_slug} service '{label}'" |

**Parameters:**

| Name | Type | Required | Secret | Description |
|---|---|---|---|---|
| `service_slug` | string | Yes | No | Catalog service slug (e.g. `"llm-openai"`, `"llm-anthropic"`, `"llm-google-ai"`) |
| `label` | string | Yes | No | User-chosen name for this service |
| `credential` | string | No | **Yes** | API key or bearer token for the service |
| `endpoint_url` | string | No | No | Endpoint URL override (for self-hosted instances) |
| `node_id` | string | No | No | Route through this credential node |

**Secret input labels** (context-specific UI copy for the `credential` field):

| When `service_slug` is | Label | Description | Placeholder |
|---|---|---|---|
| `llm-openai` | OpenAI API Key | Paste your API key from the OpenAI dashboard. Starts with sk- | `sk-proj-...` |
| `llm-anthropic` | Anthropic API Key | Paste your API key from console.anthropic.com | `sk-ant-...` |
| `llm-google-ai` | Google AI API Key | Paste your Gemini API key from aistudio.google.com | `AI...` |
| _(any other)_ | API Key | Paste the API key or bearer token for this service | `your-api-key` |

#### `delete_service`

| | |
|---|---|
| **Description** | Delete a configured service |
| **Example** | "delete my openai service" |
| **Confirmation message** | "Delete service {service_id}. This cannot be undone." |

**Parameters:**

| Name | Type | Required | Secret | Description |
|---|---|---|---|---|
| `service_id` | string | Yes | No | Service ID to delete |

#### `route_service`

| | |
|---|---|
| **Description** | Change service routing (node or direct) |
| **Example** | "route my openai service through node-abc" |
| **Confirmation message** | "Route service {service_id} through node {node_id}" |

**Parameters:**

| Name | Type | Required | Secret | Description |
|---|---|---|---|---|
| `service_id` | string | Yes | No | Service ID to update |
| `node_id` | string | Yes | No | Node ID to route through, or empty string for direct routing |

#### `set_service_credentials`

| | |
|---|---|
| **Description** | Set OAuth credentials for a service's provider |
| **Example** | "set credentials for github" |
| **Confirmation message** | "Set OAuth credentials for {slug}" |

**Parameters:**

| Name | Type | Required | Secret | Description |
|---|---|---|---|---|
| `slug` | string | Yes | No | Catalog service slug (e.g. `"github"`) |
| `client_id` | string | Yes | **Yes** | OAuth Client ID |
| `client_secret` | string | No | **Yes** | OAuth Client Secret |

**Secret input labels:**

| Parameter | Label | Description | Placeholder |
|---|---|---|---|
| `client_id` | OAuth Client ID | The client ID from your OAuth app registration | `your-client-id` |
| `client_secret` | OAuth Client Secret | The client secret from your OAuth app. Will be encrypted and stored securely. | `your-client-secret` |

---

### 6.3 Approval Actions (2 actions)

These actions decide on pending approval requests. No confirmation step is needed — the user's explicit "approve" or "deny" intent is sufficient.

#### `approve_request`

| | |
|---|---|
| **Description** | Approve a pending approval request |
| **Example** | "approve request abc123" |

**Parameters:**

| Name | Type | Required | Secret | Description |
|---|---|---|---|---|
| `request_id` | string | Yes | No | Approval request ID |
| `decision` | string | Yes (default: `"approved"`) | No | Always `"approved"`. Extract `request_id` only; NyxID fills the decision. |

#### `deny_request`

| | |
|---|---|
| **Description** | Deny a pending approval request |
| **Example** | "deny request abc123" |

**Parameters:**

| Name | Type | Required | Secret | Description |
|---|---|---|---|---|
| `request_id` | string | Yes | No | Approval request ID |
| `decision` | string | Yes (default: `"rejected"`) | No | Always `"rejected"`. Extract `request_id` only; NyxID fills the decision. |

---

### 6.4 Admin-Only Actions (2 actions)

These actions are **only valid when `is_admin` is `true`** in the request. See [Section 7](#7-role-based-access-control).

#### `list_users`

| | |
|---|---|
| **Description** | List all users |
| **Example** | "show all users" |
| **Parameters** | None |
| **Role required** | `admin` |

#### `list_service_accounts`

| | |
|---|---|
| **Description** | List service accounts |
| **Example** | "show service accounts" |
| **Parameters** | None |
| **Role required** | `admin` |

---

## 7. Role-Based Access Control

The `is_admin` boolean in the request determines which actions are available:

| `is_admin` | Available intents |
|---|---|
| `false` | 18 actions (all except `list_users` and `list_service_accounts`) + 15 FAQ topics + `chitchat` + `unknown` |
| `true` | All 20 actions + 15 FAQ topics + `chitchat` + `unknown` |

### Non-admin requesting admin action

If `is_admin` is `false` and the user's message matches an admin-only action (e.g. "show all users"), you must:

- Set `intent` to `"unknown"`
- Set `intent_type` to `"unknown"`
- Set `reply` to a message indicating this requires admin access, e.g. "That action requires admin access. I can help you with your own account, services, and API keys instead."

---

## 8. Data Security Policy

### 8.1 What NyxID Shares With You

- User's chat message text
- Conversation context (text only — last 5 message pairs)
- Admin boolean (`is_admin`)
- Pending action state (action key, non-secret collected parameters, missing parameter names, confirmation state)
- Context summary string

### 8.2 What NyxID Never Shares

- User IDs, email addresses, or any PII
- Authentication tokens (JWT, session tokens, API keys)
- API credentials or secrets (provider keys, OAuth tokens)
- Raw API response bodies from action execution
- Internal infrastructure details (database URLs, server addresses, encryption keys)

### 8.3 Secret Parameters

Parameters flagged as **secret** in the action registry (`credential`, `client_id`, `client_secret`) have special handling:

1. **Never sent to you** — NyxID never includes secret values in the request payload
2. **Never extract from messages** — if a user pastes an API key in their chat message, do **not** extract it into `params`. Instead, reply indicating that secure input is needed.
3. **Trigger secure UI** — when all non-secret required params are collected but secret params remain, your `reply` should indicate the user needs to provide credentials securely. NyxID will show a dedicated masked input popup.

**Example:** For `add_service`, once `service_slug` and `label` are collected, reply with something like: "Got it! I'll set up your OpenAI service called 'my-gpt'. I'll need your API key to continue — a secure input will appear for you to paste it."

### 8.4 Data Retention Expectations

- **No persistent storage** of user messages or conversation content
- **No training** on user data
- Debug/operational logs with **automated expiry** only (max 30 days)
- No sharing of user data with third parties

---

## 9. Error Handling

### Classification Errors

| Scenario | Expected behavior |
|---|---|
| Unclassifiable intent | Return `intent: "unknown"`, `intent_type: "unknown"`, with a helpful reply suggesting what you can help with |
| Empty `message` with no `pending_action` | Return `intent: "unknown"`, `intent_type: "unknown"`, reply asking what the user needs help with |
| Empty `message` with `pending_action` | Treat as continuation (secrets were submitted via secure UI). Return `intent_type: "continue"` with the pending action's intent |
| Non-admin requesting admin action | Return `intent: "unknown"`, `intent_type: "unknown"`, reply indicating admin access is required |
| Topic switch mid-action | Classify the new intent normally (e.g. FAQ or different action). NyxID will clear the pending action. |
| Very long message (>2000 chars) | Classify as best you can. If unable, return `unknown`. |

### HTTP Error Codes

| Status | When to return | Body |
|---|---|---|
| `200` | Successful classification (even for `unknown` intent) | Response payload |
| `400` | Malformed request (missing required fields, invalid JSON) | `{ "error": { "code": "invalid_request", "message": "..." } }` |
| `429` | Rate limited | `{ "error": { "code": "rate_limited", "message": "..." } }` |
| `500` | Internal server error | `{ "error": { "code": "internal_error", "message": "..." } }` |
| `503` | Service temporarily unavailable | `{ "error": { "code": "unavailable", "message": "..." } }` |

**Note:** A successful classification always returns `200`, even when the intent is `unknown`. HTTP errors are reserved for transport/infrastructure failures.

---

## 10. Example Flows

### Example 1: FAQ — "How does the credential broker work?"

**Request:**
```json
{
  "message": "how does the credential broker work?",
  "context": [],
  "is_admin": false,
  "pending_action": null,
  "context_summary": null
}
```

**Response:**
```json
{
  "intent": "credential_broker",
  "intent_type": "faq",
  "reply": "The Credential Broker securely stores your external API credentials, encrypted at rest. When you proxy a request through NyxID to a downstream service like OpenAI, it automatically injects your stored credential — so your API keys never leave the server. You can connect providers through catalog templates or set up custom endpoints.",
  "context_summary": null,
  "params": {}
}
```

---

### Example 2: Read Action — "Show my API keys"

**Request:**
```json
{
  "message": "show my API keys",
  "context": [],
  "is_admin": false,
  "pending_action": null,
  "context_summary": null
}
```

**Response:**
```json
{
  "intent": "list_api_keys",
  "intent_type": "action",
  "reply": "Let me fetch your API keys.",
  "context_summary": null,
  "params": {}
}
```

NyxID receives this, calls its own `GET /api/v1/api-keys` endpoint, and shows the results to the user.

---

### Example 3: Write Action with Multi-Turn — "Create an API key"

**Turn 1 — User initiates action:**

Request:
```json
{
  "message": "create an API key",
  "context": [],
  "is_admin": false,
  "pending_action": null,
  "context_summary": null
}
```

Response:
```json
{
  "intent": "create_api_key",
  "intent_type": "action",
  "reply": "I'll create an API key for you. What would you like to name it?",
  "context_summary": "Creating API key. Waiting for: name.",
  "params": {}
}
```

**Turn 2 — User provides the name:**

Request:
```json
{
  "message": "prod-key",
  "context": [
    { "role": "user", "content": "create an API key" },
    { "role": "assistant", "content": "I'll create an API key for you. What would you like to name it?" }
  ],
  "is_admin": false,
  "pending_action": {
    "action": "create_api_key",
    "collected_params": {},
    "missing_params": ["name"],
    "awaiting_confirmation": false
  },
  "context_summary": "Creating API key. Waiting for: name."
}
```

Response:
```json
{
  "intent": "create_api_key",
  "intent_type": "continue",
  "reply": "Got it — 'prod-key'. Any specific scopes (read, write, proxy) or expiry in days? Or I can create it with defaults.",
  "context_summary": "Creating API key 'prod-key'. Optional: scopes, expires_in_days.",
  "params": { "name": "prod-key" }
}
```

**Turn 3 — User confirms defaults:**

Request:
```json
{
  "message": "defaults are fine",
  "context": [
    { "role": "user", "content": "create an API key" },
    { "role": "assistant", "content": "I'll create an API key for you. What would you like to name it?" },
    { "role": "user", "content": "prod-key" },
    { "role": "assistant", "content": "Got it — 'prod-key'. Any specific scopes (read, write, proxy) or expiry in days? Or I can create it with defaults." }
  ],
  "is_admin": false,
  "pending_action": {
    "action": "create_api_key",
    "collected_params": { "name": "prod-key" },
    "missing_params": [],
    "awaiting_confirmation": true
  },
  "context_summary": "Creating API key 'prod-key'. Optional: scopes, expires_in_days."
}
```

Response:
```json
{
  "intent": "create_api_key",
  "intent_type": "continue",
  "reply": "Creating API key 'prod-key' with default settings. Confirmed!",
  "context_summary": null,
  "params": {}
}
```

NyxID receives the confirmation, executes `POST /api/v1/api-keys` with `{ "name": "prod-key" }`, and shows the result.

---

### Example 4: Action with Secrets — "Add openai service called my-gpt"

**Turn 1 — User provides slug and label in one message:**

Request:
```json
{
  "message": "add openai service called my-gpt",
  "context": [],
  "is_admin": false,
  "pending_action": null,
  "context_summary": null
}
```

Response:
```json
{
  "intent": "add_service",
  "intent_type": "action",
  "reply": "I'll set up an OpenAI service called 'my-gpt'. I'll need your API key to continue — a secure input will appear for you to paste it.",
  "context_summary": "Adding OpenAI service 'my-gpt'. Waiting for: API key (secure input).",
  "params": { "service_slug": "llm-openai", "label": "my-gpt" }
}
```

NyxID sees that `credential` (a secret parameter) is still needed. It shows a secure masked input popup to the user. The secret value is **never sent to your service**.

**Turn 2 — User submits secret via secure UI (empty message):**

Request:
```json
{
  "message": "",
  "context": [
    { "role": "user", "content": "add openai service called my-gpt" },
    { "role": "assistant", "content": "I'll set up an OpenAI service called 'my-gpt'. I'll need your API key to continue — a secure input will appear for you to paste it." }
  ],
  "is_admin": false,
  "pending_action": {
    "action": "add_service",
    "collected_params": { "service_slug": "llm-openai", "label": "my-gpt" },
    "missing_params": ["credential"],
    "awaiting_confirmation": false
  },
  "context_summary": "Adding OpenAI service 'my-gpt'. Waiting for: API key (secure input)."
}
```

Response:
```json
{
  "intent": "add_service",
  "intent_type": "continue",
  "reply": "Great, credentials received. Ready to add OpenAI service 'my-gpt'. Confirm?",
  "context_summary": "Adding OpenAI service 'my-gpt'. Awaiting confirmation.",
  "params": {}
}
```

NyxID merges the secret (which it collected directly) and asks for confirmation.

---

### Example 5: Topic Switch Mid-Action

User is in the middle of creating an API key, then asks an unrelated question.

**Request (mid-action):**
```json
{
  "message": "how does MFA work?",
  "context": [
    { "role": "user", "content": "create an API key" },
    { "role": "assistant", "content": "I'll create an API key for you. What would you like to name it?" }
  ],
  "is_admin": false,
  "pending_action": {
    "action": "create_api_key",
    "collected_params": {},
    "missing_params": ["name"],
    "awaiting_confirmation": false
  },
  "context_summary": "Creating API key. Waiting for: name."
}
```

**Response:**
```json
{
  "intent": "mfa",
  "intent_type": "faq",
  "reply": "NyxID supports TOTP-based two-factor authentication, compatible with Google Authenticator, Authy, and 1Password. You can set it up from your account settings — you'll get a QR code to scan. Recovery codes are also generated in case you lose your device.",
  "context_summary": null,
  "params": {}
}
```

The user clearly asked an FAQ question, not a parameter for the pending action. Your service classifies it as the `mfa` FAQ intent. NyxID will clear the pending action on its side. The `context_summary` is set to `null` since the previous action context is no longer relevant.

---

## Appendix A: Complete Intent Key Reference

### FAQ Keys (15)

| Key | Topic |
|---|---|
| `what_is_nyxid` | What is NyxID? |
| `authentication` | Authentication & login |
| `credential_broker` | Credential broker / vault |
| `llm_gateway` | LLM gateway |
| `transaction_approval` | Transaction approvals |
| `mcp_integration` | MCP integration |
| `credential_nodes` | Credential nodes |
| `api_keys` | API key management |
| `proxy` | API proxy |
| `security` | Security & encryption |
| `oauth_oidc` | OAuth / OIDC / SSO |
| `mfa` | Multi-factor authentication |
| `setup` | Getting started / setup |
| `mobile_app` | Mobile app |
| `use_cases` | Use cases |

### Action Keys (20)

| Key | Type | Confirmation | Admin only |
|---|---|---|---|
| `get_profile` | read | No | No |
| `list_api_keys` | read | No | No |
| `list_services` | read | No | No |
| `list_catalog` | read | No | No |
| `list_nodes` | read | No | No |
| `list_approvals` | read | No | No |
| `check_llm_status` | read | No | No |
| `list_endpoints` | read | No | No |
| `list_external_keys` | read | No | No |
| `create_api_key` | write | Yes | No |
| `rotate_api_key` | write | Yes | No |
| `delete_api_key` | write | Yes | No |
| `add_service` | write | Yes | No |
| `delete_service` | write | Yes | No |
| `route_service` | write | Yes | No |
| `set_service_credentials` | write | Yes | No |
| `approve_request` | approval | No | No |
| `deny_request` | approval | No | No |
| `list_users` | read | No | **Yes** |
| `list_service_accounts` | read | No | **Yes** |

### Special Keys

| Key | intent_type |
|---|---|
| `chitchat` | `chitchat` |
| `unknown` | `unknown` |
