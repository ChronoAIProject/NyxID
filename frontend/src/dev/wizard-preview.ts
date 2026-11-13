// Loaded only by the opt-in Vite preview route; never by a production entry.
import type { WizardBootstrap } from "@/components/cli-wizard/client";

const flows = new Set([
  "ai-key",
  "api-key-create",
  "api-key-rotate",
  "node-register-token",
  "node-rotate-token",
  "service-account-create",
  "service-account-rotate-secret",
  "developer-app-create",
  "developer-app-rotate-secret",
  "mfa-setup",
]);
const requested =
  new URLSearchParams(window.location.search).get("flow") ?? "ai-key";
const flow = flows.has(requested) ? requested : "ai-key";
window.__WIZARD_BOOTSTRAP__ = {
  flow,
  csrf: "local-preview",
  baseUrl: window.location.origin,
  context: "local",
  prefill: {
    name: "preview-agent",
    resource_id: "preview",
    display_name: "Preview",
    id: "preview",
    scopes: "read write",
    ttl_seconds: 3600,
    client_type: "confidential",
  },
} as WizardBootstrap;

const entries = [
  {
    slug: "llm-openai",
    name: "OpenAI",
    auth_method: "bearer",
    provider_type: "api_key",
    service_type: "http",
    requires_credential: true,
    requires_gateway_url: false,
    base_url: "https://api.openai.com",
  },
  {
    slug: "demo-api",
    name: "Demo API",
    auth_method: "none",
    provider_type: "api_key",
    service_type: "http",
    requires_credential: false,
    requires_gateway_url: false,
    base_url: "https://example.test",
  },
];
const originalFetch = window.fetch.bind(window);
window.fetch = async (input, init) => {
  const url = new URL(
    typeof input === "string"
      ? input
      : input instanceof URL
        ? input.href
        : input.url,
    window.location.origin,
  );
  if (!url.pathname.startsWith("/api/")) return originalFetch(input, init);
  const path = url.pathname
    .replace(/^\/api\/proxy/, "")
    .replace(/^\/api\/v1/, "");
  const method =
    init?.method ?? (input instanceof Request ? input.method : "GET");
  let body: unknown;
  if (
    path.endsWith("/heartbeat") ||
    path.endsWith("/complete") ||
    path.endsWith("/cancel")
  )
    body = { ok: true };
  else if (path === "/catalog") body = { entries, services: entries };
  else if (path.startsWith("/catalog/"))
    body = entries.find((entry) => path.endsWith(entry.slug)) ?? entries[0];
  else if (path === "/keys" && method === "GET") body = { keys: [] };
  else if (path === "/nodes" && method === "GET") body = { nodes: [] };
  else if (path === "/orgs") body = { orgs: [] };
  else if (path === "/users/me")
    body = {
      id: "preview",
      email: "preview@example.test",
      display_name: "Preview",
      mfa_enabled: false,
    };
  else if (path === "/auth/mfa/setup")
    body = {
      factor_id: "preview-factor",
      secret: "JBSWY3DPEHPK3PXP",
      qr_code_url:
        "otpauth://totp/NyxID:preview?secret=JBSWY3DPEHPK3PXP&issuer=NyxID",
    };
  else if (path === "/auth/mfa/confirm")
    body = { recovery_codes: ["DEMO-ONLY-0001", "DEMO-ONLY-0002"] };
  else if (method !== "GET")
    body = {
      id: "preview",
      api_key_id: "preview",
      token_id: "preview",
      full_key: "nyx_preview_example_only",
      token: "nyx_preview_example_only",
      registration_token: "nyx_preview_example_only",
      auth_token: "nyx_preview_example_only",
      signing_secret: "preview_example_only",
      client_id: "preview-client",
      client_secret: "preview_example_only",
      slug: "demo-api",
      label: "Demo API",
      endpoint_url: "https://example.test",
      status: "active",
    };
  else
    body = {
      id: "preview",
      name: "Preview",
      scopes: "read write",
      platform: "generic",
      status: "online",
    };
  return Response.json(body);
};

void import("../wizard-entry");
