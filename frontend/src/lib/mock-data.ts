// ── Mock User ──
const MOCK_USER = {
  id: "d4f5a6b7-c8d9-4e0f-a1b2-c3d4e5f60718",
  email: "dannick@nyxid.dev",
  display_name: "Dannick Young",
  avatar_url: null,
  email_verified: true,
  mfa_enabled: true,
  is_admin: true,
  is_active: true,
  created_at: "2025-11-20T08:00:00Z",
};

// ── NyxID API Keys (Agent Keys) ──
const MOCK_API_KEYS = [
  {
    id: "k1-0001-0001-0001-000000000001",
    name: "claude-code-agent",
    description: "Primary coding agent",
    key_prefix: "nyxid_ag_ck",
    scopes: "read write",
    created_at: "2026-03-10T09:00:00Z",
    last_used_at: "2026-05-06T14:22:00Z",
    expires_at: null,
    is_active: true,
    allowed_service_ids: [],
    allowed_node_ids: [],
    allow_all_services: true,
    allow_all_nodes: true,
    allowed_services: [],
    allowed_nodes: [],
    platform: "claude-code",
    callback_url: null,
    rate_limit_per_second: 10,
    rate_limit_burst: 30,
    bindings_count: 2,
    credential_source: { type: "personal" as const },
  },
  {
    id: "k1-0001-0001-0001-000000000002",
    name: "cursor-agent",
    description: "Cursor IDE agent",
    key_prefix: "nyxid_ag_cr",
    scopes: "read write",
    created_at: "2026-04-01T10:30:00Z",
    last_used_at: "2026-05-05T18:10:00Z",
    expires_at: null,
    is_active: true,
    allowed_service_ids: [],
    allowed_node_ids: [],
    allow_all_services: true,
    allow_all_nodes: false,
    allowed_services: [],
    allowed_nodes: [],
    platform: "cursor",
    callback_url: null,
    rate_limit_per_second: null,
    rate_limit_burst: null,
    bindings_count: 1,
    credential_source: { type: "personal" as const },
  },
  {
    id: "k1-0001-0001-0001-000000000003",
    name: "codex-agent",
    description: "OpenAI Codex agent",
    key_prefix: "nyxid_ag_cx",
    scopes: "read",
    created_at: "2026-04-15T08:00:00Z",
    last_used_at: null,
    expires_at: "2027-04-15T08:00:00Z",
    is_active: true,
    allowed_service_ids: ["svc-openai"],
    allowed_node_ids: [],
    allow_all_services: false,
    allow_all_nodes: false,
    allowed_services: [{ id: "svc-openai", name: "OpenAI", slug: "openai" }],
    allowed_nodes: [],
    platform: "codex",
    callback_url: null,
    rate_limit_per_second: 5,
    rate_limit_burst: 15,
    bindings_count: 0,
    credential_source: { type: "personal" as const },
  },
  {
    id: "k1-0001-0001-0001-000000000004",
    name: "ci-pipeline",
    description: "CI/CD pipeline key",
    key_prefix: "nyxid_ag_ci",
    scopes: "read",
    created_at: "2026-02-20T12:00:00Z",
    last_used_at: "2026-05-06T06:00:00Z",
    expires_at: null,
    is_active: true,
    allowed_service_ids: [],
    allowed_node_ids: [],
    allow_all_services: true,
    allow_all_nodes: true,
    allowed_services: [],
    allowed_nodes: [],
    platform: null,
    callback_url: null,
    rate_limit_per_second: null,
    rate_limit_burst: null,
    bindings_count: 0,
    credential_source: { type: "personal" as const },
  },
];

// ── Agent Bindings ──
const MOCK_BINDINGS: Record<string, unknown[]> = {
  "k1-0001-0001-0001-000000000001": [
    {
      id: "bind-0001",
      api_key_id: "k1-0001-0001-0001-000000000001",
      user_service_id: "key-0001",
      service_label: "OpenAI",
      service_slug: "openai",
      user_api_key_id: "eak-0001",
      credential_label: "OpenAI Production Key",
      created_at: "2026-03-12T09:00:00Z",
      updated_at: "2026-03-12T09:00:00Z",
    },
    {
      id: "bind-0002",
      api_key_id: "k1-0001-0001-0001-000000000001",
      user_service_id: "key-0002",
      service_label: "Anthropic",
      service_slug: "anthropic",
      user_api_key_id: "eak-0002",
      credential_label: "Claude API Key",
      created_at: "2026-03-12T09:05:00Z",
      updated_at: "2026-03-12T09:05:00Z",
    },
  ],
  "k1-0001-0001-0001-000000000002": [
    {
      id: "bind-0003",
      api_key_id: "k1-0001-0001-0001-000000000002",
      user_service_id: "key-0001",
      service_label: "OpenAI",
      service_slug: "openai",
      user_api_key_id: "eak-0001",
      credential_label: "OpenAI Production Key",
      created_at: "2026-04-05T10:30:00Z",
      updated_at: "2026-04-05T10:30:00Z",
    },
  ],
};

// ── External Services (Keys) ──
const MOCK_KEYS = [
  {
    id: "key-0001",
    label: "OpenAI",
    slug: "openai",
    endpoint_url: "https://api.openai.com/v1",
    endpoint_id: "ep-0001",
    api_key_id: "eak-0001",
    credential_type: "api_key",
    auth_method: "bearer",
    auth_key_name: "Authorization",
    status: "active",
    catalog_service_id: "cs-openai",
    catalog_service_slug: "openai",
    catalog_service_name: "OpenAI",
    node_id: null,
    node_priority: 0,
    is_active: true,
    custom_user_agent: null,
    default_request_headers: null,
    ws_frame_injections: [],
    auto_connected: false,
    source_app_id: null,
    source_app_name: null,
    expires_at: null,
    last_used_at: "2026-05-06T14:22:00Z",
    error_message: null,
    created_at: "2026-01-15T09:00:00Z",
    service_type: "http",
    ssh_host: null, ssh_port: null, ssh_ca_public_key: null,
    ssh_allowed_principals: null, ssh_certificate_ttl_minutes: null,
    openapi_spec_url: null,
    credential_source: { type: "personal" as const },
  },
  {
    id: "key-0002",
    label: "Anthropic",
    slug: "anthropic",
    endpoint_url: "https://api.anthropic.com/v1",
    endpoint_id: "ep-0002",
    api_key_id: "eak-0002",
    credential_type: "api_key",
    auth_method: "header",
    auth_key_name: "x-api-key",
    status: "active",
    catalog_service_id: "cs-anthropic",
    catalog_service_slug: "anthropic",
    catalog_service_name: "Anthropic",
    node_id: null,
    node_priority: 0,
    is_active: true,
    custom_user_agent: null,
    default_request_headers: null,
    ws_frame_injections: [],
    auto_connected: false,
    source_app_id: null,
    source_app_name: null,
    expires_at: null,
    last_used_at: "2026-05-05T10:15:00Z",
    error_message: null,
    created_at: "2026-01-20T10:00:00Z",
    service_type: "http",
    ssh_host: null, ssh_port: null, ssh_ca_public_key: null,
    ssh_allowed_principals: null, ssh_certificate_ttl_minutes: null,
    openapi_spec_url: null,
    credential_source: { type: "personal" as const },
  },
  {
    id: "key-0003",
    label: "GitHub",
    slug: "github",
    endpoint_url: "https://api.github.com",
    endpoint_id: "ep-0003",
    api_key_id: "eak-0003",
    credential_type: "api_key",
    auth_method: "bearer",
    auth_key_name: "Authorization",
    status: "active",
    catalog_service_id: "cs-github",
    catalog_service_slug: "github",
    catalog_service_name: "GitHub",
    node_id: null,
    node_priority: 0,
    is_active: true,
    custom_user_agent: null,
    default_request_headers: null,
    ws_frame_injections: [],
    auto_connected: false,
    source_app_id: null,
    source_app_name: null,
    expires_at: null,
    last_used_at: "2026-05-04T16:30:00Z",
    error_message: null,
    created_at: "2026-02-01T11:00:00Z",
    service_type: "http",
    ssh_host: null, ssh_port: null, ssh_ca_public_key: null,
    ssh_allowed_principals: null, ssh_certificate_ttl_minutes: null,
    openapi_spec_url: null,
    credential_source: { type: "personal" as const },
  },
  {
    id: "key-0004",
    label: "Stripe",
    slug: "stripe",
    endpoint_url: "https://api.stripe.com/v1",
    endpoint_id: "ep-0004",
    api_key_id: "eak-0004",
    credential_type: "api_key",
    auth_method: "bearer",
    auth_key_name: "Authorization",
    status: "active",
    catalog_service_id: "cs-stripe",
    catalog_service_slug: "stripe",
    catalog_service_name: "Stripe",
    node_id: null,
    node_priority: 0,
    is_active: true,
    custom_user_agent: null,
    default_request_headers: null,
    ws_frame_injections: [],
    auto_connected: false,
    source_app_id: null,
    source_app_name: null,
    expires_at: null,
    last_used_at: null,
    error_message: null,
    created_at: "2026-03-10T14:00:00Z",
    service_type: "http",
    ssh_host: null, ssh_port: null, ssh_ca_public_key: null,
    ssh_allowed_principals: null, ssh_certificate_ttl_minutes: null,
    openapi_spec_url: null,
    credential_source: { type: "personal" as const },
  },
  {
    id: "key-0005",
    label: "Supabase",
    slug: "supabase",
    endpoint_url: "https://xyzproject.supabase.co",
    endpoint_id: "ep-0005",
    api_key_id: "eak-0005",
    credential_type: "api_key",
    auth_method: "bearer",
    auth_key_name: "Authorization",
    status: "active",
    catalog_service_id: null,
    catalog_service_slug: null,
    catalog_service_name: null,
    node_id: null,
    node_priority: 0,
    is_active: true,
    custom_user_agent: null,
    default_request_headers: null,
    ws_frame_injections: [],
    auto_connected: false,
    source_app_id: null,
    source_app_name: null,
    expires_at: null,
    last_used_at: "2026-05-06T08:00:00Z",
    error_message: null,
    created_at: "2026-03-25T09:00:00Z",
    service_type: "http",
    ssh_host: null, ssh_port: null, ssh_ca_public_key: null,
    ssh_allowed_principals: null, ssh_certificate_ttl_minutes: null,
    openapi_spec_url: null,
    credential_source: { type: "personal" as const },
  },
  {
    id: "key-0006",
    label: "Vercel",
    slug: "vercel",
    endpoint_url: "https://api.vercel.com",
    endpoint_id: "ep-0006",
    api_key_id: "eak-0006",
    credential_type: "api_key",
    auth_method: "bearer",
    auth_key_name: "Authorization",
    status: "active",
    catalog_service_id: null,
    catalog_service_slug: null,
    catalog_service_name: null,
    node_id: null,
    node_priority: 0,
    is_active: true,
    custom_user_agent: null,
    default_request_headers: null,
    ws_frame_injections: [],
    auto_connected: false,
    source_app_id: null,
    source_app_name: null,
    expires_at: null,
    last_used_at: null,
    error_message: null,
    created_at: "2026-04-10T10:00:00Z",
    service_type: "http",
    ssh_host: null, ssh_port: null, ssh_ca_public_key: null,
    ssh_allowed_principals: null, ssh_certificate_ttl_minutes: null,
    openapi_spec_url: null,
    credential_source: { type: "personal" as const },
  },
  {
    id: "key-0007",
    label: "Production Server",
    slug: "prod-server",
    endpoint_url: "ssh://prod.nyxid.dev",
    endpoint_id: "ep-0007",
    api_key_id: null,
    credential_type: "certificate",
    auth_method: "certificate",
    auth_key_name: "",
    status: "active",
    catalog_service_id: null,
    catalog_service_slug: null,
    catalog_service_name: null,
    node_id: null,
    node_priority: 0,
    is_active: true,
    custom_user_agent: null,
    default_request_headers: null,
    ws_frame_injections: [],
    auto_connected: false,
    source_app_id: null,
    source_app_name: null,
    expires_at: null,
    last_used_at: "2026-05-03T20:00:00Z",
    error_message: null,
    created_at: "2026-04-20T15:00:00Z",
    service_type: "ssh",
    ssh_host: "prod.nyxid.dev",
    ssh_port: 22,
    ssh_ca_public_key: "ssh-rsa AAAA...",
    ssh_allowed_principals: ["dannick"],
    ssh_certificate_ttl_minutes: 60,
    openapi_spec_url: null,
    credential_source: { type: "personal" as const },
  },
  {
    id: "key-0008",
    label: "Shared GPT-4o",
    slug: "shared-gpt4o",
    endpoint_url: "https://api.openai.com/v1",
    endpoint_id: "ep-0008",
    api_key_id: "eak-0008",
    credential_type: "api_key",
    auth_method: "bearer",
    auth_key_name: "Authorization",
    status: "active",
    catalog_service_id: "cs-openai",
    catalog_service_slug: "openai",
    catalog_service_name: "OpenAI",
    node_id: null,
    node_priority: 0,
    is_active: true,
    custom_user_agent: null,
    default_request_headers: null,
    ws_frame_injections: [],
    auto_connected: false,
    source_app_id: null,
    source_app_name: null,
    expires_at: null,
    last_used_at: "2026-05-06T21:00:00Z",
    error_message: null,
    created_at: "2026-02-15T09:00:00Z",
    service_type: "http",
    ssh_host: null, ssh_port: null, ssh_ca_public_key: null,
    ssh_allowed_principals: null, ssh_certificate_ttl_minutes: null,
    openapi_spec_url: null,
    credential_source: { type: "org" as const, org_name: "ChronoAI", role: "member", allowed: true },
  },
];

// ── External API Keys (credentials) ──
const MOCK_EXTERNAL_API_KEYS = [
  { id: "eak-0001", label: "OpenAI Production Key", credential_type: "api_key", auth_method: "bearer", auth_key_name: "Authorization", created_at: "2026-01-15T09:00:00Z", last_used_at: "2026-05-06T14:22:00Z", service_count: 1 },
  { id: "eak-0002", label: "Claude API Key", credential_type: "api_key", auth_method: "header", auth_key_name: "x-api-key", created_at: "2026-01-20T10:00:00Z", last_used_at: "2026-05-05T10:15:00Z", service_count: 1 },
  { id: "eak-0003", label: "GitHub Token", credential_type: "api_key", auth_method: "bearer", auth_key_name: "Authorization", created_at: "2026-02-01T11:00:00Z", last_used_at: "2026-05-04T16:30:00Z", service_count: 1 },
  { id: "eak-0004", label: "Stripe Secret Key", credential_type: "api_key", auth_method: "bearer", auth_key_name: "Authorization", created_at: "2026-03-10T14:00:00Z", last_used_at: null, service_count: 1 },
  { id: "eak-0005", label: "Supabase Anon Key", credential_type: "api_key", auth_method: "bearer", auth_key_name: "Authorization", created_at: "2026-03-25T09:00:00Z", last_used_at: "2026-05-06T08:00:00Z", service_count: 1 },
  { id: "eak-0006", label: "Vercel Token", credential_type: "api_key", auth_method: "bearer", auth_key_name: "Authorization", created_at: "2026-04-10T10:00:00Z", last_used_at: null, service_count: 1 },
];

// ── User Endpoints ──
const MOCK_USER_ENDPOINTS = MOCK_KEYS.filter((k) => k.service_type === "http").map((k) => ({
  id: k.endpoint_id,
  label: k.label,
  url: k.endpoint_url,
  created_at: k.created_at,
}));

// ── User Services (proxy routing) ──
const MOCK_USER_SERVICES = MOCK_KEYS.filter((k) => k.service_type === "http").map((k) => ({
  id: k.id,
  slug: k.slug,
  label: k.label,
  endpoint_id: k.endpoint_id,
  endpoint_url: k.endpoint_url,
  api_key_id: k.api_key_id,
  auth_method: k.auth_method,
  auth_key_name: k.auth_key_name,
  node_id: k.node_id,
  custom_user_agent: k.custom_user_agent,
  default_request_headers: k.default_request_headers,
  ws_frame_injections: k.ws_frame_injections,
  is_active: k.is_active,
  created_at: k.created_at,
  last_used_at: k.last_used_at,
  credential_source: k.credential_source,
  catalog_service_id: k.catalog_service_id,
  service_type: k.service_type,
}));

// ── Connections (legacy) ──
const MOCK_CONNECTIONS = [
  { service_id: "svc-openai", service_name: "OpenAI", service_category: "ai", auth_type: "api_key", has_credential: true, credential_label: "Production Key", connected_at: "2026-01-15T09:00:00Z" },
  { service_id: "svc-anthropic", service_name: "Anthropic", service_category: "ai", auth_type: "api_key", has_credential: true, credential_label: "Claude Key", connected_at: "2026-01-20T10:00:00Z" },
  { service_id: "svc-github", service_name: "GitHub", service_category: "developer", auth_type: "oauth2", has_credential: true, credential_label: null, connected_at: "2026-02-01T11:00:00Z" },
];

// ── Nodes ──
const MOCK_NODES = [
  {
    id: "node-0001",
    name: "prod-us-east",
    owner: { kind: "user" as const, id: MOCK_USER.id, display_name: "Dannick Young" },
    status: "Online",
    is_connected: true,
    last_heartbeat_at: "2026-05-06T14:30:00Z",
    connected_at: "2026-05-01T08:00:00Z",
    metadata: { agent_version: "0.9.2", os: "linux", arch: "x86_64", ip_address: "10.0.1.50" },
    metrics: { total_requests: 12450, success_count: 12380, error_count: 70, success_rate: 99.4, avg_latency_ms: 42, last_error: null, last_error_at: null, last_success_at: "2026-05-06T14:29:00Z" },
    binding_count: 3,
    created_at: "2026-02-10T09:00:00Z",
  },
  {
    id: "node-0002",
    name: "staging-eu",
    owner: { kind: "user" as const, id: MOCK_USER.id, display_name: "Dannick Young" },
    status: "Online",
    is_connected: true,
    last_heartbeat_at: "2026-05-06T14:28:00Z",
    connected_at: "2026-05-03T10:00:00Z",
    metadata: { agent_version: "0.9.2", os: "darwin", arch: "arm64", ip_address: "192.168.1.100" },
    metrics: { total_requests: 3200, success_count: 3180, error_count: 20, success_rate: 99.4, avg_latency_ms: 85, last_error: null, last_error_at: null, last_success_at: "2026-05-06T14:25:00Z" },
    binding_count: 2,
    created_at: "2026-03-15T14:00:00Z",
  },
];

// ── Notification Settings ──
const MOCK_NOTIFICATION_SETTINGS = {
  telegram_connected: true,
  telegram_username: "dannick_nyx",
  telegram_enabled: true,
  push_enabled: true,
  push_device_count: 1,
  approval_required: true,
  approval_timeout_secs: 300,
  grant_expiry_days: 30,
};

// ── Approval Requests ──
const MOCK_APPROVAL_REQUESTS = {
  requests: [
    {
      id: "ar-0001",
      service_name: "OpenAI", service_slug: "openai",
      requester_type: "api_key", requester_label: "claude-code-agent",
      operation_summary: "POST /v1/chat/completions",
      action_description: "Generate chat completion with gpt-4o",
      tool_name: null, tool_call_id: null, tool_arguments: null,
      is_destructive: false, approval_mode: "per_request" as const,
      status: "approved" as const,
      created_at: "2026-05-06T14:20:00Z", decided_at: "2026-05-06T14:20:05Z",
      decision_channel: "telegram",
    },
    {
      id: "ar-0002",
      service_name: "GitHub", service_slug: "github",
      requester_type: "api_key", requester_label: "cursor-agent",
      operation_summary: "DELETE /repos/nyxid/branch",
      action_description: "Delete branch feature/old-auth",
      tool_name: null, tool_call_id: null, tool_arguments: null,
      is_destructive: true, approval_mode: "per_request" as const,
      status: "rejected" as const,
      created_at: "2026-05-05T18:00:00Z", decided_at: "2026-05-05T18:01:30Z",
      decision_channel: "push",
    },
    {
      id: "ar-0003",
      service_name: "Stripe", service_slug: "stripe",
      requester_type: "api_key", requester_label: "ci-pipeline",
      operation_summary: "GET /v1/charges",
      action_description: "List recent charges",
      tool_name: null, tool_call_id: null, tool_arguments: null,
      is_destructive: false, approval_mode: "grant" as const,
      status: "approved" as const,
      created_at: "2026-05-04T10:00:00Z", decided_at: "2026-05-04T10:00:12Z",
      decision_channel: "telegram",
    },
  ],
  total: 3, page: 1, per_page: 20,
};

// ── Approval Grants ──
const MOCK_APPROVAL_GRANTS = {
  grants: [
    {
      id: "ag-0001", service_id: "svc-openai", service_name: "OpenAI",
      requester_type: "api_key", requester_id: "k1-0001-0001-0001-000000000001",
      requester_label: "claude-code-agent",
      granted_at: "2026-05-01T08:00:00Z", expires_at: "2026-05-31T08:00:00Z",
    },
    {
      id: "ag-0002", service_id: "svc-github", service_name: "GitHub",
      requester_type: "api_key", requester_id: "k1-0001-0001-0001-000000000002",
      requester_label: "cursor-agent",
      granted_at: "2026-04-28T12:00:00Z", expires_at: "2026-05-28T12:00:00Z",
    },
  ],
  total: 2, page: 1, per_page: 20,
};

// ── Developer Apps ──
const MOCK_DEVELOPER_APPS = [
  {
    id: "app-0001",
    client_name: "NyxID Dashboard",
    client_type: "public" as const,
    client_id: "nyxid-dashboard-pub",
    redirect_uris: ["http://localhost:3000/callback"],
    allowed_scopes: "openid profile email",
    delegation_scopes: "",
    broker_capability_enabled: false,
    is_active: true,
    client_secret: null,
    created_at: "2026-01-10T09:00:00Z",
  },
  {
    id: "app-0002",
    client_name: "Internal API Gateway",
    client_type: "confidential" as const,
    client_id: "api-gateway-conf",
    redirect_uris: ["https://api.nyxid.dev/callback"],
    allowed_scopes: "openid profile email offline_access",
    delegation_scopes: "proxy",
    broker_capability_enabled: true,
    is_active: true,
    client_secret: null,
    created_at: "2026-02-15T10:00:00Z",
  },
];

// ── Channel Bots ──
const MOCK_CHANNEL_BOTS_DATA = [
  {
    id: "bot-0001",
    platform: "telegram" as const,
    label: "NyxID Approvals",
    platform_bot_id: "bot123456",
    platform_bot_username: "nyxid_approvals_bot",
    webhook_registered: true,
    webhook_url: "https://auth.nyxid.dev/api/v1/webhooks/channel/telegram/bot-0001",
    status: "active" as const,
    is_active: true,
    created_at: "2026-03-01T09:00:00Z",
    updated_at: "2026-05-06T12:00:00Z",
    user_id: MOCK_USER.id,
    conversations_count: 1,
    app_secret_configured: true,
    lark_verification_token_configured: false,
    lark_encrypt_key_configured: false,
  },
  {
    id: "bot-0002",
    platform: "discord" as const,
    label: "Dev Notifications",
    platform_bot_id: "bot789012",
    platform_bot_username: "NyxID Dev",
    webhook_registered: true,
    webhook_url: "https://auth.nyxid.dev/api/v1/webhooks/channel/discord/bot-0002",
    status: "active" as const,
    is_active: true,
    created_at: "2026-04-10T14:00:00Z",
    updated_at: "2026-05-05T18:00:00Z",
    user_id: MOCK_USER.id,
    conversations_count: 0,
    app_secret_configured: false,
    lark_verification_token_configured: false,
    lark_encrypt_key_configured: false,
  },
];

// ── Channel Conversations ──
const MOCK_CONVERSATIONS_DATA = [
  {
    id: "conv-0001",
    channel_bot_id: "bot-0001",
    platform: "telegram" as const,
    platform_conversation_id: "chat-12345",
    platform_conversation_type: "private" as const,
    platform_sender_id: "tg-user-001",
    agent_api_key_id: "k1-0001-0001-0001-000000000001",
    agent_api_key_name: "claude-code-agent",
    default_agent: true,
    is_active: true,
    last_message_at: "2026-05-06T14:00:00Z",
    created_at: "2026-03-05T09:00:00Z",
    updated_at: "2026-05-06T14:00:00Z",
  },
];

// ── Channel Messages ──
const MOCK_CHANNEL_MESSAGES = {
  messages: [
    {
      id: "msg-0001",
      conversation_id: "conv-0001",
      direction: "inbound" as const,
      platform: "telegram",
      platform_message_id: "tg-msg-001",
      platform_sender_id: "tg-user-001",
      content_type: "text",
      content: "Can you check the deployment status?",
      created_at: "2026-05-06T13:55:00Z",
    },
    {
      id: "msg-0002",
      conversation_id: "conv-0001",
      direction: "outbound" as const,
      platform: "telegram",
      platform_message_id: "tg-msg-002",
      platform_sender_id: null,
      content_type: "text",
      content: "The deployment to production completed successfully at 13:50 UTC. All health checks are passing.",
      created_at: "2026-05-06T13:56:00Z",
    },
    {
      id: "msg-0003",
      conversation_id: "conv-0001",
      direction: "inbound" as const,
      platform: "telegram",
      platform_message_id: "tg-msg-003",
      platform_sender_id: "tg-user-001",
      content_type: "text",
      content: "Great, thanks!",
      created_at: "2026-05-06T14:00:00Z",
    },
  ],
  total: 3, page: 1, per_page: 20,
};

// ── Organizations ──
const MOCK_ORGS = [
  {
    id: "org-0001",
    display_name: "ChronoAI",
    slug: "chronoai",
    description: "AI infrastructure company",
    avatar_url: null,
    role: "owner",
    member_count: 5,
    created_at: "2025-12-01T08:00:00Z",
  },
];

const MOCK_ORG_MEMBERS = {
  members: [
    { user_id: MOCK_USER.id, email: MOCK_USER.email, display_name: MOCK_USER.display_name, avatar_url: null, role: "owner", joined_at: "2025-12-01T08:00:00Z" },
    { user_id: "u-0002", email: "alex@chronoai.dev", display_name: "Alex Chen", avatar_url: null, role: "admin", joined_at: "2025-12-15T10:00:00Z" },
    { user_id: "u-0003", email: "sarah@chronoai.dev", display_name: "Sarah Park", avatar_url: null, role: "member", joined_at: "2026-01-05T09:00:00Z" },
    { user_id: "u-0004", email: "mike@chronoai.dev", display_name: "Mike Torres", avatar_url: null, role: "member", joined_at: "2026-02-10T14:00:00Z" },
    { user_id: "u-0005", email: "lin@chronoai.dev", display_name: "Lin Wei", avatar_url: null, role: "viewer", joined_at: "2026-03-20T11:00:00Z" },
  ],
  total: 5,
};

// ── Consents ──
const MOCK_CONSENTS = [
  { id: "consent-0001", client_name: "NyxID Dashboard", client_id: "app-0001", scopes: "openid profile email", granted_at: "2026-01-10T09:30:00Z" },
];

// ── Broker Bindings ──
const MOCK_BROKER_BINDINGS: readonly unknown[] = [];

// ── Sessions ──
const MOCK_SESSIONS = [
  { id: "sess-0001", ip_address: "192.168.1.10", user_agent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)", created_at: "2026-05-06T08:00:00Z", last_active_at: "2026-05-06T14:30:00Z", is_current: true },
  { id: "sess-0002", ip_address: "10.0.0.5", user_agent: "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0)", created_at: "2026-05-05T20:00:00Z", last_active_at: "2026-05-06T12:00:00Z", is_current: false },
];

// ── Push Devices ──
const MOCK_PUSH_DEVICES = {
  devices: [
    { id: "dev-0001", platform: "ios", device_name: "iPhone 15 Pro", registered_at: "2026-04-01T09:00:00Z", last_used_at: "2026-05-06T14:00:00Z" },
  ],
};

// ── Catalog ──
const MOCK_CATALOG = [
  {
    slug: "openai", name: "OpenAI", description: "OpenAI API — GPT-4o, DALL-E, Whisper",
    base_url: "https://api.openai.com/v1", auth_method: "bearer", auth_key_name: "Authorization",
    provider_config_id: null, provider_type: null, requires_gateway_url: false,
    credential_mode: "api_key",
    api_key_instructions: "Get your API key from platform.openai.com",
    api_key_url: "https://platform.openai.com/api-keys",
    icon_url: null, documentation_url: "https://platform.openai.com/docs",
    service_type: "http",
    ssh_host: null, ssh_port: null, ssh_ca_public_key: null, ssh_allowed_principals: null, ssh_certificate_ttl_minutes: null,
    authorization_url: null, token_url: null, device_code_url: null,
    default_scopes: null, supports_pkce: null, device_code_format: null,
    oauth_client_id: null, client_id_param_name: null,
    requires_credential: true, token_exchange_credential_fields: null, default_request_headers: null,
    homepage_url: "https://openai.com", repository_url: null, issues_url: null,
    capabilities: { supports_proxy_read: true, supports_proxy_write: true, supports_proxy_binary_upload: false, supports_direct_downstream_auth: false, supports_authoring_via_nyx: false, supports_websocket: false, supports_streaming: true },
    auth_notes: "Use your API key from the OpenAI dashboard.", known_limitations: null, required_permissions: [],
  },
  {
    slug: "anthropic", name: "Anthropic", description: "Anthropic Claude API",
    base_url: "https://api.anthropic.com/v1", auth_method: "header", auth_key_name: "x-api-key",
    provider_config_id: null, provider_type: null, requires_gateway_url: false,
    credential_mode: "api_key",
    api_key_instructions: "Get your API key from console.anthropic.com",
    api_key_url: "https://console.anthropic.com/settings/keys",
    icon_url: null, documentation_url: "https://docs.anthropic.com",
    service_type: "http",
    ssh_host: null, ssh_port: null, ssh_ca_public_key: null, ssh_allowed_principals: null, ssh_certificate_ttl_minutes: null,
    authorization_url: null, token_url: null, device_code_url: null,
    default_scopes: null, supports_pkce: null, device_code_format: null,
    oauth_client_id: null, client_id_param_name: null,
    requires_credential: true, token_exchange_credential_fields: null, default_request_headers: null,
    homepage_url: "https://anthropic.com", repository_url: null, issues_url: null,
    capabilities: { supports_proxy_read: true, supports_proxy_write: true, supports_proxy_binary_upload: false, supports_direct_downstream_auth: false, supports_authoring_via_nyx: false, supports_websocket: false, supports_streaming: true },
    auth_notes: "Requires x-api-key header.", known_limitations: null, required_permissions: [],
  },
  {
    slug: "github", name: "GitHub", description: "GitHub REST & GraphQL API",
    base_url: "https://api.github.com", auth_method: "bearer", auth_key_name: "Authorization",
    provider_config_id: null, provider_type: null, requires_gateway_url: false,
    credential_mode: "api_key",
    api_key_instructions: "Create a personal access token at github.com/settings/tokens",
    api_key_url: "https://github.com/settings/tokens",
    icon_url: null, documentation_url: "https://docs.github.com/en/rest",
    service_type: "http",
    ssh_host: null, ssh_port: null, ssh_ca_public_key: null, ssh_allowed_principals: null, ssh_certificate_ttl_minutes: null,
    authorization_url: null, token_url: null, device_code_url: null,
    default_scopes: null, supports_pkce: null, device_code_format: null,
    oauth_client_id: null, client_id_param_name: null,
    requires_credential: true, token_exchange_credential_fields: null, default_request_headers: null,
    homepage_url: "https://github.com", repository_url: null, issues_url: null,
    capabilities: { supports_proxy_read: true, supports_proxy_write: true, supports_proxy_binary_upload: false, supports_direct_downstream_auth: false, supports_authoring_via_nyx: false, supports_websocket: false, supports_streaming: false },
    auth_notes: null, known_limitations: null, required_permissions: [],
  },
  {
    slug: "stripe", name: "Stripe", description: "Stripe Payments API",
    base_url: "https://api.stripe.com/v1", auth_method: "bearer", auth_key_name: "Authorization",
    provider_config_id: null, provider_type: null, requires_gateway_url: false,
    credential_mode: "api_key",
    api_key_instructions: "Find your secret key in the Stripe Dashboard under Developers > API keys",
    api_key_url: "https://dashboard.stripe.com/apikeys",
    icon_url: null, documentation_url: "https://stripe.com/docs/api",
    service_type: "http",
    ssh_host: null, ssh_port: null, ssh_ca_public_key: null, ssh_allowed_principals: null, ssh_certificate_ttl_minutes: null,
    authorization_url: null, token_url: null, device_code_url: null,
    default_scopes: null, supports_pkce: null, device_code_format: null,
    oauth_client_id: null, client_id_param_name: null,
    requires_credential: true, token_exchange_credential_fields: null, default_request_headers: null,
    homepage_url: "https://stripe.com", repository_url: null, issues_url: null,
    capabilities: { supports_proxy_read: true, supports_proxy_write: true, supports_proxy_binary_upload: false, supports_direct_downstream_auth: false, supports_authoring_via_nyx: false, supports_websocket: false, supports_streaming: false },
    auth_notes: null, known_limitations: null, required_permissions: [],
  },
];

// ── Usage Dashboard ──
const MOCK_API_KEY_USAGE_LIST = MOCK_API_KEYS.map((k, i) => {
  const counts = [1842, 956, 327, 89];
  const errors = [12, 5, 0, 3];
  const total = counts[i] ?? 200;
  const errCount = errors[i] ?? 0;
  const baseDate = new Date("2026-05-01");
  return {
    api_key_id: k.id,
    api_key_name: k.name,
    platform: k.platform,
    request_count: total,
    success_count: total - errCount,
    error_count: errCount,
    error_rate: total > 0 ? errCount / total : 0,
    last_used_at: k.last_used_at,
    prompt_tokens: total * 820,
    completion_tokens: total * 340,
    total_tokens: total * 1160,
    reported_cost: total * 0.0032,
    top_services: [
      { service_id: "s1", service_slug: "openai", service_label: "OpenAI", request_count: Math.floor(total * 0.6), error_count: 0 },
      { service_id: "s2", service_slug: "anthropic", service_label: "Anthropic", request_count: Math.floor(total * 0.3), error_count: 0 },
      { service_id: "s3", service_slug: "github-copilot", service_label: "GitHub Copilot", request_count: Math.floor(total * 0.1), error_count: 0 },
    ],
    daily_buckets: Array.from({ length: 7 }, (_, d) => {
      const date = new Date(baseDate);
      date.setDate(date.getDate() + d);
      const dayCount = Math.floor(total / 7 + (Math.sin(d * 1.5) * total) / 14);
      return {
        date: date.toISOString().split("T")[0] ?? "",
        request_count: Math.max(dayCount, 0),
        error_count: d === 3 ? errCount : 0,
      };
    }),
  };
});

// ── Approval Service Configs ──
const MOCK_SERVICE_APPROVAL_CONFIGS = {
  configs: [
    { service_id: "svc-openai", service_name: "OpenAI", approval_required: true, approval_mode: "grant" as const, created_at: "2026-03-01T00:00:00Z", updated_at: "2026-03-01T00:00:00Z", user_service_id: "key-openai-1", user_service_slug: "openai" },
    { service_id: "svc-github", service_name: "GitHub", approval_required: true, approval_mode: "per_request" as const, created_at: "2026-03-01T00:00:00Z", updated_at: "2026-03-01T00:00:00Z", user_service_id: "key-github-1", user_service_slug: "github" },
  ],
  dominant_org_policies: [],
};

const MOCK_PUBLIC_CONFIG = {
  mcp_url: "http://localhost:3001/mcp",
  node_ws_url: "ws://localhost:3001/api/v1/nodes/ws",
  version: "0.1.0-mock",
  social_providers: ["github"],
  invite_code_required: false,
  email_auth_enabled: true,
};

// ── MCP config ──
const MOCK_MCP_CONFIG = {
  mcp_enabled: true,
  mcp_url: "http://localhost:3001/mcp",
  api_key_hint: "nyxid_ag_ck••••",
};

// ── LLM Status ──
const MOCK_LLM_STATUS = {
  enabled: true,
  providers: [
    { slug: "openai", name: "OpenAI", status: "connected" },
    { slug: "anthropic", name: "Anthropic", status: "connected" },
  ],
};

// ── Admin Users ──
const MOCK_ADMIN_USERS = [
  {
    id: "d4f5a6b7-c8d9-4e0f-a1b2-c3d4e5f60718",
    email: "dannick@nyxid.dev",
    display_name: "Dannick Young",
    avatar_url: null,
    email_verified: true,
    is_active: true,
    is_admin: true,
    is_operator: false,
    role: "admin" as const,
    mfa_enabled: true,
    role_ids: ["role-001"],
    group_ids: ["grp-001"],
    created_at: "2025-11-20T08:00:00Z",
    last_login_at: "2026-05-14T09:30:00Z",
  },
  {
    id: "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
    email: "alex@chronoai.dev",
    display_name: "Alex Chen",
    avatar_url: null,
    email_verified: true,
    is_active: true,
    is_admin: false,
    is_operator: true,
    role: "operator" as const,
    mfa_enabled: true,
    role_ids: ["role-002"],
    group_ids: ["grp-001"],
    created_at: "2025-12-15T10:00:00Z",
    last_login_at: "2026-05-13T16:45:00Z",
  },
  {
    id: "b2c3d4e5-f6a7-8901-bcde-f12345678901",
    email: "sarah@chronoai.dev",
    display_name: "Sarah Park",
    avatar_url: null,
    email_verified: true,
    is_active: true,
    is_admin: false,
    is_operator: false,
    role: "user" as const,
    mfa_enabled: false,
    role_ids: [],
    group_ids: ["grp-002"],
    created_at: "2026-01-05T09:00:00Z",
    last_login_at: "2026-05-12T11:20:00Z",
  },
  {
    id: "c3d4e5f6-a7b8-9012-cdef-123456789012",
    email: "mike@chronoai.dev",
    display_name: "Mike Torres",
    avatar_url: null,
    email_verified: true,
    is_active: true,
    is_admin: false,
    is_operator: false,
    role: "user" as const,
    mfa_enabled: true,
    role_ids: ["role-002"],
    group_ids: ["grp-002"],
    created_at: "2026-02-10T14:00:00Z",
    last_login_at: "2026-05-14T08:10:00Z",
  },
  {
    id: "d4e5f6a7-b8c9-0123-defa-234567890123",
    email: "lin@chronoai.dev",
    display_name: "Lin Wei",
    avatar_url: null,
    email_verified: false,
    is_active: true,
    is_admin: false,
    is_operator: false,
    role: "user" as const,
    mfa_enabled: false,
    role_ids: [],
    group_ids: [],
    created_at: "2026-03-20T11:00:00Z",
    last_login_at: "2026-05-10T15:00:00Z",
  },
  {
    id: "e5f6a7b8-c9d0-1234-efab-345678901234",
    email: "deactivated@example.com",
    display_name: "Former Employee",
    avatar_url: null,
    email_verified: true,
    is_active: false,
    is_admin: false,
    is_operator: false,
    role: "user" as const,
    mfa_enabled: false,
    role_ids: [],
    group_ids: [],
    created_at: "2025-10-01T12:00:00Z",
    last_login_at: "2026-01-15T09:00:00Z",
  },
];

// ── Admin Audit Log ──
const MOCK_AUDIT_LOG = [
  { id: "aud-001", user_id: MOCK_ADMIN_USERS[0]!.id, api_key_id: null, api_key_name: null, event_type: "user.login", event_data: { method: "password", ip: "192.168.1.10" }, ip_address: "192.168.1.10", user_agent: "Mozilla/5.0 (Macintosh)", created_at: "2026-05-14T09:30:00Z" },
  { id: "aud-002", user_id: MOCK_ADMIN_USERS[0]!.id, api_key_id: "k1-0001-0001-0001-000000000001", api_key_name: "claude-code-agent", event_type: "proxy.request", event_data: { service: "openai", method: "POST", path: "/v1/chat/completions" }, ip_address: "10.0.1.50", user_agent: "nyxid-agent/0.9.2", created_at: "2026-05-14T09:25:00Z" },
  { id: "aud-003", user_id: MOCK_ADMIN_USERS[1]!.id, api_key_id: null, api_key_name: null, event_type: "user.login", event_data: { method: "password" }, ip_address: "10.0.0.5", user_agent: "Mozilla/5.0 (Windows NT 10.0)", created_at: "2026-05-13T16:45:00Z" },
  { id: "aud-004", user_id: MOCK_ADMIN_USERS[0]!.id, api_key_id: null, api_key_name: null, event_type: "service_account.create", event_data: { name: "CI/CD Pipeline" }, ip_address: "192.168.1.10", user_agent: "Mozilla/5.0 (Macintosh)", created_at: "2026-05-13T14:00:00Z" },
  { id: "aud-005", user_id: MOCK_ADMIN_USERS[2]!.id, api_key_id: null, api_key_name: null, event_type: "mfa.setup", event_data: { method: "totp" }, ip_address: "172.16.0.20", user_agent: "Mozilla/5.0 (Linux)", created_at: "2026-05-13T10:00:00Z" },
  { id: "aud-006", user_id: MOCK_ADMIN_USERS[3]!.id, api_key_id: null, api_key_name: null, event_type: "user.login", event_data: { method: "password" }, ip_address: "192.168.1.42", user_agent: "Mozilla/5.0 (Macintosh)", created_at: "2026-05-14T08:10:00Z" },
  { id: "aud-007", user_id: MOCK_ADMIN_USERS[0]!.id, api_key_id: null, api_key_name: null, event_type: "invite_code.create", event_data: { max_uses: 5 }, ip_address: "192.168.1.10", user_agent: "Mozilla/5.0 (Macintosh)", created_at: "2026-05-12T15:00:00Z" },
  { id: "aud-008", user_id: MOCK_ADMIN_USERS[4]!.id, api_key_id: null, api_key_name: null, event_type: "user.register", event_data: { invite_code: "CHRONO-2026" }, ip_address: "203.0.113.50", user_agent: "Mozilla/5.0 (iPhone)", created_at: "2026-05-10T15:00:00Z" },
  { id: "aud-009", user_id: MOCK_ADMIN_USERS[0]!.id, api_key_id: null, api_key_name: null, event_type: "role.create", event_data: { name: "API Consumer" }, ip_address: "192.168.1.10", user_agent: "Mozilla/5.0 (Macintosh)", created_at: "2026-05-10T11:00:00Z" },
  { id: "aud-010", user_id: MOCK_ADMIN_USERS[0]!.id, api_key_id: null, api_key_name: null, event_type: "user.status_change", event_data: { target_user: "deactivated@example.com", is_active: false }, ip_address: "192.168.1.10", user_agent: "Mozilla/5.0 (Macintosh)", created_at: "2026-05-09T10:00:00Z" },
];

// ── Admin Invite Codes ──
const MOCK_INVITE_CODES = [
  {
    id: "inv-001", code: "CHRONO-2026", max_uses: 5, used_count: 3, is_active: true,
    created_by: MOCK_ADMIN_USERS[0]!.id,
    creator: { email: "dannick@nyxid.dev", display_name: "Donnick Young" },
    note: "Team onboarding Q1 2026",
    created_at: "2026-01-10T09:00:00Z", updated_at: "2026-03-20T11:00:00Z",
    usages: [
      { user_id: MOCK_ADMIN_USERS[2]!.id, used_at: "2026-01-05T09:00:00Z", user_email: "sarah@chronoai.dev", user_display_name: "Sarah Park" },
      { user_id: MOCK_ADMIN_USERS[3]!.id, used_at: "2026-02-10T14:00:00Z", user_email: "mike@chronoai.dev", user_display_name: "Mike Torres" },
      { user_id: MOCK_ADMIN_USERS[4]!.id, used_at: "2026-03-20T11:00:00Z", user_email: "lin@chronoai.dev", user_display_name: "Lin Wei" },
    ],
  },
  {
    id: "inv-002", code: "PARTNER-VIP", max_uses: 10, used_count: 0, is_active: true,
    created_by: MOCK_ADMIN_USERS[0]!.id,
    creator: { email: "dannick@nyxid.dev", display_name: "Dannick Young" },
    note: "Partner program invites",
    created_at: "2026-04-01T12:00:00Z", updated_at: "2026-04-01T12:00:00Z",
    usages: [],
  },
  {
    id: "inv-003", code: "BETA-TEST-42", max_uses: 1, used_count: 1, is_active: false,
    created_by: MOCK_ADMIN_USERS[1]!.id,
    creator: { email: "alex@chronoai.dev", display_name: "Alex Chen" },
    note: null,
    created_at: "2025-12-20T08:00:00Z", updated_at: "2026-01-05T09:00:00Z",
    usages: [
      { user_id: MOCK_ADMIN_USERS[2]!.id, used_at: "2026-01-05T09:00:00Z", user_email: "sarah@chronoai.dev", user_display_name: "Sarah Park" },
    ],
  },
];

// ── Admin Roles ──
const MOCK_ROLES = [
  {
    id: "role-001", name: "Platform Admin", slug: "platform-admin",
    description: "Full administrative access to all platform features",
    permissions: ["admin:read", "admin:write", "users:manage", "roles:manage", "audit:read"],
    is_default: false, is_system: true, client_id: null,
    created_at: "2025-11-01T00:00:00Z", updated_at: "2025-11-01T00:00:00Z",
  },
  {
    id: "role-002", name: "API Consumer", slug: "api-consumer",
    description: "Can connect services and use the proxy",
    permissions: ["proxy:read", "proxy:write", "services:read", "keys:manage"],
    is_default: true, is_system: false, client_id: null,
    created_at: "2026-01-15T10:00:00Z", updated_at: "2026-03-10T14:00:00Z",
  },
  {
    id: "role-003", name: "Node Operator", slug: "node-operator",
    description: "Can register and manage credential nodes",
    permissions: ["nodes:manage", "proxy:read", "proxy:write"],
    is_default: false, is_system: false, client_id: null,
    created_at: "2026-02-20T09:00:00Z", updated_at: "2026-02-20T09:00:00Z",
  },
  {
    id: "role-004", name: "Audit Viewer", slug: "audit-viewer",
    description: "Read-only access to audit logs",
    permissions: ["audit:read"],
    is_default: false, is_system: false, client_id: null,
    created_at: "2026-03-05T11:00:00Z", updated_at: "2026-03-05T11:00:00Z",
  },
];

// ── Admin Groups ──
const MOCK_GROUPS = [
  {
    id: "grp-001", name: "Engineering", slug: "engineering",
    description: "Core engineering team with full proxy and node access",
    roles: [MOCK_ROLES[1]!, MOCK_ROLES[2]!],
    parent_group_id: null, member_count: 3,
    created_at: "2025-12-01T08:00:00Z", updated_at: "2026-04-10T14:00:00Z",
  },
  {
    id: "grp-002", name: "Product", slug: "product",
    description: "Product team with service access",
    roles: [MOCK_ROLES[1]!],
    parent_group_id: null, member_count: 2,
    created_at: "2026-01-10T09:00:00Z", updated_at: "2026-03-15T10:00:00Z",
  },
  {
    id: "grp-003", name: "Security", slug: "security",
    description: "Security team with audit access",
    roles: [MOCK_ROLES[3]!],
    parent_group_id: null, member_count: 1,
    created_at: "2026-02-15T11:00:00Z", updated_at: "2026-02-15T11:00:00Z",
  },
];

const MOCK_GROUP_MEMBERS: Record<string, { members: unknown[]; total: number }> = {
  "grp-001": {
    members: [
      { id: MOCK_ADMIN_USERS[0]!.id, email: "dannick@nyxid.dev", display_name: "Dannick Young" },
      { id: MOCK_ADMIN_USERS[1]!.id, email: "alex@chronoai.dev", display_name: "Alex Chen" },
      { id: MOCK_ADMIN_USERS[3]!.id, email: "mike@chronoai.dev", display_name: "Mike Torres" },
    ],
    total: 3,
  },
  "grp-002": {
    members: [
      { id: MOCK_ADMIN_USERS[2]!.id, email: "sarah@chronoai.dev", display_name: "Sarah Park" },
      { id: MOCK_ADMIN_USERS[3]!.id, email: "mike@chronoai.dev", display_name: "Mike Torres" },
    ],
    total: 2,
  },
  "grp-003": {
    members: [
      { id: MOCK_ADMIN_USERS[1]!.id, email: "alex@chronoai.dev", display_name: "Alex Chen" },
    ],
    total: 1,
  },
};

// ── Admin Service Accounts ──
const MOCK_SERVICE_ACCOUNTS = [
  {
    id: "sa-001", name: "CI/CD Pipeline", description: "Automated deployment pipeline",
    client_id: "nyx_sa_ci_cd_pipeline_8f3a", secret_prefix: "nyx_ss_8f3a",
    allowed_scopes: "openid proxy:* llm:proxy", role_ids: ["role-002"],
    is_active: true, rate_limit_override: 50,
    created_by: MOCK_ADMIN_USERS[0]!.id,
    created_at: "2026-03-01T09:00:00Z", updated_at: "2026-05-10T14:00:00Z",
    last_authenticated_at: "2026-05-14T06:00:00Z",
  },
  {
    id: "sa-002", name: "Monitoring Agent", description: "Health check and monitoring service",
    client_id: "nyx_sa_monitoring_agent_2b7c", secret_prefix: "nyx_ss_2b7c",
    allowed_scopes: "openid proxy:read", role_ids: [],
    is_active: true, rate_limit_override: null,
    created_by: MOCK_ADMIN_USERS[0]!.id,
    created_at: "2026-04-15T11:00:00Z", updated_at: "2026-04-15T11:00:00Z",
    last_authenticated_at: "2026-05-14T09:28:00Z",
  },
  {
    id: "sa-003", name: "Data Sync Worker", description: null,
    client_id: "nyx_sa_data_sync_worker_9d1e", secret_prefix: "nyx_ss_9d1e",
    allowed_scopes: "openid proxy:read proxy:write", role_ids: ["role-002"],
    is_active: false, rate_limit_override: 20,
    created_by: MOCK_ADMIN_USERS[1]!.id,
    created_at: "2026-02-20T15:00:00Z", updated_at: "2026-05-01T10:00:00Z",
    last_authenticated_at: "2026-04-28T22:00:00Z",
  },
];

// ── Admin Nodes ──
const MOCK_ADMIN_NODES = [
  {
    id: "node-0001", name: "prod-us-east",
    user_id: MOCK_ADMIN_USERS[0]!.id, user_email: "dannick@nyxid.dev",
    status: "Online", is_connected: true,
    last_heartbeat_at: "2026-05-14T09:30:00Z", connected_at: "2026-05-12T08:00:00Z",
    metadata: { agent_version: "0.9.2", os: "linux", arch: "x86_64", ip_address: "10.0.1.50" },
    metrics: { total_requests: 12450, success_count: 12380, error_count: 70, success_rate: 0.994, avg_latency_ms: 42, last_error: null, last_error_at: null, last_success_at: "2026-05-14T09:29:00Z" },
    binding_count: 3, created_at: "2026-02-10T09:00:00Z",
  },
  {
    id: "node-0002", name: "staging-eu",
    user_id: MOCK_ADMIN_USERS[0]!.id, user_email: "dannick@nyxid.dev",
    status: "Online", is_connected: true,
    last_heartbeat_at: "2026-05-14T09:28:00Z", connected_at: "2026-05-10T10:00:00Z",
    metadata: { agent_version: "0.9.2", os: "darwin", arch: "arm64", ip_address: "192.168.1.100" },
    metrics: { total_requests: 3200, success_count: 3180, error_count: 20, success_rate: 0.994, avg_latency_ms: 85, last_error: null, last_error_at: null, last_success_at: "2026-05-14T09:25:00Z" },
    binding_count: 2, created_at: "2026-03-15T14:00:00Z",
  },
  {
    id: "node-0003", name: "alex-dev-local",
    user_id: MOCK_ADMIN_USERS[1]!.id, user_email: "alex@chronoai.dev",
    status: "Offline", is_connected: false,
    last_heartbeat_at: "2026-05-13T18:00:00Z", connected_at: null,
    metadata: { agent_version: "0.9.1", os: "darwin", arch: "arm64", ip_address: "192.168.1.42" },
    metrics: { total_requests: 890, success_count: 875, error_count: 15, success_rate: 0.983, avg_latency_ms: 120, last_error: "connection timeout", last_error_at: "2026-05-13T17:55:00Z", last_success_at: "2026-05-13T17:50:00Z" },
    binding_count: 1, created_at: "2026-04-01T10:00:00Z",
  },
  {
    id: "node-0004", name: "prod-drain-test",
    user_id: MOCK_ADMIN_USERS[0]!.id, user_email: "dannick@nyxid.dev",
    status: "Draining", is_connected: true,
    last_heartbeat_at: "2026-05-14T09:29:00Z", connected_at: "2026-05-14T06:00:00Z",
    metadata: { agent_version: "0.9.2", os: "linux", arch: "x86_64", ip_address: "10.0.1.51" },
    metrics: { total_requests: 450, success_count: 448, error_count: 2, success_rate: 0.996, avg_latency_ms: 38, last_error: null, last_error_at: null, last_success_at: "2026-05-14T09:20:00Z" },
    binding_count: 1, created_at: "2026-05-01T12:00:00Z",
  },
];

// ── Admin Sessions ──
const MOCK_ADMIN_SESSIONS: Record<string, { sessions: unknown[]; total: number }> = {
  [MOCK_ADMIN_USERS[0]!.id]: {
    sessions: [
      { id: "sess-a01", ip_address: "192.168.1.10", user_agent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)", created_at: "2026-05-14T09:30:00Z", expires_at: "2026-05-21T09:30:00Z", last_active_at: "2026-05-14T09:30:00Z", revoked: false },
      { id: "sess-a02", ip_address: "10.0.0.5", user_agent: "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0)", created_at: "2026-05-13T20:00:00Z", expires_at: "2026-05-20T20:00:00Z", last_active_at: "2026-05-14T08:00:00Z", revoked: false },
    ],
    total: 2,
  },
};

// ── Admin User Roles / Groups ──
const MOCK_USER_ROLES: Record<string, { direct_roles: unknown[]; inherited_roles: unknown[]; effective_permissions: string[] }> = {
  [MOCK_ADMIN_USERS[0]!.id]: { direct_roles: [MOCK_ROLES[0]], inherited_roles: [MOCK_ROLES[1], MOCK_ROLES[2]], effective_permissions: ["admin:read", "admin:write", "users:manage", "roles:manage", "audit:read", "proxy:read", "proxy:write", "services:read", "keys:manage", "nodes:manage"] },
  [MOCK_ADMIN_USERS[1]!.id]: { direct_roles: [MOCK_ROLES[1]], inherited_roles: [], effective_permissions: ["proxy:read", "proxy:write", "services:read", "keys:manage"] },
  [MOCK_ADMIN_USERS[3]!.id]: { direct_roles: [MOCK_ROLES[1]], inherited_roles: [], effective_permissions: ["proxy:read", "proxy:write", "services:read", "keys:manage"] },
};

const MOCK_USER_GROUPS_MAP: Record<string, { groups: unknown[] }> = {
  [MOCK_ADMIN_USERS[0]!.id]: { groups: [MOCK_GROUPS[0]] },
  [MOCK_ADMIN_USERS[1]!.id]: { groups: [MOCK_GROUPS[0]] },
  [MOCK_ADMIN_USERS[2]!.id]: { groups: [MOCK_GROUPS[1]] },
  [MOCK_ADMIN_USERS[3]!.id]: { groups: [MOCK_GROUPS[0], MOCK_GROUPS[1]] },
};

// ── Helper: find by ID in an array ──
function findById<T extends { id: string }>(items: readonly T[], id: string): T | undefined {
  return items.find((item) => item.id === id);
}

function findBySlug<T extends { slug: string }>(items: readonly T[], slug: string): T | undefined {
  return items.find((item) => item.slug === slug);
}

// ── Dynamic endpoint resolver ──
type MockHandler = (path: string) => unknown | undefined;

const MOCK_HANDLERS: MockHandler[] = [
  // User
  (p) => p === "/users/me" ? MOCK_USER : undefined,
  (p) => p === "/users/me/primary-org" ? MOCK_ORGS[0] : undefined,

  // API keys usage (must be before generic /api-keys patterns)
  (p) => p.match(/^\/api-keys\/usage/) ? { usage: MOCK_API_KEY_USAGE_LIST, since: "2026-05-01T00:00:00Z", days: 7 } : undefined,

  // API keys external
  (p) => p.match(/^\/api-keys\/external$/) ? { keys: MOCK_EXTERNAL_API_KEYS } : undefined,
  (p) => {
    const m = p.match(/^\/api-keys\/external\/([\w-]+)$/);
    return m ? findById(MOCK_EXTERNAL_API_KEYS, m[1] ?? "") ?? MOCK_EXTERNAL_API_KEYS[0] : undefined;
  },

  // API key bindings
  (p) => {
    const m = p.match(/^\/api-keys\/([\w-]+)\/bindings$/);
    if (!m) return undefined;
    return { bindings: MOCK_BINDINGS[m[1] ?? ""] ?? [] };
  },

  // API key per-key usage
  (p) => {
    const m = p.match(/^\/api-keys\/([\w-]+)\/usage/);
    if (!m) return undefined;
    return MOCK_API_KEY_USAGE_LIST.find((u) => u.api_key_id === m[1]) ?? MOCK_API_KEY_USAGE_LIST[0];
  },

  // API key detail
  (p) => {
    const m = p.match(/^\/api-keys\/([\w-]+)$/);
    return m ? findById(MOCK_API_KEYS, m[1] ?? "") ?? MOCK_API_KEYS[0] : undefined;
  },

  // API keys list
  (p) => p.match(/^\/api-keys$/) ? { keys: MOCK_API_KEYS } : undefined,

  // Keys (external services)
  (p) => {
    const m = p.match(/^\/keys\/([\w-]+)$/);
    return m ? findById(MOCK_KEYS, m[1] ?? "") ?? MOCK_KEYS[0] : undefined;
  },
  (p) => p === "/keys" ? { keys: MOCK_KEYS } : undefined,

  // User endpoints
  (p) => p.match(/^\/endpoints/) ? { endpoints: MOCK_USER_ENDPOINTS } : undefined,

  // User services
  (p) => p.match(/^\/user-services/) ? { services: MOCK_USER_SERVICES } : undefined,

  // Connections
  (p) => p.match(/^\/connections$/) ? { connections: MOCK_CONNECTIONS } : undefined,

  // Nodes
  (p) => p.match(/^\/nodes\/my-bindings/) ? { bindings: [] } : undefined,
  (p) => {
    const m = p.match(/^\/nodes\/([\w-]+)\/admins/);
    return m ? { admins: [] } : undefined;
  },
  (p) => {
    const m = p.match(/^\/nodes\/([\w-]+)\/pending-credentials/);
    return m ? { credentials: [] } : undefined;
  },
  (p) => {
    const m = p.match(/^\/nodes\/([\w-]+)$/);
    return m ? findById(MOCK_NODES, m[1] ?? "") ?? MOCK_NODES[0] : undefined;
  },
  (p) => p === "/nodes" ? { nodes: MOCK_NODES } : undefined,

  // Notifications
  (p) => p === "/notifications/settings" ? MOCK_NOTIFICATION_SETTINGS : undefined,
  (p) => p.match(/^\/notifications\/devices/) ? MOCK_PUSH_DEVICES : undefined,
  (p) => p.match(/^\/notifications\/telegram/) ? { link_code: "MOCK-LINK-CODE", bot_username: "nyxid_approvals_bot", expires_in_secs: 600 } : undefined,

  // Approvals
  (p) => p.match(/^\/approvals\/requests/) ? MOCK_APPROVAL_REQUESTS : undefined,
  (p) => p.match(/^\/approvals\/grants/) ? MOCK_APPROVAL_GRANTS : undefined,
  (p) => p.match(/^\/approvals\/service-configs/) ? MOCK_SERVICE_APPROVAL_CONFIGS : undefined,

  // Developer apps
  (p) => {
    const m = p.match(/^\/developer\/oauth-clients\/([\w-]+)$/);
    return m ? findById(MOCK_DEVELOPER_APPS, m[1] ?? "") ?? MOCK_DEVELOPER_APPS[0] : undefined;
  },
  (p) => p === "/developer/oauth-clients" ? { clients: MOCK_DEVELOPER_APPS } : undefined,

  // Channel bots
  (p) => {
    const m = p.match(/^\/channel-bots\/([\w-]+)$/);
    return m ? findById(MOCK_CHANNEL_BOTS_DATA, m[1] ?? "") ?? MOCK_CHANNEL_BOTS_DATA[0] : undefined;
  },
  (p) => p.match(/^\/channel-bots$/) ? { bots: MOCK_CHANNEL_BOTS_DATA, total: MOCK_CHANNEL_BOTS_DATA.length } : undefined,

  // Channel conversations
  (p) => {
    const m = p.match(/^\/channel-conversations\/([\w-]+)\/messages/);
    return m ? MOCK_CHANNEL_MESSAGES : undefined;
  },
  (p) => {
    const m = p.match(/^\/channel-conversations\/([\w-]+)$/);
    return m ? findById(MOCK_CONVERSATIONS_DATA, m[1] ?? "") ?? MOCK_CONVERSATIONS_DATA[0] : undefined;
  },
  (p) => p.match(/^\/channel-conversations/) ? { conversations: MOCK_CONVERSATIONS_DATA, total: MOCK_CONVERSATIONS_DATA.length } : undefined,

  // Organizations
  (p) => {
    const m = p.match(/^\/orgs\/([\w-]+)\/members/);
    return m ? MOCK_ORG_MEMBERS : undefined;
  },
  (p) => {
    const m = p.match(/^\/orgs\/([\w-]+)\/invites/);
    return m ? { invites: [] } : undefined;
  },
  (p) => {
    const m = p.match(/^\/orgs\/([\w-]+)\/role-scopes/);
    return m ? { scopes: [] } : undefined;
  },
  (p) => {
    const m = p.match(/^\/orgs\/([\w-]+)$/);
    return m ? findById(MOCK_ORGS, m[1] ?? "") ?? { ...MOCK_ORGS[0], id: m[1] } : undefined;
  },
  (p) => p === "/orgs" ? { orgs: MOCK_ORGS } : undefined,

  // Consents & broker
  (p) => p.match(/^\/users\/me\/consents/) ? { consents: MOCK_CONSENTS } : undefined,
  (p) => p.match(/^\/users\/me\/broker-bindings/) ? { bindings: MOCK_BROKER_BINDINGS } : undefined,
  (p) => p === "/auth/consents/me" ? { consents: MOCK_CONSENTS } : undefined,
  (p) => p === "/broker/bindings/me" ? { bindings: MOCK_BROKER_BINDINGS } : undefined,

  // Sessions
  (p) => p === "/sessions" ? MOCK_SESSIONS : undefined,

  // Catalog
  (p) => {
    const m = p.match(/^\/catalog\/([\w-]+)\/shape/);
    return m ? { endpoints: [{ method: "POST", path: "/v1/chat/completions", summary: "Create chat completion" }, { method: "GET", path: "/v1/models", summary: "List models" }] } : undefined;
  },
  (p) => {
    const m = p.match(/^\/catalog\/([\w-]+)\/endpoints/);
    return m ? { endpoints: [{ method: "POST", path: "/v1/chat/completions", summary: "Create chat completion" }, { method: "GET", path: "/v1/models", summary: "List models" }, { method: "POST", path: "/v1/embeddings", summary: "Create embeddings" }] } : undefined;
  },
  (p) => {
    const m = p.match(/^\/catalog\/([\w-]+)$/);
    return m ? findBySlug(MOCK_CATALOG, m[1] ?? "") ?? MOCK_CATALOG[0] : undefined;
  },
  (p) => p.match(/^\/catalog$/) ? { entries: MOCK_CATALOG } : undefined,

  // ── Admin endpoints ──

  // Admin users
  (p) => {
    const m = p.match(/^\/admin\/users\/([\w-]+)\/sessions$/);
    return m ? (MOCK_ADMIN_SESSIONS[m[1] ?? ""] ?? { sessions: [], total: 0 }) : undefined;
  },
  (p) => {
    const m = p.match(/^\/admin\/users\/([\w-]+)\/roles$/);
    return m ? (MOCK_USER_ROLES[m[1] ?? ""] ?? { direct_roles: [], inherited_roles: [], effective_permissions: [] }) : undefined;
  },
  (p) => {
    const m = p.match(/^\/admin\/users\/([\w-]+)\/groups$/);
    return m ? (MOCK_USER_GROUPS_MAP[m[1] ?? ""] ?? { groups: [] }) : undefined;
  },
  (p) => {
    const m = p.match(/^\/admin\/users\/([\w-]+)$/);
    return m ? findById(MOCK_ADMIN_USERS, m[1] ?? "") ?? MOCK_ADMIN_USERS[0] : undefined;
  },
  (p) => p.match(/^\/admin\/users$/) ? { users: MOCK_ADMIN_USERS, total: MOCK_ADMIN_USERS.length, page: 1, per_page: 20 } : undefined,

  // Admin audit log
  (p) => p.match(/^\/admin\/audit-log/) ? { entries: MOCK_AUDIT_LOG, total: MOCK_AUDIT_LOG.length, page: 1, per_page: 50 } : undefined,

  // Admin invite codes
  (p) => p.match(/^\/admin\/invite-codes$/) ? { invite_codes: MOCK_INVITE_CODES } : undefined,

  // Admin service accounts
  (p) => {
    const m = p.match(/^\/admin\/service-accounts\/([\w-]+)\/providers$/);
    return m ? { tokens: [] } : undefined;
  },
  (p) => {
    const m = p.match(/^\/admin\/service-accounts\/([\w-]+)\/connections$/);
    return m ? { connections: [] } : undefined;
  },
  (p) => {
    const m = p.match(/^\/admin\/service-accounts\/([\w-]+)$/);
    return m ? findById(MOCK_SERVICE_ACCOUNTS, m[1] ?? "") ?? MOCK_SERVICE_ACCOUNTS[0] : undefined;
  },
  (p) => p.match(/^\/admin\/service-accounts$/) ? { service_accounts: MOCK_SERVICE_ACCOUNTS, total: MOCK_SERVICE_ACCOUNTS.length, page: 1, per_page: 20 } : undefined,

  // Admin nodes
  (p) => {
    const m = p.match(/^\/admin\/nodes\/([\w-]+)$/);
    return m ? findById(MOCK_ADMIN_NODES, m[1] ?? "") ?? MOCK_ADMIN_NODES[0] : undefined;
  },
  (p) => p.match(/^\/admin\/nodes$/) ? { nodes: MOCK_ADMIN_NODES, total: MOCK_ADMIN_NODES.length, page: 1, per_page: 50 } : undefined,

  // Roles
  (p) => {
    const m = p.match(/^\/roles\/([\w-]+)$/);
    return m ? findById(MOCK_ROLES, m[1] ?? "") ?? MOCK_ROLES[0] : undefined;
  },
  (p) => p.match(/^\/roles$/) ? { roles: MOCK_ROLES } : undefined,

  // Groups
  (p) => {
    const m = p.match(/^\/groups\/([\w-]+)\/members$/);
    return m ? (MOCK_GROUP_MEMBERS[m[1] ?? ""] ?? { members: [], total: 0 }) : undefined;
  },
  (p) => {
    const m = p.match(/^\/groups\/([\w-]+)$/);
    return m ? findById(MOCK_GROUPS, m[1] ?? "") ?? MOCK_GROUPS[0] : undefined;
  },
  (p) => p.match(/^\/groups$/) ? { groups: MOCK_GROUPS } : undefined,

  // Services (admin/legacy)
  (p) => p === "/services" ? { services: [] } : undefined,

  // MCP
  (p) => p.match(/^\/mcp/) ? MOCK_MCP_CONFIG : undefined,

  // LLM
  (p) => p.match(/^\/llm\/status/) ? MOCK_LLM_STATUS : undefined,

  // Public config
  (p) => p === "/public/config" ? MOCK_PUBLIC_CONFIG : undefined,
];

let _mockLatched: boolean | null = null;

export function isMockMode(): boolean {
  if (_mockLatched !== null) return _mockLatched;
  _mockLatched = import.meta.env.DEV && new URLSearchParams(window.location.search).has("mock");
  return _mockLatched;
}

export function getMockResponse(endpoint: string): unknown | undefined {
  const path = endpoint.split("?")[0] ?? endpoint;
  for (const handler of MOCK_HANDLERS) {
    const result = handler(path);
    if (result !== undefined) return result;
  }
  return undefined;
}

export function getMockUser() {
  return MOCK_USER;
}
