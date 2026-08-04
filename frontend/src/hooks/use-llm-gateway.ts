import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import type { LlmStatusResponse } from "@/types/api";

export function useLlmStatus() {
  return useQuery({
    queryKey: ["llm-status"],
    queryFn: async (): Promise<LlmStatusResponse> => {
      return api.get<LlmStatusResponse>("/llm/status");
    },
    staleTime: 30_000,
  });
}
