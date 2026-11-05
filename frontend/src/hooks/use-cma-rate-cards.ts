import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import {
  cmaRateCardSchema,
  type PublishCmaRateCard,
} from "@/schemas/cma-rate-cards";
const key = ["admin", "billing", "cma-rate-cards"] as const;
export function useCmaRateCards() {
  return useQuery({
    queryKey: key,
    queryFn: async () =>
      cmaRateCardSchema
        .array()
        .parse(await api.get("/admin/billing/cma-rate-cards")),
  });
}
export function usePublishCmaRateCard() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: async (input: PublishCmaRateCard) =>
      cmaRateCardSchema.parse(
        await api.post("/admin/billing/cma-rate-cards", input),
      ),
    onSuccess: () => client.invalidateQueries({ queryKey: key }),
  });
}
