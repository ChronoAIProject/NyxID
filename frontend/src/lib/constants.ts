import type { DownstreamService } from "@/types/api";

export const AUTH_TYPE_LABELS: Readonly<Record<string, string>> = {
  api_key: "API Key",
  oauth2: "OAuth 2.0",
  basic: "Basic Auth",
  bearer: "Bearer Token",
  oidc: "OIDC / SSO",
  header: "API Key",
  query: "Query Param",
};

export function getAuthTypeLabel(service: DownstreamService): string {
  const key = service.auth_type ?? service.auth_method;
  return AUTH_TYPE_LABELS[key] ?? key;
}

export function isOidcService(service: DownstreamService): boolean {
  return (
    service.auth_method === "oidc" ||
    service.auth_type === "oidc" ||
    service.oauth_client_id !== null
  );
}
