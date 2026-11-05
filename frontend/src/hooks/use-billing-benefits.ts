import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import { useAuthStore } from "@/stores/auth-store";
import { billingBenefitsSchema } from "@/schemas/billing-credits";

export function useBillingBenefits(page: number) {
  const userId = useAuthStore((s) => s.user?.id);
  return useQuery({
    queryKey: ["billing", "credits", "benefits", userId, page],
    queryFn: async () =>
      billingBenefitsSchema.parse(
        await api.get(`/billing/benefits?page=${page}`),
      ),
    placeholderData: (previous, query) =>
      query?.queryKey[3] === userId ? previous : undefined,
    retry: false,
    refetchInterval: 30_000,
    refetchIntervalInBackground: false,
  });
}
