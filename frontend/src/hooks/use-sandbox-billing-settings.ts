import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import {
  sandboxBillingSettingsSchema,
  type SandboxBillingSettingsUpdate,
} from "@/schemas/sandbox-billing-settings";

const key = (serviceId: string) =>
  ["admin", "billing", "sandbox", serviceId] as const;
const path = (serviceId: string) =>
  `/admin/billing/services/${encodeURIComponent(serviceId)}/sandbox`;

export function useSandboxBillingSettings(serviceId: string) {
  return useQuery({
    queryKey: key(serviceId),
    queryFn: async () =>
      sandboxBillingSettingsSchema.parse(await api.get(path(serviceId))),
    enabled: serviceId.length > 0,
  });
}

export function useUpdateSandboxBillingSettings(serviceId: string) {
  const client = useQueryClient();
  return useMutation({
    mutationFn: async (input: SandboxBillingSettingsUpdate) =>
      sandboxBillingSettingsSchema.parse(await api.put(path(serviceId), input)),
    onSuccess: (settings) => {
      client.setQueryData(key(serviceId), settings);
      void client.invalidateQueries({
        queryKey: ["admin", "service-accounts"],
      });
    },
  });
}
