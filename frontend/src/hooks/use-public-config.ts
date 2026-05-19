import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import type { PublicConfig } from "@/types/api";

/**
 * `enabled` lets callers skip the fetch when the config is not needed
 * (e.g. telemetry-declined sessions that have no other reason to read
 * /public/config). Default `true` preserves behavior for existing
 * consumers (settings, auth-flow, MCP tabs).
 */
export function usePublicConfig(options?: { enabled?: boolean }) {
  return useQuery({
    queryKey: ["public-config"],
    queryFn: () => api.get<PublicConfig>("/public/config"),
    staleTime: Infinity,
    enabled: options?.enabled ?? true,
  });
}
