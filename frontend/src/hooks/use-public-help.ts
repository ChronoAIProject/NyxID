import { useQuery } from "@tanstack/react-query";
import { apiClient } from "@/lib/api-client";
import { publicUseCasesSchema } from "@/schemas/public-help";

export function usePublicUseCases(enabled = true) {
  return useQuery({
    queryKey: ["public-help", "use-cases"],
    queryFn: async ({ signal }) =>
      publicUseCasesSchema.parse(
        await apiClient("/public/help/use-cases", {
          signal,
          credentials: "omit",
        }),
      ).use_cases,
    enabled,
    staleTime: 60_000,
  });
}
