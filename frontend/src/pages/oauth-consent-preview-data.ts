import type { ConsentServiceDisplay } from "./oauth-consent";

export const previewServices: readonly ConsentServiceDisplay[] = [
  {
    id: "chrono-sandbox",
    label: "Chrono Sandbox",
    slug: "chrono-sandbox",
    catalog_service_name: "Chrono Sandbox",
    catalog_service_description:
      "Explore and test AI requests in a sandbox environment. Keep experiments separate from production services.",
    resource_uri: "https://nyx.example/api/v1/proxy/s/chrono-sandbox",
    is_active: true,
    credential_source: { type: "personal" },
  },
  {
    id: "chrono-llm-public",
    label: "Chrono LLM (public)",
    slug: "chrono-llm-public",
    catalog_service_name: "Chrono LLM (public)",
    catalog_service_description:
      "Access shared language models for chat and generation. Requests run through the public Chrono endpoint.",
    resource_uri: "https://nyx.example/api/v1/proxy/s/chrono-llm-public",
    is_active: true,
    credential_source: { type: "personal" },
  },
  {
    id: "aevatar",
    label: "Aevatar",
    slug: "aevatar",
    catalog_service_name: "Aevatar",
    catalog_service_description:
      "Connect agents to Aevatar workflows and capabilities. Use your approved connection through NyxID.",
    resource_uri: "https://nyx.example/api/v1/proxy/s/aevatar",
    is_active: true,
    credential_source: { type: "personal" },
  },
  {
    id: "ornn-api",
    label: "ornn-api",
    slug: "ornn-api",
    catalog_service_name: "ornn-api",
    catalog_service_description:
      "Use the Ornn API through your NyxID connection. Access is limited to the services you approve.",
    resource_uri: "https://nyx.example/api/v1/proxy/s/ornn-api",
    is_active: true,
    credential_source: { type: "personal" },
  },
  {
    id: "cma-ornn-api",
    label: "CMA Bot Father · ornn-api",
    slug: "cma-bot-father-ornn-api",
    catalog_service_name: "CMA Bot Father",
    catalog_service_description:
      "Connect CMA Bot Father to the Ornn API. Let the application use this specific service connection.",
    resource_uri: "https://nyx.example/api/v1/proxy/s/cma-bot-father-ornn-api",
    is_active: true,
    credential_source: { type: "personal" },
  },
  {
    id: "cma-chrono-llm",
    label: "CMA Bot Father · chrono-llm-public",
    slug: "cma-bot-father-chrono-llm-public",
    catalog_service_name: "CMA Bot Father",
    catalog_service_description:
      "Connect CMA Bot Father to shared Chrono models. Only this approved service route is included.",
    resource_uri:
      "https://nyx.example/api/v1/proxy/s/cma-bot-father-chrono-llm-public",
    is_active: true,
    credential_source: { type: "personal" },
  },
];

export const previewSearch = new URLSearchParams({
  response_type: "code",
  client_id: "preview-client",
  client_name: "Ornn Agent",
  redirect_uri: "https://app.example.com/callback",
  scope: "openid profile email offline_access",
  code_challenge: "preview-only",
  code_challenge_method: "S256",
  consent_request: "preview-only",
});
for (const service of previewServices) {
  previewSearch.append("preselect_service_ids", service.id);
}
