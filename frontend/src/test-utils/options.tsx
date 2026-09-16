import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { PropsWithChildren } from "react";
import type { OptionsResponse } from "@/types/options";

export function optionsResponse(url: string, overrides: Partial<OptionsResponse> = {}): OptionsResponse {
  const params = new URL(url, "http://localhost").searchParams;
  return {
    option_set: "service-scope", principal_type: "service_account",
    owner_id: params.get("owner_id") ?? "owner",
    service_account_id: params.get("service_account_id"),
    items: [
      { value: "proxy", label: "All services", description: "Proxy all services", group: "Permissions", source: "backend_definition", owner_id: null, resource_id: null, disabled: false, disabled_reason: null },
      { value: "roles", label: "Role claims", description: "Include roles in userinfo", group: "Permissions", source: "backend_definition", owner_id: null, resource_id: null, disabled: false, disabled_reason: null },
    ], selected_items: [], total: 2, next_offset: null, version: "version-1",
    freshness: { definitions_version: "v1", resources: "live", evaluated_at: "2026-09-16T00:00:00Z", max_age_seconds: 0 },
    ...overrides,
  };
}

export function optionsWrapper() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  return function Wrapper({ children }: PropsWithChildren) {
    return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
  };
}
