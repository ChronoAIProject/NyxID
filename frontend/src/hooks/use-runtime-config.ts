import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import {
  runtimeConfigSchema,
  type RuntimeConfig,
} from "@/schemas/runtime-config";

export function useRuntimeConfig() {
  return useQuery({
    queryKey: ["runtime-config"],
    queryFn: async (): Promise<RuntimeConfig> => {
      const response = await api.get<unknown>("/runtime-config");
      return runtimeConfigSchema.parse(response);
    },
    staleTime: Infinity,
  });
}
