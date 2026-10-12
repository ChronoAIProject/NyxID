import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import {
  billingConfigSchema,
  billingOverviewSchema,
  billingRateSchema,
  billingServiceSchema,
  resyncResponseSchema,
  type BillingConfigPatch,
} from "@/schemas/admin-billing";
import type { BillingLaneFormData } from "@/components/services/billing-lane-fields";

const adminBillingKey = ["admin", "billing"] as const;

export function useAdminBillingOverview() {
  return useQuery({
    queryKey: [...adminBillingKey, "overview"],
    queryFn: async () =>
      billingOverviewSchema.parse(await api.get("/admin/billing/overview")),
  });
}

export function useAdminBillingRates() {
  return useQuery({
    queryKey: [...adminBillingKey, "rates"],
    queryFn: async () =>
      billingRateSchema.array().parse(await api.get("/admin/billing/rates")),
  });
}

export function useAdminBillingConfig() {
  return useQuery({
    queryKey: [...adminBillingKey, "config"],
    queryFn: async () =>
      billingConfigSchema.parse(await api.get("/admin/billing/config")),
  });
}

/** Server-resolved effective billing for one catalog service. */
export function useAdminBillingService(serviceId: string) {
  return useQuery({
    queryKey: [...adminBillingKey, "service", serviceId],
    queryFn: async () =>
      billingServiceSchema.parse(
        await api.get(
          `/admin/billing/services/${encodeURIComponent(serviceId)}`,
        ),
      ),
    enabled: serviceId.length > 0,
  });
}

export function useUpdateAdminBillingConfig() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (patch: BillingConfigPatch) =>
      billingConfigSchema.parse(await api.put("/admin/billing/config", patch)),
    onSuccess: () =>
      queryClient.invalidateQueries({ queryKey: adminBillingKey }),
  });
}

export function useResyncAdminBillingService() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (serviceId: string) =>
      resyncResponseSchema.parse(
        await api.post(
          `/admin/billing/services/${encodeURIComponent(serviceId)}/resync`,
          {},
        ),
      ),
    onSuccess: () =>
      queryClient.invalidateQueries({ queryKey: adminBillingKey }),
  });
}

/** Billing-only lane write; shares the service editor's server write path. */
export type ServiceLaneBillingPatch = {
  readonly byok_pricing: BillingLaneFormData["byok_pricing"] | null;
  readonly platform_key_pricing:
    | BillingLaneFormData["platform_key_pricing"]
    | null;
};

export function useUpdateAdminBillingService() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async ({
      serviceId,
      billing,
    }: {
      readonly serviceId: string;
      readonly billing: Partial<ServiceLaneBillingPatch>;
    }) =>
      billingServiceSchema.parse(
        await api.put(
          `/admin/billing/services/${encodeURIComponent(serviceId)}`,
          { billing },
        ),
      ),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: adminBillingKey });
      void queryClient.invalidateQueries({ queryKey: ["services"] });
    },
  });
}
