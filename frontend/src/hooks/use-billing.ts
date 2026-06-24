import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import {
  billingUsageResponseSchema,
  billingWalletResponseSchema,
  type BillingUsageResponse,
  type BillingWalletResponse,
} from "@/schemas/billing";

export const BILLING_QUERY_KEY = ["billing"] as const;

export function useBillingWallet() {
  return useQuery({
    queryKey: [...BILLING_QUERY_KEY, "wallet"],
    queryFn: async (): Promise<BillingWalletResponse> => {
      return billingWalletResponseSchema.parse(
        await api.get<unknown>("/billing/wallet"),
      );
    },
  });
}

export function useBillingUsage(period: string) {
  return useQuery({
    queryKey: [...BILLING_QUERY_KEY, "usage", period],
    queryFn: async (): Promise<BillingUsageResponse> => {
      return billingUsageResponseSchema.parse(
        await api.get<unknown>(
          `/usage?period=${encodeURIComponent(period)}`,
        ),
      );
    },
  });
}
