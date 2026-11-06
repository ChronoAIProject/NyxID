import { useQuery } from "@tanstack/react-query";
import { billingOwnerPath } from "@/hooks/use-billing";
import { api } from "@/lib/api-client";
import { useAuthStore } from "@/stores/auth-store";
import { billingBenefitsSchema } from "@/schemas/billing-credits";

export function useBillingBenefits(page: number, ownerId?: string) {
  const userId = useAuthStore((s) => s.user?.id);
  return useQuery({
    queryKey: ["billing", "credits", "benefits", userId, ownerId, page],
    queryFn: async () =>
      billingBenefitsSchema.parse(
        await api.get(
          billingOwnerPath(`/billing/benefits?page=${page}`, ownerId),
        ),
      ),
    placeholderData: (previous, query) =>
      query?.queryKey[3] === userId && query?.queryKey[4] === ownerId
        ? previous
        : undefined,
    retry: false,
    refetchInterval: 30_000,
    refetchIntervalInBackground: false,
  });
}
