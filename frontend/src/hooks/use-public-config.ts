import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import type { PublicConfig } from "@/types/api";

export function usePublicConfig() {
  return useQuery({
    queryKey: ["public-config"],
    queryFn: () => api.get<PublicConfig>("/public/config"),
    staleTime: Infinity,
  });
}
