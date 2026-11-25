# NyxID Chatbot — Feature Spec (Revised)

> This spec replaces the original v1 spec. All decisions below were validated through an engineering review with an independent outside voice (Codex).

## Overview

An in-app chat assistant for NyxID that answers product questions and performs API actions via a two-pass architecture. Available on both the React web dashboard and React Native mobile app. Uses Gemini Flash for classification and response generation. The backend is **stateless** — no chat history is persisted server-side.

---

## Architecture

### High-Level Flow

```
  Web Frontend (React)         Mobile App (React Native)
       │                              │
       │  POST /api/v1/chat           │
       │  (human-only route)          │
       └──────────┬───────────────────┘
                  │
    ┌─────────────▼──────────────────────────────┐
    │    Axum Handler (chat.rs) — STATELESS       │
    │    - AuthUser middleware (JWT session only)  │
    │    - Rate limiting via existing middleware   │
    ├─────────────┬──────────────────────────────┤
    │    Chat Service (chat_service.rs)            │
    │                                              │
    │  Input: { message, context, pending_action?,  │
    │          secret_input? }                     │
    │                                              │
    │  ┌─ If pending_action exists ────────────┐  │
    │  │  Continue param collection / confirm   │  │
    │  └────────────────────────────────────────┘  │
    │                                              │
    │  Pass 1: Intent Classification               │
    │  ┌────────────────────────────────────────┐  │
    │  │ Gemini Flash (classify)                 │  │
    │  │ System prompt filtered by user role     │  │
    │  │ Fallback: keyword matching              │  │
    │  └────────────────────────────────────────┘  │
    │                                              │
    │  Pass 2: Branched Response                   │
    │  ┌─ FAQ ──────────────────────────────────┐  │
    │  │ Lookup from embedded JSON               │  │
    │  │ Gemini rephrase for user's question     │  │
    │  ├─ ACTION ───────────────────────────────┤  │
    │  │ Param extract via Gemini                │  │
    │  │ Execute via internal HTTP (localhost)    │  │
    │  │ Write actions require confirmation       │  │
    │  ├─ CHITCHAT ─────────────────────────────┤  │
    │  │ Lightweight Gemini response              │  │
    │  └─ UNKNOWN ──────────────────────────────┘  │
    │     Fallback message                         │
    │                                              │
    │  Output: { reply, intent, context_summary?,   │
    │    pending_action?, requires_secret_input?,   │
    │    action_result? }                           │
    └──────────────────────────────────────────────┘
```

### Why Two-Pass

- Pass 1 is a classification-only call — tiny prompt, ~10 output tokens, extremely cheap
- Pass 2 loads only the relevant context (one FAQ entry or one tool schema), keeping prompts small
- Avoids sending all FAQ content + all tool definitions on every message
- Total cost per message: ~$0.00001–0.00005 on Gemini Flash

### Key Design Decisions

1. **Rust/Axum backend** — Same language and framework as the rest of NyxID. Reuses existing auth middleware, rate limiting, error handling, and MongoDB connection. No separate microservice.
2. **Stateless backend** — No MongoDB collections for chat. No history persistence. Client (FE) holds `pending_action` state and sends it back with each request. Message history is local to the client only.
3. **Action execution via internal HTTP** — Chat service calls own API endpoints via `reqwest` using `AppConfig.base_url` (e.g. `http://localhost:3001` in dev, `https://auth.nyxid.dev` in prod). Forwards the original `Authorization` and `Cookie` headers for auth, plus adds `X-NyxID-Source: chatbot` for audit trail attribution. This ensures all middleware, auth checks, and handler side effects (credential push, audit logging) execute correctly. ~1-5ms latency overhead, negligible vs Gemini's 200-500ms.
4. **Human-only route** — `/api/v1/chat` is in the human-only route bucket (same as `/approvals`, `/notifications`). Rejects API keys, service account tokens, and delegated tokens.
5. **Gemini Flash via OpenAI-compatible proxy** — Calls an OpenAI-compatible chat completions endpoint at `CHATBOT_LLM_BASE_URL` with `CHATBOT_LLM_API_KEY`. Default model: `vibe-coding-app-gemini`. Not routed through NyxID's own proxy/LLM gateway (avoids circular dependency).
6. **Curated knowledge base** — Hand-written JSON files for FAQ answers and action definitions, NOT auto-generated from OpenAPI. Loaded via `include_str!()` at compile time.
7. **Role-based action filtering** — Classification prompt is filtered by user role. Non-admins don't see admin actions in categories. Admins get additional actions (create user, list users, manage roles).
8. **Both web + mobile** — Web gets a floating chat widget. Mobile gets a new ChatScreen with Gifted Chat.

---

## Backend — Rust/Axum

### Endpoint

#### `POST /api/v1/chat`

Main chat endpoint. Stateless — receives a message and optional pending action state, runs the two-pass flow, returns the response with any updated pending state.

**Request:**
```json
{
  "message": "How does the credential broker work?",
  "pending_action": null,
  "context": []
}
```

**Response (FAQ):**
```json
{
  "reply": "The credential broker is NyxID's standout feature...",
  "intent": "credential_broker",
  "intent_type": "faq",
  "context_summary": null,
  "pending_action": null,
  "requires_secret_input": null,
  "action_result": null
}
```

**Response (ACTION — missing params):**
```json
{
  "reply": "I'll create an API key. What label do you want for it?",
  "intent": "create_api_key",
  "intent_type": "action",
  "context_summary": "Creating API key. Waiting for: label.",
  "pending_action": {
    "action": "create_api_key",
    "collected_params": {},
    "missing_params": ["label"],
    "awaiting_confirmation": false
  },
  "requires_secret_input": null,
  "action_result": null
}
```

**Response (ACTION — confirmation):**
```json
{
  "reply": "I'm about to create API key 'my-prod-key'. Confirm?",
  "intent": "create_api_key",
  "intent_type": "action",
  "context_summary": "Creating API key 'my-prod-key'. Awaiting confirmation.",
  "pending_action": {
    "action": "create_api_key",
    "collected_params": { "label": "my-prod-key" },
    "missing_params": [],
    "awaiting_confirmation": true
  },
  "requires_secret_input": null,
  "action_result": null
}
```

**Response (ACTION — executed):**
```json
{
  "reply": "Done! Created API key 'my-prod-key'.",
  "intent": "create_api_key",
  "intent_type": "action",
  "pending_action": null,
  "requires_secret_input": null,
  "action_result": {
    "endpoint": "POST /api/v1/keys",
    "status": 200,
    "summary": "Key created successfully"
  }
}
```

**Response (ACTION — requires secret input):**

When an action needs a secret (API key, OAuth credentials), the backend returns `requires_secret_input` instead of asking in the chat reply. The frontend shows a secure masked input popup.

```json
{
  "reply": "Setting up OpenAI. I'll need your API key to continue.",
  "intent": "add_service",
  "intent_type": "action",
  "context_summary": "Adding OpenAI service 'my-openai'. Waiting for: API key.",
  "pending_action": {
    "action": "add_service",
    "collected_params": { "service_slug": "llm-openai", "label": "my-openai" },
    "missing_params": ["credential"],
    "awaiting_confirmation": false
  },
  "requires_secret_input": [
    {
      "param_name": "credential",
      "label": "OpenAI API Key",
      "description": "Paste your API key from the OpenAI dashboard. Starts with sk-",
      "placeholder": "sk-proj-..."
    }
  ],
  "action_result": null
}
```

For actions needing multiple secrets (e.g. OAuth credentials), the array contains multiple entries rendered as multiple fields in one popup:

```json
{
  "reply": "Setting OAuth credentials for GitHub. I'll need the client ID and secret.",
  "intent": "set_service_credentials",
  "intent_type": "action",
  "requires_secret_input": [
    {
      "param_name": "client_id",
      "label": "OAuth Client ID",
      "description": "The client ID from your GitHub OAuth app registration",
      "placeholder": "Iv1.abc123..."
    },
    {
      "param_name": "client_secret",
      "label": "OAuth Client Secret",
      "description": "The client secret from your GitHub OAuth app. Will be encrypted and stored securely.",
      "placeholder": "your-client-secret"
    }
  ],
  "pending_action": { "..." : "..." },
  "action_result": null
}
```

**Request with secret input:**

When the frontend submits secrets from the popup, they go in a dedicated `secret_input` array — NOT in the `message` field:

```json
{
  "message": "",
  "context": [],
  "pending_action": { "..." : "..." },
  "secret_input": [
    { "param_name": "credential", "value": "sk-proj-abc123..." }
  ]
}
```

The secret value never appears in the chat message list. The frontend displays a redacted placeholder instead (e.g. "API key provided").

---

## LLM Client — `llm_chat_client.rs`

OpenAI-compatible chat completions client. Uses the `hyperecho-proxy.aelf.dev` proxy which routes to Gemini Flash.

**Request format:**
```
POST {CHATBOT_LLM_BASE_URL}/chat/completions
Authorization: Bearer {CHATBOT_LLM_API_KEY}
Content-Type: application/json

{
  "model": "{CHATBOT_LLM_MODEL}",
  "messages": [
    { "role": "system", "content": "<system prompt>" },
    { "role": "user", "content": "<user message>" }
  ],
  "max_tokens": <per-pass value>,
  "temperature": <per-pass value>
}
```

**Response extraction:**
```
reply = response.choices[0].message.content
```

> **Note:** Using `role: "system"` for system prompts. If the proxy doesn't support the system role, fall back to concatenating system + user content into a single `role: "user"` message. Test during implementation and adjust.

**Per-pass settings:**

| Pass | Purpose | Temperature | Max tokens | Timeout |
|---|---|---|---|---|
| Pass 1 | Intent classification | 0 | 50 | 15s |
| Pass 2a | FAQ rephrase | 0.7 | 400 | 15s |
| Pass 2b | Param extraction | 0 | 200 | 15s |
| Pass 2c | Chitchat | 0.8 | 150 | 15s |

Temperature 0 for classification and param extraction (deterministic output). Higher temperature for rephrase and chitchat (natural variation).

---

## Two-Pass Logic

### Pass 1 — Intent Router

**Max tokens:** 50
**Temperature:** 0

**System prompt (dynamically built per request, filtered by user role):**
```
Classify this user message into exactly one category.
Reply with ONLY the category key, nothing else.

FAQ categories:
- what_is_nyxid: What is NyxID? e.g. "tell me about NyxID"
- authentication: How does auth work? e.g. "how do I log in?"
- credential_broker: How does credential brokering work? e.g. "what is the vault?"
- llm_gateway: How does the LLM gateway work? e.g. "how do I proxy to OpenAI?"
- transaction_approval: How do approvals work? e.g. "what are transaction approvals?"
- mcp_integration: How does MCP work? e.g. "can I use NyxID with Claude?"
- credential_nodes: How do nodes work? e.g. "what is a credential node?"
- api_keys: How do API keys work? e.g. "how do I manage API keys?"
- proxy: How does the proxy work? e.g. "how do I proxy API requests?"
- security: How is NyxID secured? e.g. "is my data encrypted?"
- oauth_oidc: How does OAuth/OIDC work? e.g. "does NyxID support SSO?"
- mfa: How does MFA work? e.g. "how do I enable 2FA?"
- setup: How do I set up NyxID? e.g. "how do I get started?"
- mobile_app: What does the mobile app do? e.g. "is there an iOS app?"
- use_cases: What can I use NyxID for? e.g. "what are common use cases?"

ACTION categories (all users):
- get_profile: Show my account info. e.g. "show my profile"
- list_api_keys: List API keys. e.g. "show my API keys"
- create_api_key: Create an API key. e.g. "create a new API key"
- rotate_api_key: Rotate an API key. e.g. "rotate my key"
- delete_api_key: Delete an API key. e.g. "delete API key xyz"
- list_services: List configured services. e.g. "what services do I have?"
- add_service: Add a service from catalog. e.g. "add openai service"
- delete_service: Delete a service. e.g. "delete my openai service"
- route_service: Change service routing. e.g. "route openai through my node"
- set_service_credentials: Set OAuth credentials for a service. e.g. "set credentials for github"
- list_catalog: Browse available services. e.g. "show the catalog"
- list_nodes: List credential nodes. e.g. "show my nodes"
- list_approvals: List pending approvals. e.g. "show pending approvals"
- approve_request: Approve a request. e.g. "approve request abc123"
- deny_request: Deny a request. e.g. "deny request abc123"
- list_endpoints: List endpoints. e.g. "show my endpoints"
- list_external_keys: List external credentials. e.g. "show my external keys"
- check_llm_status: Check LLM status. e.g. "is OpenAI up?"

ACTION categories (admin only — included only for admin users):
- list_users: List all users. e.g. "show all users"
- list_service_accounts: List service accounts. e.g. "show service accounts"

If greeting or small talk: chitchat
If nothing fits: unknown
```

**Input:** user message + conversation context (last 5 message pairs) + context_summary if pending action exists. See "Classification with Context" section below.

**Output:** single category key string

**Fallback:** if Gemini API call fails, fall back to keyword matching against FAQ keyword lists.

### Pass 2a — FAQ Response

**Model:** Gemini Flash
**Max tokens:** 400

**System prompt:**
```
You are a friendly support agent for NyxID.
Answer the user's question using ONLY the provided context.
Be concise and conversational. Do not invent features.

Context:
{faq_answer}
```

**Input:** user message

**Why rephrase instead of returning raw FAQ text:** the static answer is written generically. Gemini tailors it to the specific question asked, e.g. "does NyxID support Google login?" gets a focused answer about social OAuth, not the entire auth feature list.

### Pass 2b — Action Handler

**Model:** Gemini Flash
**Max tokens:** 200

**System prompt:**
```
Extract parameters from the user's message for this action.
Action: {action_key}
Required params: {param_list}
Description: {action_description}

Respond with JSON only:
{"params": {...}, "missing": [...]}
```

**Flow:**
1. Gemini extracts params from the user message
2. If `missing` is non-empty → return a message asking for the missing params (set `pending_action` in response)
3. If all params present AND action `requires_confirmation` → return confirmation prompt (set `awaiting_confirmation: true`)
4. If confirmed (user sent "yes" with `awaiting_confirmation: true` in pending) → execute via internal HTTP → return result
5. Clear `pending_action` after execution or rejection

### Pass 2c — Chitchat

**Model:** Gemini Flash
**Max tokens:** 150

Lightweight greeting/small talk response. No context loading needed.

---

## Conversation Context & Pending Action Flow

The backend is stateless. Both conversation context and pending action state are held client-side and sent with each request.

### Context Round-Trip

The client maintains a rolling window of the **last 5 message pairs** (10 messages). Each request includes this context so Gemini can reason about the full conversation state. The context resets on page refresh (stateless).

```json
{
  "message": "30",
  "context": [
    { "role": "user", "content": "create an API key" },
    { "role": "assistant", "content": "Creating API key. What name do you want for it?" },
    { "role": "user", "content": "prod-key with proxy scope" },
    { "role": "assistant", "content": "Got it. Name: prod-key, scopes: proxy. How many days until it expires? (0 for no expiry)" }
  ],
  "pending_action": {
    "action": "create_api_key",
    "collected_params": { "name": "prod-key", "scopes": "proxy" },
    "missing_params": ["expires_in_days"],
    "awaiting_confirmation": false
  }
}
```

### Why Context Solves Ambiguity

Pass 1 (classification) always runs, even with a pending action. The conversation context gives Gemini enough information to disambiguate:

```
User mid-action says "30"
  → Gemini sees context: creating API key, waiting for expiry days
  → Classifies as "continue" (param for pending action)
  → Param extraction: expires_in_days = 30 → all params collected → confirm

User mid-action says "how does MFA work?"
  → Gemini sees context: creating API key, waiting for expiry days
  → Classifies as "mfa" (FAQ) — clearly a topic switch
  → Clears pending_action, answers the FAQ

User mid-action says something ambiguous like "delete"
  → Gemini sees context: creating API key, waiting for expiry days
  → "delete" doesn't match expiry, and could be a new action
  → Classifies accordingly, or asks for clarification
```

No rigid "skip vs classify" logic needed. The LLM resolves ambiguity naturally with context.

### Pending Action Walkthrough

```
User: "add openai service"
  → Pass 1 classifies: add_service (ACTION)
  → Pass 2b extracts params: { service_slug: "llm-openai" }, missing = [label]
  → Response: {
      reply: "Adding OpenAI from catalog. What do you want to call this service?",
      context_summary: "Adding OpenAI service. Waiting for: label.",
      pending_action: { action: "add_service", collected_params: { service_slug: "llm-openai" }, missing_params: ["label"], awaiting_confirmation: false }
    }
  → Client stores pending_action + appends to context

User: "my-openai"
  → Client sends: { message: "my-openai", context: [...], pending_action: {...} }
  → Pass 1 with context classifies: "continue" (param response for pending action)
  → Pass 2b extracts: { label: "my-openai" }, missing = [credential]
  → credential has secret: true → return requires_secret_input
  → Response: {
      reply: "Got it. I'll need your OpenAI API key to continue.",
      requires_secret_input: [{ param_name: "credential", label: "OpenAI API Key", ... }],
      pending_action: { ..., collected_params: { service_slug: "llm-openai", label: "my-openai" }, missing_params: ["credential"] }
    }
  → Frontend shows secure input popup

User: [enters API key in popup]
  → Client sends: { message: "", context: [...], pending_action: {...}, secret_input: [{ param_name: "credential", value: "sk-proj-..." }] }
  → All params collected → requires_confirmation = true
  → Response: {
      reply: "Add OpenAI service 'my-openai'. Confirm?",
      pending_action: { ..., awaiting_confirmation: true }
    }

User: "yes"
  → Pass 1 with context classifies: "continue" (confirmation)
  → Backend executes POST /api/v1/keys via internal HTTP
  → Response: {
      reply: "Done! Service 'my-openai' added successfully.",
      pending_action: null,
      action_result: { endpoint: "POST /api/v1/keys", status: 200 }
    }
```

### Classification with Context

Pass 1 system prompt includes conversation context when available:

```
Classify this user message into exactly one category.
Reply with ONLY the category key, nothing else.

Current conversation context:
{context_summary}

[... FAQ categories ...]
[... ACTION categories ...]

If the user is continuing a pending action (providing parameters or confirming): continue
If greeting or small talk: chitchat
If nothing fits: unknown
```

The `continue` category is added to handle the pending action flow. When Pass 1 returns `continue`, the backend routes directly to param extraction for the pending action.

---

## Knowledge Base

### FAQ — `backend/src/data/chatbot_faq.json`

Curated JSON file, embedded at compile time via `include_str!()`. No vector DB, no RAG.

15 FAQ categories defined in `backend/src/data/chatbot_faq.json`: what_is_nyxid, authentication, credential_broker, llm_gateway, transaction_approval, mcp_integration, credential_nodes, api_keys, proxy, security, oauth_oidc, mfa, setup, mobile_app, use_cases. Each entry has `keywords` (for fallback matching), `answer` (2-4 sentences, user-facing tone), and `related` (links to other FAQ keys).

### Actions — `backend/src/data/chatbot_actions.json`

Curated action registry aligned with the `nyxid` CLI commands. Each action maps to a real API endpoint. Params marked `"secret": true` trigger the secure input popup on the frontend instead of being typed into chat.

**All Users — Read Actions (no confirmation):**

```json
{
  "get_profile": {
    "description": "Show your account info",
    "example": "show my profile",
    "method": "GET",
    "path": "/api/v1/users/me",
    "path_params": {},
    "body_params": {},
    "requires_confirmation": false,
    "role_required": null
  },
  "list_api_keys": {
    "description": "List your NyxID API keys",
    "example": "show my API keys",
    "method": "GET",
    "path": "/api/v1/api-keys",
    "path_params": {},
    "body_params": {},
    "requires_confirmation": false,
    "role_required": null
  },
  "list_services": {
    "description": "List your configured services",
    "example": "what services do I have?",
    "method": "GET",
    "path": "/api/v1/user-services",
    "path_params": {},
    "body_params": {},
    "requires_confirmation": false,
    "role_required": null
  },
  "list_catalog": {
    "description": "Browse available services from the catalog",
    "example": "show the service catalog",
    "method": "GET",
    "path": "/api/v1/catalog",
    "path_params": {},
    "body_params": {},
    "requires_confirmation": false,
    "role_required": null
  },
  "list_nodes": {
    "description": "List your credential nodes",
    "example": "show my nodes",
    "method": "GET",
    "path": "/api/v1/nodes",
    "path_params": {},
    "body_params": {},
    "requires_confirmation": false,
    "role_required": null
  },
  "list_approvals": {
    "description": "List pending approval requests",
    "example": "show pending approvals",
    "method": "GET",
    "path": "/api/v1/approvals/requests",
    "path_params": {},
    "body_params": {},
    "requires_confirmation": false,
    "role_required": null
  },
  "check_llm_status": {
    "description": "Check LLM provider status",
    "example": "is OpenAI up?",
    "method": "GET",
    "path": "/api/v1/llm/status",
    "path_params": {},
    "body_params": {},
    "requires_confirmation": false,
    "role_required": null
  },
  "list_endpoints": {
    "description": "List your endpoints",
    "example": "show my endpoints",
    "method": "GET",
    "path": "/api/v1/endpoints",
    "path_params": {},
    "body_params": {},
    "requires_confirmation": false,
    "role_required": null
  },
  "list_external_keys": {
    "description": "List your external API keys and credentials",
    "example": "show my external credentials",
    "method": "GET",
    "path": "/api/v1/api-keys/external",
    "path_params": {},
    "body_params": {},
    "requires_confirmation": false,
    "role_required": null
  }
}
```

**All Users — Write Actions (require confirmation):**

```json
{
  "create_api_key": {
    "description": "Create a new NyxID API key",
    "example": "create an API key called prod-key",
    "method": "POST",
    "path": "/api/v1/api-keys",
    "path_params": {},
    "body_params": {
      "name": { "type": "string", "required": true, "description": "Name for the key" },
      "scopes": { "type": "string", "required": false, "description": "Space-separated scopes (e.g. 'read write proxy')" },
      "expires_in_days": { "type": "number", "required": false, "description": "Expiry in days (0 = no expiry). Chat service converts to RFC 3339 expires_at before calling API." }
    },
    "requires_confirmation": true,
    "confirm_message": "Create API key '{name}'",
    "success_message": "API key '{name}' created successfully. Key: {full_key}",
    "role_required": null
  },
  "rotate_api_key": {
    "description": "Rotate an API key (old key stops working immediately)",
    "example": "rotate my API key abc123",
    "method": "POST",
    "path": "/api/v1/api-keys/{key_id}/rotate",
    "path_params": {
      "key_id": { "type": "string", "required": true, "description": "ID of the API key to rotate" }
    },
    "body_params": {},
    "requires_confirmation": true,
    "confirm_message": "Rotate API key {key_id}. The old key will stop working immediately.",
    "success_message": "API key rotated. New key: {full_key}",
    "role_required": null
  },
  "delete_api_key": {
    "description": "Delete an API key",
    "example": "delete API key abc123",
    "method": "DELETE",
    "path": "/api/v1/api-keys/{key_id}",
    "path_params": {
      "key_id": { "type": "string", "required": true, "description": "ID of the API key to delete" }
    },
    "body_params": {},
    "requires_confirmation": true,
    "confirm_message": "Delete API key {key_id}. This cannot be undone.",
    "success_message": "API key deleted",
    "role_required": null
  },
  "add_service": {
    "description": "Add an AI service from the catalog",
    "example": "add openai service",
    "method": "POST",
    "path": "/api/v1/keys",
    "path_params": {},
    "body_params": {
      "service_slug": { "type": "string", "required": true, "description": "Catalog service slug (e.g. 'llm-openai', 'llm-anthropic')" },
      "label": { "type": "string", "required": true, "description": "Name for this service" },
      "credential": { "type": "string", "required": false, "secret": true, "description": "API key or bearer token for the service" },
      "endpoint_url": { "type": "string", "required": false, "description": "Endpoint URL override (for self-hosted)" },
      "node_id": { "type": "string", "required": false, "description": "Route through this credential node" }
    },
    "secret_labels": {
      "credential": {
        "llm-openai": { "label": "OpenAI API Key", "description": "Paste your API key from the OpenAI dashboard. Starts with sk-", "placeholder": "sk-proj-..." },
        "llm-anthropic": { "label": "Anthropic API Key", "description": "Paste your API key from console.anthropic.com", "placeholder": "sk-ant-..." },
        "llm-google-ai": { "label": "Google AI API Key", "description": "Paste your Gemini API key from aistudio.google.com", "placeholder": "AI..." },
        "_default": { "label": "API Key", "description": "Paste the API key or bearer token for this service", "placeholder": "your-api-key" }
      }
    },
    "requires_confirmation": true,
    "confirm_message": "Add {service_slug} service '{label}'",
    "success_message": "Service '{label}' added successfully",
    "role_required": null
  },
  "delete_service": {
    "description": "Delete a configured service",
    "example": "delete my openai service",
    "method": "DELETE",
    "path": "/api/v1/user-services/{service_id}",
    "path_params": {
      "service_id": { "type": "string", "required": true, "description": "Service ID to delete" }
    },
    "body_params": {},
    "requires_confirmation": true,
    "confirm_message": "Delete service {service_id}. This cannot be undone.",
    "success_message": "Service deleted",
    "role_required": null
  },
  "route_service": {
    "description": "Change service routing (node or direct)",
    "example": "route my openai service through node-abc",
    "method": "PUT",
    "path": "/api/v1/user-services/{service_id}",
    "path_params": {
      "service_id": { "type": "string", "required": true, "description": "Service ID to update" }
    },
    "body_params": {
      "node_id": { "type": "string", "required": true, "description": "Node ID to route through, or empty string for direct routing" }
    },
    "requires_confirmation": true,
    "confirm_message": "Route service {service_id} through node {node_id}",
    "success_message": "Service routing updated",
    "role_required": null
  },
  "set_service_credentials": {
    "description": "Set OAuth credentials for a service's provider",
    "example": "set credentials for github",
    "method": "PUT",
    "path": "/api/v1/providers/{provider_id}/credentials",
    "pre_lookup": {
      "description": "Resolve provider_id from catalog slug",
      "method": "GET",
      "path": "/api/v1/catalog/{slug}",
      "extract": "provider_config_id"
    },
    "path_params": {
      "slug": { "type": "string", "required": true, "description": "Catalog service slug (e.g. 'github')" }
    },
    "body_params": {
      "client_id": { "type": "string", "required": true, "secret": true, "description": "OAuth Client ID" },
      "client_secret": { "type": "string", "required": false, "secret": true, "description": "OAuth Client Secret" }
    },
    "secret_labels": {
      "client_id": {
        "_default": { "label": "OAuth Client ID", "description": "The client ID from your OAuth app registration", "placeholder": "your-client-id" }
      },
      "client_secret": {
        "_default": { "label": "OAuth Client Secret", "description": "The client secret from your OAuth app. Will be encrypted and stored securely.", "placeholder": "your-client-secret" }
      }
    },
    "requires_confirmation": true,
    "confirm_message": "Set OAuth credentials for {slug}",
    "success_message": "OAuth credentials set for {slug}",
    "role_required": null
  },
  "approve_request": {
    "description": "Approve a pending approval request",
    "example": "approve request abc123",
    "method": "POST",
    "path": "/api/v1/approvals/requests/{id}/decide",
    "path_params": {
      "id": { "type": "string", "required": true, "description": "Approval request ID" }
    },
    "body_params": {
      "decision": { "type": "string", "required": true, "description": "Decision", "enum": ["approved"], "default": "approved" }
    },
    "requires_confirmation": false,
    "success_message": "Request {id} approved",
    "role_required": null
  },
  "deny_request": {
    "description": "Deny a pending approval request",
    "example": "deny request abc123",
    "method": "POST",
    "path": "/api/v1/approvals/requests/{id}/decide",
    "path_params": {
      "id": { "type": "string", "required": true, "description": "Approval request ID" }
    },
    "body_params": {
      "decision": { "type": "string", "required": true, "description": "Decision", "enum": ["rejected"], "default": "rejected" }
    },
    "requires_confirmation": false,
    "success_message": "Request {id} denied",
    "role_required": null
  }
}
```

**Admin Only — Read Actions:**

```json
{
  "list_users": {
    "description": "List all users",
    "example": "show all users",
    "method": "GET",
    "path": "/api/v1/admin/users",
    "path_params": {},
    "body_params": {},
    "requires_confirmation": false,
    "role_required": "admin"
  },
  "list_service_accounts": {
    "description": "List service accounts",
    "example": "show service accounts",
    "method": "GET",
    "path": "/api/v1/admin/service-accounts",
    "path_params": {},
    "body_params": {},
    "requires_confirmation": false,
    "role_required": "admin"
  }
}
```

**Request construction:** The chat service builds the HTTP request by:
1. Substituting `path_params` into the URL template: `/api/v1/api-keys/{key_id}/rotate` → `/api/v1/api-keys/abc123/rotate`
2. Serializing `body_params` as JSON request body (for POST/PUT/PATCH)
3. Using `method` to determine the HTTP verb
4. For actions with `pre_lookup`: executing the lookup request first to resolve dynamic path params (e.g. `provider_id` from catalog slug)
5. For params with `"secret": true`: returning `requires_secret_input` to the frontend instead of asking in chat. The `secret_labels` map provides context-specific copy (keyed by `service_slug` or `_default` fallback).

**POC action set (19 total, aligned with CLI):**
- **All users (17):** get_profile, list_api_keys, create_api_key, rotate_api_key, delete_api_key, list_services, add_service, delete_service, route_service, set_service_credentials, list_catalog, list_nodes, list_approvals, approve_request, deny_request, list_endpoints, list_external_keys, check_llm_status
- **Admin only (2):** list_users, list_service_accounts

### Classification Prompt (built at runtime)

The Pass 1 system prompt is **constructed dynamically in `chat_service.rs`** — not loaded from a static file. It reads FAQ category keys from `chatbot_faq.json` and action entries from `chatbot_actions.json`, filtering actions by `role_required` against the user's role. This means non-admins literally never see admin actions in the classification prompt.

Both `chatbot_faq.json` and `chatbot_actions.json` are loaded via `include_str!()` at compile time (same pattern as `handlers/llms_txt.rs` loading `AI_AGENT_PLAYBOOK.md`). The prompt assembly happens at request time.

---

## Frontend — React Web

### Floating Chat Widget — `frontend/src/components/chat/chat-widget.tsx`

A floating button in the bottom-right corner of the dashboard that opens a chat panel overlay.

**State (local React state, no server persistence):**
- `messages: ChatMessage[]` — local message history (cleared on page refresh)
- `isTyping: boolean` — controls typing indicator
- `pendingAction: PendingAction | null` — client-side pending action state
- `isOpen: boolean` — panel open/closed

**Send flow:**
1. User taps send → append user message to local state immediately
2. Set `isTyping = true`
3. `POST /api/v1/chat` with `{ message, context, pending_action: pendingAction, secret_input }`
4. On response, set `isTyping = false`, append bot message to state
5. Update `pendingAction` from response
6. If `requires_secret_input` is non-null → show secure input popup (masked fields)
7. Append both user + assistant messages to `context` (trim to last 5 pairs)

**Components:**
- Message bubbles (user = dark, bot = purple brand color)
- Typing indicator (animated dots)
- Quick-reply chips (shown on first open or when no messages)
- Action result card (monospace, shows endpoint + HTTP status)
- Text input with send button

**Integration:** Rendered inside `DashboardLayout` as a fixed-position overlay. Uses existing `apiClient` from `lib/api-client.ts`.

### Hook — `frontend/src/hooks/use-chat.ts`

TanStack Query mutation for `POST /api/v1/chat`. Manages optimistic UI updates.

### Types — `frontend/src/types/chat.ts`

```ts
interface ContextMessage {
  role: 'user' | 'assistant';
  content: string;
}

interface ChatRequest {
  message: string;
  pending_action: PendingAction | null;
  context: ContextMessage[];       // last 5 message pairs (max 10 entries)
  secret_input: SecretInput[] | null;  // secrets submitted from secure popup
}

interface ChatResponse {
  reply: string;
  intent: string;
  intent_type: 'faq' | 'action' | 'chitchat' | 'unknown' | 'continue';
  context_summary: string | null;           // natural language summary of current state
  pending_action: PendingAction | null;
  requires_secret_input: SecretInputRequest[] | null;  // triggers secure popup on FE
  action_result: ActionResult | null;
}

interface PendingAction {
  action: string;
  collected_params: Record<string, string | number | boolean>;
  missing_params: string[];
  awaiting_confirmation: boolean;
}

interface SecretInputRequest {
  param_name: string;       // which param this fills (e.g. "credential", "client_id")
  label: string;            // popup field label (e.g. "OpenAI API Key")
  description: string;      // help text below the input
  placeholder: string;      // input placeholder (e.g. "sk-proj-...")
}

interface SecretInput {
  param_name: string;       // matches the requested param_name
  value: string;            // the actual secret value
}

interface ActionResult {
  endpoint: string;
  status: number;
  summary: string;
}
```

**Secret input UX rules:**
- When `requires_secret_input` is non-null, the frontend shows a secure popup with masked input field(s) — one field per array entry
- The user submits secrets via the popup, and the frontend sends them in `secret_input` — NOT in `message`
- The chat message list shows a redacted entry for the user's turn (e.g. "API key provided") — the actual value is never displayed
- If the user dismisses the popup without entering a value, the pending action remains and the frontend can re-prompt or let the user type a different message

---

## Frontend — React Native (Mobile)

### Library

`react-native-gifted-chat` (new dependency) — handles message rendering, keyboard avoidance, typing indicator, scroll management on both iOS and Android.

### Screen: `mobile/src/features/chat/ChatScreen.tsx`

**State (local component state):**
- `messages: IMessage[]` — Gifted Chat message array
- `context: ContextMessage[]` — rolling last 5 message pairs
- `isTyping: boolean` — controls typing indicator
- `pendingAction: PendingAction | null` — client-side pending action state

**Send flow:**
1. User taps send → append user message to local state immediately
2. Set `isTyping = true`
3. `POST /api/v1/chat` with `{ message, context, pending_action: pendingAction, secret_input }`
4. On response, set `isTyping = false`, append bot message to state
5. Update `pendingAction` from response
6. If `requires_secret_input` is non-null → show secure input popup (masked fields)
7. Append both user + assistant messages to `context` (trim to last 5 pairs)

**Bot identity:**
```ts
const BOT_USER = {
  _id: 'nyxid-bot',
  name: 'NyxID',
  avatar: require('../assets/nyxid-avatar.png')
};
```

**Customization:**
- Custom bubble colors (purple for bot, dark for user — matching NyxID brand)
- Suggested prompts shown as quick-reply chips on first load
- Action responses rendered with a monospace metadata footer (endpoint + status)

### API Client: `mobile/src/features/chat/chatApi.ts`

```ts
async function sendChatMessage(
  message: string,
  context: ContextMessage[],
  pendingAction?: PendingAction,
  secretInput?: SecretInput[]
): Promise<ChatResponse>
```

Uses existing `mobileApi` from `lib/api/`. Same request/response types as web — shared `ChatRequest`, `ChatResponse`, `PendingAction`, `SecretInput`, `SecretInputRequest` interfaces.

### Navigation

Add Chat button/tab to `mobile/src/app/AppNavigator.tsx` and `mobile/src/components/BottomNav.tsx`.

---

## Quick Replies / Suggested Prompts

Shown as tappable chips below the input on first load.

```ts
const SUGGESTIONS = [
  "What is NyxID?",
  "How does the LLM gateway work?",
  "Check LLM provider status",
  "Show my API keys",
  "Show pending approvals",
  "Create a new API key"
];
```

Rendered using Gifted Chat's `quickReply` prop (mobile) or custom chip components (web).

---

## Error Handling

| Scenario | Behaviour |
|---|---|
| Gemini API unreachable (Pass 1) | Fall back to keyword matching against FAQ keywords |
| Gemini API unreachable (Pass 2) | Return raw static FAQ text for FAQ; return "service temporarily unavailable" for actions |
| Internal HTTP action fails (4xx) | Parse error response, return friendly message: "I couldn't do that: [reason]" |
| Internal HTTP action fails (5xx) | Return "Something went wrong. Try again." |
| Intent = unknown | "I'm not sure about that. I can help with NyxID features, security, or perform actions like managing API keys." |
| Gemini rate limit (429) | Retry once after 1s; if still failing, return friendly rate limit message |
| CHATBOT_LLM_API_KEY not set | Chat endpoint returns 503 "Chatbot not configured" |

---

## Environment Variables

```bash
# Optional — chatbot is disabled if CHATBOT_LLM_API_KEY is not set
CHATBOT_LLM_BASE_URL=https://hyperecho-proxy.aelf.dev/v1   # OpenAI-compatible base URL
CHATBOT_LLM_API_KEY=                                        # Bearer token for the LLM proxy
CHATBOT_LLM_MODEL=vibe-coding-app-gemini                    # Model name for chat completions
```

---

## File Structure

```
backend/src/
├── handlers/
│   └── chat.rs                     # POST /api/v1/chat handler
├── services/
│   ├── chat_service.rs             # Two-pass orchestration, action execution
│   └── llm_chat_client.rs          # OpenAI-compatible chat completions client
├── data/
│   ├── chatbot_faq.json            # Curated FAQ knowledge base
│   └── chatbot_actions.json        # Curated action registry

frontend/src/
├── components/
│   └── chat/
│       └── chat-widget.tsx         # Floating chat widget + panel
├── hooks/
│   └── use-chat.ts                 # TanStack Query mutation
└── types/
    └── chat.ts                     # ChatRequest, ChatResponse, PendingAction types

mobile/src/
├── features/
│   └── chat/
│       ├── ChatScreen.tsx          # Gifted Chat screen
│       └── chatApi.ts              # API client for chat endpoint
├── app/
│   └── AppNavigator.tsx            # Updated — add Chat route
└── components/
    └── BottomNav.tsx               # Updated — add Chat tab
```

### Files to Modify

- `backend/src/routes.rs` — Add `/chat` route in human-only section
- `backend/src/handlers/mod.rs` — Add `pub mod chat;`
- `backend/src/services/mod.rs` — Add `pub mod chat_service; pub mod llm_chat_client;`
- `backend/src/config.rs` — Add `chatbot_llm_base_url: String`, `chatbot_llm_api_key: Option<String>`, `chatbot_llm_model: String`
- `backend/src/main.rs` — Read chatbot LLM env vars, optionally add LlmChatClient to AppState
- `frontend/src/components/layout/dashboard-layout.tsx` — Import and render ChatWidget
- `mobile/package.json` — Add `react-native-gifted-chat` dependency

---

## Security Considerations

- **Route placement:** Human-only. Rejects API keys, delegated tokens, and service account tokens.
- **Role-based filtering:** Classification prompt excludes admin actions for non-admin users.
- **Action auth:** All actions execute via internal HTTP through the full middleware stack — same auth checks as the UI. Internal requests forward the original `Authorization` and `Cookie` headers, plus add `X-NyxID-Source: chatbot` for audit trail attribution.
- **No data persistence:** Chat messages are not stored on the backend. No transcripts, no sensitive data in MongoDB.
- **LLM credentials:** Stored as `CHATBOT_LLM_API_KEY` env var. Never exposed to clients. Chat endpoint proxies the LLM call.

---

## Future Enhancements (Out of Scope for V1)

- **Chat history persistence** — Add MongoDB collections for messages + sessions, GET /chat/history endpoint, TTL/retention policy
- **Streaming responses** — SSE from backend, progressive text rendering
- **Rich message types** — tables, charts, approval buttons rendered as custom chat bubbles
- **Conversation context window** — send last N messages for better follow-up understanding
- **Analytics** — track intent classification accuracy, action completion rates, FAQ hit rates
- **Admin dashboard** — view chat logs, update FAQ content without redeployment
- **Multilingual** — detect language in Pass 1, respond in same language
- **Utoipa annotation completion** — ~15 handlers need OpenAPI annotations (separate PR)
